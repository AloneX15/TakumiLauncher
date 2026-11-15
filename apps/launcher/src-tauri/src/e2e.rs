//! End-to-end check against the real Mojang / Fabric services: installs a
//! version into a scratch directory, starts the game with an offline profile
//! and stops it once it has demonstrably booted.
//!
//! Opens a game window and downloads several hundred MB, so it only runs on
//! request:
//!
//! ```text
//! TAKUMI_E2E_VERSION=1.21.1 TAKUMI_E2E_LOADER=fabric \
//!     cargo test -p takumi-launcher e2e -- --ignored --nocapture
//! ```

use crate::accounts::{takumi_offline_account, Accounts};
use crate::install;
use crate::launch::{self, LaunchSink};
use crate::session::TakumiSession;
use crate::state::AppState;
use download_manager::DownloadProgress;
use instance_manager::NewInstance;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use types::ModLoader;

/// Lines that only appear once the client is well into start-up.
const BOOT_MARKERS: &[&str] = &[
    "Sound engine started",
    "OpenAL initialized",
    "Created: ",
    "LWJGL Version",
];

struct TestSink {
    booted: mpsc::UnboundedSender<String>,
    last_stage: Mutex<String>,
}

impl LaunchSink for TestSink {
    fn status(&self, stage: &str, progress: Option<DownloadProgress>) {
        let mut last = self.last_stage.lock().unwrap();
        if *last != stage {
            println!("[stage] {stage} {progress:?}");
            *last = stage.to_string();
        }
    }

    fn log(&self, line: String) {
        println!("[game] {line}");
        if BOOT_MARKERS.iter().any(|marker| line.contains(marker)) {
            let _ = self.booted.send(line);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the game and opens a window"]
async fn e2e_install_and_launch() {
    let version = std::env::var("TAKUMI_E2E_VERSION").unwrap_or_else(|_| "1.21.1".into());
    let loader = match std::env::var("TAKUMI_E2E_LOADER").as_deref() {
        Ok("fabric") => ModLoader::Fabric,
        Ok("quilt") => ModLoader::Quilt,
        Ok("forge") => ModLoader::Forge,
        Ok("neoforge") => ModLoader::NeoForge,
        _ => ModLoader::Vanilla,
    };
    let root = std::env::var("TAKUMI_E2E_ROOT")
        .map(Into::into)
        .unwrap_or_else(|_| std::env::temp_dir().join("takumi-launcher-e2e"));
    println!("root: {}", root.display());

    let state = AppState::new(root).unwrap();
    let mut accounts = Accounts::default();
    // The offline profile comes from the Takumi account (session.json in this root).
    TakumiSession {
        username: "TakumiTest".into(),
        email: String::new(),
        role: "user".into(),
    }
    .save(&state.root)
    .unwrap();
    accounts.upsert(takumi_offline_account("TakumiTest"));
    accounts.save(&state.root).unwrap();

    let available = install::loader_versions(&state, loader, &version).await.unwrap();
    let mod_loader_version = install::default_loader_version(&available).map(|v| v.version.clone());
    println!("loader version: {mod_loader_version:?}");
    let instance = state
        .instances
        .create(NewInstance {
            name: format!("e2e {version} {loader}"),
            minecraft_version: version,
            mod_loader: loader,
            mod_loader_version,
        })
        .unwrap();

    let (booted_tx, mut booted_rx) = mpsc::unbounded_channel();
    let sink = TestSink {
        booted: booted_tx,
        last_stage: Mutex::new(String::new()),
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();

    let run = launch::run(&state, instance.id, stop_rx, &sink);
    tokio::pin!(run);

    let marker = tokio::select! {
        result = &mut run => panic!("the game exited before booting: {result:?}"),
        marker = booted_rx.recv() => marker.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(1200)) => panic!("timed out waiting for the game to boot"),
    };
    println!("booted: {marker}");

    stop_tx.send(()).unwrap();
    let code = run.await.unwrap();
    assert_eq!(code, None, "a stopped game reports no exit code");
    assert!(state.instances.get(instance.id).unwrap().last_played.is_some());

    state.instances.delete(instance.id).unwrap();
}

/// Content management and modpack import against the real Modrinth API.
/// Downloads a few mods and one small modpack; does not start the game.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to Modrinth and downloads files"]
async fn e2e_content_and_modpack() {
    use crate::content::{self, ContentKind};
    use crate::modpack;

    let root = std::env::temp_dir().join("takumi-launcher-e2e-content");
    let _ = std::fs::remove_dir_all(&root);
    let state = AppState::new(root.clone()).unwrap();
    let instance = state
        .instances
        .create(NewInstance {
            name: "content".into(),
            minecraft_version: "1.21.1".into(),
            mod_loader: ModLoader::Fabric,
            mod_loader_version: Some("0.16.9".into()),
        })
        .unwrap();

    // Search is filtered to what the instance can use.
    let found = content::search_for_instance(state.http(), &instance, ContentKind::Mod, "iris shaders", 0)
        .await
        .unwrap();
    let iris = found
        .hits
        .iter()
        .find(|hit| hit.title.contains("Iris"))
        .expect("Iris in results");

    // Iris requires Sodium: both must be installed.
    let installed = content::install_project(&state, &instance, ContentKind::Mod, &iris.project_id)
        .await
        .unwrap();
    println!("installed: {installed:?}");
    assert!(installed.iter().any(|t| t.contains("Iris")) && installed.iter().any(|t| t.contains("Sodium")));
    assert!(
        content::install_project(&state, &instance, ContentKind::Mod, &iris.project_id)
            .await
            .is_err()
    );

    let items = content::list(&state, &instance).unwrap();
    assert_eq!(items.len(), installed.len());
    assert!(items.iter().all(|i| i.enabled && i.title.is_some() && i.size > 0));

    // Freshly installed files have no updates.
    assert!(content::check_updates(&state, &instance).await.unwrap().is_empty());

    // Disable, re-enable and remove.
    let file = items[0].filename.clone();
    content::set_enabled(&state, &instance, ContentKind::Mod, &file, false).unwrap();
    let disabled = content::list(&state, &instance).unwrap();
    let entry = disabled.iter().find(|i| !i.enabled).expect("one disabled item");
    assert!(entry.filename.ends_with(".disabled") && entry.title.is_some());
    content::set_enabled(&state, &instance, ContentKind::Mod, &entry.filename, true).unwrap();
    content::remove(&state, &instance, ContentKind::Mod, &file).unwrap();
    assert_eq!(content::list(&state, &instance).unwrap().len(), installed.len() - 1);
    assert!(content::remove(&state, &instance, ContentKind::Mod, "../instance.json").is_err());

    // An outdated file is detected by hash and replaced.
    let sodium_versions = content::project_versions(state.http(), "sodium", &["fabric"], Some("1.21.1"))
        .await
        .unwrap();
    assert!(sodium_versions.len() > 1, "need an older Sodium to test updates");
    let old = sodium_versions.last().unwrap().primary_file().unwrap().clone();
    let mods = state.instances.game_dir(instance.id).join("mods");
    for item in content::list(&state, &instance).unwrap() {
        content::remove(&state, &instance, ContentKind::Mod, &item.filename).unwrap();
    }
    state
        .downloader
        .download(&download_manager::DownloadJob::new(&old.url, mods.join(&old.filename)))
        .await
        .unwrap();
    let updates = content::check_updates(&state, &instance).await.unwrap();
    println!("updates: {updates:?}");
    assert_eq!(updates.len(), 1);
    content::update(&state, &instance, ContentKind::Mod, &old.filename)
        .await
        .unwrap();
    let after = content::list(&state, &instance).unwrap();
    assert_eq!(after.len(), 1);
    assert_ne!(after[0].filename, old.filename);
    assert!(content::check_updates(&state, &instance).await.unwrap().is_empty());

    // Modpack from Modrinth: instance with the pack's loader and its mods.
    let pack = modpack::install_from_modrinth(&state, "fabulously-optimized", &|_| {})
        .await
        .unwrap();
    println!(
        "modpack: {} · {} · {} {:?}",
        pack.name, pack.minecraft_version, pack.mod_loader, pack.mod_loader_version
    );
    assert_eq!(pack.mod_loader, ModLoader::Fabric);
    assert!(pack.mod_loader_version.is_some());
    let pack_items = content::list(&state, &pack).unwrap();
    println!("modpack files: {}", pack_items.len());
    assert!(pack_items.len() > 10);
    assert!(
        state.instances.game_dir(pack.id).join("config").is_dir(),
        "overrides extracted"
    );

    std::fs::remove_dir_all(root).unwrap();
}

/// Full private-instance run against a TakumiWeb with a published Fabric instance (Fabric API + Takumi Gate in
/// `mods/`) whose server address points at a running Takumi Gate server in offline mode:
///
/// ```text
/// TAKUMI_WEB_URL=http://localhost:5180 TAKUMI_PRIVATE_ID=<uuid> TAKUMI_PRIVATE_TOKEN=<member launcher token> \
///     cargo test -p takumi-launcher e2e_private -- --ignored --nocapture
/// ```
///
/// Checks: sync into the vault, launch with a join ticket, the Gate server lets the player in, `mods/` and
/// `.fabric/processedMods/` cannot be read while the game runs (and the game keeps working), "Reiniciar y
/// actualizar" closes the game and starts it again, and nothing decrypted is left once it stops.
/// Uses its own keyring entry, never the user's session.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs TakumiWeb, a Takumi Gate server, downloads the game and opens a window"]
async fn e2e_private_instance() {
    use crate::private_runtime as runtime;

    let id: uuid::Uuid = std::env::var("TAKUMI_PRIVATE_ID").unwrap().parse().unwrap();
    let token = std::env::var("TAKUMI_PRIVATE_TOKEN").unwrap();
    let root: std::path::PathBuf = std::env::var("TAKUMI_E2E_ROOT")
        .map(Into::into)
        .unwrap_or_else(|_| std::env::temp_dir().join("takumi-launcher-e2e"));
    let service = format!("TakumiLauncher-e2e-{}", std::process::id());
    let mut state = AppState::new(root).unwrap();
    state.vault = auth_system::TokenVault::new(&service);
    state.vault.store("takumi-session", &token).unwrap();
    // Removes the test token from the keyring even if the test fails.
    struct ForgetToken(String);
    impl Drop for ForgetToken {
        fn drop(&mut self) {
            let _ = auth_system::TokenVault::new(&self.0).delete("takumi-session");
        }
    }
    let _forget = ForgetToken(service.clone());
    let mut accounts = Accounts::default();
    // The offline profile comes from the Takumi account (session.json in this root).
    TakumiSession {
        username: "TakumiTest".into(),
        email: String::new(),
        role: "user".into(),
    }
    .save(&state.root)
    .unwrap();
    accounts.upsert(takumi_offline_account("TakumiTest"));
    accounts.save(&state.root).unwrap();

    {
        crate::private_instances::sync_with_token(&state, id, &token, &PrintSink)
            .await
            .unwrap();
        let game = state.instances.game_dir(id);

        let (lines_tx, mut lines) = mpsc::unbounded_channel();
        let sink = LineSink(lines_tx);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let run = launch::run(&state, id, stop_rx, &sink);
        tokio::pin!(run);

        async fn wait_for(
            lines: &mut mpsc::UnboundedReceiver<String>,
            what: &str,
            run: &mut std::pin::Pin<&mut impl std::future::Future<Output = anyhow::Result<Option<i32>>>>,
        ) {
            tokio::select! {
                result = run.as_mut() => panic!("the game stopped while waiting for '{what}': {result:?}"),
                _ = async { while let Some(line) = lines.recv().await { if line.contains(what) { break; } } } => {}
                _ = tokio::time::sleep(Duration::from_secs(900)) => panic!("timed out waiting for '{what}'"),
            }
            println!(">>> {what}");
        }

        wait_for(&mut lines, "Carpeta de mods bloqueada", &mut run).await;
        let jar = std::fs::read_dir(state.instances.instance_dir(id).join("vault/objects"))
            .unwrap()
            .count();
        assert!(jar > 0, "event files are in the vault");
        assert!(
            std::fs::read_dir(game.join("mods")).is_err(),
            "mods/ cannot be listed while playing"
        );
        assert!(
            std::fs::read_dir(game.join(".fabric/processedMods")).is_err(),
            "processedMods cannot be listed while playing"
        );

        // The Takumi Gate server accepts the ticket the launcher asked for, and the game loads the world with
        // the folders locked.
        // Quick play connects to the event server; the Gate checks the ticket during login (the server log shows
        // "TakumiTest joined the game"). The game must keep running in the world with the folders locked.
        wait_for(&mut lines, "Connecting to localhost", &mut run).await;
        tokio::select! {
            result = run.as_mut() => panic!("the game stopped in the world: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(45)) => {}
        }

        // "Reiniciar y actualizar": the mod closes the game and the launcher starts it again.
        runtime::request_restart(&game).unwrap();
        wait_for(&mut lines, "Reiniciando para aplicar", &mut run).await;
        wait_for(&mut lines, "Carpeta de mods bloqueada", &mut run).await;

        stop_tx.send(()).unwrap();
        assert_eq!(run.await.unwrap(), None);
        let left: Vec<_> = std::fs::read_dir(game.join("mods"))
            .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "decrypted mods left behind: {left:?}");
        assert!(
            !game.join(".fabric/processedMods").exists(),
            "extracted jars left behind"
        );
    }
}

struct PrintSink;

impl LaunchSink for PrintSink {
    fn status(&self, _stage: &str, _progress: Option<DownloadProgress>) {}
    fn log(&self, line: String) {
        println!("[sync] {line}");
    }
}

struct LineSink(mpsc::UnboundedSender<String>);

impl LaunchSink for LineSink {
    fn status(&self, _stage: &str, _progress: Option<DownloadProgress>) {}
    fn log(&self, line: String) {
        if line.contains("[Takumi]") || line.contains("Connecting to") || line.contains("ERROR") {
            println!("[game] {line}");
        }
        let _ = self.0.send(line);
    }
}

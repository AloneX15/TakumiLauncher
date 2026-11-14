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

use crate::accounts::{offline_account, Accounts};
use crate::install::loader_kind;
use crate::launch::{self, LaunchSink};
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
        _ => ModLoader::Vanilla,
    };
    let root = std::env::var("TAKUMI_E2E_ROOT")
        .map(Into::into)
        .unwrap_or_else(|_| std::env::temp_dir().join("takumi-launcher-e2e"));
    println!("root: {}", root.display());

    let state = AppState::new(root).unwrap();
    let mut accounts = Accounts::default();
    accounts.upsert(offline_account("TakumiTest").unwrap());
    accounts.save(&state.root).unwrap();

    let mod_loader_version = match loader_kind(loader).unwrap() {
        Some(kind) => {
            let versions = kind.versions(state.http(), &version).await.unwrap();
            let chosen = versions.iter().find(|v| v.stable).or(versions.first()).unwrap();
            Some(chosen.version.clone())
        }
        None => None,
    };
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

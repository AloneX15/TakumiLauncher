//! Private instances: event instances whose game folder is managed from the Takumi website
//! (`/instancias` in takumi-webpage) and that only invited members can play.
//!
//! - The list comes from `GET /api/launcher/instances` (members only).
//! - Before every launch the launcher downloads the signed manifest of the latest published version,
//!   verifies its Ed25519 signature with the key it was built with, and makes the game folder match it:
//!   missing or modified files are downloaded again (SHA-256 checked), files the organizer removed are
//!   deleted, and anything a player added to a locked folder (`mods/` by default) is moved out to
//!   `instances/<id>/quarantine/`. There is no way to play an outdated version: launching always
//!   updates first, and without a connection a private instance cannot start.
//! - The local instance id is the website's instance id.
//! - Mods, resource packs and shader packs are stored encrypted (see `private_runtime`): they only exist
//!   in the game folder while the game runs.

use crate::commands::{fail, AppStateRef, CmdResult};
use crate::install;
use crate::launch::LaunchSink;
use crate::private_runtime::{self as runtime, is_vaulted};
use crate::session::{self, api_error, api_token, endpoint, unreachable, TIMEOUT};
use crate::state::{write_atomic, AppState};
use anyhow::{anyhow, bail, Context};
use download_manager::reqwest::StatusCode;
use download_manager::DownloadJob;
use instance_manager::{InstanceError, ManagedSource, NewInstance};
use manifest_system::vault::VaultKey;
use manifest_system::{FilePolicy, ManifestFile, PrivateManifest};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use types::{Instance, ModLoader};
use uuid::Uuid;

/// Raw Ed25519 public key (base64) that verifies manifests, set for release builds
/// (`TAKUMI_MANIFEST_PUBKEY`, printed by the website's `GET /api/launcher/manifest-key`).
const BUILD_PUBKEY: Option<&str> = option_env!("TAKUMI_MANIFEST_PUBKEY");
const STATE_FILE: &str = "private-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInstance {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub server: Option<String>,
    /// Published version; 0 = nothing published yet.
    pub version: u32,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct PrivateInstanceView {
    #[serde(flatten)]
    pub remote: RemoteInstance,
    /// Version on disk, `None` when it was never downloaded.
    pub installed_version: Option<u32>,
    /// A newer version is published: it is downloaded before the next launch.
    pub update_required: bool,
}

#[derive(Deserialize)]
struct InstancesResponse {
    instances: Vec<RemoteInstance>,
}

#[derive(Deserialize)]
struct ManifestResponse {
    manifest: String,
    signature: String,
}

#[derive(Deserialize)]
struct AcceptResponse {
    instance_id: Uuid,
}

#[derive(Deserialize)]
struct KeyResponse {
    public_key: String,
}

/// Files the launcher placed in the game folder, to remove those the organizer later deletes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct SyncState {
    pub version: u32,
    pub files: HashMap<String, FilePolicy>,
    /// SHA-256 of each file, to tell which changed between two versions.
    #[serde(default)]
    pub hashes: HashMap<String, String>,
}

/// Result of a sync: what the game needs to start.
pub struct Synced {
    pub instance: Instance,
    pub manifest: PrivateManifest,
    pub key: Arc<VaultKey>,
}

// ------------------------------------------------------------------ API

async fn list_remote(state: &AppState) -> anyhow::Result<Vec<RemoteInstance>> {
    let token = api_token(state)?;
    let response = state
        .http()
        .get(endpoint("/api/launcher/instances"))
        .bearer_auth(&token)
        .timeout(TIMEOUT)
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: InstancesResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    Ok(body.instances)
}

/// Accepts an invite link (`https://…/i/<token>`) or a bare token.
async fn join(state: &AppState, link: &str) -> anyhow::Result<Uuid> {
    let token = invite_token(link).ok_or_else(|| anyhow!("Pega el enlace de invitación completo."))?;
    let api = api_token(state)?;
    let response = state
        .http()
        .post(endpoint(&format!("/api/launcher/invites/{token}/accept")))
        .bearer_auth(&api)
        .timeout(TIMEOUT)
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: AcceptResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    Ok(body.instance_id)
}

fn invite_token(link: &str) -> Option<String> {
    let link = link.trim().trim_end_matches('/');
    let token = match link.rfind("/i/") {
        Some(at) => &link[at + 3..],
        None => link,
    };
    let token = token.split(['?', '#']).next().unwrap_or("");
    let valid = (16..=100).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then(|| token.to_string())
}

async fn public_key(state: &AppState) -> anyhow::Result<String> {
    if let Some(key) = BUILD_PUBKEY.filter(|key| !key.trim().is_empty()) {
        return Ok(key.trim().to_string());
    }
    // Development builds talk to a local website and take its key.
    if session::is_local_web() {
        let response = state
            .http()
            .get(endpoint("/api/launcher/manifest-key"))
            .timeout(TIMEOUT)
            .send()
            .await
            .context(unreachable())?;
        if !response.status().is_success() {
            return Err(api_error(response).await);
        }
        let body: KeyResponse = response.json().await.context("Respuesta del servidor no válida.")?;
        return Ok(body.public_key);
    }
    bail!("Esta versión del launcher no puede verificar instancias privadas (falta TAKUMI_MANIFEST_PUBKEY).")
}

pub(crate) async fn fetch_manifest(state: &AppState, id: Uuid, token: &str) -> anyhow::Result<PrivateManifest> {
    let response = state
        .http()
        .get(endpoint(&format!("/api/launcher/instances/{id}/manifest")))
        .bearer_auth(token)
        .timeout(TIMEOUT)
        .send()
        .await
        .context("Para jugar a una instancia privada hace falta conexión: el launcher comprueba que tienes la última versión.")?;
    if response.status() == StatusCode::UNAUTHORIZED {
        bail!("La sesión de Takumi ha caducado. Vuelve a iniciar sesión.");
    }
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: ManifestResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    let key = public_key(state).await?;
    let manifest = manifest_system::verify_and_parse(&body.manifest, &body.signature, &key)?;
    if manifest.instance != id.to_string() {
        bail!("El manifiesto recibido es de otra instancia.");
    }
    Ok(manifest)
}

// ----------------------------------------------------------------- sync

/// Bring the local copy of a private instance up to the latest published version and remove anything
/// the player added to its locked folders. Creates the instance on first use.
pub async fn sync(state: &AppState, id: Uuid, sink: &dyn LaunchSink) -> anyhow::Result<Synced> {
    sink.status("sync", None);
    let token = api_token(state)?;
    sync_with_token(state, id, &token, sink).await
}

pub(crate) async fn sync_with_token(
    state: &AppState,
    id: Uuid,
    token: &str,
    sink: &dyn LaunchSink,
) -> anyhow::Result<Synced> {
    let manifest = fetch_manifest(state, id, token).await?;
    let loader: ModLoader = serde_json::from_value(serde_json::Value::String(manifest.loader.clone()))
        .map_err(|_| anyhow!("Loader desconocido: {}", manifest.loader))?;

    let existing = match state.instances.get(id) {
        Ok(instance) if instance.managed.is_none() => bail!("Ya existe una instancia normal con este identificador."),
        Ok(instance) => Some(instance),
        Err(InstanceError::NotFound) => None,
        Err(error) => return Err(error.into()),
    };

    // A pinned loader version wins; otherwise keep the one already installed for the same game version.
    let loader_version = match (&manifest.loader_version, &existing) {
        _ if loader == ModLoader::Vanilla => None,
        (Some(pinned), _) => Some(pinned.clone()),
        (None, Some(local))
            if local.mod_loader == loader
                && local.minecraft_version == manifest.minecraft
                && local.mod_loader_version.is_some() =>
        {
            local.mod_loader_version.clone()
        }
        (None, _) => {
            let available = install::loader_versions(state, loader, &manifest.minecraft).await?;
            let chosen = install::default_loader_version(&available)
                .ok_or_else(|| anyhow!("{loader} no está disponible para Minecraft {}.", manifest.minecraft))?;
            Some(chosen.version.clone())
        }
    };

    let name: String = manifest.name.chars().take(64).collect();
    let mut instance = match existing {
        Some(instance) => instance,
        None => state.instances.create_with_id(
            id,
            NewInstance {
                name: name.clone(),
                minecraft_version: manifest.minecraft.clone(),
                mod_loader: loader,
                mod_loader_version: loader_version.clone(),
            },
            Some(ManagedSource::default()),
        )?,
    };

    let instance_dir = state.instances.instance_dir(id);
    let game_dir = state.instances.game_dir(id);
    let previous = load_state(&instance_dir);
    let key = runtime::fetch_vault_key(state, id, token).await?;
    runtime::protect(&instance_dir);
    // Decrypted files a crash left behind.
    runtime::scrub(&instance_dir, &game_dir);

    download_and_seal(state, &manifest.files, &instance_dir, &game_dir, id, token, &key, sink).await?;

    let removed = cleanup(&manifest, &previous, &game_dir, &instance_dir.join("quarantine"))?;
    if removed > 0 {
        sink.log(format!(
            "[Takumi] {removed} archivos que no forman parte de la instancia se han retirado."
        ));
    }

    let keep = manifest
        .files
        .iter()
        .filter(|f| is_vaulted(f))
        .map(|f| f.sha256.clone())
        .collect();
    runtime::gc_objects(&instance_dir, &keep);
    save_state(&instance_dir, &state_of(&manifest))?;

    instance.name = name;
    instance.minecraft_version = manifest.minecraft.clone();
    instance.mod_loader = loader;
    instance.mod_loader_version = loader_version;
    instance.managed = Some(ManagedSource {
        version: manifest.version,
        server: manifest.server.clone(),
        locked_dirs: manifest.locked_dirs.clone(),
    });
    state.instances.save(&mut instance)?;
    Ok(Synced {
        instance,
        manifest,
        key,
    })
}

pub(crate) fn state_of(manifest: &PrivateManifest) -> SyncState {
    SyncState {
        version: manifest.version,
        files: manifest.files.iter().map(|f| (f.path.clone(), f.policy)).collect(),
        hashes: manifest
            .files
            .iter()
            .map(|f| (f.path.clone(), f.sha256.clone()))
            .collect(),
    }
}

/// Downloads what is missing (SHA-256 checked) and encrypts event mods and packs into the vault.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_and_seal(
    state: &AppState,
    files: &[ManifestFile],
    instance_dir: &Path,
    game_dir: &Path,
    id: Uuid,
    token: &str,
    key: &Arc<VaultKey>,
    sink: &dyn LaunchSink,
) -> anyhow::Result<()> {
    let jobs = download_jobs(files, instance_dir, game_dir, id, token, key);
    state
        .downloader
        .download_all(jobs, |progress| sink.status("sync", Some(progress)))
        .await
        .context("No se pudieron descargar los archivos de la instancia")?;
    let (dir, key) = (instance_dir.to_path_buf(), key.clone());
    tokio::task::spawn_blocking(move || runtime::seal_incoming(&dir, &key)).await??;
    Ok(())
}

fn download_jobs(
    files: &[ManifestFile],
    instance_dir: &Path,
    game_dir: &Path,
    id: Uuid,
    token: &str,
    key: &VaultKey,
) -> Vec<DownloadJob> {
    files
        .iter()
        .filter_map(|file| {
            let destination = if is_vaulted(file) {
                if runtime::has_object(instance_dir, &file.sha256, key) {
                    return None;
                }
                runtime::incoming_dir(instance_dir).join(&file.sha256)
            } else {
                game_dir.join(&file.path)
            };
            // Player settings are only seeded: once present they belong to the player.
            if file.policy == FilePolicy::Default && destination.exists() {
                return None;
            }
            let url = endpoint(&format!("/api/launcher/instances/{id}/files/{}", file.sha256));
            Some(
                DownloadJob::new(url, destination)
                    .sha256(file.sha256.clone())
                    .size(Some(file.size))
                    .verify_existing(true)
                    .bearer(token),
            )
        })
        .collect()
}

/// Removes what the organizer deleted since the last sync, deletes plain copies of vaulted files (they
/// must only exist while the game runs) and quarantines anything else found in locked folders. Returns
/// how many files were taken out of the game folder, not counting those plain copies.
fn cleanup(
    manifest: &PrivateManifest,
    previous: &SyncState,
    game_dir: &Path,
    quarantine: &Path,
) -> anyhow::Result<usize> {
    // Windows and macOS folders are case-insensitive: compare lower-cased.
    let wanted: HashSet<String> = manifest.files.iter().map(|f| f.path.to_lowercase()).collect();
    let vaulted: HashSet<String> = manifest
        .files
        .iter()
        .filter(|f| is_vaulted(f))
        .map(|f| f.path.to_lowercase())
        .collect();
    let mut removed = 0;

    for file in manifest.files.iter().filter(|f| is_vaulted(f)) {
        let target = game_dir.join(&file.path);
        if target.is_file() {
            std::fs::remove_file(&target).with_context(|| format!("No se pudo retirar {}", file.path))?;
        }
    }

    for (path, policy) in &previous.files {
        if *policy == FilePolicy::Enforced
            && !wanted.contains(&path.to_lowercase())
            && manifest_system::is_safe_path(path)
        {
            let target = game_dir.join(path);
            if target.is_file() {
                std::fs::remove_file(&target).with_context(|| format!("No se pudo borrar {path}"))?;
                removed += 1;
            }
        }
    }

    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    for dir in &manifest.locked_dirs {
        let root = game_dir.join(dir);
        let mut extra = Vec::new();
        collect_files(&root, dir, &mut extra)?;
        for (relative, absolute) in extra {
            let lower = relative.to_lowercase();
            if vaulted.contains(&lower) {
                // A plain copy of an event file (same name, other case): never kept or quarantined.
                std::fs::remove_file(&absolute).with_context(|| format!("No se pudo retirar {relative}"))?;
                continue;
            }
            if wanted.contains(&lower) {
                continue;
            }
            let target = quarantine.join(&stamp).join(&relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Moving keeps the player's own files; fall back to deleting across drives.
            if std::fs::rename(&absolute, &target).is_err() {
                std::fs::remove_file(&absolute).with_context(|| format!("No se pudo retirar {relative}"))?;
            }
            removed += 1;
        }
    }
    Ok(removed)
}

/// Every file under `root` as (`prefix/relative/path`, absolute path). Symbolic links are listed as files
/// (and so removed) without following them.
fn collect_files(root: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = format!("{prefix}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&entry.path(), &relative, out)?;
        } else {
            out.push((relative, entry.path()));
        }
    }
    Ok(())
}

pub(crate) fn load_state(instance_dir: &Path) -> SyncState {
    std::fs::read_to_string(instance_dir.join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn save_state(instance_dir: &Path, sync: &SyncState) -> anyhow::Result<()> {
    write_atomic(&instance_dir.join(STATE_FILE), &serde_json::to_vec_pretty(sync)?)?;
    Ok(())
}

// ----------------------------------------------------------------- live

/// What a version published while the game runs needs.
pub(crate) enum LiveOutcome {
    /// Only resource packs changed and they are in place; `packs` are the pack ids (`file/<name>`) the
    /// game should enable and reload.
    Applied { packs: Vec<String> },
    /// Code, config or settings changed: the game must restart.
    NeedsRestart,
}

/// Applies a newly published version to a running game when it only touches `resourcepacks/`.
/// Returns the published version and what happened.
pub(crate) async fn live_apply(
    state: &AppState,
    id: Uuid,
    token: &str,
    key: &Arc<VaultKey>,
    mark: &str,
    running: &runtime::Materialized,
    sink: &dyn LaunchSink,
) -> anyhow::Result<(u32, LiveOutcome)> {
    let manifest = fetch_manifest(state, id, token).await?;
    let instance_dir = state.instances.instance_dir(id);
    let game_dir = state.instances.game_dir(id);
    let previous = load_state(&instance_dir);
    let mut instance = state.instances.get(id)?;

    let changed: Vec<ManifestFile> = manifest
        .files
        .iter()
        .filter(|f| previous.hashes.get(&f.path) != Some(&f.sha256))
        .cloned()
        .collect();
    let now: HashSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let dropped: Vec<String> = previous
        .files
        .keys()
        .filter(|p| !now.contains(p.as_str()))
        .cloned()
        .collect();
    let only_packs = changed
        .iter()
        .map(|f| f.path.as_str())
        .chain(dropped.iter().map(String::as_str))
        .all(|p| p.starts_with("resourcepacks/"));
    let same_game = instance.minecraft_version == manifest.minecraft
        && serde_json::to_value(instance.mod_loader).ok() == Some(serde_json::Value::String(manifest.loader.clone()))
        && manifest
            .loader_version
            .as_ref()
            .is_none_or(|v| Some(v) == instance.mod_loader_version.as_ref())
        && instance.managed.as_ref().map(|m| &m.locked_dirs) == Some(&manifest.locked_dirs);
    if previous.hashes.is_empty() || !only_packs || !same_game {
        return Ok((manifest.version, LiveOutcome::NeedsRestart));
    }

    download_and_seal(state, &changed, &instance_dir, &game_dir, id, token, key, sink).await?;
    let vaulted: Vec<&ManifestFile> = changed.iter().filter(|f| is_vaulted(f)).collect();
    running.track(&vaulted.iter().map(|f| f.path.clone()).collect::<Vec<_>>())?;
    match runtime::decrypt_into(&instance_dir, &game_dir, &vaulted, key, mark) {
        Ok(()) => {}
        // A pack with the same name is open in the game (Windows locks it): restart instead.
        Err(_) => return Ok((manifest.version, LiveOutcome::NeedsRestart)),
    }
    for path in &dropped {
        if manifest_system::is_safe_path(path) {
            // Still open in the game: removed when it exits (it is tracked as decrypted).
            let _ = std::fs::remove_file(game_dir.join(path));
        }
    }

    save_state(&instance_dir, &state_of(&manifest))?;
    if let Some(managed) = instance.managed.as_mut() {
        managed.version = manifest.version;
    }
    state.instances.save(&mut instance)?;

    let mut packs: Vec<String> = changed
        .iter()
        .filter_map(|f| f.path.split('/').nth(1))
        .map(|name| format!("file/{name}"))
        .collect();
    packs.sort();
    packs.dedup();
    Ok((manifest.version, LiveOutcome::Applied { packs }))
}

/// Content commands call this: private instances can only change from the website.
pub fn ensure_editable(instance: &Instance) -> anyhow::Result<()> {
    if instance.managed.is_some() {
        bail!("Esta instancia privada la gestiona el organizador desde la web de Takumi; no se puede modificar aquí.");
    }
    Ok(())
}

// ------------------------------------------------------------- commands

/// Private instances of the signed-in user, with what is installed locally.
#[tauri::command]
pub async fn list_private_instances(state: AppStateRef<'_>) -> CmdResult<Vec<PrivateInstanceView>> {
    let remote = list_remote(&state).await.map_err(fail)?;
    Ok(remote
        .into_iter()
        .map(|remote| {
            let installed = state
                .instances
                .get(remote.id)
                .ok()
                .and_then(|i| i.managed)
                .map(|m| m.version)
                .filter(|v| *v > 0);
            let update_required = remote.version > 0 && installed != Some(remote.version);
            PrivateInstanceView {
                remote,
                installed_version: installed,
                update_required,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn join_private_instance(state: AppStateRef<'_>, link: String) -> CmdResult<Uuid> {
    join(&state, &link).await.map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use download_manager::DownloadProgress;
    use manifest_system::ManifestFile;

    fn manifest(files: &[(&str, FilePolicy)], locked: &[&str]) -> PrivateManifest {
        PrivateManifest {
            format: 1,
            instance: Uuid::nil().to_string(),
            name: "Evento".into(),
            version: 1,
            minecraft: "1.21.1".into(),
            loader: "fabric".into(),
            loader_version: None,
            server: None,
            locked_dirs: locked.iter().map(|d| d.to_string()).collect(),
            restart_required: true,
            grace_minutes: 5,
            published_at: String::new(),
            files: files
                .iter()
                .map(|(path, policy)| ManifestFile {
                    path: path.to_string(),
                    sha256: "a".repeat(64),
                    size: 1,
                    policy: *policy,
                })
                .collect(),
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn parses_invite_links() {
        let token = "AbCdEfGhIjKlMnOpQrStUvWxYz012345";
        assert_eq!(
            invite_token(&format!("https://takumilabs.org/i/{token}")).as_deref(),
            Some(token)
        );
        assert_eq!(
            invite_token(&format!(" http://localhost:5173/i/{token}/ ")).as_deref(),
            Some(token)
        );
        assert_eq!(
            invite_token(&format!("https://x/i/{token}?ref=1")).as_deref(),
            Some(token)
        );
        assert_eq!(invite_token(token).as_deref(), Some(token));
        assert_eq!(invite_token("https://x/i/short"), None);
        assert_eq!(invite_token("https://x/i/bad token with spaces!!"), None);
    }

    #[test]
    fn cleanup_quarantines_extras_and_removes_dropped_files() {
        let root = std::env::temp_dir().join(format!("takumi-private-{}", Uuid::new_v4()));
        let game = root.join("minecraft");
        let quarantine = root.join("quarantine");
        for path in [
            "mods/event.jar",
            "mods/cheat.jar",
            "mods/sub/x.jar",
            "mods/event.jar.disabled",
            "config/old.json",
            "config/player.json",
            "options.txt",
            "saves/world/level.dat",
        ] {
            touch(&game.join(path));
        }
        let manifest = manifest(
            &[
                ("mods/event.jar", FilePolicy::Enforced),
                ("options.txt", FilePolicy::Default),
            ],
            &["mods"],
        );
        let previous = SyncState {
            version: 1,
            hashes: HashMap::new(),
            files: [
                ("mods/event.jar".to_string(), FilePolicy::Enforced),
                ("config/old.json".to_string(), FilePolicy::Enforced),
                ("options.txt".to_string(), FilePolicy::Default),
            ]
            .into(),
        };

        let removed = cleanup(&manifest, &previous, &game, &quarantine).unwrap();
        assert_eq!(removed, 4);
        // Vaulted: the plain copy is deleted (not quarantined); it only exists while the game runs.
        assert!(!game.join("mods/event.jar").exists());
        assert!(!game.join("mods/cheat.jar").exists());
        assert!(!game.join("mods/sub/x.jar").exists());
        assert!(!game.join("mods/event.jar.disabled").exists());
        // Dropped by the organizer: deleted. Not locked and never managed: left alone.
        assert!(!game.join("config/old.json").exists());
        assert!(game.join("config/player.json").is_file());
        assert!(game.join("options.txt").is_file());
        assert!(game.join("saves/world/level.dat").is_file());
        let kept: Vec<_> = std::fs::read_dir(&quarantine).unwrap().collect();
        assert_eq!(kept.len(), 1, "one quarantine folder per sync");
        let stamp = kept[0].as_ref().unwrap().path();
        assert!(stamp.join("mods/cheat.jar").is_file());
        assert!(
            !stamp.join("mods/event.jar").exists(),
            "event files never land in quarantine"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    struct PrintSink;

    impl LaunchSink for PrintSink {
        fn status(&self, stage: &str, progress: Option<DownloadProgress>) {
            println!("[stage] {stage} {progress:?}");
        }
        fn log(&self, line: String) {
            println!("[log] {line}");
        }
    }

    /// Syncs a published private instance from a running TakumiWeb and exercises the vault, the join
    /// ticket and the status endpoint:
    ///
    /// ```text
    /// TAKUMI_WEB_URL=http://localhost:5173 TAKUMI_PRIVATE_ID=<uuid> TAKUMI_PRIVATE_TOKEN=<launcher token> \
    ///     cargo test -p takumi-launcher private_sync -- --ignored --nocapture
    /// ```
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a running TakumiWeb with a published private instance"]
    async fn private_sync_against_web() {
        let id: Uuid = std::env::var("TAKUMI_PRIVATE_ID").unwrap().parse().unwrap();
        let token = std::env::var("TAKUMI_PRIVATE_TOKEN").unwrap();
        let root = std::env::temp_dir().join(format!("takumi-private-e2e-{}", Uuid::new_v4()));
        let state = AppState::new(root.clone()).unwrap();

        let synced = sync_with_token(&state, id, &token, &PrintSink).await.unwrap();
        let managed = synced.instance.managed.clone().unwrap();
        println!("instance {} v{} {:?}", synced.instance.name, managed.version, managed);
        let instance_dir = state.instances.instance_dir(id);
        let game = state.instances.game_dir(id);
        let saved = load_state(&instance_dir);
        assert_eq!(saved.version, managed.version);
        for file in &synced.manifest.files {
            if is_vaulted(file) {
                assert!(!game.join(&file.path).exists(), "{} is plain on disk", file.path);
                assert!(
                    runtime::has_object(&instance_dir, &file.sha256, &synced.key),
                    "{} not in the vault",
                    file.path
                );
            } else {
                assert!(game.join(&file.path).is_file(), "{} missing", file.path);
            }
        }

        // A player drops a mod in and leaves a plain copy of an event mod: the next sync removes both.
        touch(&game.join("mods/cheat.jar"));
        let event_mod = synced
            .manifest
            .files
            .iter()
            .find(|f| is_vaulted(f) && f.path.starts_with("mods/"));
        if let Some(file) = event_mod {
            touch(&game.join(&file.path));
        }
        let synced = sync_with_token(&state, id, &token, &PrintSink).await.unwrap();
        assert!(!game.join("mods/cheat.jar").exists());
        if let Some(file) = event_mod {
            assert!(!game.join(&file.path).exists());

            // A tampered vault object fails to decrypt, is dropped and comes back after a new download.
            let object = runtime::object_path(&instance_dir, &file.sha256);
            let mut bytes = std::fs::read(&object).unwrap();
            let middle = bytes.len() / 2;
            bytes[middle] ^= 0xff;
            std::fs::write(&object, bytes).unwrap();
            let mark = runtime::watermark("e2e", "Evento", id);
            match runtime::materialize(&instance_dir, &game, &synced.manifest.files, &synced.key, &mark) {
                Err(runtime::MaterializeError::Corrupt(list)) => assert_eq!(list, vec![file.sha256.clone()]),
                _ => panic!("tampering was not detected"),
            }
            download_and_seal(
                &state,
                &synced.manifest.files,
                &instance_dir,
                &game,
                id,
                &token,
                &synced.key,
                &PrintSink,
            )
            .await
            .unwrap();
            let running =
                runtime::materialize(&instance_dir, &game, &synced.manifest.files, &synced.key, &mark).unwrap();
            assert!(game.join(&file.path).is_file());
            if file.path.ends_with(".jar") {
                let archive = zip::ZipArchive::new(std::fs::File::open(game.join(&file.path)).unwrap()).unwrap();
                assert!(String::from_utf8_lossy(archive.comment()).contains("copia de e2e"));
            }
            drop(running);
            assert!(!game.join(&file.path).exists(), "decrypted files leave with the game");
        }
        assert!(ensure_editable(&state.instances.get(id).unwrap()).is_err());

        // Join ticket and live status.
        let ticket = runtime::request_ticket(
            &state,
            id,
            &token,
            "0123456789abcdef0123456789abcdef",
            "Steve",
            managed.version,
        )
        .await
        .unwrap();
        assert!(ticket.starts_with("tkt_"));
        println!("ticket {ticket}");
        let status = runtime::fetch_status(&state, id, &token, 0).await.unwrap();
        assert_eq!(status.version, managed.version);
        let current = runtime::fetch_status(&state, id, &token, managed.version)
            .await
            .unwrap();
        assert!(!current.restart_required);

        // A wrong token is refused.
        assert!(sync_with_token(&state, id, "not-a-token", &PrintSink).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

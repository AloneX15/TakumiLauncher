//! What happens around the game process of a private instance (phases 2–4):
//!
//! - **Vault:** event mods, resource packs and shader packs are stored only encrypted, in
//!   `instances/<id>/vault/objects/<sha256>.tkv`, with a per-member key the website hands out
//!   (`/vault-key`). They are decrypted into the game folder right before the game starts
//!   ([`materialize`]) and deleted again when it exits ([`Materialized`] / [`scrub`]). Each decrypted
//!   `.jar`/`.zip` carries the player's name in its zip comment, so a leaked copy says whose it was.
//! - **Hidden folder:** the instance folder is marked hidden + system ([`protect`]); %APPDATA% is already
//!   private to the Windows user.
//! - **Takumi Gate:** a join ticket for the Minecraft account is requested before every start and passed
//!   to the game in `TAKUMI_GATE_TICKET`; the Takumi Gate mod presents it to the event server.
//! - **Signals for the game** (`<game>/.takumi/`): `notice.json` (mandatory update published, restart),
//!   `reload.json` (resource packs updated live), `quit.request` (close the game cleanly) and
//!   `restart.request` (the launcher starts the game again, updated, once it exits). The game writes
//!   `loaded.json` once every mod jar is open.
//! - **Locked while playing:** after `loaded.json`, the launcher takes read access away from `mods/` and
//!   `.fabric/processedMods/` (the jars Fabric extracts from other jars) for everybody ([`lock_running`]).
//!   The game keeps reading through the handles it already has open; Explorer, `copy` or another program
//!   get "access denied". [`scrub`] gives access back and deletes everything when the game exits.

use crate::session::{api_error, endpoint, unreachable, TIMEOUT};
use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use manifest_system::vault::{self, VaultError, VaultKey};
use manifest_system::{FilePolicy, ManifestFile};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// Top folders whose (enforced) files are kept encrypted while the game is not running.
pub const VAULT_DIRS: [&str; 3] = ["mods", "resourcepacks", "shaderpacks"];
const RUNTIME_FILE: &str = "runtime.json";
pub const SIGNAL_DIR: &str = ".takumi";
pub const RESTART_REQUEST: &str = "restart.request";
pub const QUIT_REQUEST: &str = "quit.request";
pub const NOTICE_FILE: &str = "notice.json";
pub const RELOAD_FILE: &str = "reload.json";
pub const LOADED_FILE: &str = "loaded.json";
/// Folders whose files are only readable by the game while it runs. Not `resourcepacks/` or `shaderpacks/`:
/// the game opens those again every time packs or shaders reload.
pub const LOCKED_WHILE_RUNNING: [&str; 2] = ["mods", ".fabric/processedMods"];
/// Where Fabric extracts jars nested in other jars (plain copies of event code).
const PROCESSED_MODS: &str = ".fabric/processedMods";

pub fn is_vaulted(file: &ManifestFile) -> bool {
    file.policy == FilePolicy::Enforced
        && file.path.contains('/')
        && VAULT_DIRS.contains(&file.path.split('/').next().unwrap_or(""))
}

pub fn objects_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join("vault").join("objects")
}

pub fn incoming_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join("vault").join("incoming")
}

pub fn object_path(instance_dir: &Path, sha256: &str) -> PathBuf {
    objects_dir(instance_dir).join(format!("{sha256}.tkv"))
}

/// The object exists and was sealed with the current key.
pub fn has_object(instance_dir: &Path, sha256: &str, key: &VaultKey) -> bool {
    matches!(vault::sealed_key_id(&object_path(instance_dir, sha256)), Ok(Some(id)) if id == key.id())
}

// ------------------------------------------------------------------ API

#[derive(Deserialize)]
struct VaultKeyResponse {
    id: String,
    key: String,
}

pub async fn fetch_vault_key(state: &AppState, id: Uuid, token: &str) -> anyhow::Result<Arc<VaultKey>> {
    let response = state
        .http()
        .get(endpoint(&format!("/api/launcher/instances/{id}/vault-key")))
        .bearer_auth(token)
        .timeout(TIMEOUT)
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: VaultKeyResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    let raw = BASE64.decode(body.key.trim()).context("Clave del almacén no válida.")?;
    Ok(Arc::new(VaultKey::new(&raw, &body.id)?))
}

#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

/// Join ticket for the Takumi Gate server of the event, bound to this Minecraft account.
pub async fn request_ticket(
    state: &AppState,
    id: Uuid,
    token: &str,
    mc_uuid: &str,
    mc_name: &str,
    version: u32,
) -> anyhow::Result<String> {
    let response = state
        .http()
        .post(endpoint(&format!("/api/launcher/instances/{id}/ticket")))
        .bearer_auth(token)
        .json(&serde_json::json!({ "uuid": mc_uuid, "name": mc_name, "version": version }))
        .timeout(TIMEOUT)
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: TicketResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    Ok(body.ticket)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseStatus {
    pub version: u32,
    pub restart_required: bool,
    pub grace_minutes: u32,
    #[serde(default)]
    pub notes: String,
}

pub async fn fetch_status(state: &AppState, id: Uuid, token: &str, since: u32) -> anyhow::Result<ReleaseStatus> {
    let response = state
        .http()
        .get(endpoint(&format!("/api/launcher/instances/{id}/status?since={since}")))
        .bearer_auth(token)
        .timeout(TIMEOUT)
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    response.json().await.context("Respuesta del servidor no válida.")
}

// ---------------------------------------------------------------- vault

/// Encrypts the verified downloads waiting in `vault/incoming/` into `vault/objects/`.
pub fn seal_incoming(instance_dir: &Path, key: &VaultKey) -> anyhow::Result<usize> {
    let incoming = incoming_dir(instance_dir);
    let entries = match std::fs::read_dir(&incoming) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    std::fs::create_dir_all(objects_dir(instance_dir))?;
    let mut sealed = 0;
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            vault::seal_file(key, &name, &path, &object_path(instance_dir, &name))
                .with_context(|| format!("No se pudo cifrar {name}"))?;
            sealed += 1;
        }
        // Partial downloads and leftovers are dropped; they are fetched again if still needed.
        let _ = std::fs::remove_file(&path);
    }
    Ok(sealed)
}

/// Deletes vault objects no published file uses any more.
pub fn gc_objects(instance_dir: &Path, keep: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(objects_dir(instance_dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let sha = name.strip_suffix(".tkv").unwrap_or(&name);
        if !keep.contains(sha) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Hides the instance folder from Explorer (hidden + system) or makes it owner-only on Unix. Best effort.
pub fn protect(instance_dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let result = std::process::Command::new("attrib")
            .arg("+h")
            .arg("+s")
            .arg(instance_dir)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        if !matches!(result, Ok(status) if status.success()) {
            tracing::warn!("no se pudo ocultar {}", instance_dir.display());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(instance_dir, std::fs::Permissions::from_mode(0o700));
    }
}

// --------------------------------------------------------- materialize

/// Decrypted event files in the game folder. Dropping it deletes them again.
pub struct Materialized {
    instance_dir: PathBuf,
    game_dir: PathBuf,
}

impl Materialized {
    /// Adds files decrypted later (resource packs applied live) to what is removed on exit.
    pub fn track(&self, paths: &[String]) -> anyhow::Result<()> {
        let mut list = read_runtime(&self.instance_dir);
        list.extend(paths.iter().cloned());
        list.sort();
        list.dedup();
        write_atomic(&self.instance_dir.join(RUNTIME_FILE), &serde_json::to_vec(&list)?)?;
        Ok(())
    }
}

impl Drop for Materialized {
    fn drop(&mut self) {
        scrub(&self.instance_dir, &self.game_dir);
    }
}

#[derive(Debug)]
pub enum MaterializeError {
    /// These objects failed to decrypt (modified, or sealed with another key); they were deleted.
    Corrupt(Vec<String>),
    Other(anyhow::Error),
}

/// Text written into each decrypted archive.
pub fn watermark(username: &str, instance_name: &str, id: Uuid) -> String {
    format!(
        "Takumi Events - copia de {username} - {instance_name} ({id}) - {}. Uso exclusivo en Takumi Launcher; prohibido redistribuir.",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    )
}

/// Decrypts every vaulted file of `files` into the game folder.
pub fn materialize(
    instance_dir: &Path,
    game_dir: &Path,
    files: &[ManifestFile],
    key: &VaultKey,
    mark: &str,
) -> Result<Materialized, MaterializeError> {
    let guard = Materialized {
        instance_dir: instance_dir.to_path_buf(),
        game_dir: game_dir.to_path_buf(),
    };
    let wanted: Vec<&ManifestFile> = files.iter().filter(|f| is_vaulted(f)).collect();
    // Recorded first, so a crash still lets the next start remove whatever was written.
    guard
        .track(&wanted.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
        .map_err(MaterializeError::Other)?;
    decrypt_into(instance_dir, game_dir, &wanted, key, mark).map(|()| guard)
}

/// Decrypts `files` into the game folder (used at start and for live resource pack updates).
pub fn decrypt_into(
    instance_dir: &Path,
    game_dir: &Path,
    files: &[&ManifestFile],
    key: &VaultKey,
    mark: &str,
) -> Result<(), MaterializeError> {
    let mut corrupt = Vec::new();
    for file in files {
        if !manifest_system::is_safe_path(&file.path) {
            continue;
        }
        let target = game_dir.join(&file.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MaterializeError::Other(e.into()))?;
        }
        let object = object_path(instance_dir, &file.sha256);
        match vault::open_file(key, &file.sha256, &object, &target) {
            Ok(()) => {
                let lower = file.path.to_ascii_lowercase();
                if lower.ends_with(".jar") || lower.ends_with(".zip") {
                    if let Err(error) = vault::set_zip_comment(&target, mark) {
                        tracing::warn!("sin marca de agua en {}: {error}", file.path);
                    }
                }
            }
            Err(VaultError::Corrupt) => {
                let _ = std::fs::remove_file(&object);
                corrupt.push(file.sha256.clone());
            }
            // Missing object (deleted by hand): downloaded again.
            Err(VaultError::Io(_)) if !object.is_file() => corrupt.push(file.sha256.clone()),
            Err(error) => {
                return Err(MaterializeError::Other(
                    anyhow::Error::from(error).context(format!("No se pudo preparar {}", file.path)),
                ))
            }
        }
    }
    if corrupt.is_empty() {
        Ok(())
    } else {
        Err(MaterializeError::Corrupt(corrupt))
    }
}

fn read_runtime(instance_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(instance_dir.join(RUNTIME_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Removes decrypted event files from the game folder. Files still locked by a running game stay listed
/// and are retried next time.
pub fn scrub(instance_dir: &Path, game_dir: &Path) {
    unlock_running(game_dir);
    // Fabric extracts nested jars here on every start; they must not stay around in plain form.
    let processed = game_dir.join(PROCESSED_MODS);
    if processed.exists() {
        if let Err(error) = std::fs::remove_dir_all(&processed) {
            tracing::warn!("no se pudo borrar {}: {error}", processed.display());
        }
    }
    let list = read_runtime(instance_dir);
    if list.is_empty() {
        let _ = std::fs::remove_file(instance_dir.join(RUNTIME_FILE));
        return;
    }
    let mut left = Vec::new();
    for path in list {
        if !manifest_system::is_safe_path(&path) {
            continue;
        }
        let target = game_dir.join(&path);
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!("no se pudo retirar {path}: {error}");
                left.push(path);
            }
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_file(instance_dir.join(RUNTIME_FILE));
    } else if let Ok(bytes) = serde_json::to_vec(&left) {
        let _ = write_atomic(&instance_dir.join(RUNTIME_FILE), &bytes);
    }
}

/// On startup: removes decrypted files a crash may have left behind in any private instance.
pub fn scrub_all(state: &AppState) {
    let Ok(instances) = state.instances.list() else { return };
    for instance in instances.into_iter().filter(|i| i.managed.is_some()) {
        scrub(
            &state.instances.instance_dir(instance.id),
            &state.instances.game_dir(instance.id),
        );
    }
}

// ------------------------------------------------------ lock while running

/// Takes read access to [`LOCKED_WHILE_RUNNING`] away from everybody. Only call it once the game has every
/// jar open (`loaded.json`): a jar the game opens later would fail to load. Returns the folders locked.
pub fn lock_running(game_dir: &Path) -> anyhow::Result<usize> {
    let mut locked = 0;
    for dir in LOCKED_WHILE_RUNNING {
        let path = game_dir.join(dir);
        if !path.is_dir() {
            continue;
        }
        set_locked(&path, true)?;
        locked += 1;
    }
    Ok(locked)
}

/// Gives access back (also after a crash); harmless when nothing is locked.
pub fn unlock_running(game_dir: &Path) {
    for dir in LOCKED_WHILE_RUNNING {
        let path = game_dir.join(dir);
        if path.is_dir() {
            if let Err(error) = set_locked(&path, false) {
                tracing::warn!("no se pudo desbloquear {}: {error:#}", path.display());
            }
        }
    }
}

#[cfg(windows)]
fn set_locked(dir: &Path, lock: bool) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Everyone (S-1-1-0) loses "read data" (on a folder: "list folder"). Open handles keep working.
    const EVERYONE: &str = "*S-1-1-0";
    let icacls = |target: &Path, args: &[&str]| {
        std::process::Command::new("icacls")
            .arg(target)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    };
    let files = dir.join("*");
    let has_files = std::fs::read_dir(dir).map(|mut e| e.next().is_some()).unwrap_or(true);
    if lock {
        // Files first: once the folder cannot be listed, the wildcard no longer matches anything.
        if has_files {
            let status = icacls(&files, &["/deny", &format!("{EVERYONE}:(RD)")])?;
            if !status.success() {
                bail!("icacls no pudo bloquear {}", dir.display());
            }
        }
        let status = icacls(dir, &["/deny", &format!("{EVERYONE}:(RD)")])?;
        if !status.success() {
            bail!("icacls no pudo bloquear {}", dir.display());
        }
    } else {
        // Folder first, so its files can be listed again.
        icacls(dir, &["/remove:d", EVERYONE])?;
        if std::fs::read_dir(dir).map(|mut e| e.next().is_some()).unwrap_or(false) {
            icacls(&files, &["/remove:d", EVERYONE])?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_locked(dir: &Path, lock: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (file_mode, dir_mode) = if lock { (0o000, 0o300) } else { (0o644, 0o755) };
    if !lock {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dir_mode))?;
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(file_mode))?;
        }
    }
    if lock {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dir_mode))?;
    }
    Ok(())
}

// -------------------------------------------------------------- signals

pub fn signal_path(game_dir: &Path, name: &str) -> PathBuf {
    game_dir.join(SIGNAL_DIR).join(name)
}

pub fn write_signal(game_dir: &Path, name: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    write_atomic(&signal_path(game_dir, name), &serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

/// Clears every signal before a new game starts.
pub fn clear_signals(game_dir: &Path) {
    for name in [RESTART_REQUEST, QUIT_REQUEST, NOTICE_FILE, RELOAD_FILE, LOADED_FILE] {
        let _ = std::fs::remove_file(signal_path(game_dir, name));
    }
}

/// Asks the running game to close (the Takumi Gate mod does it cleanly) and to start again updated.
pub fn request_restart(game_dir: &Path) -> anyhow::Result<()> {
    let now = serde_json::json!({ "at": chrono::Utc::now().to_rfc3339() });
    write_signal(game_dir, RESTART_REQUEST, &now)?;
    write_signal(game_dir, QUIT_REQUEST, &now)?;
    Ok(())
}

/// Takes a pending restart request (true if there was one).
pub fn take_restart_request(game_dir: &Path) -> bool {
    let path = signal_path(game_dir, RESTART_REQUEST);
    let present = path.is_file();
    if present {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_file(signal_path(game_dir, QUIT_REQUEST));
    present
}

/// Seconds since the restart request was written, if there is one.
pub fn restart_request_age(game_dir: &Path) -> Option<u64> {
    let modified = std::fs::metadata(signal_path(game_dir, RESTART_REQUEST))
        .ok()?
        .modified()
        .ok()?;
    Some(modified.elapsed().map(|d| d.as_secs()).unwrap_or(0))
}

pub fn ensure_safe_signal_dir(game_dir: &Path) -> anyhow::Result<()> {
    let dir = game_dir.join(SIGNAL_DIR);
    if dir.is_symlink() {
        bail!("La carpeta de señales de la instancia no es válida.");
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, sha: &str, policy: FilePolicy) -> ManifestFile {
        ManifestFile {
            path: path.into(),
            sha256: sha.into(),
            size: 1,
            policy,
        }
    }

    #[test]
    fn only_enforced_files_in_vault_folders_are_vaulted() {
        assert!(is_vaulted(&file("mods/a.jar", "x", FilePolicy::Enforced)));
        assert!(is_vaulted(&file("resourcepacks/p.zip", "x", FilePolicy::Enforced)));
        assert!(!is_vaulted(&file("mods/a.jar", "x", FilePolicy::Default)));
        assert!(!is_vaulted(&file("config/a.json", "x", FilePolicy::Enforced)));
        assert!(!is_vaulted(&file("options.txt", "x", FilePolicy::Enforced)));
    }

    #[test]
    fn vault_round_trip_materializes_and_scrubs() {
        let root = std::env::temp_dir().join(format!("takumi-runtime-{}", Uuid::new_v4()));
        let instance = root.join("instance");
        let game = instance.join("minecraft");
        let key = VaultKey::new(&[9u8; 32], "k").unwrap();

        // A "downloaded" jar waiting in incoming/ under its hash.
        let mut jar = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut jar));
            zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, b"{}").unwrap();
            zip.finish().unwrap();
        }
        let sha = "b".repeat(64);
        std::fs::create_dir_all(incoming_dir(&instance)).unwrap();
        std::fs::write(incoming_dir(&instance).join(&sha), &jar).unwrap();
        assert_eq!(seal_incoming(&instance, &key).unwrap(), 1);
        assert!(has_object(&instance, &sha, &key));
        assert!(!incoming_dir(&instance).join(&sha).exists());

        let files = vec![
            file("mods/event.jar", &sha, FilePolicy::Enforced),
            file("config/x.json", &"c".repeat(64), FilePolicy::Enforced),
        ];
        let guard = materialize(&instance, &game, &files, &key, "copia de steve").unwrap();
        let archive = zip::ZipArchive::new(std::fs::File::open(game.join("mods/event.jar")).unwrap()).unwrap();
        assert_eq!(archive.comment(), b"copia de steve");
        assert!(!game.join("config/x.json").exists(), "config is not vaulted");
        drop(guard);
        assert!(!game.join("mods/event.jar").exists(), "removed when the game exits");
        assert!(!instance.join(RUNTIME_FILE).exists());

        // Another key (a member who was removed, or a rotated secret) cannot open it.
        let other = VaultKey::new(&[1u8; 32], "k").unwrap();
        match materialize(&instance, &game, &files, &other, "x") {
            Err(MaterializeError::Corrupt(list)) => assert_eq!(list, vec![sha.clone()]),
            Err(other) => panic!("unexpected {other:?}"),
            Ok(_) => panic!("decrypted with the wrong key"),
        }
        assert!(
            !object_path(&instance, &sha).exists(),
            "bad object dropped so it is downloaded again"
        );

        gc_objects(&instance, &HashSet::new());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn locked_folders_keep_open_handles_and_scrub_restores_them() {
        use std::io::Read;
        let root = std::env::temp_dir().join(format!("takumi-lock-{}", Uuid::new_v4()));
        let instance = root.join("instance");
        let game = instance.join("minecraft");
        let jar = game.join("mods/evento.jar");
        let nested = game.join(".fabric/processedMods/nested.jar");
        for path in [&jar, &nested] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"codigo del evento").unwrap();
        }
        write_atomic(&instance.join(RUNTIME_FILE), br#"["mods/evento.jar"]"#).unwrap();

        // The game opened the jar before the lock.
        let mut open = std::fs::File::open(&jar).unwrap();
        assert_eq!(lock_running(&game).unwrap(), 2);
        assert!(std::fs::read(&jar).is_err(), "a new reader must be refused");
        assert!(std::fs::read(&nested).is_err());
        assert!(
            std::fs::read_dir(game.join("mods")).is_err(),
            "the folder cannot be listed"
        );
        let mut text = String::new();
        open.read_to_string(&mut text).unwrap();
        assert_eq!(text, "codigo del evento", "the open handle keeps working");
        drop(open);

        scrub(&instance, &game);
        assert!(!jar.exists(), "decrypted jar removed");
        assert!(!game.join(".fabric/processedMods").exists(), "extracted jars removed");
        assert!(std::fs::read_dir(game.join("mods")).is_ok(), "access restored");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_requests_round_trip() {
        let game = std::env::temp_dir().join(format!("takumi-signal-{}", Uuid::new_v4()));
        ensure_safe_signal_dir(&game).unwrap();
        assert!(!take_restart_request(&game));
        request_restart(&game).unwrap();
        assert!(restart_request_age(&game).unwrap() < 5);
        assert!(take_restart_request(&game));
        assert!(!signal_path(&game, QUIT_REQUEST).exists());
        std::fs::remove_dir_all(game).unwrap();
    }
}

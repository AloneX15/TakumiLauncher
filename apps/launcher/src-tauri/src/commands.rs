//! Tauri commands invoked by the frontend.
//!
//! Events emitted: `launch-status`, `launch-log`, `launch-exit`,
//! `auth-done` and `auth-error`.

use crate::accounts::{self, Accounts, MISSING_CLIENT_ID};
use crate::install::loader_kind;
use crate::launch::{self, LaunchSink};
use crate::settings::{Settings, MIN_MEMORY_MB};
use crate::state::AppState;
use auth_system::{DeviceCode, MicrosoftAuth};
use download_manager::DownloadProgress;
use instance_manager::NewInstance;
use minecraft_resolver::LoaderVersion;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tokio::sync::oneshot;
use types::{Instance, InstanceSettings, ModLoader};
use uuid::Uuid;

type CmdResult<T> = Result<T, String>;
type AppStateRef<'a> = State<'a, Arc<AppState>>;

/// Flatten any error (including its causes) into a message for the UI.
fn fail(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

// ---------------------------------------------------------------- app

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    data_dir: String,
}

#[tauri::command]
pub fn get_app_info(state: AppStateRef<'_>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.root.to_string_lossy().into_owned(),
    }
}

#[tauri::command]
pub fn open_data_folder(state: AppStateRef<'_>) -> CmdResult<()> {
    open_in_file_manager(&state.root)
}

fn open_in_file_manager(path: &Path) -> CmdResult<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(drop)
        .map_err(fail)
}

// ----------------------------------------------------------- versions

#[derive(Serialize)]
pub struct VersionSummary {
    id: String,
    kind: String,
    release_time: String,
}

#[derive(Serialize)]
pub struct VersionList {
    latest_release: String,
    versions: Vec<VersionSummary>,
}

#[tauri::command]
pub async fn get_versions(state: AppStateRef<'_>) -> CmdResult<VersionList> {
    let manifest = state.version_manifest().await.map_err(fail)?;
    Ok(VersionList {
        latest_release: manifest.latest.release,
        versions: manifest
            .versions
            .into_iter()
            .map(|v| VersionSummary {
                id: v.id,
                kind: v.kind,
                release_time: v.release_time,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn get_loader_versions(
    state: AppStateRef<'_>,
    loader: ModLoader,
    version: String,
) -> CmdResult<Vec<LoaderVersion>> {
    match loader_kind(loader).map_err(fail)? {
        Some(kind) => kind.versions(state.http(), &version).await.map_err(fail),
        None => Ok(Vec::new()),
    }
}

// ---------------------------------------------------------- instances

#[tauri::command]
pub fn list_instances(state: AppStateRef<'_>) -> CmdResult<Vec<Instance>> {
    state.instances.list().map_err(fail)
}

#[tauri::command]
pub async fn create_instance(
    state: AppStateRef<'_>,
    name: String,
    version: String,
    loader: ModLoader,
    loader_version: Option<String>,
) -> CmdResult<Instance> {
    let manifest = state.version_manifest().await.map_err(fail)?;
    manifest.find(&version).map_err(fail)?;

    let mod_loader_version = match loader_kind(loader).map_err(fail)? {
        None => None,
        Some(kind) => {
            let available = kind.versions(state.http(), &version).await.map_err(fail)?;
            let chosen = match loader_version {
                Some(wanted) => available.iter().find(|v| v.version == wanted),
                None => available.iter().find(|v| v.stable).or(available.first()),
            };
            let chosen = chosen.ok_or_else(|| format!("{loader} no está disponible para Minecraft {version}."))?;
            Some(chosen.version.clone())
        }
    };

    state
        .instances
        .create(NewInstance {
            name,
            minecraft_version: version,
            mod_loader: loader,
            mod_loader_version,
        })
        .map_err(fail)
}

/// Update the editable parts of an instance: its name and settings.
#[tauri::command]
pub fn update_instance(
    state: AppStateRef<'_>,
    id: Uuid,
    name: String,
    settings: InstanceSettings,
) -> CmdResult<Instance> {
    if settings.width.is_some() != settings.height.is_some() {
        return Err("Indica el ancho y el alto de la ventana, o ninguno.".into());
    }
    if settings.memory_mb.is_some_and(|memory| memory < MIN_MEMORY_MB) {
        return Err(format!("La memoria mínima es de {MIN_MEMORY_MB} MB."));
    }
    let clean = |value: Option<String>| value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    let mut instance = state.instances.get(id).map_err(fail)?;
    instance.name = name;
    instance.settings = InstanceSettings {
        java_path: clean(settings.java_path),
        jvm_args: clean(settings.jvm_args),
        ..settings
    };
    state.instances.save(&mut instance).map_err(fail)?;
    Ok(instance)
}

#[tauri::command]
pub fn delete_instance(state: AppStateRef<'_>, id: Uuid) -> CmdResult<()> {
    if state.running.lock().unwrap().contains_key(&id) {
        return Err("Detén el juego antes de eliminar la instancia.".into());
    }
    state.instances.delete(id).map_err(fail)
}

#[tauri::command]
pub fn open_instance_folder(state: AppStateRef<'_>, id: Uuid) -> CmdResult<()> {
    let dir = state.instances.game_dir(id);
    if !dir.is_dir() {
        return Err("La carpeta de la instancia no existe.".into());
    }
    open_in_file_manager(&dir)
}

// ----------------------------------------------------------- accounts

#[tauri::command]
pub fn list_accounts(state: AppStateRef<'_>) -> Accounts {
    Accounts::load(&state.root)
}

#[tauri::command]
pub fn add_offline_account(state: AppStateRef<'_>, username: String) -> CmdResult<Accounts> {
    let mut accounts = Accounts::load(&state.root);
    accounts.upsert(accounts::offline_account(&username).map_err(fail)?);
    accounts.save(&state.root).map_err(fail)?;
    Ok(accounts)
}

#[tauri::command]
pub fn remove_account(state: AppStateRef<'_>, id: String) -> CmdResult<Accounts> {
    accounts::remove_account(&state, &id).map_err(fail)?;
    Ok(Accounts::load(&state.root))
}

#[tauri::command]
pub fn set_active_account(state: AppStateRef<'_>, id: String) -> CmdResult<Accounts> {
    let mut accounts = Accounts::load(&state.root);
    accounts.set_active(&id).map_err(fail)?;
    accounts.save(&state.root).map_err(fail)?;
    Ok(accounts)
}

/// Start the Microsoft device code flow. Returns the code to show the user;
/// the result arrives later as an `auth-done` or `auth-error` event.
#[tauri::command]
pub async fn start_microsoft_login(app: AppHandle, state: AppStateRef<'_>) -> CmdResult<DeviceCode> {
    let client_id = Settings::load(&state.root)
        .microsoft_client_id()
        .ok_or(MISSING_CLIENT_ID)?;
    let auth = MicrosoftAuth::new(state.http().clone(), client_id);
    let code = auth.start_device_flow().await.map_err(fail)?;

    let state = state.inner().clone();
    let task_state = state.clone();
    let pending = code.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_state;
        let result = match auth.complete_device_flow(&pending).await {
            Ok(session) => accounts::store_microsoft_session(&state, session).map_err(fail),
            Err(error) => Err(fail(error)),
        };
        let _ = match result {
            Ok(_) => app.emit_all("auth-done", Accounts::load(&state.root)),
            Err(error) => app.emit_all("auth-error", error),
        };
    });
    let previous = state.login_task.lock().unwrap().replace(task);
    if let Some(previous) = previous {
        previous.abort();
    }
    Ok(code)
}

#[tauri::command]
pub fn cancel_microsoft_login(state: AppStateRef<'_>) {
    let task = state.login_task.lock().unwrap().take();
    if let Some(task) = task {
        task.abort();
    }
}

// ----------------------------------------------------------- settings

#[derive(Serialize)]
pub struct SettingsView {
    settings: Settings,
    /// Whether Microsoft sign-in is possible (settings, env or build-time id).
    microsoft_login_available: bool,
}

fn settings_view(settings: Settings) -> SettingsView {
    SettingsView {
        microsoft_login_available: settings.microsoft_client_id().is_some(),
        settings,
    }
}

#[tauri::command]
pub fn get_settings(state: AppStateRef<'_>) -> SettingsView {
    settings_view(Settings::load(&state.root))
}

#[tauri::command]
pub fn save_settings(state: AppStateRef<'_>, mut settings: Settings) -> CmdResult<SettingsView> {
    settings.save(&state.root).map_err(fail)?;
    Ok(settings_view(settings))
}

// ------------------------------------------------------------- launch

struct EventSink {
    app: AppHandle,
    id: Uuid,
}

impl LaunchSink for EventSink {
    fn status(&self, stage: &str, progress: Option<DownloadProgress>) {
        let payload = json!({ "instance_id": self.id, "stage": stage, "progress": progress });
        let _ = self.app.emit_all("launch-status", payload);
    }

    fn log(&self, line: String) {
        let _ = self
            .app
            .emit_all("launch-log", json!({ "instance_id": self.id, "line": line }));
    }
}

/// Prepare and start an instance in the background. Progress, game output
/// and the final result are delivered as events.
#[tauri::command]
pub fn launch_instance(app: AppHandle, state: AppStateRef<'_>, id: Uuid) -> CmdResult<()> {
    let state = state.inner().clone();
    let (stop_tx, stop_rx) = oneshot::channel();
    {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&id) {
            return Err("Esta instancia ya está en ejecución.".into());
        }
        running.insert(id, Some(stop_tx));
    }

    tauri::async_runtime::spawn(async move {
        let sink = EventSink { app: app.clone(), id };
        let result = launch::run(&state, id, stop_rx, &sink).await;
        state.running.lock().unwrap().remove(&id);

        let payload = match result {
            Ok(code) => json!({ "instance_id": id, "code": code, "error": null }),
            Err(error) => {
                let message = fail(error);
                let error = (message != launch::CANCELLED).then_some(message);
                json!({ "instance_id": id, "code": null, "error": error })
            }
        };
        let _ = app.emit_all("launch-exit", payload);
    });
    Ok(())
}

#[tauri::command]
pub fn stop_instance(state: AppStateRef<'_>, id: Uuid) {
    let sender = state.running.lock().unwrap().get_mut(&id).and_then(Option::take);
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
}

#[tauri::command]
pub fn running_instances(state: AppStateRef<'_>) -> Vec<Uuid> {
    state.running.lock().unwrap().keys().copied().collect()
}

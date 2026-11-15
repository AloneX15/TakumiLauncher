//! Tauri commands for instance content, modpacks and instance duplication.
//!
//! Long modpack installs report progress through `modpack-progress` events.

use crate::commands::{fail, AppStateRef, CmdResult};
use crate::content::{self, ContentItem, ContentKind, ContentUpdate, SearchResult};
use crate::modpack;
use download_manager::DownloadProgress;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use types::Instance;
use uuid::Uuid;

#[tauri::command]
pub async fn search_content(
    state: AppStateRef<'_>,
    id: Uuid,
    kind: ContentKind,
    query: String,
    offset: u32,
) -> CmdResult<SearchResult> {
    let instance = state.instances.get(id).map_err(fail)?;
    content::search_for_instance(state.http(), &instance, kind, &query, offset)
        .await
        .map_err(fail)
}

#[tauri::command]
pub fn list_content(state: AppStateRef<'_>, id: Uuid) -> CmdResult<Vec<ContentItem>> {
    let instance = state.instances.get(id).map_err(fail)?;
    content::list(&state, &instance).map_err(fail)
}

/// Install a Modrinth project (and required dependencies) into an instance.
/// Returns the titles of everything that was installed.
#[tauri::command]
pub async fn install_content(
    state: AppStateRef<'_>,
    id: Uuid,
    kind: ContentKind,
    project: String,
) -> CmdResult<Vec<String>> {
    let instance = editable(&state, id)?;
    content::install_project(&state, &instance, kind, &project)
        .await
        .map_err(fail)
}

/// Install the client performance mods (Sodium, Lithium…) the instance is missing.
#[tauri::command]
pub async fn optimize_instance(state: AppStateRef<'_>, id: Uuid) -> CmdResult<content::OptimizeReport> {
    let instance = editable(&state, id)?;
    content::install_performance_mods(&state, &instance).await.map_err(fail)
}

#[tauri::command]
pub fn set_content_enabled(
    state: AppStateRef<'_>,
    id: Uuid,
    kind: ContentKind,
    filename: String,
    enabled: bool,
) -> CmdResult<()> {
    let instance = editable(&state, id)?;
    content::set_enabled(&state, &instance, kind, &filename, enabled).map_err(fail)
}

#[tauri::command]
pub fn remove_content(state: AppStateRef<'_>, id: Uuid, kind: ContentKind, filename: String) -> CmdResult<()> {
    let instance = editable(&state, id)?;
    content::remove(&state, &instance, kind, &filename).map_err(fail)
}

#[tauri::command]
pub async fn check_content_updates(state: AppStateRef<'_>, id: Uuid) -> CmdResult<Vec<ContentUpdate>> {
    let instance = state.instances.get(id).map_err(fail)?;
    content::check_updates(&state, &instance).await.map_err(fail)
}

#[tauri::command]
pub async fn update_content(state: AppStateRef<'_>, id: Uuid, kind: ContentKind, filename: String) -> CmdResult<()> {
    let instance = editable(&state, id)?;
    content::update(&state, &instance, kind, &filename).await.map_err(fail)
}

// ------------------------------------------------------------ modpacks

#[tauri::command]
pub async fn search_modpacks(state: AppStateRef<'_>, query: String, offset: u32) -> CmdResult<SearchResult> {
    content::search(state.http(), "modpack", &[], None, &query, offset)
        .await
        .map_err(fail)
}

fn progress_emitter(app: AppHandle) -> impl Fn(DownloadProgress) + Send + Sync {
    move |progress| {
        let _ = app.emit_all("modpack-progress", progress);
    }
}

/// Create an instance from a Modrinth modpack project.
#[tauri::command]
pub async fn install_modpack(app: AppHandle, state: AppStateRef<'_>, project: String) -> CmdResult<Instance> {
    modpack::install_from_modrinth(&state, &project, &progress_emitter(app))
        .await
        .map_err(fail)
}

/// Create an instance from a local `.mrpack` file.
#[tauri::command]
pub async fn import_modpack(app: AppHandle, state: AppStateRef<'_>, path: String) -> CmdResult<Instance> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err("No se encuentra el archivo del modpack.".into());
    }
    modpack::import(&state, &path, &progress_emitter(app))
        .await
        .map_err(fail)
}

// ----------------------------------------------------------- instances

/// Copy an instance with all of its worlds, mods and settings.
#[tauri::command]
pub async fn duplicate_instance(state: AppStateRef<'_>, id: Uuid) -> CmdResult<Instance> {
    if state.running.lock().unwrap().contains_key(&id) {
        return Err("Detén el juego antes de duplicar la instancia.".into());
    }
    editable(&state, id)?;
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || state.instances.duplicate(id))
        .await
        .map_err(fail)?
        .map_err(fail)
}

/// The instance, unless it is a private instance managed from the website.
fn editable(state: &AppStateRef<'_>, id: Uuid) -> CmdResult<Instance> {
    let instance = state.instances.get(id).map_err(fail)?;
    crate::private_instances::ensure_editable(&instance).map_err(fail)?;
    Ok(instance)
}

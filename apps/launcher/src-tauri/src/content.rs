//! Instance content (mods, resource packs, shaders) backed by Modrinth.
//!
//! Files live in the instance's game directory. A small index
//! (`content.json`) remembers which Modrinth project each file came from so
//! the UI can show names and icons; files added by hand simply have no entry.

use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use download_manager::reqwest::Client;
use download_manager::{sha1_hex, DownloadJob};
use minecraft_resolver::is_safe_file_name;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use types::{Instance, ModLoader};

const API: &str = "https://api.modrinth.com/v2";
const INDEX_FILE: &str = "content.json";
const DISABLED_SUFFIX: &str = ".disabled";
pub const PAGE_SIZE: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Mod,
    ResourcePack,
    Shader,
}

impl ContentKind {
    const ALL: [ContentKind; 3] = [ContentKind::Mod, ContentKind::ResourcePack, ContentKind::Shader];

    /// Directory inside the game directory.
    fn dir(self) -> &'static str {
        match self {
            ContentKind::Mod => "mods",
            ContentKind::ResourcePack => "resourcepacks",
            ContentKind::Shader => "shaderpacks",
        }
    }

    fn project_type(self) -> &'static str {
        match self {
            ContentKind::Mod => "mod",
            ContentKind::ResourcePack => "resourcepack",
            ContentKind::Shader => "shader",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            ContentKind::Mod => &[".jar"],
            ContentKind::ResourcePack | ContentKind::Shader => &[".zip"],
        }
    }

    /// Modrinth "loaders" that files of this kind must declare to be usable
    /// by `loader`. `None` when the instance cannot use this kind at all.
    fn loaders(self, loader: ModLoader) -> Option<Vec<&'static str>> {
        match self {
            ContentKind::Mod => match loader {
                ModLoader::Vanilla => None,
                ModLoader::Fabric => Some(vec!["fabric"]),
                // Quilt loads Fabric mods too.
                ModLoader::Quilt => Some(vec!["quilt", "fabric"]),
                ModLoader::Forge => Some(vec!["forge"]),
                ModLoader::NeoForge => Some(vec!["neoforge"]),
            },
            ContentKind::ResourcePack => Some(vec!["minecraft"]),
            ContentKind::Shader => Some(vec!["iris", "optifine", "canvas", "vanilla"]),
        }
    }

    fn loaders_for(self, instance: &Instance) -> anyhow::Result<Vec<&'static str>> {
        self.loaders(instance.mod_loader)
            .context("Las instancias Vanilla no admiten mods. Crea una con Fabric, Quilt, Forge o NeoForge.")
    }
}

// ------------------------------------------------------------ Modrinth

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub offset: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub version_number: String,
    /// `release`, `beta` or `alpha`
    #[serde(default)]
    pub version_type: String,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: Option<u64>,
    pub hashes: FileHashes,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileHashes {
    pub sha1: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    pub dependency_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Project {
    title: String,
    #[serde(default)]
    icon_url: Option<String>,
}

/// Newest stable release, or the newest pre-release when there is none.
/// Expects `versions` newest first, as Modrinth returns them.
fn preferred_version(versions: Vec<Version>) -> Option<Version> {
    let index = versions.iter().position(|v| v.version_type == "release").unwrap_or(0);
    versions.into_iter().nth(index)
}

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }
}

async fn get<T: DeserializeOwned>(http: &Client, path: &str, query: &[(&str, String)]) -> anyhow::Result<T> {
    let response = http
        .get(format!("{API}{path}"))
        .query(query)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("No se pudo conectar con Modrinth: {}", e.without_url()))?;
    if !response.status().is_success() {
        bail!("Modrinth respondió HTTP {}", response.status());
    }
    response.json().await.context("Respuesta de Modrinth no válida.")
}

/// Modrinth identifiers are short base62 strings; anything else is rejected
/// before it is placed in a URL.
fn check_id(id: &str) -> anyhow::Result<&str> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        bail!("Identificador de Modrinth no válido.");
    }
    Ok(id)
}

/// Search projects of one type, optionally restricted to what an instance
/// (loaders + game version) can use.
pub async fn search(
    http: &Client,
    project_type: &str,
    loaders: &[&str],
    game_version: Option<&str>,
    query: &str,
    offset: u32,
) -> anyhow::Result<SearchResult> {
    let mut facets = vec![vec![format!("project_type:{project_type}")]];
    if !loaders.is_empty() {
        facets.push(loaders.iter().map(|l| format!("categories:{l}")).collect());
    }
    if let Some(version) = game_version {
        facets.push(vec![format!("versions:{version}")]);
    }
    get(
        http,
        "/search",
        &[
            ("query", query.trim().to_string()),
            ("facets", serde_json::to_string(&facets)?),
            ("limit", PAGE_SIZE.to_string()),
            ("offset", offset.to_string()),
        ],
    )
    .await
}

/// Search content of one kind that the instance can actually use.
pub async fn search_for_instance(
    http: &Client,
    instance: &Instance,
    kind: ContentKind,
    query: &str,
    offset: u32,
) -> anyhow::Result<SearchResult> {
    let loaders = kind.loaders_for(instance)?;
    // Only mods are tied to a loader in search; packs list theirs loosely.
    let loader_filter: &[&str] = if kind == ContentKind::Mod { &loaders } else { &[] };
    search(
        http,
        kind.project_type(),
        loader_filter,
        Some(&instance.minecraft_version),
        query,
        offset,
    )
    .await
}

/// Versions of a project usable with the given loaders and game version,
/// newest first. Empty filters mean "any".
pub async fn project_versions(
    http: &Client,
    project: &str,
    loaders: &[&str],
    game_version: Option<&str>,
) -> anyhow::Result<Vec<Version>> {
    let mut query = Vec::new();
    if !loaders.is_empty() {
        query.push(("loaders", serde_json::to_string(loaders)?));
    }
    if let Some(version) = game_version {
        query.push(("game_versions", serde_json::to_string(&[version])?));
    }
    get(http, &format!("/project/{}/version", check_id(project)?), &query).await
}

// --------------------------------------------------------------- index

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexEntry {
    project_id: String,
    version_id: String,
    title: String,
    version_number: String,
    #[serde(default)]
    icon_url: Option<String>,
}

/// `"<dir>/<file name without .disabled>"` -> origin
type Index = HashMap<String, IndexEntry>;

fn index_key(kind: ContentKind, filename: &str) -> String {
    format!("{}/{}", kind.dir(), filename.trim_end_matches(DISABLED_SUFFIX))
}

fn index_path(state: &AppState, instance: &Instance) -> PathBuf {
    state.instances.instance_dir(instance.id).join(INDEX_FILE)
}

fn load_index(state: &AppState, instance: &Instance) -> Index {
    std::fs::read_to_string(index_path(state, instance))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_index(state: &AppState, instance: &Instance, index: &Index) -> anyhow::Result<()> {
    write_atomic(&index_path(state, instance), &serde_json::to_vec_pretty(index)?)?;
    Ok(())
}

// ------------------------------------------------------------- content

#[derive(Debug, Clone, Serialize)]
pub struct ContentItem {
    pub kind: ContentKind,
    /// File name on disk, including `.disabled` when disabled.
    pub filename: String,
    pub enabled: bool,
    pub size: u64,
    pub title: Option<String>,
    pub version_number: Option<String>,
    pub icon_url: Option<String>,
    pub project_id: Option<String>,
}

fn content_dir(state: &AppState, instance: &Instance, kind: ContentKind) -> PathBuf {
    state.instances.game_dir(instance.id).join(kind.dir())
}

/// Path of an existing content file, validating the name first.
fn content_file(state: &AppState, instance: &Instance, kind: ContentKind, filename: &str) -> anyhow::Result<PathBuf> {
    if !is_safe_file_name(filename) {
        bail!("Nombre de archivo no válido.");
    }
    let path = content_dir(state, instance, kind).join(filename);
    if !path.is_file() {
        bail!("El archivo ya no existe: {filename}");
    }
    Ok(path)
}

/// Everything installed in the instance, by kind then name.
pub fn list(state: &AppState, instance: &Instance) -> anyhow::Result<Vec<ContentItem>> {
    let index = load_index(state, instance);
    let mut items = Vec::new();
    for kind in ContentKind::ALL {
        let Ok(entries) = std::fs::read_dir(content_dir(state, instance, kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().into_owned();
            let active_name = filename.trim_end_matches(DISABLED_SUFFIX);
            let lower = active_name.to_lowercase();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || !kind.extensions().iter().any(|ext| lower.ends_with(ext)) {
                continue;
            }
            let origin = index.get(&index_key(kind, &filename));
            items.push(ContentItem {
                kind,
                enabled: filename == active_name,
                size: meta.len(),
                title: origin.map(|o| o.title.clone()),
                version_number: origin.map(|o| o.version_number.clone()),
                icon_url: origin.and_then(|o| o.icon_url.clone()),
                project_id: origin.map(|o| o.project_id.clone()),
                filename,
            });
        }
    }
    items.sort_by_key(|item| {
        (
            item.kind as u8,
            item.title
                .clone()
                .unwrap_or_else(|| item.filename.clone())
                .to_lowercase(),
        )
    });
    Ok(items)
}

pub fn set_enabled(
    state: &AppState,
    instance: &Instance,
    kind: ContentKind,
    filename: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let path = content_file(state, instance, kind, filename)?;
    let active_name = filename.trim_end_matches(DISABLED_SUFFIX);
    let target = if enabled {
        active_name.to_string()
    } else {
        format!("{active_name}{DISABLED_SUFFIX}")
    };
    if target != filename {
        std::fs::rename(&path, path.with_file_name(target)).context("No se pudo cambiar el estado del archivo.")?;
    }
    Ok(())
}

pub fn remove(state: &AppState, instance: &Instance, kind: ContentKind, filename: &str) -> anyhow::Result<()> {
    let path = content_file(state, instance, kind, filename)?;
    std::fs::remove_file(path).context("No se pudo eliminar (¿está el juego abierto?).")?;
    let mut index = load_index(state, instance);
    if index.remove(&index_key(kind, filename)).is_some() {
        save_index(state, instance, &index)?;
    }
    Ok(())
}

/// Download one version's primary file into the instance and record it.
async fn install_version(
    state: &AppState,
    instance: &Instance,
    kind: ContentKind,
    version: &Version,
    index: &mut Index,
) -> anyhow::Result<String> {
    let file = version.primary_file().context("La versión no tiene archivos.")?;
    let lower = file.filename.to_lowercase();
    if !is_safe_file_name(&file.filename) || !kind.extensions().iter().any(|ext| lower.ends_with(ext)) {
        bail!("Archivo no admitido: {}", file.filename);
    }
    if !file.url.starts_with("https://") {
        bail!("Descarga no segura rechazada: {}", file.url);
    }
    let destination = content_dir(state, instance, kind).join(&file.filename);
    let job = DownloadJob::new(&file.url, &destination)
        .sha1(Some(file.hashes.sha1.clone()))
        .size(file.size);
    state.downloader.download_all(vec![job], |_| {}).await?;

    let project: Project = get(
        state.http(),
        &format!("/project/{}", check_id(&version.project_id)?),
        &[],
    )
    .await?;
    index.insert(
        index_key(kind, &file.filename),
        IndexEntry {
            project_id: version.project_id.clone(),
            version_id: version.id.clone(),
            title: project.title.clone(),
            version_number: version.version_number.clone(),
            icon_url: project.icon_url,
        },
    );
    Ok(project.title)
}

/// Install the newest compatible version of a project, plus (for mods) its
/// required dependencies. Returns the titles that were installed.
pub async fn install_project(
    state: &AppState,
    instance: &Instance,
    kind: ContentKind,
    project: &str,
) -> anyhow::Result<Vec<String>> {
    let loaders = kind.loaders_for(instance)?;
    let game_version = instance.minecraft_version.as_str();
    let mut index = load_index(state, instance);
    let mut present: HashSet<String> = index.values().map(|e| e.project_id.clone()).collect();
    if present.contains(project) {
        bail!("Ya está instalado en esta instancia.");
    }

    let mut installed = Vec::new();
    let mut queue = VecDeque::from([(project.to_string(), None::<String>)]);
    let result: anyhow::Result<()> = async {
        while let Some((project_id, pinned_version)) = queue.pop_front() {
            if !present.insert(project_id.clone()) {
                continue;
            }
            let version = match pinned_version {
                Some(id) => get::<Version>(state.http(), &format!("/version/{}", check_id(&id)?), &[]).await?,
                None => {
                    preferred_version(project_versions(state.http(), &project_id, &loaders, Some(game_version)).await?)
                        .with_context(|| {
                            if installed.is_empty() {
                                format!("No hay ninguna versión compatible con Minecraft {game_version}.")
                            } else {
                                format!("Una dependencia ({project_id}) no tiene versión compatible.")
                            }
                        })?
                }
            };
            installed.push(install_version(state, instance, kind, &version, &mut index).await?);

            if kind == ContentKind::Mod {
                for dependency in version.dependencies.iter().filter(|d| d.dependency_type == "required") {
                    match (&dependency.project_id, &dependency.version_id) {
                        (Some(id), _) if present.contains(id) => {}
                        (Some(id), version_id) => queue.push_back((id.clone(), version_id.clone())),
                        (None, Some(version_id)) => {
                            let pinned: Version =
                                get(state.http(), &format!("/version/{}", check_id(version_id)?), &[]).await?;
                            if !present.contains(&pinned.project_id) {
                                queue.push_back((pinned.project_id, Some(pinned.id)));
                            }
                        }
                        (None, None) => {}
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    // Keep the index in step with whatever did get installed.
    save_index(state, instance, &index)?;
    result?;
    Ok(installed)
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentUpdate {
    pub kind: ContentKind,
    pub filename: String,
    pub version_number: String,
}

/// Ask Modrinth, by file hash, which installed files have a newer compatible
/// version. Works for files added by hand as well.
async fn find_updates(
    state: &AppState,
    instance: &Instance,
    files: Vec<(ContentKind, String)>,
) -> anyhow::Result<Vec<(ContentKind, String, Version)>> {
    let mut updates = Vec::new();
    for kind in ContentKind::ALL {
        let Some(loaders) = kind.loaders(instance.mod_loader) else {
            continue;
        };
        let mut by_hash = HashMap::new();
        for (_, filename) in files.iter().filter(|(k, _)| *k == kind) {
            let path = content_dir(state, instance, kind).join(filename);
            if let Ok(data) = tokio::fs::read(&path).await {
                by_hash.insert(sha1_hex(&data), filename.clone());
            }
        }
        if by_hash.is_empty() {
            continue;
        }
        let response = state
            .http()
            .post(format!("{API}/version_files/update"))
            .json(&json!({
                "hashes": by_hash.keys().collect::<Vec<_>>(),
                "algorithm": "sha1",
                "loaders": loaders,
                "game_versions": [instance.minecraft_version],
            }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("No se pudo conectar con Modrinth: {}", e.without_url()))?;
        if !response.status().is_success() {
            bail!("Modrinth respondió HTTP {}", response.status());
        }
        let latest: HashMap<String, Version> = response.json().await.context("Respuesta de Modrinth no válida.")?;
        for (hash, version) in latest {
            let Some(filename) = by_hash.get(&hash) else { continue };
            let is_same = version.files.iter().any(|f| f.hashes.sha1.eq_ignore_ascii_case(&hash));
            if !is_same {
                updates.push((kind, filename.clone(), version));
            }
        }
    }
    Ok(updates)
}

pub async fn check_updates(state: &AppState, instance: &Instance) -> anyhow::Result<Vec<ContentUpdate>> {
    let files = list(state, instance)?
        .into_iter()
        .map(|item| (item.kind, item.filename))
        .collect();
    Ok(find_updates(state, instance, files)
        .await?
        .into_iter()
        .map(|(kind, filename, version)| ContentUpdate {
            kind,
            filename,
            version_number: version.version_number,
        })
        .collect())
}

/// Replace one file with its newest compatible version, keeping it enabled
/// or disabled as it was.
pub async fn update(state: &AppState, instance: &Instance, kind: ContentKind, filename: &str) -> anyhow::Result<()> {
    let old_path = content_file(state, instance, kind, filename)?;
    let (_, _, version) = find_updates(state, instance, vec![(kind, filename.to_string())])
        .await?
        .into_iter()
        .next()
        .context("Ya está en la última versión compatible.")?;

    let was_enabled = !filename.ends_with(DISABLED_SUFFIX);
    let mut index = load_index(state, instance);
    install_version(state, instance, kind, &version, &mut index).await?;

    let new_name = version.primary_file().map(|f| f.filename.clone()).unwrap_or_default();
    if new_name != filename.trim_end_matches(DISABLED_SUFFIX) {
        remove_quietly(&old_path);
        index.remove(&index_key(kind, filename));
    } else if !was_enabled {
        // Same name: the fresh download sits next to the disabled copy.
        remove_quietly(&old_path);
    }
    save_index(state, instance, &index)?;
    if !was_enabled {
        set_enabled(state, instance, kind, &new_name, false)?;
    }
    Ok(())
}

fn remove_quietly(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        tracing::warn!("no se pudo eliminar {}: {error}", path.display());
    }
}

// ------------------------------------------------------- performance

/// Well-known client performance mods (Modrinth project id, name, words that
/// give the mod away in a file name, including forks that replace it).
const PERFORMANCE_MODS: &[(&str, &str, &[&str])] = &[
    (
        "AANobbMI",
        "Sodium",
        &["sodium", "embeddium", "rubidium", "optifine", "optifabric"],
    ),
    ("gvQqBUqZ", "Lithium", &["lithium", "canary", "radium"]),
    ("uXXizFIs", "FerriteCore", &["ferritecore", "ferrite-core"]),
    ("NNAgCjsB", "Entity Culling", &["entityculling", "entity-culling"]),
    ("5ZwdcRci", "ImmediatelyFast", &["immediatelyfast"]),
];

#[derive(Debug, Default, Serialize)]
pub struct OptimizeReport {
    /// Titles of everything installed, dependencies included.
    pub installed: Vec<String>,
    /// Performance mods the instance already had.
    pub already: Vec<String>,
    /// Mods with no build for this loader and game version.
    pub unavailable: Vec<String>,
}

/// Modrinth projects of the mods in the instance, identified by file hash so
/// that mods from a modpack or added by hand count too.
async fn installed_mod_projects(state: &AppState, instance: &Instance) -> anyhow::Result<HashSet<String>> {
    let mut projects: HashSet<String> = load_index(state, instance)
        .values()
        .map(|entry| entry.project_id.clone())
        .collect();

    let mut hashes = Vec::new();
    for item in list(state, instance)?
        .into_iter()
        .filter(|item| item.kind == ContentKind::Mod)
    {
        let path = content_dir(state, instance, ContentKind::Mod).join(&item.filename);
        if let Ok(data) = tokio::fs::read(&path).await {
            hashes.push(sha1_hex(&data));
        }
    }
    if hashes.is_empty() {
        return Ok(projects);
    }
    let response = state
        .http()
        .post(format!("{API}/version_files"))
        .json(&json!({ "hashes": hashes, "algorithm": "sha1" }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("No se pudo conectar con Modrinth: {}", e.without_url()))?;
    if !response.status().is_success() {
        bail!("Modrinth respondió HTTP {}", response.status());
    }
    let known: HashMap<String, Version> = response.json().await.context("Respuesta de Modrinth no válida.")?;
    projects.extend(known.into_values().map(|version| version.project_id));
    Ok(projects)
}

/// Install the performance mods the instance is missing. Mods without a build
/// for its loader and game version are skipped, as are ones it already has
/// (even disabled, or under the name of a fork).
pub async fn install_performance_mods(state: &AppState, instance: &Instance) -> anyhow::Result<OptimizeReport> {
    optimize(state, instance, false).await
}

/// What `install_performance_mods` would do, without installing anything
/// (`installed` lists the mods it would add).
#[cfg(test)]
pub async fn performance_plan(state: &AppState, instance: &Instance) -> anyhow::Result<OptimizeReport> {
    optimize(state, instance, true).await
}

async fn optimize(state: &AppState, instance: &Instance, dry_run: bool) -> anyhow::Result<OptimizeReport> {
    let loaders = ContentKind::Mod.loaders_for(instance)?;
    let present = installed_mod_projects(state, instance).await?;
    let file_names: Vec<String> = list(state, instance)?
        .into_iter()
        .filter(|item| item.kind == ContentKind::Mod)
        .map(|item| item.filename.to_lowercase())
        .collect();

    let mut report = OptimizeReport::default();
    for (project, name, file_hints) in PERFORMANCE_MODS {
        let by_name = file_names
            .iter()
            .any(|file| file_hints.iter().any(|hint| file.contains(hint)));
        if present.contains(*project) || by_name {
            report.already.push(name.to_string());
            continue;
        }
        let versions = project_versions(state.http(), project, &loaders, Some(&instance.minecraft_version)).await?;
        if versions.is_empty() {
            report.unavailable.push(name.to_string());
            continue;
        }
        if dry_run {
            report.installed.push(name.to_string());
            continue;
        }
        match install_project(state, instance, ContentKind::Mod, project).await {
            Ok(titles) => report.installed.extend(titles),
            // A required dependency has no compatible build: leave this mod out.
            Err(error) if report_is_incompatible(&error) => report.unavailable.push(name.to_string()),
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

fn report_is_incompatible(error: &anyhow::Error) -> bool {
    error.to_string().contains("no tiene versión compatible")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_serialize_lowercase() {
        assert_eq!(
            serde_json::to_string(&ContentKind::ResourcePack).unwrap(),
            "\"resourcepack\""
        );
        let kind: ContentKind = serde_json::from_str("\"shader\"").unwrap();
        assert_eq!(kind, ContentKind::Shader);
    }

    #[test]
    fn loaders_per_kind() {
        assert!(ContentKind::Mod.loaders(ModLoader::Vanilla).is_none());
        assert_eq!(ContentKind::Mod.loaders(ModLoader::Quilt).unwrap(), ["quilt", "fabric"]);
        assert_eq!(ContentKind::Mod.loaders(ModLoader::NeoForge).unwrap(), ["neoforge"]);
        assert_eq!(
            ContentKind::ResourcePack.loaders(ModLoader::Vanilla).unwrap(),
            ["minecraft"]
        );
    }

    #[test]
    fn index_key_ignores_disabled_suffix() {
        assert_eq!(index_key(ContentKind::Mod, "sodium.jar.disabled"), "mods/sodium.jar");
        assert_eq!(index_key(ContentKind::Shader, "pack.zip"), "shaderpacks/pack.zip");
    }

    #[test]
    fn prefers_stable_releases() {
        let version = |id: &str, kind: &str| -> Version {
            serde_json::from_value(json!({
                "id": id, "project_id": "p", "version_number": id, "version_type": kind, "files": []
            }))
            .unwrap()
        };
        let mixed = vec![version("3", "beta"), version("2", "release"), version("1", "release")];
        assert_eq!(preferred_version(mixed).unwrap().id, "2");
        let only_beta = vec![version("3", "beta"), version("2", "alpha")];
        assert_eq!(preferred_version(only_beta).unwrap().id, "3");
        assert!(preferred_version(Vec::new()).is_none());
    }

    #[test]
    fn rejects_malformed_ids() {
        assert!(check_id("AANobbMI").is_ok());
        assert!(check_id("fabric-api").is_ok());
        assert!(check_id("../version").is_err());
        assert!(check_id("a/b").is_err());
        assert!(check_id("").is_err());
    }

    #[test]
    fn primary_file_falls_back_to_first() {
        let version: Version = serde_json::from_value(json!({
            "id": "v", "project_id": "p", "version_number": "1.0",
            "files": [
                {"url": "https://cdn.modrinth.com/a.jar", "filename": "a.jar", "primary": false, "size": 1, "hashes": {"sha1": "x", "sha512": "y"}},
                {"url": "https://cdn.modrinth.com/b.jar", "filename": "b.jar", "primary": true, "size": 2, "hashes": {"sha1": "z"}}
            ]
        }))
        .unwrap();
        assert_eq!(version.primary_file().unwrap().filename, "b.jar");
        assert!(version.dependencies.is_empty());
    }
}

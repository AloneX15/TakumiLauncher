//! Modrinth modpack (`.mrpack`) import.
//!
//! A pack is a zip with `modrinth.index.json` (game version, loader and a
//! list of files to download) plus `overrides/` directories that are copied
//! into the game directory as they are.

use crate::content;
use crate::install;
use crate::state::AppState;
use anyhow::{bail, Context};
use download_manager::{DownloadJob, DownloadProgress};
use instance_manager::NewInstance;
use minecraft_resolver::{is_safe_file_name, is_safe_relative_path};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use types::{Instance, ModLoader};

/// Hosts a pack may download from, as required by the format specification.
const ALLOWED_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];
const OVERRIDE_DIRS: &[&str] = &["overrides/", "client-overrides/"];
/// Upper bound for a single extracted override, against zip bombs.
const MAX_OVERRIDE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackIndex {
    format_version: u32,
    game: String,
    name: String,
    #[serde(default)]
    files: Vec<PackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    path: String,
    hashes: content::FileHashes,
    #[serde(default)]
    env: Option<PackEnv>,
    downloads: Vec<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PackEnv {
    #[serde(default)]
    client: Option<String>,
}

impl PackIndex {
    fn minecraft_version(&self) -> anyhow::Result<&str> {
        self.dependencies
            .get("minecraft")
            .map(String::as_str)
            .context("El modpack no indica la versión de Minecraft.")
    }

    /// The pack's loader and the version it asks for.
    fn loader(&self) -> (ModLoader, Option<&str>) {
        const KEYS: [(&str, ModLoader); 4] = [
            ("fabric-loader", ModLoader::Fabric),
            ("quilt-loader", ModLoader::Quilt),
            ("neoforge", ModLoader::NeoForge),
            ("forge", ModLoader::Forge),
        ];
        KEYS.iter()
            .find_map(|(key, loader)| self.dependencies.get(*key).map(|v| (*loader, Some(v.as_str()))))
            .unwrap_or((ModLoader::Vanilla, None))
    }
}

/// `https` URL on one of the allowed hosts.
fn is_allowed_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Userinfo or a port would make the "host" something else entirely.
    !host.contains(['@', ':']) && ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str())
}

fn read_index(pack: &Path) -> anyhow::Result<PackIndex> {
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(pack)?).context("El archivo no es un modpack válido.")?;
    let mut text = String::new();
    archive
        .by_name("modrinth.index.json")
        .context("No es un modpack de Modrinth (falta modrinth.index.json).")?
        .read_to_string(&mut text)?;
    let index: PackIndex = serde_json::from_str(&text).context("Índice del modpack no válido.")?;
    if index.format_version != 1 || index.game != "minecraft" {
        bail!("Formato de modpack no compatible.");
    }
    Ok(index)
}

/// Copy the pack's override directories into the game directory.
fn extract_overrides(pack: &Path, game_dir: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.replace('\\', "/");
        let Some(relative) = OVERRIDE_DIRS.iter().find_map(|dir| name.strip_prefix(dir)) else {
            continue;
        };
        if entry.is_dir() || !is_safe_relative_path(relative) {
            continue;
        }
        let destination = game_dir.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&destination)?;
        let copied = std::io::copy(&mut entry.by_ref().take(MAX_OVERRIDE_BYTES + 1), &mut output)?;
        if copied > MAX_OVERRIDE_BYTES {
            bail!("El modpack contiene un archivo demasiado grande: {relative}");
        }
    }
    Ok(())
}

/// Pick the loader version to install for a pack.
async fn resolve_loader_version(
    state: &AppState,
    loader: ModLoader,
    minecraft: &str,
    wanted: Option<&str>,
) -> anyhow::Result<Option<String>> {
    if loader == ModLoader::Vanilla {
        return Ok(None);
    }
    let available = install::loader_versions(state, loader, minecraft).await?;
    let chosen = match wanted {
        // Forge versions are listed as `<minecraft>-<build>`.
        Some(wanted) => available
            .iter()
            .find(|v| v.version == wanted || v.version == format!("{minecraft}-{wanted}")),
        None => install::default_loader_version(&available),
    };
    let chosen = chosen.with_context(|| {
        format!(
            "{loader} {} no está disponible para Minecraft {minecraft}.",
            wanted.unwrap_or_default()
        )
    })?;
    Ok(Some(chosen.version.clone()))
}

/// Create an instance from a `.mrpack` file.
pub async fn import(
    state: &AppState,
    pack: &Path,
    on_progress: &(dyn Fn(DownloadProgress) + Send + Sync),
) -> anyhow::Result<Instance> {
    let pack_path = pack.to_path_buf();
    let index = tokio::task::spawn_blocking(move || read_index(&pack_path)).await??;

    let minecraft = index.minecraft_version()?.to_string();
    state.version_manifest().await?.find(&minecraft)?;
    let (loader, wanted) = index.loader();
    let loader_version = resolve_loader_version(state, loader, &minecraft, wanted).await?;

    // Validate every file before anything is created on disk.
    let mut planned: Vec<(&PackFile, &str)> = Vec::new();
    for file in &index.files {
        if file.env.as_ref().and_then(|e| e.client.as_deref()) == Some("unsupported") {
            continue;
        }
        if !is_safe_relative_path(&file.path) {
            bail!("El modpack contiene una ruta no válida: {}", file.path);
        }
        let url = file
            .downloads
            .iter()
            .find(|url| is_allowed_download(url))
            .with_context(|| format!("El modpack descarga {} desde un sitio no permitido.", file.path))?;
        planned.push((file, url));
    }

    let name: String = index.name.chars().filter(|c| !c.is_control()).take(64).collect();
    let instance = state.instances.create(NewInstance {
        name: if name.trim().is_empty() { "Modpack".into() } else { name },
        minecraft_version: minecraft,
        mod_loader: loader,
        mod_loader_version: loader_version,
    })?;
    let game_dir = state.instances.game_dir(instance.id);

    let result: anyhow::Result<()> = async {
        let jobs: Vec<DownloadJob> = planned
            .iter()
            .map(|(file, url)| {
                DownloadJob::new(*url, game_dir.join(&file.path))
                    .sha1(Some(file.hashes.sha1.clone()))
                    .size(file.file_size)
            })
            .collect();
        state.downloader.download_all(jobs, on_progress).await?;

        let (pack_path, game_dir) = (pack.to_path_buf(), game_dir.clone());
        tokio::task::spawn_blocking(move || extract_overrides(&pack_path, &game_dir)).await??;
        Ok(())
    }
    .await;

    if let Err(error) = result {
        // Do not leave a half-installed instance behind.
        let _ = state.instances.delete(instance.id);
        return Err(error);
    }
    Ok(instance)
}

/// Download the newest version of a Modrinth modpack and import it.
pub async fn install_from_modrinth(
    state: &AppState,
    project: &str,
    on_progress: &(dyn Fn(DownloadProgress) + Send + Sync),
) -> anyhow::Result<Instance> {
    let versions = content::project_versions(state.http(), project, &[], None).await?;
    let version = versions
        .first()
        .context("Este modpack no tiene versiones publicadas.")?;
    let file = version
        .files
        .iter()
        .find(|f| f.primary && f.filename.ends_with(".mrpack"))
        .or(version.files.iter().find(|f| f.filename.ends_with(".mrpack")))
        .context("Este modpack no ofrece un archivo .mrpack.")?;
    if !is_safe_file_name(&file.filename) || !is_allowed_download(&file.url) {
        bail!("Descarga de modpack rechazada: {}", file.url);
    }

    let pack: PathBuf = state.root.join("tmp").join(&file.filename);
    let job = DownloadJob::new(&file.url, &pack)
        .sha1(Some(file.hashes.sha1.clone()))
        .size(file.size);
    state.downloader.download(&job).await?;
    let result = import(state, &pack, on_progress).await;
    let _ = tokio::fs::remove_file(&pack).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn download_allowlist() {
        assert!(is_allowed_download(
            "https://cdn.modrinth.com/data/x/versions/y/mod.jar"
        ));
        assert!(is_allowed_download(
            "https://github.com/a/b/releases/download/v1/mod.jar"
        ));
        assert!(!is_allowed_download("http://cdn.modrinth.com/data/mod.jar"));
        assert!(!is_allowed_download("https://evil.example/mod.jar"));
        assert!(!is_allowed_download("https://cdn.modrinth.com.evil.example/mod.jar"));
        assert!(!is_allowed_download("https://cdn.modrinth.com@evil.example/mod.jar"));
        assert!(!is_allowed_download("https://evil.example/?https://cdn.modrinth.com/"));
    }

    fn index(dependencies: &str) -> PackIndex {
        serde_json::from_str(&format!(
            r#"{{"formatVersion": 1, "game": "minecraft", "versionId": "1.0", "name": "Pack",
                "files": [], "dependencies": {dependencies}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn loader_detection() {
        let fabric = index(r#"{"minecraft": "1.21.1", "fabric-loader": "0.16.9"}"#);
        assert_eq!(fabric.minecraft_version().unwrap(), "1.21.1");
        assert_eq!(fabric.loader(), (ModLoader::Fabric, Some("0.16.9")));
        assert_eq!(
            index(r#"{"minecraft": "1.20.1", "forge": "47.4.0"}"#).loader(),
            (ModLoader::Forge, Some("47.4.0"))
        );
        assert_eq!(
            index(r#"{"minecraft": "1.21.1", "neoforge": "21.1.77"}"#).loader(),
            (ModLoader::NeoForge, Some("21.1.77"))
        );
        assert_eq!(index(r#"{"minecraft": "1.21.1"}"#).loader(), (ModLoader::Vanilla, None));
        assert!(index("{}").minecraft_version().is_err());
    }

    #[test]
    fn overrides_are_extracted_safely() {
        let dir = std::env::temp_dir().join(format!("takumi-pack-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pack = dir.join("pack.mrpack");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&pack).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("modrinth.index.json", "{}"),
                ("overrides/config/a.toml", "a"),
                ("client-overrides/options.txt", "b"),
                ("overrides/../../escape.txt", "evil"),
                ("server-overrides/server.properties", "s"),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(body.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }

        let game = dir.join("game");
        extract_overrides(&pack, &game).unwrap();
        assert_eq!(std::fs::read_to_string(game.join("config/a.toml")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(game.join("options.txt")).unwrap(), "b");
        assert!(!game.join("server.properties").exists());
        assert!(!dir.join("escape.txt").exists());
        assert!(read_index(&pack).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

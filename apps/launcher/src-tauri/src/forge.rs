//! Forge and NeoForge support.
//!
//! Both ship an installer jar containing the launcher profile
//! (`version.json`), bundled libraries (`maven/`) and, since 1.13, a list of
//! "processors": Java programs that patch the vanilla client. This module
//! performs what the installer's "install client" button does.

use crate::launch::LaunchSink;
use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use download_manager::{DownloadJob, DownloadProgress};
use minecraft_resolver::{
    fetch_json, fetch_text, is_safe_file_name, is_safe_relative_path, maven_path, Library, LoaderVersion, VersionInfo,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const FORGE_VERSIONS_URL: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json";
const FORGE_PROMOTIONS_URL: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_VERSIONS_URL: &str = "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Forge,
    NeoForge,
}

impl Family {
    fn slug(self) -> &'static str {
        match self {
            Family::Forge => "forge",
            Family::NeoForge => "neoforge",
        }
    }

    fn installer_url(self, version: &str) -> String {
        match self {
            Family::Forge => format!("{FORGE_MAVEN}/{version}/forge-{version}-installer.jar"),
            Family::NeoForge => format!("{NEOFORGE_MAVEN}/{version}/neoforge-{version}-installer.jar"),
        }
    }

    /// Directory (under `versions/`) holding this loader version's files.
    fn profile_dir(self, state: &AppState, version: &str) -> anyhow::Result<PathBuf> {
        let id = format!("{}-{version}", self.slug());
        if !is_safe_file_name(&id) {
            bail!("Versión del cargador no válida: {version}");
        }
        Ok(state.versions_dir().join(id))
    }

    /// Loader versions for a Minecraft version, newest first.
    pub async fn versions(
        self,
        http: &download_manager::reqwest::Client,
        minecraft: &str,
    ) -> anyhow::Result<Vec<LoaderVersion>> {
        match self {
            Family::Forge => {
                #[derive(Deserialize)]
                struct Promotions {
                    promos: HashMap<String, String>,
                }
                let all: HashMap<String, Vec<String>> = fetch_json(http, FORGE_VERSIONS_URL).await?;
                let promotions: Promotions = fetch_json(http, FORGE_PROMOTIONS_URL).await?;
                let recommended = promotions
                    .promos
                    .get(&format!("{minecraft}-recommended"))
                    .map(|build| format!("{minecraft}-{build}"));
                let mut versions = all.get(minecraft).cloned().unwrap_or_default();
                versions.reverse();
                Ok(versions
                    .into_iter()
                    .map(|version| LoaderVersion {
                        stable: recommended.as_ref().is_some_and(|r| version.starts_with(r.as_str())),
                        version,
                    })
                    .collect())
            }
            Family::NeoForge => {
                #[derive(Deserialize)]
                struct Listing {
                    versions: Vec<String>,
                }
                let Some(prefix) = neoforge_prefix(minecraft) else {
                    return Ok(Vec::new());
                };
                let listing: Listing = fetch_json(http, NEOFORGE_VERSIONS_URL).await?;
                let mut versions: Vec<String> = listing
                    .versions
                    .into_iter()
                    .filter(|v| v.starts_with(&prefix))
                    .collect();
                versions.reverse();
                Ok(versions
                    .into_iter()
                    .map(|version| LoaderVersion {
                        stable: !version.contains('-'),
                        version,
                    })
                    .collect())
            }
        }
    }
}

/// NeoForge versions mirror the Minecraft version: `1.21.1` -> `21.1.x`,
/// `1.21` -> `21.0.x`, `26.3` -> `26.3.0.x`.
fn neoforge_prefix(minecraft: &str) -> Option<String> {
    let parts: Vec<&str> = minecraft.split('.').collect();
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    match parts.as_slice() {
        ["1", minor] => Some(format!("{minor}.0.")),
        ["1", minor, patch] => Some(format!("{minor}.{patch}.")),
        [year, drop] if *year != "1" => Some(format!("{year}.{drop}.0.")),
        [year, drop, patch] => Some(format!("{year}.{drop}.{patch}.")),
        _ => None,
    }
}

/// Download the installer if needed and return the loader's version JSON.
/// Libraries bundled in the installer are unpacked into the library store.
pub async fn ensure_profile(state: &AppState, family: Family, version: &str) -> anyhow::Result<VersionInfo> {
    let dir = family.profile_dir(state, version)?;
    let json_path = dir.join("version.json");
    if let Some(info) = std::fs::read_to_string(&json_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        return Ok(info);
    }

    let url = family.installer_url(version);
    let installer = dir.join("installer.jar");
    // Maven publishes a checksum next to every artifact. The installer's code
    // runs on this machine, so it is never used unverified.
    let checksum = fetch_text(state.http(), &format!("{url}.sha1"))
        .await
        .with_context(|| {
            format!(
                "No se pudo obtener la suma de verificación del instalador de {}",
                family.slug()
            )
        })?;
    let sha1 = parse_sha1(&checksum)
        .with_context(|| format!("Suma de verificación no válida para el instalador de {}", family.slug()))?;
    state
        .downloader
        .download(&DownloadJob::new(&url, &installer).sha1(Some(sha1)))
        .await
        .with_context(|| format!("No se pudo descargar el instalador de {} {version}", family.slug()))?;

    let libraries_dir = state.libraries_dir();
    let text = tokio::task::spawn_blocking(move || unpack_installer(&installer, &dir, &libraries_dir)).await??;
    let info = serde_json::from_str(&text).context("Perfil del cargador de mods no válido.")?;
    // Written last: its presence marks the unpacking as complete.
    write_atomic(&json_path, text.as_bytes())?;
    Ok(info)
}

/// Extract the profile and bundled libraries; returns the version JSON text.
fn unpack_installer(installer: &Path, dir: &Path, libraries_dir: &Path) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(installer)?).context("Instalador corrupto.")?;
    let profile_text = read_entry(&mut archive, "install_profile.json")?;
    let profile: Value = serde_json::from_str(&profile_text).context("install_profile.json no válido.")?;

    let version_text = if let Some(entry) = profile["json"].as_str() {
        // Modern format: profile and libraries are separate entries.
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name()?.to_string();
            let Some(relative) = name.strip_prefix("maven/") else {
                continue;
            };
            if entry.is_dir() || !is_safe_relative_path(relative) {
                continue;
            }
            write_entry(&mut entry, &libraries_dir.join(relative))?;
        }
        read_entry(&mut archive, entry.trim_start_matches('/'))?
    } else if profile["versionInfo"].is_object() {
        // Legacy format (before 1.13): one embedded profile plus the Forge jar.
        let install = &profile["install"];
        let (Some(coordinate), Some(file)) = (install["path"].as_str(), install["filePath"].as_str()) else {
            bail!("install_profile.json antiguo sin datos de instalación.");
        };
        let relative = maven_path(coordinate)
            .filter(|p| is_safe_relative_path(p))
            .context("Ruta de Forge no válida.")?;
        let mut entry = archive.by_name(file)?;
        write_entry(&mut entry, &libraries_dir.join(relative))?;
        serde_json::to_string(&profile["versionInfo"])?
    } else {
        bail!("Formato de instalador no reconocido.");
    };

    write_atomic(&dir.join("install_profile.json"), profile_text.as_bytes())?;
    // Old profiles point at a host that now only redirects.
    Ok(version_text.replace(
        "http://files.minecraftforge.net/maven",
        "https://maven.minecraftforge.net",
    ))
}

fn read_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> anyhow::Result<String> {
    let mut text = String::new();
    archive
        .by_name(name)
        .with_context(|| format!("Falta {name} en el instalador."))?
        .read_to_string(&mut text)?;
    Ok(text)
}

fn write_entry(entry: &mut impl Read, destination: &Path) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::File::create(destination)?;
    std::io::copy(entry, &mut output)?;
    Ok(())
}

#[derive(Deserialize)]
struct InstallProfile {
    #[serde(default)]
    data: HashMap<String, SidedValue>,
    #[serde(default)]
    processors: Vec<Processor>,
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Deserialize)]
struct SidedValue {
    client: String,
}

#[derive(Deserialize)]
struct Processor {
    #[serde(default)]
    sides: Option<Vec<String>>,
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
}

/// What the processors need to know about the surrounding installation.
pub struct ProcessorContext<'a> {
    pub minecraft_version: &'a str,
    /// The vanilla client jar.
    pub client_jar: &'a Path,
    pub java: &'a Path,
}

/// Run the installer's client processors once per loader version.
pub async fn run_processors(
    state: &AppState,
    family: Family,
    version: &str,
    context: ProcessorContext<'_>,
    sink: &dyn LaunchSink,
) -> anyhow::Result<()> {
    let dir = family.profile_dir(state, version)?;
    let marker = dir.join(".processed");
    if marker.is_file() {
        return Ok(());
    }
    let profile_text = tokio::fs::read_to_string(dir.join("install_profile.json")).await?;
    // Legacy profiles have a different shape and no processors.
    let profile: InstallProfile = match serde_json::from_str::<Value>(&profile_text)? {
        value if value["processors"].is_array() => serde_json::from_value(value)?,
        _ => {
            tokio::fs::write(&marker, b"").await?;
            return Ok(());
        }
    };
    let processors: Vec<&Processor> = profile
        .processors
        .iter()
        .filter(|p| p.sides.as_ref().is_none_or(|sides| sides.iter().any(|s| s == "client")))
        .collect();

    sink.status("loader", Some(DownloadProgress::default()));
    let libraries_dir = state.libraries_dir();
    let mut jobs = Vec::new();
    for library in &profile.libraries {
        let Some(artifact) = library.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let path = artifact.path.clone().or_else(|| maven_path(&library.name));
        match path {
            // An empty URL means the file came bundled in the installer.
            Some(path) if is_safe_relative_path(&path) && !artifact.url.is_empty() => jobs.push(
                DownloadJob::new(&artifact.url, libraries_dir.join(path))
                    .sha1(artifact.sha1.clone())
                    .size(artifact.size),
            ),
            _ => {}
        }
    }
    state
        .downloader
        .download_all(jobs, |progress| sink.status("loader", Some(progress)))
        .await?;

    // Resolve the installer's data table into concrete values.
    let installer = dir.join("installer.jar");
    let mut data: HashMap<String, String> = HashMap::new();
    {
        let entries: Vec<(String, String)> = profile
            .data
            .iter()
            .map(|(k, v)| (k.clone(), v.client.clone()))
            .collect();
        let (installer, data_dir, libraries_dir) = (installer.clone(), dir.join("data"), libraries_dir.clone());
        let resolved = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<(String, String)>> {
            let mut archive = zip::ZipArchive::new(std::fs::File::open(&installer)?)?;
            let mut out = Vec::new();
            for (key, value) in entries {
                let value = if let Some(path) = artifact_reference(&value, &libraries_dir) {
                    path
                } else if let Some(literal) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
                    literal.to_string()
                } else {
                    // A file inside the installer.
                    let name = value.trim_start_matches('/');
                    if !is_safe_relative_path(name) {
                        bail!("Entrada de datos no válida en el instalador: {value}");
                    }
                    let destination = data_dir.join(name);
                    write_entry(&mut archive.by_name(name)?, &destination)?;
                    destination.to_string_lossy().into_owned()
                };
                out.push((key, value));
            }
            Ok(out)
        })
        .await??;
        data.extend(resolved);
    }
    let text = |path: &Path| path.to_string_lossy().into_owned();
    data.insert("SIDE".into(), "client".into());
    data.insert("MINECRAFT_JAR".into(), text(context.client_jar));
    data.insert("MINECRAFT_VERSION".into(), context.minecraft_version.to_string());
    data.insert("ROOT".into(), text(&state.root));
    data.insert("INSTALLER".into(), text(&installer));
    data.insert("LIBRARY_DIR".into(), text(&libraries_dir));

    let separator = if cfg!(windows) { ";" } else { ":" };
    for (index, processor) in processors.iter().enumerate() {
        sink.status(
            "loader",
            Some(DownloadProgress {
                files_done: index as u64,
                files_total: processors.len() as u64,
                ..DownloadProgress::default()
            }),
        );
        let jar = library_path(&processor.jar, &libraries_dir)?;
        let main_class = {
            let jar = jar.clone();
            tokio::task::spawn_blocking(move || main_class_of(&jar)).await??
        };
        let mut classpath = vec![text(&jar)];
        for entry in &processor.classpath {
            classpath.push(text(&library_path(entry, &libraries_dir)?));
        }
        let args: Vec<String> = processor
            .args
            .iter()
            .map(|arg| expand_argument(arg, &data, &libraries_dir))
            .collect();

        let mut command = tokio::process::Command::new(context.java);
        command
            .arg("-cp")
            .arg(classpath.join(separator))
            .arg(&main_class)
            .args(&args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let output = command.output().await.context("No se pudo ejecutar Java.")?;
        if !output.status.success() {
            let log = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = log
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            bail!(
                "La instalación de {} falló en el paso {} ({main_class}):\n{}",
                family.slug(),
                index + 1,
                tail.join("\n")
            );
        }
    }

    tokio::fs::write(&marker, b"").await?;
    Ok(())
}

/// `[group:artifact:version]` -> absolute path in the library store.
fn artifact_reference(value: &str, libraries_dir: &Path) -> Option<String> {
    let coordinate = value.strip_prefix('[')?.strip_suffix(']')?;
    let relative = maven_path(coordinate).filter(|p| is_safe_relative_path(p))?;
    Some(libraries_dir.join(relative).to_string_lossy().into_owned())
}

fn library_path(coordinate: &str, libraries_dir: &Path) -> anyhow::Result<PathBuf> {
    let relative = maven_path(coordinate)
        .filter(|p| is_safe_relative_path(p))
        .with_context(|| format!("Coordenada de librería no válida: {coordinate}"))?;
    Ok(libraries_dir.join(relative))
}

fn expand_argument(argument: &str, data: &HashMap<String, String>, libraries_dir: &Path) -> String {
    if let Some(path) = artifact_reference(argument, libraries_dir) {
        return path;
    }
    let mut out = argument.to_string();
    if out.contains('{') {
        for (key, value) in data {
            out = out.replace(&format!("{{{key}}}"), value);
        }
    }
    out
}

fn main_class_of(jar: &Path) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
    let mut manifest = String::new();
    archive.by_name("META-INF/MANIFEST.MF")?.read_to_string(&mut manifest)?;
    manifest
        .lines()
        .find_map(|line| line.strip_prefix("Main-Class:"))
        .map(|class| class.trim().to_string())
        .with_context(|| format!("{} no declara Main-Class.", jar.display()))
}

/// The hex SHA-1 in a Maven `.sha1` file (some also carry the file name after it).
fn parse_sha1(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_sha1_files() {
        let hash = "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3";
        assert_eq!(parse_sha1(hash).as_deref(), Some(hash));
        assert_eq!(
            parse_sha1(&format!("{}  forge-installer.jar\n", hash.to_uppercase())).as_deref(),
            Some(hash)
        );
        assert_eq!(parse_sha1("<html>404</html>"), None);
        assert_eq!(parse_sha1(""), None);
        assert_eq!(parse_sha1("zz4a8fe5ccb19ba61c4c0873d391e987982fbbd3"), None);
    }

    #[test]
    fn neoforge_prefixes() {
        assert_eq!(neoforge_prefix("1.21.1").unwrap(), "21.1.");
        assert_eq!(neoforge_prefix("1.21").unwrap(), "21.0.");
        assert_eq!(neoforge_prefix("1.20.2").unwrap(), "20.2.");
        assert_eq!(neoforge_prefix("26.3").unwrap(), "26.3.0.");
        assert_eq!(neoforge_prefix("26.3.1").unwrap(), "26.3.1.");
        assert!(neoforge_prefix("24w14a").is_none());
    }

    #[test]
    fn argument_expansion() {
        let libs = Path::new("libs");
        let data = HashMap::from([
            ("SIDE".to_string(), "client".to_string()),
            ("BINPATCH".to_string(), "data/client.lzma".to_string()),
        ]);
        assert_eq!(expand_argument("--side={SIDE}", &data, libs), "--side=client");
        assert_eq!(expand_argument("{BINPATCH}", &data, libs), "data/client.lzma");
        assert_eq!(expand_argument("--plain", &data, libs), "--plain");
        let path = expand_argument("[net.minecraft:client:1.21.1:slim]", &data, libs);
        assert!(path
            .replace('\\', "/")
            .ends_with("libs/net/minecraft/client/1.21.1/client-1.21.1-slim.jar"));
        assert!(artifact_reference("[../../evil:x:1]", libs).is_none());
    }

    #[test]
    fn installer_urls() {
        assert_eq!(
            Family::Forge.installer_url("1.20.1-47.4.0"),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-installer.jar"
        );
        assert_eq!(
            Family::NeoForge.installer_url("21.1.77"),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.77/neoforge-21.1.77-installer.jar"
        );
    }
}

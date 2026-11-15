//! Makes an instance ready to launch: version metadata, client jar,
//! libraries, assets, natives and a Java runtime.

use crate::forge::{self, Family, ProcessorContext};
use crate::launch::LaunchSink;
use crate::settings::Settings;
use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use download_manager::{DownloadJob, DownloadProgress};
use minecraft_resolver::{
    fetch_json, fetch_text, is_safe_file_name, is_safe_relative_path, java_executable_path, java_runtime_platform,
    merge, resolve_libraries, AssetIndex, Env, JavaRuntimeIndex, JavaRuntimeManifest, LibraryKind, LoaderKind,
    LoaderVersion, VersionInfo, ASSET_BASE_URL, JAVA_RUNTIME_URL,
};
use std::path::{Path, PathBuf};
use types::{Instance, ModLoader};

/// Everything needed to build the launch command.
pub struct Prepared {
    pub info: VersionInfo,
    pub classpath: Vec<PathBuf>,
    pub java: PathBuf,
    pub game_dir: PathBuf,
    pub natives_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub libraries_dir: PathBuf,
    /// Asset directory for legacy versions that read a plain file tree.
    pub game_assets_dir: PathBuf,
    /// JVM argument pointing at the downloaded log4j configuration.
    pub logging_argument: Option<String>,
}

/// How a mod loader is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderSource {
    /// A ready-made launcher profile (Fabric, Quilt).
    Profile(LoaderKind),
    /// An installer jar that has to be processed (Forge, NeoForge).
    Installer(Family),
}

pub fn loader_source(loader: ModLoader) -> Option<LoaderSource> {
    match loader {
        ModLoader::Vanilla => None,
        ModLoader::Fabric => Some(LoaderSource::Profile(LoaderKind::Fabric)),
        ModLoader::Quilt => Some(LoaderSource::Profile(LoaderKind::Quilt)),
        ModLoader::Forge => Some(LoaderSource::Installer(Family::Forge)),
        ModLoader::NeoForge => Some(LoaderSource::Installer(Family::NeoForge)),
    }
}

/// Loader versions available for a Minecraft version, newest first.
pub async fn loader_versions(
    state: &AppState,
    loader: ModLoader,
    minecraft: &str,
) -> anyhow::Result<Vec<LoaderVersion>> {
    match loader_source(loader) {
        None => Ok(Vec::new()),
        Some(LoaderSource::Profile(kind)) => Ok(kind.versions(state.http(), minecraft).await?),
        Some(LoaderSource::Installer(family)) => family.versions(state.http(), minecraft).await,
    }
}

/// The version a new instance gets when none is chosen explicitly.
pub fn default_loader_version(available: &[LoaderVersion]) -> Option<&LoaderVersion> {
    available.iter().find(|v| v.stable).or(available.first())
}

/// Download and check everything the instance needs to start.
///
/// With `repair`, files already on disk are re-hashed instead of trusted by
/// size, and cached version metadata and the Java runtime are checked again,
/// so corrupted or modified files are downloaded anew.
pub async fn prepare(
    state: &AppState,
    instance: &Instance,
    settings: &Settings,
    sink: &dyn LaunchSink,
    repair: bool,
) -> anyhow::Result<Prepared> {
    sink.status("metadata", None);
    let env = Env::current();
    let info = resolve_version(state, instance, repair).await?;

    let mut jobs = Vec::new();

    let installer = match loader_source(instance.mod_loader) {
        Some(LoaderSource::Installer(family)) => Some(family),
        _ => None,
    };

    // Client jar (always the vanilla one; loaders patch it at runtime).
    let client = info
        .downloads
        .as_ref()
        .and_then(|d| d.client.as_ref())
        .context("Esta versión no incluye un cliente descargable.")?;
    let version = &instance.minecraft_version;
    let client_jar = state.versions_dir().join(version).join(format!("{version}.jar"));
    jobs.push(
        DownloadJob::new(&client.url, &client_jar)
            .sha1(client.sha1.clone())
            .size(client.size)
            .verify_existing(repair),
    );

    // Libraries
    let libraries_dir = state.libraries_dir();
    let mut classpath = Vec::new();
    let mut native_jars = Vec::new();
    for library in resolve_libraries(&info, &env) {
        let destination = libraries_dir.join(&library.path);
        // An empty URL marks a file that the loader's installer provides.
        if !library.url.is_empty() {
            jobs.push(
                DownloadJob::new(&library.url, &destination)
                    .sha1(library.sha1)
                    .size(library.size)
                    .verify_existing(repair),
            );
        }
        match library.kind {
            LibraryKind::Classpath => classpath.push(destination),
            LibraryKind::Native => native_jars.push(destination),
        }
    }
    // Forge expects the game jar to be named after its own version id, as
    // the official launcher does for inherited versions.
    let launch_jar = match installer {
        Some(_) if is_safe_file_name(&info.id) => client_jar.with_file_name(format!("{}.jar", info.id)),
        _ => client_jar.clone(),
    };
    classpath.push(launch_jar.clone());

    // Assets
    let assets_dir = state.assets_dir();
    let asset_ref = info
        .asset_index
        .as_ref()
        .context("Esta versión no declara un índice de recursos.")?;
    if !is_safe_file_name(&asset_ref.id) {
        bail!("Índice de recursos no válido: {}", asset_ref.id);
    }
    let index_path = assets_dir.join("indexes").join(format!("{}.json", asset_ref.id));
    let index_job = DownloadJob::new(&asset_ref.url, &index_path)
        .sha1(asset_ref.sha1.clone())
        .size(asset_ref.size)
        .verify_existing(repair);
    state.downloader.download_all(vec![index_job], |_| {}).await?;
    let index: AssetIndex =
        serde_json::from_str(&tokio::fs::read_to_string(&index_path).await?).context("Índice de recursos corrupto.")?;

    let objects_dir = assets_dir.join("objects");
    for object in index.objects.values() {
        let Some(relative) = object.relative_path() else {
            continue;
        };
        jobs.push(
            DownloadJob::new(format!("{ASSET_BASE_URL}{relative}"), objects_dir.join(&relative))
                .sha1(Some(object.hash.clone()))
                .size(Some(object.size))
                .verify_existing(repair),
        );
    }

    // Log4j configuration (Mojang's patched configs for old versions).
    let mut logging_argument = None;
    if let Some(logging) = info.logging.as_ref().and_then(|l| l.client.as_ref()) {
        if is_safe_file_name(&logging.file.id) {
            let path = assets_dir.join("log_configs").join(&logging.file.id);
            jobs.push(
                DownloadJob::new(&logging.file.url, &path)
                    .sha1(logging.file.sha1.clone())
                    .size(logging.file.size)
                    .verify_existing(repair),
            );
            logging_argument = Some(logging.argument.replace("${path}", &path.to_string_lossy()));
        }
    }

    sink.status("downloading", Some(DownloadProgress::default()));
    state
        .downloader
        .download_all(jobs, |progress| sink.status("downloading", Some(progress)))
        .await?;

    if launch_jar != client_jar {
        let up_to_date = match (std::fs::metadata(&launch_jar), std::fs::metadata(&client_jar)) {
            (Ok(copy), Ok(original)) => copy.len() == original.len(),
            _ => false,
        };
        if !up_to_date {
            tokio::fs::copy(&client_jar, &launch_jar).await?;
        }
    }

    let game_dir = state.instances.game_dir(instance.id);
    tokio::fs::create_dir_all(&game_dir).await?;
    let game_assets_dir = link_legacy_assets(&index, &asset_ref.id, &assets_dir, &game_dir).await?;

    sink.status("natives", None);
    let natives_dir = state.instances.natives_dir(instance.id);
    {
        let natives_dir = natives_dir.clone();
        tokio::task::spawn_blocking(move || extract_natives(&native_jars, &natives_dir)).await??;
    }

    let custom_java = instance
        .settings
        .java_path
        .clone()
        .or_else(|| settings.java_path.clone())
        .filter(|path| !path.trim().is_empty());
    let java = match custom_java {
        Some(path) => {
            let path = PathBuf::from(path.trim());
            if !path.is_file() {
                bail!("No se encuentra el ejecutable de Java configurado: {}", path.display());
            }
            path
        }
        None => {
            let component = info
                .java_version
                .as_ref()
                .map(|java| java.component.as_str())
                .unwrap_or("jre-legacy");
            ensure_java_runtime(state, component, sink, repair).await?
        }
    };

    if let Some(family) = installer {
        let loader_version = instance.mod_loader_version.as_deref().unwrap_or_default();
        let context = ProcessorContext {
            minecraft_version: version,
            client_jar: &client_jar,
            java: &java,
        };
        forge::run_processors(state, family, loader_version, context, sink).await?;
    }

    Ok(Prepared {
        info,
        classpath,
        java,
        game_dir,
        natives_dir,
        assets_dir,
        libraries_dir,
        game_assets_dir,
        logging_argument,
    })
}

/// Load the instance's version metadata, merged with the mod loader profile.
async fn resolve_version(state: &AppState, instance: &Instance, repair: bool) -> anyhow::Result<VersionInfo> {
    let version = &instance.minecraft_version;
    if !is_safe_file_name(version) {
        bail!("Versión de Minecraft no válida: {version}");
    }
    let vanilla = load_vanilla_version(state, version, repair).await?;

    let Some(source) = loader_source(instance.mod_loader) else {
        return Ok(vanilla);
    };
    let loader_version = instance
        .mod_loader_version
        .as_deref()
        .context("La instancia no tiene versión del cargador de mods.")?;
    let kind = match source {
        LoaderSource::Profile(kind) => kind,
        LoaderSource::Installer(family) => {
            let profile = forge::ensure_profile(state, family, loader_version).await?;
            return Ok(merge(profile, vanilla));
        }
    };
    let id = kind.profile_id(version, loader_version);
    if !is_safe_file_name(&id) {
        bail!("Versión del cargador no válida: {loader_version}");
    }

    let path = state.versions_dir().join(&id).join(format!("{id}.json"));
    // The loader profile has no published checksum: repair fetches it again.
    let cached = (!repair)
        .then(|| std::fs::read_to_string(&path).ok())
        .flatten()
        .and_then(|text| serde_json::from_str::<VersionInfo>(&text).ok());
    let profile = match cached {
        Some(profile) => profile,
        None => {
            let text = fetch_text(state.http(), &kind.profile_url(version, loader_version)).await?;
            let profile: VersionInfo = serde_json::from_str(&text).context("Perfil del cargador de mods no válido.")?;
            write_atomic(&path, text.as_bytes())?;
            profile
        }
    };
    Ok(merge(profile, vanilla))
}

async fn load_vanilla_version(state: &AppState, version: &str, repair: bool) -> anyhow::Result<VersionInfo> {
    let path = state.versions_dir().join(version).join(format!("{version}.json"));
    if !repair {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(info) = serde_json::from_str(&text) {
                return Ok(info);
            }
        }
    }
    let manifest = state.version_manifest().await?;
    let entry = manifest.find(version)?;
    // Repair keeps the cached file when it still matches the manifest's hash.
    state
        .downloader
        .download_all(
            vec![DownloadJob::new(&entry.url, &path)
                .sha1(entry.sha1.clone())
                .verify_existing(true)],
            |_| {},
        )
        .await?;
    serde_json::from_str(&tokio::fs::read_to_string(&path).await?).context("Metadatos de versión no válidos.")
}

/// Legacy versions read assets by name instead of by hash. Returns the
/// directory to pass as `${game_assets}`.
async fn link_legacy_assets(
    index: &AssetIndex,
    index_id: &str,
    assets_dir: &Path,
    game_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let virtual_dir = assets_dir.join("virtual").join(index_id);
    let target = if index.map_to_resources {
        game_dir.join("resources")
    } else if index.is_virtual {
        virtual_dir.clone()
    } else {
        return Ok(virtual_dir);
    };

    for (name, object) in &index.objects {
        let (true, Some(relative)) = (is_safe_relative_path(name), object.relative_path()) else {
            continue;
        };
        let destination = target.join(name);
        let up_to_date = tokio::fs::metadata(&destination)
            .await
            .is_ok_and(|meta| meta.len() == object.size);
        if !up_to_date {
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(assets_dir.join("objects").join(relative), &destination).await?;
        }
    }
    Ok(target)
}

/// Extract native libraries from legacy natives jars into a clean directory.
fn extract_natives(jars: &[PathBuf], natives_dir: &Path) -> anyhow::Result<()> {
    if natives_dir.exists() {
        std::fs::remove_dir_all(natives_dir)?;
    }
    std::fs::create_dir_all(natives_dir)?;

    for jar in jars {
        let file = std::fs::File::open(jar)?;
        let mut archive = zip::ZipArchive::new(file).with_context(|| format!("jar corrupto: {}", jar.display()))?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            // Natives are flat; taking only the file name also rules out
            // entries that try to escape the directory.
            let Some(name) = entry
                .enclosed_name()
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
            else {
                continue;
            };
            let lower = name.to_string_lossy().to_lowercase();
            if ![".dll", ".so", ".dylib", ".jnilib"]
                .iter()
                .any(|ext| lower.ends_with(ext))
            {
                continue;
            }
            let mut output = std::fs::File::create(natives_dir.join(name))?;
            std::io::copy(&mut entry, &mut output)?;
        }
    }
    Ok(())
}

/// Install Mojang's Java runtime `component` if needed and return the path
/// of its executable.
async fn ensure_java_runtime(
    state: &AppState,
    component: &str,
    sink: &dyn LaunchSink,
    repair: bool,
) -> anyhow::Result<PathBuf> {
    if !is_safe_file_name(component) {
        bail!("Componente de Java no válido: {component}");
    }
    let dir = state.runtimes_dir().join(component);
    let executable = dir.join(java_executable_path());
    let marker = dir.join(".installed");
    if !repair && marker.is_file() && executable.is_file() {
        return Ok(executable);
    }

    sink.status("java", None);
    const MANUAL: &str = "Indica la ruta de un Java instalado en Ajustes.";
    let platform =
        java_runtime_platform().with_context(|| format!("No hay Java automático para este sistema. {MANUAL}"))?;
    let index: JavaRuntimeIndex = fetch_json(state.http(), JAVA_RUNTIME_URL).await?;
    let entry = index
        .get(platform)
        .and_then(|components| components.get(component))
        .and_then(|releases| releases.first())
        .with_context(|| format!("Mojang no publica {component} para {platform}. {MANUAL}"))?;
    let manifest: JavaRuntimeManifest = fetch_json(state.http(), &entry.manifest.url).await?;

    let mut jobs = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (name, file) in &manifest.files {
        if !is_safe_relative_path(name) {
            continue;
        }
        let path = dir.join(name);
        match file.kind.as_str() {
            "directory" => tokio::fs::create_dir_all(&path).await?,
            "file" => {
                let Some(raw) = file.downloads.as_ref().and_then(|d| d.raw.as_ref()) else {
                    continue;
                };
                jobs.push(
                    DownloadJob::new(&raw.url, &path)
                        .sha1(raw.sha1.clone())
                        .size(raw.size)
                        .verify_existing(repair),
                );
                if file.executable {
                    executables.push(path);
                }
            }
            "link" => {
                // A link may only point at another file of the same runtime.
                if let Some(target) = file.target.as_ref().filter(|target| link_stays_inside(name, target)) {
                    links.push((path, target.clone()));
                }
            }
            _ => {}
        }
    }

    sink.status("java", Some(DownloadProgress::default()));
    state
        .downloader
        .download_all(jobs, |progress| sink.status("java", Some(progress)))
        .await?;
    finish_runtime_install(&executables, &links)?;

    if !executable.is_file() {
        bail!("La instalación de Java está incompleta ({component}).");
    }
    tokio::fs::write(&marker, b"").await?;
    Ok(executable)
}

/// Whether a symlink at `link` (relative to the runtime directory) pointing at
/// `target` (relative to the link's own directory) resolves inside the runtime.
fn link_stays_inside(link: &str, target: &str) -> bool {
    if target.is_empty() || target.starts_with(['/', '\\']) || target.contains(':') {
        return false;
    }
    let mut parts: Vec<&str> = link.split('/').collect();
    parts.pop(); // the link's own name
    for part in target.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return false;
                }
            }
            other => parts.push(other),
        }
    }
    !parts.is_empty()
}

#[cfg(unix)]
fn finish_runtime_install(executables: &[PathBuf], links: &[(PathBuf, String)]) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for path in executables {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    for (path, target) in links {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(path);
        std::os::unix::fs::symlink(target, path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn finish_runtime_install(_executables: &[PathBuf], _links: &[(PathBuf, String)]) -> anyhow::Result<()> {
    // Windows runtimes contain no links and need no executable bit.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn runtime_links_stay_inside_the_runtime() {
        assert!(link_stays_inside(
            "jre.bundle/Contents/MacOS/libjli.dylib",
            "../Home/lib/libjli.dylib"
        ));
        assert!(link_stays_inside("bin/java", "../lib/java"));
        assert!(link_stays_inside("legal/java.base/LICENSE", "../../legal/LICENSE"));
        assert!(!link_stays_inside("bin/java", "../../evil"));
        assert!(!link_stays_inside("java", "../x"));
        assert!(!link_stays_inside("bin/java", "/usr/bin/evil"));
        assert!(!link_stays_inside("bin/java", "C:\\evil"));
        assert!(!link_stays_inside("bin/java", ""));
    }

    #[test]
    fn extracts_only_native_libraries_flat() {
        let dir = std::env::temp_dir().join(format!("takumi-natives-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("natives.jar");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("lwjgl64.dll", "dll"),
                ("nested/libopenal.so", "so"),
                ("META-INF/MANIFEST.MF", "manifest"),
                ("../escape.dll", "evil"),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(body.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }

        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("stale.dll"), "old").unwrap();
        extract_natives(&[jar], &out).unwrap();

        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["libopenal.so", "lwjgl64.dll"]);
        assert!(!dir.join("escape.dll").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loader_sources() {
        assert!(loader_source(ModLoader::Vanilla).is_none());
        assert_eq!(
            loader_source(ModLoader::Fabric),
            Some(LoaderSource::Profile(LoaderKind::Fabric))
        );
        assert_eq!(
            loader_source(ModLoader::NeoForge),
            Some(LoaderSource::Installer(Family::NeoForge))
        );
    }

    #[test]
    fn default_loader_version_prefers_stable() {
        let version = |v: &str, stable| LoaderVersion {
            version: v.into(),
            stable,
        };
        let list = [version("3-beta", false), version("2", true), version("1", true)];
        assert_eq!(default_loader_version(&list).unwrap().version, "2");
        assert_eq!(default_loader_version(&list[..1]).unwrap().version, "3-beta");
        assert!(default_loader_version(&[]).is_none());
    }
}

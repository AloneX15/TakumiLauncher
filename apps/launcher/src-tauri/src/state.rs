//! Shared application state and the on-disk layout of the launcher data.
//!
//! ```text
//! <root>/versions/<id>/<id>.{json,jar}   version metadata and client jars
//! <root>/libraries/                      shared Maven-layout libraries
//! <root>/assets/{indexes,objects}/       shared game assets
//! <root>/runtimes/<component>/           Java runtimes
//! <root>/instances/<uuid>/               per-instance data
//! <root>/{accounts,settings}.json
//! ```

use anyhow::Context;
use auth_system::{MinecraftSession, TokenVault};
use download_manager::{DownloadConfig, Downloader};
use instance_manager::InstanceStore;
use minecraft_resolver::{fetch_json, VersionManifest, VERSION_MANIFEST_URL};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::oneshot;
use uuid::Uuid;

pub struct AppState {
    pub root: PathBuf,
    pub downloader: Downloader,
    pub instances: InstanceStore,
    pub vault: TokenVault,
    manifest: tokio::sync::Mutex<Option<VersionManifest>>,
    /// Instances being prepared or running, with their stop signal. The
    /// signal is `None` once a stop has been requested; the entry itself
    /// stays until the launch task has finished.
    pub running: Mutex<HashMap<Uuid, Option<oneshot::Sender<()>>>>,
    /// Minecraft sessions by account id; kept in memory only.
    pub sessions: Mutex<HashMap<String, MinecraftSession>>,
    /// In-flight Microsoft sign-in, if any.
    pub login_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AppState {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root).with_context(|| format!("no se pudo crear {}", root.display()))?;
        Ok(Self {
            downloader: Downloader::new(DownloadConfig::default())?,
            instances: InstanceStore::new(root.join("instances"))?,
            vault: TokenVault::new("TakumiLauncher"),
            manifest: tokio::sync::Mutex::new(None),
            running: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            login_task: Mutex::new(None),
            root,
        })
    }

    pub fn http(&self) -> &download_manager::reqwest::Client {
        self.downloader.client()
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn runtimes_dir(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    /// The official version list: fetched once per session, with the last
    /// downloaded copy as an offline fallback.
    pub async fn version_manifest(&self) -> anyhow::Result<VersionManifest> {
        let mut cached = self.manifest.lock().await;
        if let Some(manifest) = cached.as_ref() {
            return Ok(manifest.clone());
        }

        let cache_file = self.root.join("version_manifest.json");
        let manifest = match fetch_json::<VersionManifest>(self.http(), VERSION_MANIFEST_URL).await {
            Ok(manifest) => {
                if let Ok(bytes) = serde_json::to_vec(&manifest) {
                    let _ = write_atomic(&cache_file, &bytes);
                }
                manifest
            }
            Err(error) => {
                // Not memoised, so the next call retries the network.
                let text = std::fs::read_to_string(&cache_file).map_err(|_| error)?;
                return serde_json::from_str(&text).context("lista de versiones en caché corrupta");
            }
        };
        *cached = Some(manifest.clone());
        Ok(manifest)
    }
}

/// Write a file via a temporary sibling so readers never see partial content.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

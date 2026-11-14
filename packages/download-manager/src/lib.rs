//! TakumiLauncher - Download Manager
//!
//! Parallel download engine with retry and hash verification.

use futures::stream::{self, StreamExt};
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

pub use reqwest;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("Descarga fallida: {0}")]
    DownloadFailed(String),

    #[error("La verificación de integridad falló para {0}")]
    HashVerificationFailed(String),

    #[error("Error de E/S: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Error de red: {0}")]
    NetworkError(String),
}

pub type DownloadResult<T> = Result<T, DownloadError>;

/// Download configuration
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub max_concurrent_downloads: usize,
    pub retry_attempts: u32,
    /// Maximum time to wait for the next chunk of a response.
    pub stall_timeout_seconds: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 16,
            retry_attempts: 3,
            stall_timeout_seconds: 30,
        }
    }
}

/// Expected checksum of a downloaded file (lowercase or uppercase hex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha1(String),
    Sha256(String),
}

impl Checksum {
    fn expected(&self) -> &str {
        match self {
            Checksum::Sha1(h) | Checksum::Sha256(h) => h,
        }
    }

    /// Check `data` against this checksum.
    pub fn matches(&self, data: &[u8]) -> bool {
        let mut hasher = Hasher::new(Some(self));
        hasher.update(data);
        hasher.finish().is_some_and(|h| h.eq_ignore_ascii_case(self.expected()))
    }
}

enum Hasher {
    Sha1(Sha1),
    Sha256(Sha256),
    None,
}

impl Hasher {
    fn new(checksum: Option<&Checksum>) -> Self {
        match checksum {
            Some(Checksum::Sha1(_)) => Hasher::Sha1(Sha1::new()),
            Some(Checksum::Sha256(_)) => Hasher::Sha256(Sha256::new()),
            None => Hasher::None,
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::None => {}
        }
    }

    fn finish(self) -> Option<String> {
        match self {
            Hasher::Sha1(h) => Some(hex::encode(h.finalize())),
            Hasher::Sha256(h) => Some(hex::encode(h.finalize())),
            Hasher::None => None,
        }
    }
}

/// Download job
#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub url: String,
    pub destination: PathBuf,
    pub checksum: Option<Checksum>,
    pub size: Option<u64>,
}

impl DownloadJob {
    pub fn new(url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            checksum: None,
            size: None,
        }
    }

    pub fn sha1(mut self, hash: Option<String>) -> Self {
        self.checksum = hash.map(Checksum::Sha1);
        self
    }

    pub fn size(mut self, size: Option<u64>) -> Self {
        self.size = size;
        self
    }

    /// Whether the destination already holds this file.
    ///
    /// When the size is known it is compared (cheap, used for the thousands of
    /// asset objects); otherwise the checksum is computed if there is one.
    fn already_present(&self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.destination) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        match (self.size, &self.checksum) {
            (Some(size), _) => meta.len() == size,
            (None, Some(checksum)) => std::fs::read(&self.destination)
                .map(|data| checksum.matches(&data))
                .unwrap_or(false),
            (None, None) => true,
        }
    }
}

/// Aggregate progress of a batch of downloads
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct DownloadProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

type ProgressFn<'a> = &'a (dyn Fn(DownloadProgress) + Send + Sync);

struct Tracker<'a> {
    files_total: u64,
    bytes_total: u64,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    last_emit: Mutex<Instant>,
    callback: ProgressFn<'a>,
}

impl Tracker<'_> {
    fn emit(&self, force: bool) {
        {
            let mut last = self.last_emit.lock().unwrap();
            if !force && last.elapsed() < Duration::from_millis(100) {
                return;
            }
            *last = Instant::now();
        }
        (self.callback)(DownloadProgress {
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total,
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total,
        });
    }
}

/// Download engine
pub struct Downloader {
    client: reqwest::Client,
    config: DownloadConfig,
}

impl Downloader {
    pub fn new(config: DownloadConfig) -> DownloadResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("TakumiLauncher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| DownloadError::NetworkError(e.to_string()))?;
        Ok(Self { client, config })
    }

    /// Shared HTTP client (connection pool reuse for metadata requests).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Download every job that is not already present on disk.
    ///
    /// Jobs pointing at the same destination are deduplicated. The first
    /// failure (after retries) aborts the batch.
    pub async fn download_all<F>(&self, jobs: Vec<DownloadJob>, on_progress: F) -> DownloadResult<()>
    where
        F: Fn(DownloadProgress) + Send + Sync,
    {
        let mut seen = HashSet::new();
        let pending: Vec<DownloadJob> = jobs
            .into_iter()
            .filter(|job| seen.insert(job.destination.clone()))
            .filter(|job| !job.already_present())
            .collect();

        let tracker = Tracker {
            files_total: pending.len() as u64,
            bytes_total: pending.iter().filter_map(|j| j.size).sum(),
            files_done: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            last_emit: Mutex::new(Instant::now()),
            callback: &on_progress,
        };
        tracker.emit(true);

        let concurrency = self.config.max_concurrent_downloads.max(1);
        // Collected first: a lazily mapped iterator inside the stream makes
        // the resulting future fail `Send` inference.
        let downloads: Vec<_> = pending.iter().map(|job| self.download_tracked(job, &tracker)).collect();
        let mut results = stream::iter(downloads).buffer_unordered(concurrency);
        while let Some(result) = results.next().await {
            result?;
        }
        drop(results);

        tracker.emit(true);
        Ok(())
    }

    /// Download a single file, bypassing the "already present" check.
    pub async fn download(&self, job: &DownloadJob) -> DownloadResult<()> {
        let noop = |_: DownloadProgress| {};
        let tracker = Tracker {
            files_total: 1,
            bytes_total: job.size.unwrap_or(0),
            files_done: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            last_emit: Mutex::new(Instant::now()),
            callback: &noop,
        };
        self.download_tracked(job, &tracker).await
    }

    async fn download_tracked(&self, job: &DownloadJob, tracker: &Tracker<'_>) -> DownloadResult<()> {
        let mut attempt = 0;
        loop {
            let mut counted = 0u64;
            match self.fetch(job, tracker, &mut counted).await {
                Ok(()) => {
                    tracker.files_done.fetch_add(1, Ordering::Relaxed);
                    tracker.emit(false);
                    return Ok(());
                }
                Err(err) => {
                    tracker.bytes_done.fetch_sub(counted, Ordering::Relaxed);
                    let _ = tokio::fs::remove_file(part_path(&job.destination)).await;
                    if attempt >= self.config.retry_attempts {
                        return Err(err);
                    }
                    tracing::warn!("reintentando {} ({err})", job.url);
                    tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn fetch(&self, job: &DownloadJob, tracker: &Tracker<'_>, counted: &mut u64) -> DownloadResult<()> {
        let net = |e: reqwest::Error| DownloadError::NetworkError(format!("{}: {e}", job.url));

        let mut response = self.client.get(&job.url).send().await.map_err(net)?;
        if !response.status().is_success() {
            return Err(DownloadError::DownloadFailed(format!(
                "{} respondió HTTP {}",
                job.url,
                response.status()
            )));
        }

        if let Some(parent) = job.destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = part_path(&job.destination);
        let mut file = tokio::fs::File::create(&part).await?;
        let mut hasher = Hasher::new(job.checksum.as_ref());
        let stall = Duration::from_secs(self.config.stall_timeout_seconds);

        loop {
            let chunk = tokio::time::timeout(stall, response.chunk())
                .await
                .map_err(|_| DownloadError::NetworkError(format!("{}: tiempo de espera agotado", job.url)))?
                .map_err(net)?;
            let Some(chunk) = chunk else { break };
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
            *counted += chunk.len() as u64;
            tracker.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            tracker.emit(false);
        }
        file.flush().await?;
        drop(file);

        if let (Some(checksum), Some(actual)) = (&job.checksum, hasher.finish()) {
            if !actual.eq_ignore_ascii_case(checksum.expected()) {
                return Err(DownloadError::HashVerificationFailed(job.url.clone()));
            }
        }

        tokio::fs::rename(&part, &job.destination).await?;
        Ok(())
    }
}

fn part_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    destination.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = DownloadConfig::default();
        assert_eq!(config.max_concurrent_downloads, 16);
        assert_eq!(config.retry_attempts, 3);
    }

    #[test]
    fn checksum_matches() {
        // sha1("abc")
        let sha1 = Checksum::Sha1("A9993E364706816ABA3E25717850C26C9CD0D89D".into());
        assert!(sha1.matches(b"abc"));
        assert!(!sha1.matches(b"abd"));
        let sha256 = Checksum::Sha256("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into());
        assert!(sha256.matches(b"abc"));
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(part_path(Path::new("a/b/c.jar")), Path::new("a/b/c.jar.part"));
    }

    #[test]
    fn present_check_uses_size() {
        let dir = std::env::temp_dir().join(format!("takumi-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.bin");
        std::fs::write(&file, b"abc").unwrap();

        assert!(DownloadJob::new("http://x", &file).size(Some(3)).already_present());
        assert!(!DownloadJob::new("http://x", &file).size(Some(4)).already_present());
        assert!(!DownloadJob::new("http://x", &file)
            .sha1(Some("00".into()))
            .already_present());
        assert!(!DownloadJob::new("http://x", dir.join("missing")).already_present());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

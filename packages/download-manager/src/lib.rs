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

/// Lowercase hex SHA-1 of `data`.
pub fn sha1_hex(data: &[u8]) -> String {
    hex::encode(Sha1::digest(data))
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
    /// Re-hash a file that is already on disk instead of trusting its size
    /// ("repair": finds corrupted or modified files).
    pub verify_existing: bool,
    /// `Authorization: Bearer …` for files that need a signed-in user (private instances).
    pub bearer: Option<BearerToken>,
}

/// An API token that never shows up in `Debug` output or logs.
#[derive(Clone)]
pub struct BearerToken(pub String);

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BearerToken(****)")
    }
}

impl DownloadJob {
    pub fn new(url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            checksum: None,
            size: None,
            verify_existing: false,
            bearer: None,
        }
    }

    pub fn sha256(mut self, hash: impl Into<String>) -> Self {
        self.checksum = Some(Checksum::Sha256(hash.into()));
        self
    }

    pub fn bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(BearerToken(token.into()));
        self
    }

    pub fn verify_existing(mut self, verify: bool) -> Self {
        self.verify_existing = verify;
        self
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
    /// asset objects on every launch); otherwise, or with `verify_existing`,
    /// the checksum is computed if there is one.
    fn already_present(&self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.destination) else {
            return false;
        };
        if !meta.is_file() || self.size.is_some_and(|size| meta.len() != size) {
            return false;
        }
        match &self.checksum {
            Some(checksum) if self.verify_existing || self.size.is_none() => file_matches(&self.destination, checksum),
            _ => true,
        }
    }
}

/// Stream `path` through the checksum's hash without loading it into memory.
fn file_matches(path: &std::path::Path, checksum: &Checksum) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut hasher = Hasher::new(Some(checksum));
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(_) => return false,
        }
    }
    hasher
        .finish()
        .is_some_and(|h| h.eq_ignore_ascii_case(checksum.expected()))
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

/// Follows up to 10 redirects but never from https to plain http: some
/// downloads (loader profiles, the Java runtime index) carry no checksum, so
/// TLS is their only integrity guarantee and must hold for the whole chain.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let downgrade =
            attempt.previous().iter().any(|url| url.scheme() == "https") && attempt.url().scheme() != "https";
        if downgrade {
            let message = format!("redirección insegura a {}", attempt.url());
            attempt.error(message)
        } else if attempt.previous().len() >= 10 {
            attempt.error("demasiadas redirecciones")
        } else {
            attempt.follow()
        }
    })
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
            .redirect(redirect_policy())
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
        // Checking what is already on disk may hash many files (repair), so it
        // runs off the async workers.
        let pending: Vec<DownloadJob> = tokio::task::spawn_blocking(move || {
            let mut seen = HashSet::new();
            jobs.into_iter()
                .filter(|job| seen.insert(job.destination.clone()))
                .filter(|job| !job.already_present())
                .collect()
        })
        .await
        .map_err(|e| DownloadError::DownloadFailed(e.to_string()))?;

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

        let mut request = self.client.get(&job.url);
        if let Some(BearerToken(token)) = &job.bearer {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(net)?;
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

    #[test]
    fn repair_rehashes_files_with_the_right_size() {
        let dir = std::env::temp_dir().join(format!("takumi-dl-repair-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.jar");
        std::fs::write(&file, b"abc").unwrap();
        let good = Some(sha1_hex(b"abc"));
        let bad = Some(sha1_hex(b"xyz"));

        // Normal launches trust the size; repair checks the content.
        let tampered = DownloadJob::new("http://x", &file).sha1(bad).size(Some(3));
        assert!(tampered.already_present());
        assert!(!tampered.verify_existing(true).already_present());
        let intact = DownloadJob::new("http://x", &file)
            .sha1(good)
            .size(Some(3))
            .verify_existing(true);
        assert!(intact.already_present());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

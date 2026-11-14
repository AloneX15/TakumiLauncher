//! TakumiLauncher - Update Engine
//!
//! Handles differential updates, staging, and atomic application.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("Update failed: {0}")]
    UpdateFailed(String),

    #[error("Verification failed")]
    VerificationFailed,

    #[error("Rollback failed")]
    RollbackFailed,

    #[error("Staging error: {0}")]
    StagingError(String),
}

pub type UpdateResult<T> = Result<T, UpdateError>;

/// Update check result
#[derive(Debug, Clone)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub changelog: Option<String>,
    pub download_size: Option<u64>,
}

/// Staged update (in progress)
#[derive(Debug, Clone)]
pub struct StagedUpdate {
    pub instance_id: String,
    pub from_version: String,
    pub to_version: String,
    pub staging_dir: PathBuf,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateStatus {
    Preparing,
    Downloading,
    Verifying,
    Ready,
    Applied,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_status() {
        assert_eq!(UpdateStatus::Ready, UpdateStatus::Ready);
    }
}

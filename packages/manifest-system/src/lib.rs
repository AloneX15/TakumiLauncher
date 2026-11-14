//! TakumiLauncher - Manifest System
//!
//! Handles instance manifests with version control, signing, and validation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Invalid manifest: {0}")]
    Invalid(String),

    #[error("Signature verification failed")]
    SignatureVerificationFailed,

    #[error("Manifest not found")]
    NotFound,

    #[error("Version mismatch")]
    VersionMismatch,
}

pub type ManifestResult<T> = Result<T, ManifestError>;

/// Instance manifest (TakumiLauncher format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub instance: String,
    pub version: String,
    pub minecraft_version: String,
    pub mod_loader: ModLoaderConfig,
    pub java_version: JavaVersionConfig,
    pub files: Vec<ManifestFile>,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModLoaderConfig {
    #[serde(rename = "type")]
    pub loader_type: String, // "vanilla", "fabric", "forge", etc
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersionConfig {
    pub major_version: u32,
    pub recommended_build: Option<String>,
}

/// File entry in manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_url: Option<String>,
    #[serde(rename = "type")]
    pub file_type: FileType,
    pub platform: Platform,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Mod,
    Config,
    Library,
    Asset,
    Native,
    ResourcePack,
    ShaderPack,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    MacOS,
    #[serde(rename = "universal")]
    Universal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_creation() {
        let manifest = Manifest {
            schema_version: 1,
            instance: "test".to_string(),
            version: "1.0.0".to_string(),
            minecraft_version: "1.20.1".to_string(),
            mod_loader: ModLoaderConfig {
                loader_type: "fabric".to_string(),
                version: "0.15.0".to_string(),
            },
            java_version: JavaVersionConfig {
                major_version: 17,
                recommended_build: None,
            },
            files: vec![],
            timestamp: 0,
            signature: None,
        };
        assert_eq!(manifest.schema_version, 1);
    }
}

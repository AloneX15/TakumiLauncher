//! TakumiLauncher - Types
//!
//! Shared type definitions for the launcher and related services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Core instance representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub minecraft_version: String,
    pub mod_loader: ModLoader,
    /// Loader version; `None` for vanilla.
    #[serde(default)]
    pub mod_loader_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
    #[serde(default)]
    pub settings: InstanceSettings,
}

/// Per-instance overrides; `None` falls back to the global settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceSettings {
    pub memory_mb: Option<u32>,
    pub java_path: Option<String>,
    pub jvm_args: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Supported mod loaders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModLoader {
    #[serde(rename = "vanilla")]
    Vanilla,
    #[serde(rename = "fabric")]
    Fabric,
    #[serde(rename = "forge")]
    Forge,
    #[serde(rename = "neoforge")]
    NeoForge,
    #[serde(rename = "quilt")]
    Quilt,
}

impl std::fmt::Display for ModLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModLoader::Vanilla => write!(f, "Vanilla"),
            ModLoader::Fabric => write!(f, "Fabric"),
            ModLoader::Forge => write!(f, "Forge"),
            ModLoader::NeoForge => write!(f, "NeoForge"),
            ModLoader::Quilt => write!(f, "Quilt"),
        }
    }
}

/// User account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

/// Authentication provider
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AuthProvider {
    #[serde(rename = "microsoft")]
    Microsoft,
    #[serde(rename = "takumilabs")]
    TakumiLabs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modloader_display() {
        assert_eq!(ModLoader::Fabric.to_string(), "Fabric");
        assert_eq!(ModLoader::Forge.to_string(), "Forge");
    }

    #[test]
    fn instance_defaults_for_missing_fields() {
        let json = r#"{
            "id": "6f1c2b0e-6f0f-4a59-9d8c-0f5e3c2a1b00",
            "name": "Test",
            "minecraft_version": "1.21.1",
            "mod_loader": "fabric",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#;
        let instance: Instance = serde_json::from_str(json).unwrap();
        assert_eq!(instance.mod_loader, ModLoader::Fabric);
        assert_eq!(instance.settings, InstanceSettings::default());
        assert!(instance.last_played.is_none());
    }
}

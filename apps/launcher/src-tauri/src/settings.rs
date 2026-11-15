//! Global launcher settings (`settings.json`).

use crate::state::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "settings.json";
pub const MIN_MEMORY_MB: u32 = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Maximum Java heap, in MiB.
    pub memory_mb: u32,
    /// Java executable to use instead of the managed runtimes.
    pub java_path: Option<String>,
    /// Extra JVM arguments, whitespace separated.
    pub jvm_args: String,
    /// Azure application (client) id used for Microsoft sign-in.
    pub ms_client_id: String,
    pub show_snapshots: bool,
    /// Tune the JVM (heap, garbage collector) and process priority for the game.
    pub optimize_game: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            memory_mb: 4096,
            java_path: None,
            jvm_args: String::new(),
            ms_client_id: String::new(),
            show_snapshots: false,
            optimize_game: true,
        }
    }
}

impl Settings {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self, root: &Path) -> anyhow::Result<()> {
        self.memory_mb = self.memory_mb.max(MIN_MEMORY_MB);
        self.ms_client_id = self.ms_client_id.trim().to_string();
        self.java_path = self
            .java_path
            .take()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        write_atomic(&root.join(FILE), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Client id from the settings, the `TAKUMI_MS_CLIENT_ID` environment
    /// variable, or the value baked in at build time, in that order.
    pub fn microsoft_client_id(&self) -> Option<String> {
        let from_env = std::env::var("TAKUMI_MS_CLIENT_ID").ok();
        [
            Some(self.ms_client_id.clone()),
            from_env,
            option_env!("TAKUMI_MS_CLIENT_ID").map(str::to_string),
        ]
        .into_iter()
        .flatten()
        .map(|id| id.trim().to_string())
        .find(|id| !id.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"memory_mb": 2048}"#).unwrap();
        assert_eq!(settings.memory_mb, 2048);
        assert!(settings.java_path.is_none());
        assert!(settings.optimize_game);
        assert!(!settings.show_snapshots);
    }

    #[test]
    fn save_normalises_values() {
        let root = std::env::temp_dir().join(format!("takumi-settings-{}", uuid::Uuid::new_v4()));
        let mut settings = Settings {
            memory_mb: 16,
            java_path: Some("   ".into()),
            ms_client_id: "  abc  ".into(),
            ..Settings::default()
        };
        settings.save(&root).unwrap();

        let loaded = Settings::load(&root);
        assert_eq!(loaded.memory_mb, MIN_MEMORY_MB);
        assert_eq!(loaded.java_path, None);
        assert_eq!(loaded.ms_client_id, "abc");
        std::fs::remove_dir_all(root).unwrap();
    }
}

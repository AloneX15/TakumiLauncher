//! TakumiLauncher - Instance Manager
//!
//! Stores instances on disk, one directory per instance:
//!
//! ```text
//! instances/<uuid>/instance.json   metadata
//! instances/<uuid>/minecraft/      game directory (saves, mods, options...)
//! instances/<uuid>/natives/        extracted native libraries (legacy versions)
//! ```

use chrono::Utc;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

pub use types::{Instance, InstanceSettings, ModLoader};

const METADATA_FILE: &str = "instance.json";
const MAX_NAME_LEN: usize = 64;

#[derive(Error, Debug)]
pub enum InstanceError {
    #[error("Instancia no encontrada")]
    NotFound,

    #[error("Nombre de instancia no válido")]
    InvalidName,

    #[error("Error de E/S: {0}")]
    Io(#[from] std::io::Error),

    #[error("Metadatos de instancia corruptos: {0}")]
    Corrupt(#[from] serde_json::Error),
}

pub type InstanceResult<T> = Result<T, InstanceError>;

/// Parameters for a new instance
#[derive(Debug, Clone)]
pub struct NewInstance {
    pub name: String,
    pub minecraft_version: String,
    pub mod_loader: ModLoader,
    pub mod_loader_version: Option<String>,
}

/// On-disk instance store
pub struct InstanceStore {
    root: PathBuf,
}

impl InstanceStore {
    pub fn new(root: impl Into<PathBuf>) -> InstanceResult<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn instance_dir(&self, id: Uuid) -> PathBuf {
        self.root.join(id.to_string())
    }

    pub fn game_dir(&self, id: Uuid) -> PathBuf {
        self.instance_dir(id).join("minecraft")
    }

    pub fn natives_dir(&self, id: Uuid) -> PathBuf {
        self.instance_dir(id).join("natives")
    }

    /// All readable instances, most recently played first.
    pub fn list(&self) -> InstanceResult<Vec<Instance>> {
        let mut instances = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path().join(METADATA_FILE);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match serde_json::from_str::<Instance>(&text) {
                Ok(instance) => instances.push(instance),
                Err(err) => tracing::warn!("ignorando {}: {err}", path.display()),
            }
        }
        instances.sort_by(|a, b| {
            b.last_played
                .cmp(&a.last_played)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(instances)
    }

    pub fn get(&self, id: Uuid) -> InstanceResult<Instance> {
        let path = self.instance_dir(id).join(METADATA_FILE);
        let text = std::fs::read_to_string(path).map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => InstanceError::NotFound,
            _ => InstanceError::Io(err),
        })?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn create(&self, new: NewInstance) -> InstanceResult<Instance> {
        let now = Utc::now();
        let instance = Instance {
            id: Uuid::new_v4(),
            name: validate_name(&new.name)?,
            description: None,
            minecraft_version: new.minecraft_version,
            mod_loader: new.mod_loader,
            mod_loader_version: new.mod_loader_version,
            created_at: now,
            updated_at: now,
            last_played: None,
            settings: InstanceSettings::default(),
        };
        std::fs::create_dir_all(self.game_dir(instance.id))?;
        self.write(&instance)?;
        Ok(instance)
    }

    /// Persist changes to an existing instance.
    pub fn save(&self, instance: &mut Instance) -> InstanceResult<()> {
        if !self.instance_dir(instance.id).is_dir() {
            return Err(InstanceError::NotFound);
        }
        instance.name = validate_name(&instance.name)?;
        instance.updated_at = Utc::now();
        self.write(instance)
    }

    /// Delete an instance and all of its game data.
    pub fn delete(&self, id: Uuid) -> InstanceResult<()> {
        let dir = self.instance_dir(id);
        if !dir.is_dir() {
            return Err(InstanceError::NotFound);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    fn write(&self, instance: &Instance) -> InstanceResult<()> {
        let dir = self.instance_dir(instance.id);
        // Write-then-rename so a crash never leaves a truncated file.
        let tmp = dir.join(format!("{METADATA_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(instance)?)?;
        std::fs::rename(tmp, dir.join(METADATA_FILE))?;
        Ok(())
    }
}

fn validate_name(name: &str) -> InstanceResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(InstanceError::InvalidName);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (InstanceStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("takumi-inst-{}", Uuid::new_v4()));
        (InstanceStore::new(&root).unwrap(), root)
    }

    fn new_instance(name: &str) -> NewInstance {
        NewInstance {
            name: name.to_string(),
            minecraft_version: "1.21.1".to_string(),
            mod_loader: ModLoader::Vanilla,
            mod_loader_version: None,
        }
    }

    #[test]
    fn create_list_update_delete() {
        let (store, root) = store();

        let mut instance = store.create(new_instance("  Survival  ")).unwrap();
        assert_eq!(instance.name, "Survival");
        assert!(store.game_dir(instance.id).is_dir());
        assert_eq!(store.list().unwrap().len(), 1);

        instance.settings.memory_mb = Some(8192);
        store.save(&mut instance).unwrap();
        assert_eq!(store.get(instance.id).unwrap().settings.memory_mb, Some(8192));

        store.delete(instance.id).unwrap();
        assert!(matches!(store.get(instance.id), Err(InstanceError::NotFound)));
        assert!(store.list().unwrap().is_empty());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_names() {
        let (store, root) = store();
        assert!(matches!(
            store.create(new_instance("   ")),
            Err(InstanceError::InvalidName)
        ));
        assert!(matches!(
            store.create(new_instance(&"x".repeat(65))),
            Err(InstanceError::InvalidName)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}

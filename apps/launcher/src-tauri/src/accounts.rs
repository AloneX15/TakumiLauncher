//! Account list (`accounts.json`) and session resolution for launching.
//!
//! Only public profile data is written to disk. Microsoft refresh tokens live
//! in the OS credential store and Minecraft access tokens only in memory.

use crate::settings::Settings;
use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use auth_system::{MicrosoftAuth, MinecraftSession};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

const FILE: &str = "accounts.json";

pub const MISSING_CLIENT_ID: &str = "Falta el ID de cliente de Microsoft. Añádelo en Ajustes para iniciar sesión.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub username: String,
    /// Profile UUID without dashes.
    pub uuid: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Accounts {
    pub accounts: Vec<Account>,
    pub active: Option<String>,
}

impl Accounts {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        write_atomic(&root.join(FILE), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn active(&self) -> Option<&Account> {
        let id = self.active.as_ref()?;
        self.accounts.iter().find(|account| &account.id == id)
    }

    /// Add or replace an account and make it the active one.
    pub fn upsert(&mut self, account: Account) {
        self.active = Some(account.id.clone());
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => self.accounts.push(account),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Account> {
        let index = self.accounts.iter().position(|a| a.id == id)?;
        let removed = self.accounts.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self.accounts.first().map(|a| a.id.clone());
        }
        Some(removed)
    }

    pub fn set_active(&mut self, id: &str) -> anyhow::Result<()> {
        if !self.accounts.iter().any(|a| a.id == id) {
            bail!("Cuenta no encontrada");
        }
        self.active = Some(id.to_string());
        Ok(())
    }
}

pub fn offline_account(username: &str) -> anyhow::Result<Account> {
    let username = username.trim();
    let valid = (3..=16).contains(&username.len()) && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        bail!("El nombre debe tener entre 3 y 16 caracteres: letras, números o guion bajo.");
    }
    let uuid = Uuid::new_v4().simple().to_string();
    Ok(Account {
        id: format!("offline-{uuid}"),
        kind: AccountKind::Offline,
        username: username.to_string(),
        uuid,
    })
}

/// Record a completed Microsoft sign-in.
pub fn store_microsoft_session(state: &AppState, session: MinecraftSession) -> anyhow::Result<Account> {
    let account = Account {
        id: format!("ms-{}", session.uuid),
        kind: AccountKind::Microsoft,
        username: session.username.clone(),
        uuid: session.uuid.clone(),
    };
    state.vault.store(&account.id, &session.refresh_token)?;
    let mut accounts = Accounts::load(&state.root);
    accounts.upsert(account.clone());
    accounts.save(&state.root)?;
    state.sessions.lock().unwrap().insert(account.id.clone(), session);
    Ok(account)
}

pub fn remove_account(state: &AppState, id: &str) -> anyhow::Result<()> {
    let mut accounts = Accounts::load(&state.root);
    if let Some(removed) = accounts.remove(id) {
        accounts.save(&state.root)?;
        state.sessions.lock().unwrap().remove(id);
        if removed.kind == AccountKind::Microsoft {
            state.vault.delete(id)?;
        }
    }
    Ok(())
}

/// Credentials passed to the game.
pub struct LaunchAuth {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: &'static str,
}

/// Resolve the active account into launch credentials, refreshing the
/// Microsoft session when the cached one is missing or about to expire.
pub async fn launch_auth(state: &AppState, settings: &Settings) -> anyhow::Result<LaunchAuth> {
    let account = Accounts::load(&state.root)
        .active()
        .cloned()
        .context("No hay ninguna cuenta seleccionada. Añade una en Cuentas.")?;

    if account.kind == AccountKind::Offline {
        return Ok(LaunchAuth {
            username: account.username,
            uuid: account.uuid,
            access_token: "0".to_string(),
            user_type: "legacy",
        });
    }

    let cached = state.sessions.lock().unwrap().get(&account.id).cloned();
    let session = match cached.filter(MinecraftSession::is_valid) {
        Some(session) => session,
        None => {
            let client_id = settings.microsoft_client_id().context(MISSING_CLIENT_ID)?;
            let refresh_token = state
                .vault
                .load(&account.id)?
                .context("La sesión de esta cuenta ya no está guardada. Vuelve a iniciar sesión.")?;
            let session = MicrosoftAuth::new(state.http().clone(), client_id)
                .refresh(&refresh_token)
                .await?;
            // Persists the rotated refresh token and any username change.
            store_session_keeping_active(state, &account, session.clone())?;
            session
        }
    };

    Ok(LaunchAuth {
        username: session.username,
        uuid: session.uuid,
        access_token: session.access_token,
        user_type: "msa",
    })
}

fn store_session_keeping_active(state: &AppState, account: &Account, session: MinecraftSession) -> anyhow::Result<()> {
    state.vault.store(&account.id, &session.refresh_token)?;
    if session.username != account.username {
        let mut accounts = Accounts::load(&state.root);
        if let Some(stored) = accounts.accounts.iter_mut().find(|a| a.id == account.id) {
            stored.username = session.username.clone();
            accounts.save(&state.root)?;
        }
    }
    state.sessions.lock().unwrap().insert(account.id.clone(), session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_name_validation() {
        assert!(offline_account("Steve_01").is_ok());
        assert!(offline_account("ab").is_err());
        assert!(offline_account("name with spaces").is_err());
        assert!(offline_account("seventeen_chars__").is_err());
    }

    #[test]
    fn upsert_and_remove_track_active() {
        let mut accounts = Accounts::default();
        let a = offline_account("Alice").unwrap();
        let b = offline_account("Bob").unwrap();
        accounts.upsert(a.clone());
        accounts.upsert(b.clone());
        assert_eq!(accounts.active().unwrap().username, "Bob");

        accounts.remove(&b.id);
        assert_eq!(accounts.active().unwrap().username, "Alice");
        accounts.remove(&a.id);
        assert!(accounts.active().is_none());
        assert!(accounts.set_active("missing").is_err());
    }
}

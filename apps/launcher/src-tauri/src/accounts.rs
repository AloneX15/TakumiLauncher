//! Account list (`accounts.json`) and session resolution for launching.
//!
//! Only public profile data is written to disk. Microsoft refresh tokens live
//! in the OS credential store and Minecraft access tokens only in memory.
//!
//! There is exactly one offline profile and nobody edits it by hand: its name
//! is the Takumi account's username (unique on TakumiWeb and validated with the
//! Minecraft name rules), so nobody can play offline as someone else. A
//! Microsoft (premium) account takes priority when there is one.

use crate::session::TakumiSession;
use crate::settings::Settings;
use crate::state::{write_atomic, AppState};
use anyhow::{bail, Context};
use auth_system::{MicrosoftAuth, MinecraftSession};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "accounts.json";
/// Id of the offline profile derived from the Takumi account.
pub const TAKUMI_OFFLINE_ID: &str = "takumi-offline";

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
            self.active = None;
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

    /// Bring the list in line with the signed-in Takumi account: one offline
    /// profile named after it (none when signed out), no hand-made offline
    /// profiles, and a valid active account, preferring Microsoft.
    /// Returns whether anything changed.
    pub fn reconcile(&mut self, takumi_username: Option<&str>) -> bool {
        let wanted = takumi_username
            .filter(|name| is_valid_name(name))
            .map(takumi_offline_account);
        let before = (self.accounts.len(), self.active.clone());
        let mut changed = false;

        self.accounts
            .retain(|a| a.kind == AccountKind::Microsoft || (a.id == TAKUMI_OFFLINE_ID && wanted.is_some()));
        if let Some(wanted) = wanted {
            match self.accounts.iter_mut().find(|a| a.id == TAKUMI_OFFLINE_ID) {
                Some(existing) if existing.username == wanted.username && existing.uuid == wanted.uuid => {}
                Some(existing) => {
                    *existing = wanted;
                    changed = true;
                }
                None => {
                    self.accounts.push(wanted);
                    changed = true;
                }
            }
        }

        if self.active().is_none() {
            self.active = self
                .accounts
                .iter()
                .find(|a| a.kind == AccountKind::Microsoft)
                .or_else(|| self.accounts.first())
                .map(|a| a.id.clone());
        }
        changed || before != (self.accounts.len(), self.active.clone())
    }
}

/// Minecraft name rules (the same ones TakumiWeb enforces on usernames).
fn is_valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The offline profile of a Takumi user.
pub fn takumi_offline_account(username: &str) -> Account {
    Account {
        id: TAKUMI_OFFLINE_ID.to_string(),
        kind: AccountKind::Offline,
        username: username.to_string(),
        uuid: offline_uuid(username),
    }
}

/// The UUID a vanilla server in offline mode gives this name
/// (`UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`), so the player keeps
/// the same inventory and permissions on those servers.
pub fn offline_uuid(username: &str) -> String {
    let digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).into();
    uuid::Builder::from_md5_bytes(digest).into_uuid().simple().to_string()
}

/// The account list reconciled with the Takumi session on disk (saved back
/// when that changed something).
pub fn current(state: &AppState) -> Accounts {
    let mut accounts = Accounts::load(&state.root);
    if accounts.reconcile(takumi_username(state).as_deref()) {
        if let Err(error) = accounts.save(&state.root) {
            tracing::warn!("could not save accounts: {error:#}");
        }
    }
    accounts
}

fn takumi_username(state: &AppState) -> Option<String> {
    TakumiSession::load(&state.root).map(|session| session.username)
}

pub fn set_active(state: &AppState, id: &str) -> anyhow::Result<Accounts> {
    let mut accounts = current(state);
    accounts.set_active(id)?;
    accounts.save(&state.root)?;
    Ok(accounts)
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
    let mut accounts = current(state);
    accounts.upsert(account.clone());
    accounts.save(&state.root)?;
    state.sessions.lock().unwrap().insert(account.id.clone(), session);
    Ok(account)
}

pub fn remove_account(state: &AppState, id: &str) -> anyhow::Result<Accounts> {
    if id == TAKUMI_OFFLINE_ID {
        bail!("El perfil sin conexión es el de tu cuenta Takumi y no se puede quitar.");
    }
    let mut accounts = current(state);
    if let Some(removed) = accounts.remove(id) {
        accounts.reconcile(takumi_username(state).as_deref());
        accounts.save(&state.root)?;
        state.sessions.lock().unwrap().remove(id);
        if removed.kind == AccountKind::Microsoft {
            state.vault.delete(id)?;
        }
    }
    Ok(accounts)
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
    let account = current(state)
        .active()
        .cloned()
        .context("No hay ninguna cuenta seleccionada. Inicia sesión con tu cuenta Takumi o añade una de Microsoft.")?;

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

    fn microsoft(name: &str) -> Account {
        Account {
            id: format!("ms-{name}"),
            kind: AccountKind::Microsoft,
            username: name.to_string(),
            uuid: format!("{:032x}", name.len()),
        }
    }

    #[test]
    fn offline_uuid_matches_vanilla_servers() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8)) in Java.
        assert_eq!(offline_uuid("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
        assert_eq!(takumi_offline_account("Notch").uuid, offline_uuid("Notch"));
    }

    #[test]
    fn offline_profile_follows_the_takumi_account() {
        let mut accounts = Accounts::default();
        // A hand-made offline profile from an older version is dropped.
        accounts.accounts.push(Account {
            id: "offline-1".into(),
            kind: AccountKind::Offline,
            username: "Cualquiera".into(),
            uuid: "1".repeat(32),
        });
        accounts.active = Some("offline-1".into());

        assert!(accounts.reconcile(Some("Takumi_01")));
        assert_eq!(accounts.accounts.len(), 1);
        assert_eq!(accounts.active().unwrap().id, TAKUMI_OFFLINE_ID);
        assert_eq!(accounts.active().unwrap().username, "Takumi_01");
        assert!(!accounts.reconcile(Some("Takumi_01")));

        // Renamed on the web (by an admin): the profile follows.
        assert!(accounts.reconcile(Some("Takumi_02")));
        assert_eq!(accounts.active().unwrap().username, "Takumi_02");
        assert_eq!(accounts.active().unwrap().uuid, offline_uuid("Takumi_02"));

        // Signed out of Takumi: no offline profile at all.
        assert!(accounts.reconcile(None));
        assert!(accounts.accounts.is_empty());
        assert!(accounts.active.is_none());
    }

    #[test]
    fn microsoft_account_has_priority() {
        let mut accounts = Accounts::default();
        accounts.reconcile(Some("Jugador"));
        assert_eq!(accounts.active().unwrap().id, TAKUMI_OFFLINE_ID);

        // Signing in with Microsoft selects it.
        accounts.upsert(microsoft("Premium"));
        accounts.reconcile(Some("Jugador"));
        assert_eq!(accounts.active().unwrap().username, "Premium");

        // With no valid selection, Microsoft wins over the offline profile.
        accounts.active = None;
        accounts.reconcile(Some("Jugador"));
        assert_eq!(accounts.active().unwrap().username, "Premium");

        // The player can still pick the offline profile on purpose.
        accounts.set_active(TAKUMI_OFFLINE_ID).unwrap();
        accounts.reconcile(Some("Jugador"));
        assert_eq!(accounts.active().unwrap().id, TAKUMI_OFFLINE_ID);

        // Removing the selected Microsoft account falls back to the offline profile.
        accounts.set_active("ms-Premium").unwrap();
        accounts.remove("ms-Premium");
        accounts.reconcile(Some("Jugador"));
        assert_eq!(accounts.active().unwrap().id, TAKUMI_OFFLINE_ID);
        assert!(accounts.set_active("missing").is_err());
    }

    #[test]
    fn invalid_takumi_names_get_no_offline_profile() {
        let mut accounts = Accounts::default();
        accounts.reconcile(Some("con espacios"));
        assert!(accounts.accounts.is_empty());
    }
}

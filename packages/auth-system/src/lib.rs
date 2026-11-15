//! TakumiLauncher - Authentication System
//!
//! Microsoft account sign-in for Minecraft (OAuth2 device code flow →
//! Xbox Live → XSTS → Minecraft services) and secure refresh-token storage
//! in the operating system's credential store.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use thiserror::Error;

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("El código de inicio de sesión ha caducado. Inténtalo de nuevo.")]
    TokenExpired,

    #[error("Autenticación fallida: {0}")]
    AuthenticationFailed(String),

    #[error("Esta cuenta de Microsoft no tiene Minecraft: Java Edition.")]
    GameNotOwned,

    #[error("Error de red: {0}")]
    Network(String),

    #[error("Almacén de credenciales: {0}")]
    Vault(String),
}

pub type AuthResult<T> = Result<T, AuthError>;

/// Code the user enters at `verification_uri` to authorise the launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    #[serde(skip_serializing)]
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// An authenticated Minecraft session
#[derive(Clone)]
pub struct MinecraftSession {
    pub access_token: String,
    /// Profile UUID without dashes.
    pub uuid: String,
    pub username: String,
    /// Microsoft refresh token (rotates on every refresh).
    pub refresh_token: String,
    /// Unix timestamp after which `access_token` is no longer valid.
    pub expires_at: i64,
}

/// Tokens are never printed, so a `{:?}` in a log or error cannot leak them.
impl std::fmt::Debug for MinecraftSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinecraftSession")
            .field("access_token", &"<redacted>")
            .field("uuid", &self.uuid)
            .field("username", &self.username)
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl MinecraftSession {
    pub fn is_valid(&self) -> bool {
        self.expires_at - 60 > chrono::Utc::now().timestamp()
    }
}

#[derive(Deserialize)]
struct MicrosoftToken {
    access_token: String,
    refresh_token: String,
}

/// Microsoft sign-in client for a registered Azure application.
pub struct MicrosoftAuth {
    client: reqwest::Client,
    client_id: String,
}

impl MicrosoftAuth {
    pub fn new(client: reqwest::Client, client_id: impl Into<String>) -> Self {
        Self {
            client,
            client_id: client_id.into(),
        }
    }

    /// Begin the device code flow.
    pub async fn start_device_flow(&self) -> AuthResult<DeviceCode> {
        let body = self
            .post_form(DEVICE_CODE_URL, &[("client_id", &self.client_id), ("scope", SCOPE)])
            .await?;
        if let Some(error) = oauth_error(&body) {
            return Err(AuthError::AuthenticationFailed(error));
        }
        serde_json::from_value(body).map_err(|e| AuthError::AuthenticationFailed(e.to_string()))
    }

    /// Wait until the user authorises `code` (or it expires), then complete
    /// the Minecraft sign-in.
    pub async fn complete_device_flow(&self, code: &DeviceCode) -> AuthResult<MinecraftSession> {
        let deadline = Instant::now() + Duration::from_secs(code.expires_in);
        let mut interval = code.interval.max(1);
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if Instant::now() >= deadline {
                return Err(AuthError::TokenExpired);
            }
            let body = self
                .post_form(
                    TOKEN_URL,
                    &[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ("client_id", &self.client_id),
                        ("device_code", &code.device_code),
                    ],
                )
                .await?;
            match body["error"].as_str() {
                Some("authorization_pending") => continue,
                Some("slow_down") => interval += 5,
                Some("expired_token") => return Err(AuthError::TokenExpired),
                Some("authorization_declined") => {
                    return Err(AuthError::AuthenticationFailed("inicio de sesión rechazado".into()))
                }
                Some(_) => return Err(AuthError::AuthenticationFailed(oauth_error(&body).unwrap_or_default())),
                None => return self.exchange(parse_token(body)?).await,
            }
        }
    }

    /// Obtain a fresh session from a stored refresh token.
    pub async fn refresh(&self, refresh_token: &str) -> AuthResult<MinecraftSession> {
        let body = self
            .post_form(
                TOKEN_URL,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &self.client_id),
                    ("refresh_token", refresh_token),
                    ("scope", SCOPE),
                ],
            )
            .await?;
        if let Some(error) = oauth_error(&body) {
            return Err(AuthError::AuthenticationFailed(format!(
                "la sesión ha caducado, vuelve a iniciar sesión ({error})"
            )));
        }
        self.exchange(parse_token(body)?).await
    }

    /// Microsoft token → Xbox Live → XSTS → Minecraft token + profile.
    async fn exchange(&self, microsoft: MicrosoftToken) -> AuthResult<MinecraftSession> {
        let xbl = self
            .post_json(
                XBL_AUTH_URL,
                &json!({
                    "Properties": {
                        "AuthMethod": "RPS",
                        "SiteName": "user.auth.xboxlive.com",
                        "RpsTicket": format!("d={}", microsoft.access_token),
                    },
                    "RelyingParty": "http://auth.xboxlive.com",
                    "TokenType": "JWT",
                }),
            )
            .await?;
        let xbl_token = string_field(&xbl, "Token")?;

        let xsts = self
            .post_json(
                XSTS_AUTH_URL,
                &json!({
                    "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
                    "RelyingParty": "rp://api.minecraftservices.com/",
                    "TokenType": "JWT",
                }),
            )
            .await?;
        if let Some(code) = xsts["XErr"].as_u64() {
            return Err(AuthError::AuthenticationFailed(xsts_error(code)));
        }
        let xsts_token = string_field(&xsts, "Token")?;
        let user_hash = xsts["DisplayClaims"]["xui"][0]["uhs"]
            .as_str()
            .ok_or_else(|| AuthError::AuthenticationFailed("respuesta de Xbox Live sin usuario".into()))?;

        let login = self
            .post_json(
                MC_LOGIN_URL,
                &json!({ "identityToken": format!("XBL3.0 x={user_hash};{xsts_token}") }),
            )
            .await?;
        let access_token = string_field(&login, "access_token")?;
        let expires_in = login["expires_in"].as_i64().unwrap_or(3600);

        let response = self
            .client
            .get(MC_PROFILE_URL)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(network)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AuthError::GameNotOwned);
        }
        let profile: Value = response.json().await.map_err(network)?;

        Ok(MinecraftSession {
            uuid: string_field(&profile, "id")?,
            username: string_field(&profile, "name")?,
            access_token,
            refresh_token: microsoft.refresh_token,
            expires_at: chrono::Utc::now().timestamp() + expires_in,
        })
    }

    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> AuthResult<Value> {
        // OAuth errors come back as JSON with a 4xx status, so the status is
        // deliberately not checked here.
        let response = self.client.post(url).form(form).send().await.map_err(network)?;
        response.json().await.map_err(network)
    }

    async fn post_json(&self, url: &str, body: &Value) -> AuthResult<Value> {
        let response = self
            .client
            .post(url)
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        let text = response.text().await.map_err(network)?;
        match serde_json::from_str(&text) {
            Ok(value) => Ok(value),
            Err(_) => Err(AuthError::AuthenticationFailed(format!(
                "{url} respondió HTTP {status}"
            ))),
        }
    }
}

fn network(error: reqwest::Error) -> AuthError {
    // `without_url` keeps query strings out of error messages and logs.
    AuthError::Network(error.without_url().to_string())
}

fn parse_token(body: Value) -> AuthResult<MicrosoftToken> {
    serde_json::from_value(body).map_err(|_| AuthError::AuthenticationFailed("respuesta de Microsoft no válida".into()))
}

fn string_field(value: &Value, field: &str) -> AuthResult<String> {
    value[field].as_str().map(str::to_string).ok_or_else(|| {
        let detail = value["errorMessage"]
            .as_str()
            .or(value["error"].as_str())
            .unwrap_or("respuesta inesperada del servidor");
        AuthError::AuthenticationFailed(detail.to_string())
    })
}

fn oauth_error(body: &Value) -> Option<String> {
    let error = body["error"].as_str()?;
    let description = body["error_description"].as_str().unwrap_or_default();
    // Only the first line: the rest is trace ids and timestamps.
    let description = description.lines().next().unwrap_or_default();
    Some(if description.is_empty() {
        error.to_string()
    } else {
        format!("{error}: {description}")
    })
}

fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => "esta cuenta de Microsoft no tiene perfil de Xbox; créalo en xbox.com".into(),
        2148916235 => "Xbox Live no está disponible en tu país".into(),
        2148916236 | 2148916237 => "la cuenta necesita verificación de adulto en xbox.com".into(),
        2148916238 => "la cuenta es de un menor y debe añadirse a un grupo familiar".into(),
        other => format!("error de Xbox Live {other}"),
    }
}

/// Refresh-token storage backed by the OS credential store (Windows
/// Credential Manager, macOS Keychain, Secret Service on Linux).
pub struct TokenVault {
    service: String,
}

impl TokenVault {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> AuthResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(vault)
    }

    pub fn store(&self, account: &str, secret: &str) -> AuthResult<()> {
        self.entry(account)?.set_password(secret).map_err(vault)
    }

    pub fn load(&self, account: &str) -> AuthResult<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(vault(error)),
        }
    }

    pub fn delete(&self, account: &str) -> AuthResult<()> {
        match self.entry(account)?.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(vault(error)),
        }
    }
}

fn vault(error: keyring::Error) -> AuthError {
    AuthError::Vault(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_validity() {
        let mut session = MinecraftSession {
            access_token: "a".into(),
            uuid: "u".into(),
            username: "n".into(),
            refresh_token: "r".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
        };
        assert!(session.is_valid());
        session.expires_at = chrono::Utc::now().timestamp() + 30;
        assert!(!session.is_valid());
    }

    #[test]
    fn device_code_hides_secret_when_serialized() {
        let code: DeviceCode = serde_json::from_value(json!({
            "device_code": "secret",
            "user_code": "ABCD1234",
            "verification_uri": "https://www.microsoft.com/link",
            "expires_in": 900,
            "interval": 5,
            "message": "ignored"
        }))
        .unwrap();
        let serialized = serde_json::to_value(&code).unwrap();
        assert!(serialized.get("device_code").is_none());
        assert_eq!(serialized["user_code"], "ABCD1234");
    }

    #[test]
    fn session_debug_hides_tokens() {
        let session = MinecraftSession {
            access_token: "access-secret".into(),
            uuid: "abc".into(),
            username: "Steve".into(),
            refresh_token: "refresh-secret".into(),
            expires_at: 1,
        };
        let printed = format!("{session:?}");
        assert!(!printed.contains("secret"), "{printed}");
        assert!(printed.contains("Steve"));
    }

    #[test]
    fn oauth_error_formatting() {
        assert_eq!(oauth_error(&json!({"access_token": "x"})), None);
        assert_eq!(
            oauth_error(&json!({"error": "invalid_grant", "error_description": "Bad token\r\nTrace ID: 1"})).unwrap(),
            "invalid_grant: Bad token"
        );
    }

    #[test]
    fn missing_field_reports_server_message() {
        let err = string_field(&json!({"errorMessage": "nope"}), "access_token").unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    /// Round-trips a token-sized secret through the real OS credential store.
    #[test]
    #[ignore = "writes to the OS credential store"]
    fn vault_round_trip() {
        let vault = TokenVault::new("TakumiLauncher-test");
        let secret = "M.C123_".to_string() + &"x".repeat(1200);
        vault.store("test-account", &secret).unwrap();
        assert_eq!(vault.load("test-account").unwrap().as_deref(), Some(secret.as_str()));
        vault.delete("test-account").unwrap();
        assert_eq!(vault.load("test-account").unwrap(), None);
        vault.delete("test-account").unwrap();
    }
}

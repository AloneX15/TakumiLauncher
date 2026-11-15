//! Takumi account session, backed by the takumilabs.org API (`takumi-webpage`).
//!
//! The launcher's own sign-in, separate from the Minecraft accounts in
//! `accounts.rs`. The API token lives in the OS credential store; only the
//! public profile is cached in `session.json` so the launcher still opens when
//! the server cannot be reached.

use crate::commands::{fail, AppStateRef, CmdResult};
use crate::state::{write_atomic, AppState};
use anyhow::{anyhow, Context};
use download_manager::reqwest::{self, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const FILE: &str = "session.json";
const VAULT_KEY: &str = "takumi-session";
/// Website used when nothing else is configured: the local TakumiWeb dev server
/// (`pnpm dev` in takumi-webpage). Release builds bake the public URL in with
/// `TAKUMI_WEB_URL` at compile time (see .github/workflows/release.yml).
const LOCAL_WEB_URL: &str = "http://localhost:5173";
const BUILD_WEB_URL: Option<&str> = option_env!("TAKUMI_WEB_URL");
const TIMEOUT: Duration = Duration::from_secs(15);
const UNREACHABLE: &str = "No se pudo conectar con el servidor de Takumi. Comprueba tu conexión a internet.";
const UNREACHABLE_LOCAL: &str =
    "No se pudo conectar con la web de Takumi en local. Arráncala con «pnpm dev» en la carpeta takumi-webpage.";

/// Base URL of the website/API: `TAKUMI_WEB_URL` at run time, then at build
/// time, then the local dev server.
///
/// Passwords and the session token go to this URL, so anything but https
/// (or plain http to this same machine) is ignored.
pub fn web_url() -> String {
    [std::env::var("TAKUMI_WEB_URL").ok(), BUILD_WEB_URL.map(str::to_string)]
        .into_iter()
        .flatten()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .find(|url| is_secure_web_url(url))
        .unwrap_or_else(|| LOCAL_WEB_URL.to_string())
}

/// `https://…`, or `http://` to localhost / 127.0.0.1 / [::1] only.
fn is_secure_web_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        _ => false,
    }
}

/// Connection error message, with a hint to start the web when it is local.
fn unreachable() -> &'static str {
    let url = web_url();
    if url.contains("://localhost") || url.contains("://127.0.0.1") {
        UNREACHABLE_LOCAL
    } else {
        UNREACHABLE
    }
}

fn endpoint(path: &str) -> String {
    format!("{}{path}", web_url())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakumiSession {
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "user".to_string()
}

impl TakumiSession {
    pub fn load(root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(root.join(FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        write_atomic(&root.join(FILE), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn clear(root: &Path) -> anyhow::Result<()> {
        match std::fs::remove_file(root.join(FILE)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

#[derive(Deserialize)]
struct ApiUser {
    username: String,
    email: String,
    role: String,
}

impl From<ApiUser> for TakumiSession {
    fn from(user: ApiUser) -> Self {
        Self {
            username: user.username,
            email: user.email,
            role: user.role,
        }
    }
}

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
    user: ApiUser,
}

#[derive(Deserialize)]
struct MeResponse {
    user: ApiUser,
}

#[derive(Deserialize)]
struct ApiError {
    error: String,
}

/// Turn a non-success response into the server's `{ "error": "…" }` message.
async fn api_error(response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    match response.json::<ApiError>().await {
        Ok(body) => anyhow!(body.error),
        Err(_) => anyhow!("El servidor de Takumi respondió con un error ({status})."),
    }
}

fn device_label() -> String {
    format!(
        "TakumiLauncher {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    )
}

async fn login(state: &AppState, user: &str, password: &str) -> anyhow::Result<TakumiSession> {
    let user = user.trim();
    if user.is_empty() || password.is_empty() {
        return Err(anyhow!("Escribe tu usuario o correo y tu contraseña."));
    }
    let response = state
        .http()
        .post(endpoint("/api/launcher/login"))
        .timeout(TIMEOUT)
        .json(&serde_json::json!({ "user": user, "password": password, "device": device_label() }))
        .send()
        .await
        .context(unreachable())?;
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let body: LoginResponse = response.json().await.context("Respuesta del servidor no válida.")?;
    state.vault.store(VAULT_KEY, &body.token)?;
    let session = TakumiSession::from(body.user);
    session.save(&state.root)?;
    Ok(session)
}

/// What a background check of the stored token found.
enum Revalidation {
    /// Still valid; carries the profile when it changed on the server.
    Valid(Option<TakumiSession>),
    /// Expired, revoked (password change, "close sessions") or suspended.
    Ended,
    /// Offline or server trouble: keep trusting the cached profile.
    Unknown,
}

/// The stored session, without touching the network, so start-up never waits
/// for the server. `revalidate` checks it afterwards.
fn cached_session(state: &AppState) -> anyhow::Result<Option<(TakumiSession, String)>> {
    let Some(cached) = TakumiSession::load(&state.root) else {
        return Ok(None);
    };
    match state.vault.load(VAULT_KEY)? {
        Some(token) => Ok(Some((cached, token))),
        None => {
            TakumiSession::clear(&state.root)?;
            Ok(None)
        }
    }
}

async fn revalidate(state: &AppState, cached: &TakumiSession, token: &str) -> Revalidation {
    let response = state
        .http()
        .get(endpoint("/api/launcher/me"))
        .bearer_auth(token)
        .timeout(TIMEOUT)
        .send()
        .await;
    // The user may have signed out or in again while the request was in flight.
    let still_current = || matches!(state.vault.load(VAULT_KEY), Ok(Some(current)) if current == token);
    match response {
        Ok(response) if response.status().is_success() => match response.json::<MeResponse>().await {
            Ok(body) => {
                let session = TakumiSession::from(body.user);
                if session == *cached || !still_current() || session.save(&state.root).is_err() {
                    Revalidation::Valid(None)
                } else {
                    Revalidation::Valid(Some(session))
                }
            }
            Err(_) => Revalidation::Unknown,
        },
        Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
            if still_current() && forget(state).is_ok() {
                Revalidation::Ended
            } else {
                Revalidation::Unknown
            }
        }
        _ => Revalidation::Unknown,
    }
}

async fn logout(state: &AppState) -> anyhow::Result<()> {
    if let Ok(Some(token)) = state.vault.load(VAULT_KEY) {
        // Best effort: the local sign-out must work offline too.
        let _ = state
            .http()
            .post(endpoint("/api/launcher/logout"))
            .bearer_auth(token)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    }
    forget(state)
}

fn forget(state: &AppState) -> anyhow::Result<()> {
    state.vault.delete(VAULT_KEY)?;
    TakumiSession::clear(&state.root)
}

/// Returns the stored session at once and checks it with the server in the
/// background. Emits `session-changed` (new profile) or `session-ended` when
/// the server disagrees with what was returned.
#[tauri::command]
pub fn get_session(app: AppHandle, state: AppStateRef<'_>) -> CmdResult<Option<TakumiSession>> {
    let Some((cached, token)) = cached_session(&state).map_err(fail)? else {
        return Ok(None);
    };
    let state = state.inner().clone();
    let profile = cached.clone();
    tauri::async_runtime::spawn(async move {
        match revalidate(&state, &profile, &token).await {
            Revalidation::Valid(Some(updated)) => {
                let _ = app.emit_all("session-changed", updated);
            }
            Revalidation::Ended => {
                let _ = app.emit_all("session-ended", ());
            }
            Revalidation::Valid(None) | Revalidation::Unknown => {}
        }
    });
    Ok(Some(cached))
}

#[tauri::command]
pub async fn takumi_login(state: AppStateRef<'_>, user: String, password: String) -> CmdResult<TakumiSession> {
    login(&state, &user, &password).await.map_err(fail)
}

#[tauri::command]
pub async fn takumi_logout(state: AppStateRef<'_>) -> CmdResult<()> {
    logout(&state).await.map_err(fail)
}

// ------------------------------------------------------------- news

#[derive(Debug, Serialize, Deserialize)]
pub struct NewsItem {
    pub slug: String,
    pub title: String,
    pub excerpt: String,
    pub category: String,
    pub published_at: Option<String>,
    pub url: String,
}

#[derive(Deserialize)]
struct NewsResponse {
    posts: Vec<NewsItem>,
}

#[tauri::command]
pub async fn get_news(state: AppStateRef<'_>) -> CmdResult<Vec<NewsItem>> {
    let response = state
        .http()
        .get(endpoint("/api/news?limit=20"))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| unreachable().to_string())?;
    if !response.status().is_success() {
        return Err(fail(api_error(response).await));
    }
    let body: NewsResponse = response
        .json()
        .await
        .map_err(|_| "Respuesta del servidor no válida.".to_string())?;
    Ok(body.posts)
}

// ---------------------------------------------------------- updates

#[derive(Debug, Serialize, Deserialize)]
pub struct LauncherUpdate {
    pub version: String,
    pub notes: String,
    pub download_url: String,
    pub page_url: String,
}

/// The latest published release when it is newer than this build. Network
/// failures are silent (`None`): the notice is not worth an error toast.
#[tauri::command]
pub async fn check_launcher_update(state: AppStateRef<'_>) -> CmdResult<Option<LauncherUpdate>> {
    let response = state
        .http()
        .get(endpoint("/api/releases/latest"))
        .timeout(TIMEOUT)
        .send()
        .await;
    let Ok(response) = response else { return Ok(None) };
    if !response.status().is_success() {
        return Ok(None);
    }
    let Ok(latest) = response.json::<LauncherUpdate>().await else {
        return Ok(None);
    };
    Ok(is_newer(&latest.version, env!("CARGO_PKG_VERSION")).then_some(latest))
}

/// Semver-style "a > b" (pre-releases sort before their release).
pub fn is_newer(a: &str, b: &str) -> bool {
    parse_version(a) > parse_version(b)
}

fn parse_version(value: &str) -> ([u64; 3], (bool, Vec<PreField>)) {
    let value = value.trim().trim_start_matches('v');
    let (core, pre) = value.split_once('-').unwrap_or((value, ""));
    let mut numbers = [0; 3];
    for (slot, part) in numbers.iter_mut().zip(core.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    // `true` for a final release so it sorts after any pre-release of the same core.
    let fields = pre.split('.').filter(|p| !p.is_empty()).map(PreField::parse).collect();
    (numbers, (pre.is_empty(), fields))
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PreField {
    Number(u64),
    Text(String),
}

impl PreField {
    fn parse(part: &str) -> Self {
        part.parse()
            .map(Self::Number)
            .unwrap_or_else(|_| Self::Text(part.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_secure_web_urls() {
        assert!(is_secure_web_url("https://takumilabs.org"));
        assert!(is_secure_web_url("http://localhost:5173"));
        assert!(is_secure_web_url("http://127.0.0.1:3000"));
        assert!(is_secure_web_url("http://[::1]:5173"));
        assert!(!is_secure_web_url("http://takumilabs.org"));
        assert!(!is_secure_web_url("http://localhost.evil.example"));
        assert!(!is_secure_web_url("ftp://takumilabs.org"));
        assert!(!is_secure_web_url("takumilabs.org"));
        assert!(!is_secure_web_url(""));
    }

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0"));
        assert!(is_newer("0.1.0", "0.1.0-beta.1"));
        assert!(is_newer("0.1.0-beta.10", "0.1.0-beta.2"));
    }

    #[test]
    fn session_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("takumi-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(TakumiSession::load(&dir).is_none());

        let session = TakumiSession {
            username: "steve".into(),
            email: "steve@example.com".into(),
            role: "user".into(),
        };
        session.save(&dir).unwrap();
        assert_eq!(TakumiSession::load(&dir), Some(session));

        // Files written by the previous (offline) version only had the username.
        std::fs::write(dir.join(FILE), r#"{"username":"old"}"#).unwrap();
        assert_eq!(TakumiSession::load(&dir).unwrap().role, "user");

        TakumiSession::clear(&dir).unwrap();
        assert!(TakumiSession::load(&dir).is_none());
        TakumiSession::clear(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Answers every request on a local port with the given status and JSON body.
    async fn mock_server(status: &'static str, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0u8; 2048];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 {status}
content-type: application/json
content-length: {}
connection: close

{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    /// The background check keeps, updates or ends the cached session.
    /// One test because it drives the process-wide `TAKUMI_WEB_URL`.
    #[tokio::test(flavor = "multi_thread")]
    async fn background_revalidation() {
        let dir = std::env::temp_dir().join(format!("takumi-revalidate-test-{}", std::process::id()));
        let mut state = AppState::new(dir.clone()).unwrap();
        // Never the real credential entry.
        state.vault = auth_system::TokenVault::new("TakumiLauncherTest");
        let cached = TakumiSession {
            username: "steve".into(),
            email: "steve@example.com".into(),
            role: "user".into(),
        };
        let sign_in = |state: &AppState| {
            state.vault.store(VAULT_KEY, "token-1").unwrap();
            cached.save(&state.root).unwrap();
        };

        // No session stored: nothing to check.
        let _ = state.vault.delete(VAULT_KEY);
        assert!(cached_session(&state).unwrap().is_none());

        // Server unreachable: the cached profile stays.
        sign_in(&state);
        std::env::set_var("TAKUMI_WEB_URL", "http://127.0.0.1:9");
        let (profile, token) = cached_session(&state).unwrap().unwrap();
        assert_eq!(profile, cached);
        assert!(matches!(
            revalidate(&state, &profile, &token).await,
            Revalidation::Unknown
        ));
        assert!(cached_session(&state).unwrap().is_some());

        // Same profile on the server: nothing to report.
        let same = r#"{"user":{"id":"1","username":"steve","email":"steve@example.com","role":"user"}}"#;
        std::env::set_var("TAKUMI_WEB_URL", mock_server("200 OK", same).await);
        assert!(matches!(
            revalidate(&state, &profile, &token).await,
            Revalidation::Valid(None)
        ));

        // Profile changed on the server: saved and reported.
        let changed = r#"{"user":{"id":"1","username":"steve","email":"steve@example.com","role":"editor"}}"#;
        std::env::set_var("TAKUMI_WEB_URL", mock_server("200 OK", changed).await);
        match revalidate(&state, &profile, &token).await {
            Revalidation::Valid(Some(updated)) => assert_eq!(updated.role, "editor"),
            _ => panic!("expected an updated profile"),
        }
        assert_eq!(TakumiSession::load(&state.root).unwrap().role, "editor");

        // A stale check must not end a newer sign-in.
        let rejected = mock_server("401 Unauthorized", r#"{"error":"caducada"}"#).await;
        std::env::set_var("TAKUMI_WEB_URL", &rejected);
        state.vault.store(VAULT_KEY, "token-2").unwrap();
        assert!(matches!(
            revalidate(&state, &profile, &token).await,
            Revalidation::Unknown
        ));
        assert!(cached_session(&state).unwrap().is_some());

        // Token rejected: the session is forgotten.
        sign_in(&state);
        assert!(matches!(
            revalidate(&state, &profile, &token).await,
            Revalidation::Ended
        ));
        assert!(cached_session(&state).unwrap().is_none());
        assert!(state.vault.load(VAULT_KEY).unwrap().is_none());

        std::env::remove_var("TAKUMI_WEB_URL");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

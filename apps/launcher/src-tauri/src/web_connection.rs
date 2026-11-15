//! Read-only website/API availability probe, independent of cached login state.

use crate::commands::{AppStateRef, CmdResult};
use crate::session::{self, NewsItem};
use download_manager::reqwest;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connected,
    Unreachable,
    Error,
}

#[derive(Serialize)]
pub struct WebConnection {
    status: ConnectionStatus,
    url: String,
    message: String,
    latency_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ProbeResponse {
    posts: Vec<NewsItem>,
}

async fn probe(client: &reqwest::Client, url: String) -> WebConnection {
    let start = Instant::now();
    let response = client
        .get(format!("{url}/api/news?limit=1"))
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let (status, message) = match response {
        Err(_) => (ConnectionStatus::Unreachable, session::unreachable().to_string()),
        Ok(response) if !response.status().is_success() => (
            ConnectionStatus::Error,
            format!("La web respondió con un error (HTTP {}).", response.status().as_u16()),
        ),
        Ok(response) => match response.json::<ProbeResponse>().await {
            Ok(body) => {
                let _ = body.posts;
                (
                    ConnectionStatus::Connected,
                    "Conexión con la API de Takumi verificada.".to_string(),
                )
            }
            Err(_) => (
                ConnectionStatus::Error,
                "La web no devolvió una respuesta válida de la API de Takumi.".to_string(),
            ),
        },
    };
    let latency_ms = (status == ConnectionStatus::Connected).then(|| start.elapsed().as_millis() as u64);
    WebConnection {
        status,
        url,
        message,
        latency_ms,
    }
}

#[tauri::command]
pub async fn check_web_connection(state: AppStateRef<'_>) -> CmdResult<WebConnection> {
    Ok(probe(state.http(), session::web_url()).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn probe_requires_a_working_takumi_api() {
        // All traffic stays on loopback; no account, vault or environment changes.
        for (status, body, expected) in [
            ("200 OK", r#"{"posts":[]}"#, ConnectionStatus::Connected),
            ("200 OK", "<html>Maintenance</html>", ConnectionStatus::Error),
            ("200 OK", r#"{"ok":true}"#, ConnectionStatus::Error),
            ("503 Service Unavailable", r#"{"posts":[]}"#, ConnectionStatus::Error),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                let size = socket.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /api/news?limit=1 "));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result = probe(&reqwest::Client::new(), url).await;
            assert_eq!(result.status, expected);
            assert_eq!(result.latency_ms.is_some(), expected == ConnectionStatus::Connected);
            server.await.unwrap();
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(
            probe(&reqwest::Client::new(), url).await.status,
            ConnectionStatus::Unreachable
        );
    }
}

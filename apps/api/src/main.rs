// TakumiLauncher Backend API
// Rust + Axum + Tokio

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use tracing::info;

#[tokio::main]
async fn main() {
    telemetry::init_telemetry(tracing::Level::INFO);

    info!("Starting TakumiLauncher API");

    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/info", get(api_info));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3001")
        .await
        .expect("Failed to bind to port 3001");

    info!("API listening on http://127.0.0.1:3001");

    axum::serve(listener, app).await.expect("Server error");
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "healthy" }))
}

async fn api_info() -> Json<Value> {
    Json(json!({
        "name": "TakumiLauncher API",
        "version": env!("CARGO_PKG_VERSION"),
        "endpoints": [
            "GET /api/v1/health",
            "GET /api/v1/info",
        ]
    }))
}

//! TakumiLauncher - Telemetry System
//!
//! Structured logging and telemetry collection (optional, no sensitive data).

use tracing::Level;
use tracing_subscriber::fmt;

/// Initialize telemetry
pub fn init_telemetry(level: Level) {
    fmt()
        .with_max_level(level)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();
}

/// Telemetry event (for optional analytics)
#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    pub event_type: String,
    pub timestamp: i64,
    pub user_id: Option<String>,
    pub data: serde_json::Value,
}

impl TelemetryEvent {
    pub fn new(event_type: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.into(),
            timestamp: chrono::Utc::now().timestamp(),
            user_id: None,
            data,
        }
    }

    pub fn with_user(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_event() {
        let event = TelemetryEvent::new("launcher_start", serde_json::json!({ "version": "0.1.0" }));
        assert_eq!(event.event_type, "launcher_start");
        assert!(event.user_id.is_none());
    }
}

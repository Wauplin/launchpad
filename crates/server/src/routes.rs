use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use launchpad_core::config::Config;
use launchpad_core::status::Status;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::ApiResult;

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/log", post(log))
}

async fn health(State(s): State<Server>) -> Json<Value> {
    Json(json!({"ok": true, "profile": s.app.paths.profile.name(), "pid": std::process::id()}))
}

async fn status(State(s): State<Server>) -> Json<Status> {
    Json(s.app.status.get())
}

async fn get_config(State(s): State<Server>) -> Json<Value> {
    Json(json!({"config": s.app.config.get(), "error": s.app.config.error()}))
}

async fn put_config(State(s): State<Server>, Json(config): Json<Config>) -> ApiResult<Value> {
    s.app.config.save(config)?;
    Ok(Json(json!({"config": s.app.config.get(), "error": null})))
}

#[derive(Deserialize)]
struct LogLine {
    level: Option<String>,
    message: String,
}

/// Lets the UI write to the daemon log (startup timings, uncaught errors).
async fn log(Json(line): Json<LogLine>) -> Json<Value> {
    match line.level.as_deref() {
        Some("error") => tracing::error!(target: "ui", "{}", line.message),
        Some("warn") => tracing::warn!(target: "ui", "{}", line.message),
        _ => tracing::info!(target: "ui", "{}", line.message),
    }
    Json(json!({"ok": true}))
}

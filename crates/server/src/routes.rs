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

async fn health(State(s): State<Server>, access: Option<axum::Extension<crate::auth::Access>>) -> Json<Value> {
    if access.is_none() {
        return Json(json!({"ok": true}));
    }
    Json(json!({"ok": true, "profile": s.app.paths.profile.name(), "pid": std::process::id()}))
}

async fn status(State(s): State<Server>) -> Json<Status> {
    Json(s.app.status.get())
}

fn browser_config(s: &Server) -> Config {
    let mut config = s.app.config.get();
    if s.auth.remote() {
        for variant in &mut config.variants { variant.env.clear(); }
    }
    config
}
async fn get_config(State(s): State<Server>) -> Json<Value> {
    Json(json!({"config": browser_config(&s), "error": s.app.config.error(), "remote": s.auth.remote()}))
}

async fn put_config(State(s): State<Server>, Json(mut config): Json<Config>) -> ApiResult<Value> {
    let saved = s.app.config.get();
    if config.server != saved.server {
        return Err(crate::error::ApiError::bad_request("Edit [server] in config.toml and restart the daemon"));
    }
    if s.auth.remote() {
        if config.commands != saved.commands || config.variants.iter().any(|v| !v.env.is_empty()) {
            return Err(crate::error::ApiError::bad_request("Edit [commands] and variant environments in config.toml on the host"));
        }
        for variant in &mut config.variants {
            if let Some(old) = saved.variant(&variant.id) { variant.env = old.env.clone(); }
        }
    }
    config.validate().map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    s.app.config.save(config)?;
    Ok(Json(json!({"config": browser_config(&s), "error": null, "remote": s.auth.remote()})))
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

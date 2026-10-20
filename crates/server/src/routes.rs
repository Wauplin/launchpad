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
        .route("/api/quota-history", get(quota_history))
        .route("/api/config", get(get_config).put(put_config).patch(patch_general_config))
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
    s.app.config.get()
}
async fn get_config(State(s): State<Server>) -> Json<Value> {
    Json(json!({"config": browser_config(&s), "error": s.app.config.error(), "remote": s.auth.remote()}))
}

async fn put_config(State(s): State<Server>, Json(config): Json<Config>) -> ApiResult<Value> {
    let saved = s.app.config.get();
    if config.server != saved.server {
        return Err(crate::error::ApiError::bad_request("Edit [server] in config.toml and restart the daemon"));
    }
    if s.auth.remote() && config.commands != saved.commands {
        return Err(crate::error::ApiError::bad_request("Edit [commands] in config.toml on the host"));
    }
    config.validate().map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    s.app.config.save(config)?;
    Ok(Json(json!({"config": browser_config(&s), "error": null, "remote": s.auth.remote()})))
}

// General settings only send fields the owner edited; merge against the current host config.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneralPatch {
    projects_root: Option<String>,
    remove_published_worktree: Option<bool>,
    presets: Option<Vec<launchpad_core::config::Preset>>,
}
async fn patch_general_config(State(s): State<Server>, Json(patch): Json<GeneralPatch>) -> ApiResult<Value> {
    let mut config = s.app.config.get();
    if let Some(value) = patch.projects_root { config.projects_root = value; }
    if let Some(value) = patch.remove_published_worktree { config.remove_published_worktree = value; }
    if let Some(value) = patch.presets { config.presets = value; }
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

#[derive(Deserialize)]
struct HistoryRange { from: i64, until: i64 }

async fn quota_history(State(s): State<Server>, axum::extract::Query(range): axum::extract::Query<HistoryRange>) -> ApiResult<Vec<launchpad_core::quota_history::WeeklyReset>> {
    if range.from < 0 || range.until <= range.from || range.until.saturating_sub(range.from) > 32 * 86_400_000 {
        return Err(crate::error::ApiError::bad_request("Select a calendar month"));
    }
    Ok(Json(launchpad_core::quota_history::list(&s.app.state_db, range.from, range.until).await?))
}

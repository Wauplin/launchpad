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
        .route("/api/analytics/runs", get(agent_runs))
        .route("/api/config", get(get_config).put(put_config).patch(patch_general_config))
        .route("/api/settings/shared", get(get_shared).patch(patch_shared))
        .route("/api/log", post(log))
}

async fn health(State(s): State<Server>, access: Option<axum::Extension<crate::auth::Access>>) -> Json<Value> {
    if access.is_none() {
        return Json(json!({"ok": true}));
    }
    Json(json!({"ok": true, "env": s.app.paths.app_env.name(), "pid": std::process::id()}))
}

async fn status(State(s): State<Server>) -> Json<Status> {
    Json(s.app.status.get())
}

fn config_response(s: &Server, error: Option<String>) -> Value {
    json!({
        "config": s.app.config.get(), "error": error, "remote": s.auth.remote(),
        "delegated": std::env::var("LAUNCHPAD_DELEGATED_ACCESS").as_deref() == Ok("true"),
        "external_setup": std::env::var("LAUNCHPAD_EXTERNAL_SETUP").ok()
            .filter(|s| s.starts_with('/') && !s.starts_with("//") && !s.contains('\\'))
    })
}
async fn get_config(State(s): State<Server>) -> Json<Value> {
    Json(config_response(&s, s.app.config.error()))
}

async fn put_config(State(s): State<Server>, Json(config): Json<Config>) -> ApiResult<Value> {
    let remote = s.auth.remote();
    s.app.config.update(|saved| {
        anyhow::ensure!(config.server == saved.server, "Edit [server] in config.toml and restart the daemon");
        anyhow::ensure!(!remote || config.commands == saved.commands, "Edit [commands] in config.toml on the host");
        *saved = config;
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(config_response(&s, None)))
}

// General settings only send fields the owner edited; merge against the current host config.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneralPatch {
    projects_root: Option<String>,
    remove_published_worktree: Option<bool>,
    presets: Option<Vec<launchpad_core::config::Preset>>,
    notify: Option<bool>,
    github_poll_interval_seconds: Option<u64>,
    worktrees_keep: Option<u32>,
    browser: Option<String>,
}
async fn patch_general_config(State(s): State<Server>, Json(patch): Json<GeneralPatch>) -> ApiResult<Value> {
    if patch.browser.is_some() && s.auth.remote() {
        return Err(crate::error::ApiError::bad_request("Edit [commands] in config.toml on the host"));
    }
    s.app.config.update(|config| {
        if let Some(value) = patch.projects_root { config.projects_root = value; }
        if let Some(value) = patch.remove_published_worktree { config.remove_published_worktree = value; }
        if let Some(value) = patch.presets { config.presets = value; }
        if let Some(value) = patch.notify { config.agents.notify = value; }
        if let Some(value) = patch.github_poll_interval_seconds { config.github.poll_interval_seconds = value.max(15); }
        if let Some(value) = patch.worktrees_keep { config.worktrees.keep = value; }
        if let Some(value) = patch.browser { config.commands.browser = value; }
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(config_response(&s, None)))
}

/// The shared settings: the closed list of config.toml values that sessions with admin Launchpad CLI access may
/// change (`settings.set` in the action list), with Undo. Profiles, server, commands and the rest of
/// config.toml stay the owner's. The keys match the PATCH body, so the activity list can put old values back.
fn shared(config: &Config) -> Value {
    json!({
        "presets": config.presets,
        "notify": config.agents.notify,
        "github_poll_interval_seconds": config.github.poll_interval_seconds,
        "worktrees_keep": config.worktrees.keep,
    })
}

async fn get_shared(State(s): State<Server>) -> Json<Value> {
    Json(shared(&s.app.config.get()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedPatch {
    presets: Option<Vec<launchpad_core::config::Preset>>,
    notify: Option<bool>,
    github_poll_interval_seconds: Option<u64>,
    worktrees_keep: Option<u32>,
}

async fn patch_shared(State(s): State<Server>, Json(patch): Json<SharedPatch>) -> ApiResult<Value> {
    if let Some(presets) = &patch.presets {
        let mut ids = std::collections::HashSet::new();
        for p in presets {
            // An empty text is a preset being written in Settings: launches skip it.
            if p.id.trim().is_empty() || p.label.trim().is_empty() || !ids.insert(p.id.as_str()) {
                return Err(crate::error::ApiError::bad_request("each preset needs a unique id and a label"));
            }
        }
    }
    s.app.config.update(|config| {
        if let Some(presets) = patch.presets { config.presets = presets; }
        if let Some(v) = patch.notify { config.agents.notify = v; }
        if let Some(v) = patch.github_poll_interval_seconds { config.github.poll_interval_seconds = v.max(15); }
        if let Some(v) = patch.worktrees_keep { config.worktrees.keep = v; }
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(shared(&s.app.config.get())))
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

async fn agent_runs(State(s): State<Server>, axum::extract::Query(range): axum::extract::Query<HistoryRange>) -> ApiResult<Vec<launchpad_core::analytics::AgentRun>> {
    if range.from < 0 || range.until <= range.from || range.until.saturating_sub(range.from) > 32 * 86_400_000 {
        return Err(crate::error::ApiError::bad_request("Select a calendar month"));
    }
    Ok(Json(launchpad_core::analytics::runs(&s.app.state_db, range.from, range.until).await?))
}

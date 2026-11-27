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
        .route("/api/version", get(version))
        .route("/api/analytics/runs", get(agent_runs))
        .route("/api/config", get(get_config).put(put_config).patch(patch_general_config))
        .route("/api/settings/shared", get(get_shared).patch(patch_shared))
        .route("/api/settings/profiles/{id}", get(get_profile_defaults).patch(patch_profile_defaults))
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

/// The client API level of this Home (`GET /api/version`): raise it when a separately released client (the desktop
/// companion) must change to keep working, and `MIN_CLIENT` when Home stops serving older clients.
pub const API_VERSION: u32 = 1;
pub const MIN_CLIENT: u32 = 1;
/// What separately released clients may test for, so they degrade per feature instead of per version.
pub const CAPABILITIES: &[&str] = &[
    // `POST /_home/desktop/connect` (Space) and reverse pairing answering JSON to `X-Launchpad-Client: desktop/…`.
    "desktop_device",
    // `DELETE /api/devices/self`.
    "devices_self",
    // `group` on light session records and `sessions` bus events (`Session::group`).
    "session_group",
    // The Android app: `POST /_home/phone/connect` (Space) and devices of kind `phone` (`X-Launchpad-Client: android/…`).
    "phone_device",
    // `GET /api/events?topics=`: only the topics a client names (prefixes).
    "event_topics",
    // `POST /api/devices/self/presence`: presence ("seen", notification suppression) for clients without a WebSocket.
    "device_presence",
    // `PUT`/`DELETE /api/devices/self/push` (a phone's FCM token) and `GET /api/phone/notifications`.
    "device_push",
    // `/folio-embed/`: the folio reader alone, for a native client's WebView (`window.LaunchpadNative`).
    "folio_embed",
];

fn version_body(commit: &str, role: &str) -> Value {
    json!({"api": API_VERSION, "min_client": MIN_CLIENT, "commit": commit, "role": role, "capabilities": CAPABILITIES})
}

/// Any signed-in client (a browser, the extension, the desktop companion, the CLI) learns what this Home serves.
async fn version(State(s): State<Server>) -> Json<Value> {
    Json(version_body(launchpad_core::commit(), s.app.paths.role.name()))
}

fn config_response(s: &Server, error: Option<String>) -> Value {
    json!({
        "config": s.app.config.get(), "error": error, "remote": s.auth.remote(),
        "delegated": std::env::var("LAUNCHPAD_DELEGATED_ACCESS").as_deref() == Ok("true"),
    })
}
async fn get_config(State(s): State<Server>) -> Json<Value> {
    Json(config_response(&s, s.app.config.error()))
}

async fn put_config(State(s): State<Server>, Json(config): Json<Config>) -> ApiResult<Value> {
    s.app.config.update(|saved| {
        anyhow::ensure!(config.server == saved.server, "Edit [server] in config.toml and restart the daemon");
        *saved = config;
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(config_response(&s, None)))
}

// General settings only send fields the owner edited; merge against the current host config.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneralPatch {
    presets: Option<Vec<launchpad_core::config::Preset>>,
    worktrees_keep: Option<u32>,
}
async fn patch_general_config(State(s): State<Server>, Json(patch): Json<GeneralPatch>) -> ApiResult<Value> {
    s.app.config.update(|config| {
        if let Some(value) = patch.presets { config.presets = value; }
        if let Some(value) = patch.worktrees_keep { config.worktrees.keep = value; }
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(config_response(&s, None)))
}

/// The shared settings: the closed list of config.toml values that sessions with admin Launchpad CLI access may
/// change (`settings.set` in the action list), with Undo. Profiles, server and the rest of
/// config.toml stay the owner's. The keys match the PATCH body, so the activity list can put old values back.
fn shared(config: &Config) -> Value {
    json!({
        "presets": config.presets,
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
        if let Some(v) = patch.worktrees_keep { config.worktrees.keep = v; }
        Ok(())
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(shared(&s.app.config.get())))
}

/// A profile's non-secret defaults (`profile.set`): the closed list of profile fields that sessions with admin
/// Launchpad CLI access may change, with Undo. The keys match the PATCH body, so the activity list can put old
/// values back. Env (it may hold secrets or references), login, billing (paid usage), cost and spending limits, the
/// model list URL, harness and ID stay the owner's (Settings → Profiles, `PUT /api/config`).
fn profile_defaults(p: &launchpad_core::config::Profile) -> Value {
    json!({"model": p.model, "effort": p.effort})
}

fn no_profile(id: &str) -> crate::error::ApiError {
    crate::error::ApiError::not_found(format!("no profile {id}: `launchpad profiles list` lists them"))
}

async fn get_profile_defaults(State(s): State<Server>, axum::extract::Path(id): axum::extract::Path<String>) -> ApiResult<Value> {
    let config = s.app.config.get();
    Ok(Json(profile_defaults(config.profile(&id).ok_or_else(|| no_profile(&id))?)))
}

/// Absent: unchanged; null or empty: cleared (the harness's default); a string: set.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfilePatch {
    #[serde(default, deserialize_with = "set_or_clear")]
    model: Option<Option<String>>,
    #[serde(default, deserialize_with = "set_or_clear")]
    effort: Option<Option<String>>,
}

fn set_or_clear<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    let v: Option<String> = Option::deserialize(d)?;
    Ok(Some(v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())))
}

/// Pure: why a profile's default model and effort cannot be saved, with what the owner's form offers
/// (`ProfilesView`): a profile with its own model list (`models_url`) takes any model id of it and no effort;
/// otherwise the model is one of the harness's known models (when the list is known) and the effort one the model
/// offers, else one of the harness's. `${LAUNCHPAD_MODEL}` in the env needs a default model.
fn defaults_problem(p: &launchpad_core::config::Profile, d: &launchpad_core::model::HarnessDefaults) -> Option<String> {
    let name = &p.name;
    if let Some(m) = &p.model {
        if m.chars().count() > 200 || m.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Some(format!("{m:?} is not a model id (no spaces, at most 200 characters)"));
        }
        if p.models_url.is_none() && !d.models.is_empty() && !d.models.iter().any(|x| &x.id == m) {
            let known: Vec<&str> = d.models.iter().map(|x| x.id.as_str()).collect();
            return Some(format!("{m} is not a model of {name}'s harness; known models: {}", known.join(", ")));
        }
    } else if p.env.values().any(|v| v.contains("${LAUNCHPAD_MODEL}")) {
        return Some(format!("{name} uses ${{LAUNCHPAD_MODEL}} in its environment, so it needs a default model"));
    }
    let Some(e) = &p.effort else { return None };
    if p.models_url.is_some() {
        return Some(format!("{name} uses its own model list, which has no effort levels"));
    }
    let model = p.model.as_ref().or(d.model.as_ref());
    let efforts = match model.and_then(|m| d.models.iter().find(|x| &x.id == m)) {
        Some(x) if !x.efforts.is_empty() => &x.efforts,
        _ => &d.efforts,
    };
    if efforts.is_empty() {
        return Some(format!("{name}'s harness does not support an effort setting"));
    }
    if !efforts.contains(e) {
        let of = model.map(|m| format!("{m} offers")).unwrap_or_else(|| "the harness offers".into());
        return Some(format!("effort {e} is not available: {of} {}", efforts.join(", ")));
    }
    None
}

async fn patch_profile_defaults(State(s): State<Server>, axum::extract::Path(id): axum::extract::Path<String>, Json(patch): Json<ProfilePatch>) -> ApiResult<Value> {
    let harness = s.app.config.get().profile(&id).ok_or_else(|| no_profile(&id))?.harness;
    let defaults = s.app.models.defaults(&s.app, harness);
    let saved = s.app.config.update(|config| {
        let p = config.profiles.iter_mut().find(|p| p.id == id).ok_or_else(|| anyhow::anyhow!("no profile {id}"))?;
        anyhow::ensure!(p.harness == harness, "profile {id} changed its harness meanwhile: try again");
        if let Some(m) = patch.model { p.model = m; }
        if let Some(e) = patch.effort { p.effort = e; }
        if let Some(why) = defaults_problem(p, &defaults) { anyhow::bail!(why); }
        Ok(profile_defaults(p))
    }).map_err(|e| crate::error::ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(saved))
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

#[cfg(test)]
mod tests {
    use super::defaults_problem;
    use launchpad_core::config::Profile;
    use launchpad_core::model::{HarnessDefaults, ModelInfo};

    fn profile(model: Option<&str>, effort: Option<&str>) -> Profile {
        Profile {
            id: "codex".into(), name: "Codex".into(), harness: launchpad_core::model::HarnessKind::new("codex"), env: Default::default(),
            model: model.map(str::to_string), effort: effort.map(str::to_string), billing: Default::default(), budget_usd: None,
            models_url: None, spend_limits: vec![], login: None,
        }
    }
    fn model(id: &str, efforts: &[&str]) -> ModelInfo {
        ModelInfo { id: id.into(), display_name: String::new(), description: String::new(), efforts: efforts.iter().map(|e| e.to_string()).collect(), providers: vec![] }
    }

    #[test]
    fn profile_defaults_follow_the_owners_form() {
        let d = HarnessDefaults { model: Some("a".into()), effort: None, models: vec![model("a", &["low", "high"]), model("b", &[])], efforts: vec!["medium".into()] };
        assert_eq!(defaults_problem(&profile(Some("a"), Some("high")), &d), None);
        assert_eq!(defaults_problem(&profile(None, Some("low")), &d), None, "the harness's default model's efforts");
        assert_eq!(defaults_problem(&profile(Some("b"), Some("medium")), &d), None, "a model without its own list takes the harness's");
        assert_eq!(defaults_problem(&profile(None, None), &d), None);
        assert!(defaults_problem(&profile(Some("a"), Some("medium")), &d).unwrap().contains("a offers low, high"));
        assert!(defaults_problem(&profile(Some("zzz"), None), &d).unwrap().contains("known models: a, b"));
        assert!(defaults_problem(&profile(Some("a b"), None), &d).is_some());
        // Unknown list (not read yet): any well-formed id; no effort levels at all: no effort.
        let empty = HarnessDefaults { model: None, effort: None, models: vec![], efforts: vec![] };
        assert_eq!(defaults_problem(&profile(Some("new-model"), None), &empty), None);
        assert!(defaults_problem(&profile(Some("new-model"), Some("high")), &empty).is_some());
        // A provider's own model list: any id, no effort.
        let mut own = profile(Some("org/model:fast"), None);
        own.models_url = Some("https://example.com/v1/models".into());
        assert_eq!(defaults_problem(&own, &d), None);
        own.effort = Some("high".into());
        assert!(defaults_problem(&own, &d).is_some());
        // ${LAUNCHPAD_MODEL} needs a default model.
        let mut referenced = profile(None, None);
        referenced.env.insert("MODEL".into(), "${LAUNCHPAD_MODEL}".into());
        assert!(defaults_problem(&referenced, &d).is_some());
    }
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn version_names_levels_and_capabilities() {
        let v = super::version_body("abc123", "home");
        assert_eq!(v["api"], super::API_VERSION);
        assert!(v["min_client"].as_u64().unwrap() <= v["api"].as_u64().unwrap());
        assert_eq!(v["commit"], "abc123");
        let caps: Vec<&str> = v["capabilities"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        for c in ["desktop_device", "devices_self", "session_group"] {
            assert!(caps.contains(&c), "{c}");
        }
    }
}

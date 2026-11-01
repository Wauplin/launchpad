//! Host setup helpers used from a browser: sign-in checks (Settings → Profiles) and Clone from GitHub.
use crate::{Server, error::{ApiError, ApiResult}};
use axum::{Json, Router, extract::{Path, Query, State}, routing::{get, post}};
use launchpad_core::{model::Project, onboarding, project_clone, setup_checklist, signin, terminals::TerminalInfo, workspace};
use serde::Deserialize;

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/setup/sign-in", get(sign_in))
        .route("/api/setup/trace-backup", get(trace_backup))
        .route("/api/projects/clone", get(clone_suggestions).post(clone))
        .route("/api/projects/clone/suggestions", post(refresh_clone_suggestions))
        .route("/api/onboarding", get(onboarding).patch(patch_onboarding))
        .route("/api/setup/checklist", get(checklist))
        .route("/api/setup/checklist/summary", get(checklist_summary))
        .route("/api/setup/fix/{item}", post(fix))
        .route("/api/setup/git-identity", post(git_identity))
        .route("/api/setup/profile-from-login", post(profile_from_login))
}

async fn sign_in(State(s): State<Server>) -> Json<Vec<signin::SignInCheck>> {
    Json(signin::check_all(&s.app).await)
}

async fn clone_suggestions(State(s): State<Server>) -> ApiResult<Vec<project_clone::CloneSuggestion>> {
    Ok(Json(project_clone::suggestions(&s.app).await?))
}

async fn refresh_clone_suggestions(State(s): State<Server>) -> ApiResult<Vec<project_clone::CloneSuggestion>> {
    Ok(Json(project_clone::refresh_suggestions(&s.app).await?))
}

#[derive(Deserialize)]
struct Clone {
    repo: String,
    /// Clone onto this SSH host, into its projects root.
    host: Option<String>,
    workspace_id: Option<String>,
}

async fn clone(State(s): State<Server>, Json(b): Json<Clone>) -> ApiResult<Project> {
    let p = project_clone::clone(&s.app, &b.repo, b.host.as_deref()).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    if let Some(ws) = &b.workspace_id {
        workspace::add_project(&s.app, ws, &p.id).await?;
    }
    Ok(Json(p))
}

async fn trace_backup(State(s): State<Server>) -> Json<serde_json::Value> {
    let (check, account) = launchpad_core::trace_backup::check(&s.app.env).await;
    let state = s.app.trace_backup.0.read().unwrap().clone();
    Json(serde_json::json!({"check": check, "account": account, "error": state.error, "last_upload": state.last_upload, "last_sync": state.last_sync}))
}

async fn onboarding(State(s): State<Server>) -> ApiResult<onboarding::OnboardingState> {
    Ok(Json(onboarding::get(&s.app).await?))
}

async fn patch_onboarding(State(s): State<Server>, Json(p): Json<onboarding::Patch>) -> ApiResult<onboarding::OnboardingState> {
    Ok(Json(onboarding::patch(&s.app, p).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

#[derive(Deserialize)]
struct HostQuery {
    /// SSH host, or none for this computer.
    host: Option<String>,
}

fn known_host(s: &Server, host: Option<String>) -> Result<Option<String>, ApiError> {
    let host = host.filter(|h| !h.is_empty());
    if let Some(h) = &host
        && s.app.hosts.status(h).is_none()
    {
        return Err(ApiError::not_found(format!("unknown machine {h}")));
    }
    Ok(host)
}

/// Every setup item of this computer or an SSH host: state, detected values with their source, missing parts.
async fn checklist(State(s): State<Server>, Query(q): Query<HostQuery>) -> ApiResult<setup_checklist::Checklist> {
    let host = known_host(&s, q.host)?;
    let devices = if host.is_none() {
        let remote = s.auth.remote();
        let tailscale = if remote { None } else { Some(s.app.env.which("tailscale").is_some()) };
        Some(setup_checklist::devices(remote, &s.auth.origin(), tailscale))
    } else {
        None
    };
    Ok(Json(setup_checklist::checklist(&s.app, host.as_deref(), devices).await))
}

async fn checklist_summary(State(s): State<Server>) -> Json<setup_checklist::ChecklistSummary> {
    Json(setup_checklist::summary(&s.app).await)
}

#[derive(Deserialize)]
struct Fix {
    part: String,
    host: Option<String>,
}

/// Opens a setup terminal with the fixed command of a missing part. Parts detected as set up are refused.
async fn fix(State(s): State<Server>, Path(item): Path<String>, Json(b): Json<Fix>) -> ApiResult<TerminalInfo> {
    let host = known_host(&s, b.host)?;
    let (title, command) = setup_checklist::fix_command(&s.app, host.as_deref(), &item, &b.part).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    setup_checklist::invalidate();
    Ok(Json(s.app.terminals.open_setup_command(&s.app, &title, &command, host.as_deref()).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

#[derive(Deserialize)]
struct Identity {
    name: Option<String>,
    email: Option<String>,
    host: Option<String>,
}

async fn git_identity(State(s): State<Server>, Json(b): Json<Identity>) -> ApiResult<serde_json::Value> {
    let host = known_host(&s, b.host)?;
    setup_checklist::set_identity(&s.app, host.as_deref(), b.name.as_deref(), b.email.as_deref()).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
struct FromLogin {
    harness: String,
}

async fn profile_from_login(State(s): State<Server>, Json(b): Json<FromLogin>) -> ApiResult<launchpad_core::config::Profile> {
    Ok(Json(setup_checklist::profile_from_login(&s.app, &b.harness).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

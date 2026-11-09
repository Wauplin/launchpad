//! Setup helpers used from a browser: sign-in checks (Settings → Profiles) and Clone from GitHub.
use crate::{Ctx, error::{ApiError, ApiResult}};
use axum::{Json, Router, extract::{Path, State}, routing::{get, post}};
use launchpad_core::{model::Project, signin, terminals::TerminalInfo};
use launchpad_setup::{home_update, onboarding, project_clone, setup_checklist};
use serde::Deserialize;

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/setup/sign-in", get(sign_in))
        .route("/api/setup/huggingface", get(huggingface))
        .route("/api/projects/clone", get(clone_suggestions).post(clone))
        .route("/api/projects/clone/suggestions", post(refresh_clone_suggestions))
        .route("/api/onboarding", get(onboarding).patch(patch_onboarding))
        .route("/api/setup/checklist", get(checklist))
        .route("/api/setup/checklist/summary", get(checklist_summary))
        .route("/api/setup/fix/{item}", post(fix))
        .route("/api/setup/git-identity", post(git_identity))
        .route("/api/setup/profile-from-login", post(profile_from_login))
        .route("/api/home/update", get(home_update).post(start_home_update))
        .route("/api/home/update/check", post(check_home_update))
}

async fn sign_in(State(s): State<Ctx>) -> Json<Vec<signin::SignInCheck>> {
    Json(signin::check_all(&s.app).await)
}

async fn clone_suggestions(State(s): State<Ctx>) -> ApiResult<Vec<project_clone::CloneSuggestion>> {
    Ok(Json(project_clone::suggestions(&s.app).await?))
}

async fn refresh_clone_suggestions(State(s): State<Ctx>) -> ApiResult<Vec<project_clone::CloneSuggestion>> {
    Ok(Json(project_clone::refresh_suggestions(&s.app).await?))
}

#[derive(Deserialize)]
struct Clone {
    repo: String,
    /// The one workspace of the new folder (required).
    workspace_id: String,
}

async fn clone(State(s): State<Ctx>, Json(b): Json<Clone>) -> ApiResult<Project> {
    // Folders of Home: Home mirrors the repository, runners clone it on first use.
    if s.app.paths.role == launchpad_core::role::Role::Runner {
        let v = launchpad_core::home_link::home_request("POST", "/api/projects/clone", Some(serde_json::json!({"repo": b.repo, "workspace_id": b.workspace_id}))).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
        let p: Project = serde_json::from_value(v).map_err(|e| ApiError::bad_request(format!("{e}")))?;
        launchpad_core::projects::store_home(&s.app, vec![p.clone()], false).await?;
        return Ok(Json(launchpad_core::projects::get(&s.app, &p.id).await?.unwrap_or(p)));
    }
    let p = project_clone::clone_to_home(&s.app, &b.repo, &b.workspace_id).await;
    Ok(Json(p.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

async fn huggingface(State(s): State<Ctx>) -> Json<serde_json::Value> {
    let (check, account) = launchpad_core::hf::check(&s.app.env).await;
    Json(serde_json::json!({"check": check, "account": account}))
}

async fn onboarding(State(s): State<Ctx>) -> ApiResult<onboarding::OnboardingState> {
    Ok(Json(onboarding::get(&s.app).await?))
}

async fn patch_onboarding(State(s): State<Ctx>, Json(p): Json<onboarding::Patch>) -> ApiResult<onboarding::OnboardingState> {
    Ok(Json(onboarding::patch(&s.app, p).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

/// Every setup item of this computer: state, detected values with their source, missing parts.
async fn checklist(State(s): State<Ctx>) -> ApiResult<setup_checklist::Checklist> {
    let remote = s.auth.remote();
    let tailscale = if remote { None } else { Some(s.app.env.which("tailscale").is_some()) };
    let devices = Some(setup_checklist::devices(remote, &s.auth.origin(), tailscale));
    Ok(Json(setup_checklist::checklist(&s.app, devices).await))
}

async fn checklist_summary(State(s): State<Ctx>) -> Json<setup_checklist::ChecklistSummary> {
    Json(setup_checklist::summary(&s.app).await)
}

#[derive(Deserialize)]
struct Fix {
    part: String,
}

/// Opens a setup terminal with the fixed command of a missing part. Parts detected as set up are refused.
async fn fix(State(s): State<Ctx>, Path(item): Path<String>, Json(b): Json<Fix>) -> ApiResult<TerminalInfo> {
    let (title, command) = setup_checklist::fix_command(&s.app, &item, &b.part).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    setup_checklist::invalidate();
    Ok(Json(s.app.terminals.open_setup_command(&s.app, &title, &command).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

#[derive(Deserialize)]
struct Identity {
    name: Option<String>,
    email: Option<String>,
}

async fn git_identity(State(s): State<Ctx>, Json(b): Json<Identity>) -> ApiResult<serde_json::Value> {
    setup_checklist::set_identity(&s.app, b.name.as_deref(), b.email.as_deref()).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
struct FromLogin {
    harness: String,
}

async fn profile_from_login(State(s): State<Ctx>, Json(b): Json<FromLogin>) -> ApiResult<launchpad_core::config::Profile> {
    Ok(Json(setup_checklist::profile_from_login(&s.app, &b.harness).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

/// Home on a Hugging Face Space: the last update check (`launchpad_setup::home_update`).
async fn home_update(State(s): State<Ctx>) -> Json<home_update::HomeUpdate> {
    Json(home_update::current(&s.app))
}

async fn check_home_update(State(s): State<Ctx>) -> Json<home_update::HomeUpdate> {
    Json(home_update::check(&s.app).await)
}

async fn start_home_update(State(s): State<Ctx>, Json(b): Json<home_update::Start>) -> ApiResult<home_update::HomeUpdate> {
    Ok(Json(home_update::start(&s.app, b).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

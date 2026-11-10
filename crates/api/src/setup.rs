//! Setup helpers used from a browser: Home's GitHub and Hugging Face sign-ins, Clone from GitHub, onboarding, a
//! runner's setup checklist, Home updates and agent CLI updates.
use crate::{Ctx, error::{ApiError, ApiResult}};
use axum::{Json, Router, extract::{Path, State}, routing::{get, post, put}};
use launchpad_core::{model::Project, signin};
use launchpad_setup::{cli_update, home_update, onboarding, project_clone, setup_checklist};
use serde::Deserialize;

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/setup/sign-in", get(sign_in))
        .route("/api/setup/huggingface", get(huggingface))
        .route("/api/projects/clone", get(clone_suggestions).post(clone))
        .route("/api/projects/clone/suggestions", post(refresh_clone_suggestions))
        .route("/api/onboarding", get(onboarding).patch(patch_onboarding))
        .route("/api/setup/checklist", get(checklist))
        .route("/api/home/update", get(home_update).post(start_home_update))
        .route("/api/home/update/check", post(check_home_update))
        .route("/api/cli-updates", get(cli_updates))
        .route("/api/cli-updates/check", post(check_cli_updates))
        .route("/api/cli-updates/auto", put(set_cli_auto))
        .route("/api/cli-updates/{id}/approve", post(approve_cli_update))
        .route("/api/cli-updates/{id}/retry", post(retry_cli_update))
        .route("/api/cli-updates/{id}/dismiss", post(dismiss_cli_update))
        .route("/api/runner/clis", get(runner_clis))
        .route("/api/runner/clis/update", post(update_runner_cli))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// Home: agent CLI updates on the runners (`launchpad_setup::cli_update`).
async fn cli_updates(State(s): State<Ctx>) -> ApiResult<cli_update::CliUpdates> {
    Ok(Json(cli_update::overview(&s.app).await.map_err(bad)?))
}

async fn check_cli_updates(State(s): State<Ctx>) -> ApiResult<cli_update::CliUpdates> {
    Ok(Json(cli_update::check(&s.app).await.map_err(bad)?))
}

async fn set_cli_auto(State(s): State<Ctx>, Json(b): Json<cli_update::Auto>) -> ApiResult<cli_update::CliUpdates> {
    Ok(Json(cli_update::set_auto(&s.app, b).await.map_err(bad)?))
}

async fn approve_cli_update(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<cli_update::CliUpdate> {
    Ok(Json(cli_update::approve(&s.app, &id).await.map_err(bad)?))
}

async fn retry_cli_update(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<cli_update::CliUpdate> {
    Ok(Json(cli_update::retry(&s.app, &id).await.map_err(bad)?))
}

async fn dismiss_cli_update(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<cli_update::CliUpdate> {
    Ok(Json(cli_update::dismiss(&s.app, &id).await.map_err(bad)?))
}

/// Runner: its CLI versions and updates, and Home's request to install one (through the tunnel only).
async fn runner_clis(State(s): State<Ctx>) -> ApiResult<cli_update::RunnerClis> {
    Ok(Json(cli_update::runner_state(&s.app).await.map_err(bad)?))
}

async fn update_runner_cli(State(s): State<Ctx>, Json(b): Json<cli_update::Request>) -> ApiResult<cli_update::Job> {
    Ok(Json(cli_update::start_job(&s.app, b).await.map_err(bad)?))
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

/// A runner's setup items: state, detected values with their source, missing parts. Home's setup is the UI's.
async fn checklist(State(s): State<Ctx>) -> ApiResult<setup_checklist::Checklist> {
    launchpad_core::role::ensure_runs_agents(s.app.paths.role).map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(setup_checklist::checklist(&s.app).await))
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

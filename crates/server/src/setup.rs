//! Host setup helpers used from a browser: sign-in checks (Settings → Profiles) and Clone from GitHub.
use crate::{Server, error::{ApiError, ApiResult}};
use axum::{Json, Router, extract::State, routing::get};
use launchpad_core::{model::Project, project_clone, signin, workspace};
use serde::Deserialize;

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/setup/sign-in", get(sign_in))
        .route("/api/setup/trace-backup", get(trace_backup))
        .route("/api/projects/clone", get(clone_suggestions).post(clone))
}

async fn sign_in(State(s): State<Server>) -> Json<Vec<signin::SignInCheck>> {
    Json(signin::check_all(&s.app).await)
}

async fn clone_suggestions(State(s): State<Server>) -> ApiResult<Vec<String>> {
    Ok(Json(project_clone::suggestions(&s.app).await?))
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
    Json(serde_json::json!({"check": check, "account": account, "error": state.error, "last_upload": state.last_upload}))
}

//! Task worktrees: session view, Settings → Worktrees, and the `launchpad work` commands.
use crate::{Server, error::{ApiError, ApiResult}};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use launchpad_core::worktrees;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/worktrees", get(list))
        .route("/api/worktrees/sizes", post(sizes))
        .route("/api/worktrees/finish", post(finish))
        .route("/api/worktrees/add-project", post(add_project))
        .route("/api/worktrees/{id}/{action}", post(action))
        .route("/api/sessions/{id}/worktrees", get(session))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> ApiResult<Value> {
    let mut out = vec![];
    for w in worktrees::list(&s.app).await? {
        out.push(worktrees::info(&s.app, w).await);
    }
    Ok(Json(json!(out)))
}

async fn sizes(State(s): State<Server>) -> ApiResult<Value> {
    Ok(Json(json!(worktrees::sizes(&s.app).await?)))
}

async fn session(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!(worktrees::session_info(&s.app, &id).await?)))
}

#[derive(Deserialize)]
struct Finish {
    session_id: String,
}
async fn finish(State(s): State<Server>, Json(b): Json<Finish>) -> ApiResult<Value> {
    Ok(Json(json!({ "message": worktrees::finish(&s.app, &b.session_id).await.map_err(bad)? })))
}

#[derive(Deserialize)]
struct AddProject {
    session_id: String,
    project: String,
}
async fn add_project(State(s): State<Server>, Json(b): Json<AddProject>) -> ApiResult<Value> {
    Ok(Json(json!({ "message": worktrees::add_project(&s.app, &b.session_id, &b.project).await.map_err(bad)? })))
}

async fn action(State(s): State<Server>, Path((id, a)): Path<(String, String)>) -> ApiResult<Value> {
    let app = &s.app;
    let w = match a.as_str() {
        "merge" => worktrees::request_merge(app, &id, false).await,
        "cancel-merge" => worktrees::cancel_merge(app, &id).await,
        "fix" => worktrees::fix_with_agent(app, &id).await.and(worktrees::get(app, &id).await),
        "handoff" => worktrees::handoff(app, &id).await,
        "handback" => worktrees::handback(app, &id).await,
        "remove" => worktrees::remove(app, &id).await,
        "restore" => worktrees::restore(app, &id).await,
        "discard" => worktrees::discard(app, &id).await,
        _ => return Err(ApiError::not_found("unknown worktree action")),
    }
    .map_err(bad)?;
    Ok(Json(json!(worktrees::info(app, w).await)))
}

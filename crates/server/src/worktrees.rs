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

fn home(s: &Server) -> bool {
    s.app.paths.role == launchpad_core::role::Role::Home
}

/// Home never touches a worktree folder: the relay sends requests that name their session (`?session=`) to its
/// runner, or answers them from Home Git when it is away (`runners::relay`). What reaches Home's handlers is refused.
fn not_on_home(s: &Server) -> Result<(), ApiError> {
    if home(s) {
        return Err(ApiError::bad_request("Home has no worktrees: they live on the runner of their session. Reload the page and use the session's own view."));
    }
    Ok(())
}

async fn list(State(s): State<Server>) -> ApiResult<Value> {
    // Home has no worktree: they live on the runners.
    if home(&s) {
        let all = crate::runners::ask_connected(&s, "GET", "/api/worktrees").await;
        let out: Vec<Value> = all.into_iter().filter_map(|v| v.as_array().cloned()).flatten().collect();
        // Merges and Local PRs are Home's; worktrees whose folder is gone are read from their branch on Home.
        let mut out = launchpad_core::away::complete(&s.app, out).await;
        // Sessions of offline runners and stopped Burst jobs: from their branches on Home.
        out.extend(launchpad_core::away::all_worktrees(&s.app).await.into_iter().map(|i| json!(i)));
        return Ok(Json(json!(out)));
    }
    let mut out = vec![];
    for w in worktrees::list(&s.app).await? {
        out.push(worktrees::info(&s.app, w).await);
    }
    Ok(Json(json!(out)))
}

async fn sizes(State(s): State<Server>) -> ApiResult<Value> {
    if home(&s) {
        let all = crate::runners::ask_connected(&s, "POST", "/api/worktrees/sizes").await;
        return Ok(Json(json!(all.into_iter().filter_map(|v| v.as_object().cloned()).flatten().collect::<serde_json::Map<_, _>>())));
    }
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
    not_on_home(&s)?;
    Ok(Json(json!({ "message": worktrees::finish(&s.app, &b.session_id).await.map_err(bad)? })))
}

#[derive(Deserialize)]
struct AddProject {
    session_id: String,
    project: String,
}
async fn add_project(State(s): State<Server>, Json(b): Json<AddProject>) -> ApiResult<Value> {
    not_on_home(&s)?;
    Ok(Json(json!({ "message": worktrees::add_project(&s.app, &b.session_id, &b.project).await.map_err(bad)? })))
}

#[derive(Deserialize, Default)]
struct ActionBody {
    /// Merge: the branch tip the owner reviewed.
    head: Option<String>,
}
async fn action(State(s): State<Server>, Path((id, a)): Path<(String, String)>, body: Option<Json<ActionBody>>) -> ApiResult<Value> {
    not_on_home(&s)?;
    let app = &s.app;
    let body = body.map(|b| b.0).unwrap_or_default();
    let w = match a.as_str() {
        "merge" => {
            let head = body.head.filter(|h| !h.is_empty()).ok_or_else(|| ApiError::bad_request("Merge needs the reviewed commit (head); reload the page and review again"))?;
            worktrees::request_merge(app, &id, false, Some(head)).await
        }
        "cancel-merge" => worktrees::cancel_merge(app, &id).await,
        "fix" => worktrees::fix_with_agent(app, &id).await.and(worktrees::get(app, &id).await),
        "remove" => worktrees::remove(app, &id).await,
        "restore" => worktrees::restore(app, &id).await,
        "discard" => worktrees::discard(app, &id).await,
        _ => return Err(ApiError::not_found("unknown worktree action")),
    }
    .map_err(bad)?;
    Ok(Json(json!(worktrees::info(app, w).await)))
}

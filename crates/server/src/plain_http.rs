//! Plain and asset folders over HTTP.
//!
//! On Home, runners call `/api/plain/<id>/…` with their credential (`X-Launchpad-Runner`) to read the tree,
//! download and upload content, commit changes and take the lock; the owner reads the history and restores
//! versions. A runner forwards the owner's requests to Home (`home_client::home_owned`).

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::plain::{self, SESSION_HEADER, Update};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/plain/{id}/tree", get(tree))
        .route("/api/plain/{id}/blob/{sha}", get(blob).put(upload).layer(DefaultBodyLimit::max(4 << 30)))
        .route("/api/plain/{id}/commit", post(commit).layer(DefaultBodyLimit::max(64 << 20)))
        .route("/api/plain/{id}/lock", post(lock))
        .route("/api/plain/{id}/unlock", post(unlock))
        .route("/api/plain/{id}/history", get(history))
        .route("/api/plain/{id}/restore", post(restore))
        .route("/api/plain/assets/{project}", post(create_asset))
        .route("/api/assets", post(create_in_worktree))
}

/// Runner requests (with their credential) skip the browser sign-in; the handlers check the credential.
pub fn runner_request(path: &str, headers: &HeaderMap) -> bool {
    path.starts_with("/api/plain/") && headers.contains_key(crate::runners::CREDENTIAL_HEADER)
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// Home: checks a runner credential when there is one (the owner passed the sign-in otherwise), and that the
/// named session still runs on that runner. Returns the session.
async fn caller(s: &Server, headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    if !launchpad_core::home_git::here(&s.app) {
        return Err(ApiError::bad_request("plain folders are stored by Home"));
    }
    let session = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
    if let Some(credential) = headers.get(crate::runners::CREDENTIAL_HEADER).and_then(|v| v.to_str().ok()) {
        let runner = launchpad_core::runners::authenticate(&s.app, credential).await.filter(|r| r.state == launchpad_core::runners::RunnerState::Approved).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"))?;
        if let Some(sid) = &session {
            launchpad_core::home_git::check_session(&s.app, Some(&runner.id), sid).map_err(|e| ApiError::new(StatusCode::FORBIDDEN, format!("{e:#}")))?;
        }
    }
    Ok(session)
}

async fn tree(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<plain::Tree> {
    caller(&s, &headers).await?;
    Ok(Json(plain::tree(&s.app, &id).await.map_err(bad)?))
}

async fn blob(State(s): State<Server>, Path((id, sha)): Path<(String, String)>, headers: HeaderMap) -> Result<Response, ApiError> {
    caller(&s, &headers).await?;
    plain::tree(&s.app, &id).await.map_err(bad)?;
    let data = plain::read_blob(&s.app, &sha).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

async fn upload(State(s): State<Server>, Path((id, sha)): Path<(String, String)>, headers: HeaderMap, body: Bytes) -> ApiResult<Value> {
    caller(&s, &headers).await?;
    plain::tree(&s.app, &id).await.map_err(bad)?;
    plain::write_blob(&s.app, &sha, &body).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn commit(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(updates): Json<Vec<Update>>) -> ApiResult<Value> {
    let session = caller(&s, &headers).await?.ok_or_else(|| ApiError::bad_request("a change needs the session that holds the folder"))?;
    Ok(Json(json!({"version": plain::commit(&s.app, &id, Some(&session), updates).await.map_err(bad)?})))
}

async fn lock(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    let session = caller(&s, &headers).await?.ok_or_else(|| ApiError::bad_request("the lock belongs to a session"))?;
    plain::lock(&s.app, &id, &session).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

async fn unlock(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    let session = caller(&s, &headers).await?.ok_or_else(|| ApiError::bad_request("the lock belongs to a session"))?;
    plain::unlock(&s.app, &id, &session).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn history(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    caller(&s, &headers).await?;
    let holder = plain::lock_holder(&s.app, &id).await?;
    Ok(Json(json!({"changes": plain::history(&s.app, &id).await.map_err(bad)?, "locked_by": holder})))
}

#[derive(Deserialize)]
struct Restore {
    change: i64,
}

async fn restore(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(b): Json<Restore>) -> ApiResult<Value> {
    caller(&s, &headers).await?;
    Ok(Json(json!({"version": plain::restore(&s.app, &id, b.change).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct NewAsset {
    name: String,
}

async fn create_asset(State(s): State<Server>, Path(project): Path<String>, headers: HeaderMap, Json(b): Json<NewAsset>) -> ApiResult<plain::Asset> {
    caller(&s, &headers).await?;
    Ok(Json(plain::create_asset(&s.app, &project, &b.name).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct InWorktree {
    session_id: String,
    cwd: String,
    path: String,
}

/// `launchpad assets create`: on the machine of the worktree.
async fn create_in_worktree(State(s): State<Server>, Json(b): Json<InWorktree>) -> ApiResult<Value> {
    Ok(Json(json!({"message": plain::create_in_worktree(&s.app, &b.session_id, &b.cwd, &b.path).await.map_err(bad)?})))
}

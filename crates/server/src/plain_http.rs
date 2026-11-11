//! Plain and asset folders over HTTP.
//!
//! On Home, runners call `/api/plain/<id>/…` with their credential (`X-Launchpad-Runner`) to read the tree,
//! download and upload content, commit changes and take the lock; the owner reads the history and restores
//! versions (never with a runner credential). A runner creates asset folders through its tunnel
//! (`runner_trust`), which reaches these routes without a runner header.

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
        .route("/api/plain/assets/{project}", post(create_asset).get(list_assets))
        .route("/api/plain/{id}/untrash", post(untrash))
        .route("/api/plain/{id}/keep", post(keep))
        .route("/api/assets", post(create_in_worktree))
}

/// Home: runner requests (with their credential) skip the browser sign-in; the handlers check the credential
/// (`caller`), and refuse a present but unreadable one. Never on a runner: it would forward the request to Home as
/// itself (`home_client::forward_writes`).
pub fn runner_request(role: launchpad_core::role::Role, path: &str, headers: &HeaderMap) -> bool {
    role == launchpad_core::role::Role::Home && path.starts_with("/api/plain/") && headers.contains_key(crate::runners::CREDENTIAL_HEADER)
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// Home: checks a runner credential when there is one (the owner passed the sign-in otherwise), that the
/// named session still runs on that runner, and that `store` belongs to a workspace of the sessions Home placed
/// on it (`scope::of_runner`: their knowledge base, bots, plain, asset and temporary folders), the named
/// session's own workspace for a session's request. Returns the
/// session. A credential header that is present always authenticates, even when it is unreadable:
/// `runner_request` skipped the sign-in because of it.
async fn caller(s: &Server, headers: &HeaderMap, store: Option<&str>) -> Result<Option<String>, ApiError> {
    if !launchpad_core::home_git::here(&s.app) {
        return Err(ApiError::bad_request("plain folders are stored by Home"));
    }
    let session = match headers.get(SESSION_HEADER) {
        Some(v) => Some(v.to_str().map_err(|_| ApiError::bad_request("unreadable session header"))?.to_string()),
        None => None,
    };
    if let Some(credential) = headers.get(crate::runners::CREDENTIAL_HEADER) {
        let credential = credential.to_str().unwrap_or("");
        let runner = launchpad_core::runners::authenticate(&s.app, credential).await.filter(|r| r.state == launchpad_core::runners::RunnerState::Approved).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"))?;
        if let Some(sid) = &session {
            launchpad_core::home_git::check_session(&s.app, &runner.id, sid).map_err(|e| ApiError::new(StatusCode::FORBIDDEN, format!("{e:#}")))?;
        }
        if let Some(store) = store {
            let ws = launchpad_core::scope::of_store(&s.app, store).await;
            // A session's request (its locks, syncs and commits; asset folders an agent maps in its worktree)
            // stays in the session's own workspace.
            if let Some(sid) = &session {
                let own = launchpad_core::scope::of_session(&s.app, sid).await;
                if own.is_none() || own != ws {
                    return Err(ApiError::new(StatusCode::FORBIDDEN, "this folder belongs to another workspace than the session"));
                }
            }
            if !launchpad_core::scope::runner_reads(&s.app, &runner.id, ws).await {
                return Err(ApiError::new(StatusCode::FORBIDDEN, "this folder belongs to a workspace with no session on this runner"));
            }
        }
    }
    Ok(session)
}

async fn tree(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<plain::Tree> {
    caller(&s, &headers, Some(&id)).await?;
    if let Some(ws) = launchpad_core::kb::workspace_of(&id) {
        launchpad_core::kb::ensure_seeded(&s.app, ws).await.map_err(bad)?;
    }
    Ok(Json(plain::tree(&s.app, &id).await.map_err(bad)?))
}

async fn blob(State(s): State<Server>, Path((id, sha)): Path<(String, String)>, headers: HeaderMap) -> Result<Response, ApiError> {
    caller(&s, &headers, Some(&id)).await?;
    plain::tree(&s.app, &id).await.map_err(bad)?;
    if !plain::holds(&s.app, &id, &sha).await.map_err(bad)? {
        return Err(ApiError::not_found("this folder holds no such content"));
    }
    let data = plain::read_blob(&s.app, &sha).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

async fn upload(State(s): State<Server>, Path((id, sha)): Path<(String, String)>, headers: HeaderMap, body: Bytes) -> ApiResult<Value> {
    caller(&s, &headers, Some(&id)).await?;
    plain::tree(&s.app, &id).await.map_err(bad)?;
    plain::write_blob(&s.app, &sha, &body).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn commit(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(updates): Json<Vec<Update>>) -> ApiResult<Value> {
    let session = caller(&s, &headers, Some(&id)).await?.ok_or_else(|| ApiError::bad_request("a change needs the session that holds the folder"))?;
    Ok(Json(json!({"version": plain::commit(&s.app, &id, Some(&session), updates).await.map_err(bad)?})))
}

async fn lock(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    let session = caller(&s, &headers, Some(&id)).await?.ok_or_else(|| ApiError::bad_request("the lock belongs to a session"))?;
    plain::lock(&s.app, &id, &session).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

async fn unlock(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    let session = caller(&s, &headers, Some(&id)).await?.ok_or_else(|| ApiError::bad_request("the lock belongs to a session"))?;
    plain::unlock(&s.app, &id, &session).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// The owner's routes (history, Restore, untrash, the asset list): never on a runner's credential.
fn owner_only(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key(crate::runners::CREDENTIAL_HEADER) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "this is the owner's action: open Home's window"));
    }
    Ok(())
}

async fn history(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    owner_only(&headers)?;
    caller(&s, &headers, None).await?;
    let holder = plain::lock_holder(&s.app, &id).await?;
    Ok(Json(json!({"changes": plain::history(&s.app, &id).await.map_err(bad)?, "locked_by": holder})))
}

#[derive(Deserialize)]
struct Restore {
    change: i64,
}

async fn restore(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(b): Json<Restore>) -> ApiResult<Value> {
    owner_only(&headers)?;
    caller(&s, &headers, None).await?;
    Ok(Json(json!({"version": plain::restore(&s.app, &id, b.change).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct Keep {
    name: String,
}

/// Keep as folder, from the runner that has the temporary folder `id` (it moves its copy afterwards).
async fn keep(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(b): Json<Keep>) -> ApiResult<launchpad_core::model::Project> {
    caller(&s, &headers, None).await?;
    let credential = headers.get(crate::runners::CREDENTIAL_HEADER).and_then(|v| v.to_str().ok()).ok_or_else(|| ApiError::bad_request("Keep as folder runs on the runner that has the temporary folder"))?;
    let runner = launchpad_core::runners::authenticate(&s.app, credential).await.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"))?;
    // Only the runner that holds the folder, after the owner's Keep as folder that Home relayed to it.
    let held = launchpad_core::projects::get(&s.app, &id).await?.is_some_and(|p| p.is_temp && p.runner_id.as_deref() == Some(runner.id.as_str()));
    if !held || !s.app.runners.granted(&runner.id, &format!("keep:{id}")) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "Keep as folder comes from the owner's window, for a temporary folder of this runner"));
    }
    Ok(Json(launchpad_core::projects::keep_temp(&s.app, &id, &b.name).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct NewAsset {
    name: String,
}

async fn create_asset(State(s): State<Server>, Path(project): Path<String>, headers: HeaderMap, Json(b): Json<NewAsset>) -> ApiResult<plain::Asset> {
    owner_only(&headers)?;
    caller(&s, &headers, None).await?;
    Ok(Json(plain::create_asset(&s.app, &project, &b.name).await.map_err(bad)?))
}

async fn list_assets(State(s): State<Server>, Path(project): Path<String>, headers: HeaderMap) -> ApiResult<Vec<plain::AssetInfo>> {
    owner_only(&headers)?;
    caller(&s, &headers, None).await?;
    Ok(Json(plain::asset_infos(&s.app, &project).await.map_err(bad)?))
}

async fn untrash(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    owner_only(&headers)?;
    caller(&s, &headers, None).await?;
    Ok(Json(json!({"version": plain::untrash_asset(&s.app, &id).await.map_err(bad)?})))
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

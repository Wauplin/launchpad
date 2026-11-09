//! Knowledge base over HTTP.
//!
//! `/api/knowledge/…` are the owner's requests, answered by Home (a runner forwards them, see
//! `home_client::home_owned`). `/api/kb` is the runner-local endpoint of `launchpad kb` (session CLI token), and
//! `/api/kb/memories` lists this machine's harness memories for an import (Home asks the runner).

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::kb::{self, LocalOp};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/knowledge/unseen", get(unseen))
        .route("/api/knowledge/memories", get(memories))
        .route("/api/knowledge/{ws}", get(notes))
        .route("/api/knowledge/{ws}/file", get(file).put(write).delete(remove))
        .route("/api/knowledge/{ws}/changes", get(changes))
        .route("/api/knowledge/{ws}/changes/{id}/diff", get(diff))
        .route("/api/knowledge/{ws}/restore", post(restore))
        .route("/api/knowledge/{ws}/seen", post(seen))
        .route("/api/knowledge/{ws}/resolve", post(resolve))
        .route("/api/knowledge/{ws}/search", get(search))
        .route("/api/knowledge/{ws}/import", post(import))
        .route("/api/kb", post(session_write).layer(axum::extract::DefaultBodyLimit::max(crate::agent_auth::KB_BODY_MAX)))
        .route("/api/kb/memories", get(local_memories))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

fn lines(v: Vec<String>) -> Json<Value> {
    Json(json!({"lines": v}))
}

async fn unseen(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!(kb::unseen(&s.app).await.map_err(bad)?)))
}

async fn notes(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<kb::Notes> {
    Ok(Json(kb::notes(&s.app, &ws).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct FileQuery {
    path: Option<String>,
    sha256: Option<String>,
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "pdf" => "application/pdf",
        "md" | "txt" | "toml" | "json" | "yaml" | "yml" | "csv" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn file(State(s): State<Ctx>, Path(ws): Path<String>, Query(q): Query<FileQuery>) -> Result<Response, ApiError> {
    let data = kb::read(&s.app, &ws, q.path.as_deref(), q.sha256.as_deref()).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    let m = mime(q.path.as_deref().unwrap_or_default());
    // SVG and HTML never render as documents of this origin.
    Ok(([(header::CONTENT_TYPE, m), (header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], data).into_response())
}

#[derive(Deserialize)]
struct Write {
    path: String,
    content: String,
    base: Option<String>,
}

async fn write(State(s): State<Ctx>, Path(ws): Path<String>, Json(b): Json<Write>) -> ApiResult<Value> {
    Ok(lines(kb::owner_write(&s.app, &ws, b.path.trim(), b.content.as_bytes(), b.base).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

async fn remove(State(s): State<Ctx>, Path(ws): Path<String>, Query(q): Query<PathQuery>) -> ApiResult<Value> {
    Ok(lines(kb::apply(&s.app, &ws, kb::Author::owner(None), vec![kb::Op::Remove { path: q.path }]).await.map_err(bad)?))
}

async fn changes(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!({"changes": kb::changes(&s.app, &ws).await.map_err(bad)?})))
}

async fn diff(State(s): State<Ctx>, Path((ws, id)): Path<(String, i64)>) -> ApiResult<Value> {
    Ok(Json(json!({"patch": kb::diff(&s.app, &ws, id).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct ChangeBody {
    change: i64,
}

async fn restore(State(s): State<Ctx>, Path(ws): Path<String>, Json(b): Json<ChangeBody>) -> ApiResult<Value> {
    Ok(lines(kb::restore(&s.app, &ws, b.change).await.map_err(bad)?))
}

async fn seen(State(s): State<Ctx>, Path(ws): Path<String>, Json(b): Json<ChangeBody>) -> ApiResult<Value> {
    kb::mark_seen(&s.app, &ws, b.change).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Resolve {
    path: String,
    use_other: bool,
}

async fn resolve(State(s): State<Ctx>, Path(ws): Path<String>, Json(b): Json<Resolve>) -> ApiResult<Value> {
    Ok(lines(kb::resolve(&s.app, &ws, &b.path, b.use_other).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search(State(s): State<Ctx>, Path(ws): Path<String>, Query(q): Query<SearchQuery>) -> ApiResult<Value> {
    Ok(Json(json!({"hits": kb::search(&s.app, &ws, &q.q).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct ImportFile {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct Import {
    from: String,
    files: Vec<ImportFile>,
}

async fn import(State(s): State<Ctx>, Path(ws): Path<String>, Json(b): Json<Import>) -> ApiResult<Value> {
    Ok(lines(kb::import(&s.app, &ws, &b.from, b.files.into_iter().map(|f| (f.path, f.content)).collect()).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct MemoriesQuery {
    runner: Option<String>,
}

/// Home asks a runner for its harness memories; a runner's window reads its own.
async fn memories(State(s): State<Ctx>, Query(q): Query<MemoriesQuery>) -> ApiResult<Value> {
    match q.runner {
        Some(r) => {
            let (status, body) = s.app.runners.request(&r, "GET", "/api/kb/memories", None).await.map_err(bad)?;
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            if !(200..300).contains(&status) {
                return Err(ApiError::bad_request(v["error"].as_str().unwrap_or("the runner could not list its memories").to_string()));
            }
            Ok(Json(v))
        }
        None => local_memories(State(s)).await,
    }
}

async fn local_memories(State(s): State<Ctx>) -> ApiResult<Value> {
    if !s.app.paths.role.runs_agents() {
        return Err(ApiError::bad_request("Home has no harness memories; import them from a runner"));
    }
    Ok(Json(json!({"memories": kb::native_memories(&s.app).await})))
}

#[derive(Deserialize)]
struct SessionWrite {
    session_id: String,
    ops: Vec<LocalOp>,
}

async fn session_write(State(s): State<Ctx>, Json(b): Json<SessionWrite>) -> ApiResult<Value> {
    if !s.app.paths.role.runs_agents() {
        return Err(ApiError::bad_request("launchpad kb runs in a session on a runner"));
    }
    Ok(lines(kb::session_write(&s.app, &b.session_id, b.ops).await.map_err(bad)?))
}

//! Settings → Hosts: SSH hosts, their preflight checks, `ssh` questions (askpass) and folder browsing.
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::hosts::{HostStatus, loc, preflight, sshconfig};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/hosts", get(list).post(add))
        .route("/api/hosts/ssh-config", get(ssh_config))
        .route("/api/hosts/{id}", axum::routing::patch(update).delete(remove))
        .route("/api/hosts/{id}/retry", post(retry))
        .route("/api/hosts/{id}/checks", post(checks))
        .route("/api/hosts/{id}/askpass", post(askpass))
        .route("/api/hosts/{id}/prompt", post(answer))
        .route("/api/hosts/{id}/browse", get(browse))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> Json<Vec<HostStatus>> {
    Json(s.app.hosts.snapshot())
}

async fn ssh_config() -> Json<Vec<sshconfig::SshConfigHost>> {
    Json(sshconfig::hosts())
}

#[derive(Deserialize)]
struct Add {
    destination: String,
    id: Option<String>,
    port: Option<u16>,
    projects_root: Option<String>,
}

async fn add(State(s): State<Server>, Json(b): Json<Add>) -> ApiResult<Value> {
    let h = s.app.hosts.add(&b.destination, b.id.as_deref(), b.port, b.projects_root.as_deref()).await.map_err(bad)?;
    Ok(Json(json!(h)))
}

#[derive(Deserialize)]
struct Update {
    projects_root: Option<String>,
}

async fn update(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Update>) -> ApiResult<Value> {
    Ok(Json(json!(s.app.hosts.update(&id, b.projects_root.as_deref()).await.map_err(bad)?)))
}

async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.hosts.remove(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn retry(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.hosts.retry(&id).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn checks(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Vec<preflight::Check>> {
    Ok(Json(preflight::run(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Ask {
    prompt: String,
}

/// From `launchpad` as `SSH_ASKPASS`: waits until the owner answers in the UI (or cancels).
async fn askpass(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Ask>) -> ApiResult<Value> {
    match s.app.hosts.ask(&id, &b.prompt).await {
        Some(answer) => Ok(Json(json!({"answer": answer}))),
        None => Err(ApiError::new(axum::http::StatusCode::GONE, "cancelled")),
    }
}

#[derive(Deserialize)]
struct Answer {
    prompt_id: String,
    answer: Option<String>,
}

async fn answer(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Answer>) -> ApiResult<Value> {
    s.app.hosts.answer(&id, &b.prompt_id, b.answer).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Browse {
    path: Option<String>,
}

/// Folders of a host, to add a project. Starts at the host's projects root (or its home).
async fn browse(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<Browse>) -> ApiResult<Value> {
    let home = s.app.hosts.home(&id).ok_or_else(|| ApiError::bad_request(format!("{id} is not connected")))?;
    let native = match q.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some("~") => home.clone(),
        Some(p) if p.starts_with("~/") => format!("{}/{}", home.trim_end_matches('/'), &p[2..]),
        Some(p) if p.starts_with('/') => p.to_string(),
        Some(_) => return Err(ApiError::bad_request("use an absolute folder or ~/…")),
        None => {
            let root = s.app.hosts.projects_root(&id).map_err(bad)?;
            let root = loc::native(&root).to_string();
            if launchpad_core::hostfs::is_dir(loc::join(Some(&id), &root)).await { root } else { home.clone() }
        }
    };
    let location = loc::join(Some(&id), &native);
    let path = launchpad_core::hostfs::canonicalize(&location).await.map_err(|e| ApiError::bad_request(format!("{location}: {e}")))?;
    let mut entries: Vec<Value> = launchpad_core::hostfs::read_dir(&path)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .into_iter()
        .filter(|e| e.kind == "dir" && !e.name.starts_with('.'))
        .map(|e| json!({"name": e.name}))
        .collect();
    entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_lowercase());
    let native = loc::native(&path.to_string_lossy()).to_string();
    let parent = std::path::Path::new(&native).parent().map(|p| p.to_string_lossy().into_owned());
    let git = launchpad_core::hostfs::exists(path.join(".git")).await;
    Ok(Json(json!({"path": native, "location": path.to_string_lossy(), "parent": parent, "home": home, "git": git, "entries": entries})))
}

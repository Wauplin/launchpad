//! Ask Launchpad routes: the assistant thread, inert proposals and the read-only context the assistant's
//! CLI reads (`/api/context/*`, workspace-scoped JSON without secrets).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::model::Session;
use launchpad_core::proposals::{self, Create, Proposal};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/proposals", get(list).post(create))
        .route("/api/proposals/{id}/accept", post(accept))
        .route("/api/proposals/{id}/dismiss", post(dismiss))
        .route("/api/proposals/{id}/recheck", post(recheck))
        .route("/api/assistant", get(thread).post(ask))
        .route("/api/context/folders", get(folders))
        .route("/api/context/sessions", get(sessions))
        .route("/api/context/sessions/{id}", get(session))
        .route("/api/context/profiles", get(profiles))
        .route("/api/context/presets", get(presets))
        .route("/api/context/inbox", get(inbox))
        .route("/api/context/schedules", get(schedules))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> ApiResult<Vec<Proposal>> {
    Ok(Json(proposals::list(&s.app).await?))
}
async fn create(State(s): State<Server>, Json(c): Json<Create>) -> ApiResult<Proposal> {
    Ok(Json(proposals::create(&s.app, c).await.map_err(bad)?))
}
async fn accept(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::accept(&s.app, &id).await.map_err(bad)?))
}
async fn dismiss(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::dismiss(&s.app, &id).await.map_err(bad)?))
}
async fn recheck(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::recheck(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Scope {
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct Ask {
    workspace_id: Option<String>,
    text: String,
}

async fn thread(State(s): State<Server>, Query(q): Query<Scope>) -> Json<Option<Session>> {
    Json(proposals::thread(&s.app, q.workspace.as_deref()))
}
async fn ask(State(s): State<Server>, Json(b): Json<Ask>) -> ApiResult<Session> {
    Ok(Json(proposals::ask(&s.app, b.workspace_id, b.text).await.map_err(bad)?))
}

/// The workspace (by ID or name) and its folder IDs; None means every folder.
async fn scope(s: &Server, workspace: Option<&str>) -> Result<Option<Vec<String>>, ApiError> {
    let Some(w) = workspace.filter(|w| !w.is_empty() && *w != "all") else { return Ok(None) };
    let all = launchpad_core::workspace::list(&s.app).await?;
    let ws = all.iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(w)).ok_or_else(|| ApiError::not_found(format!("no workspace {w}")))?;
    Ok(Some(ws.project_ids.clone()))
}

async fn folders(State(s): State<Server>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let list = launchpad_core::projects::list(&s.app).await?;
    Ok(Json(json!(list.into_iter().filter(|p| ids.as_ref().is_none_or(|ids| ids.contains(&p.id))).map(|p| json!({
        "id": p.id, "name": p.name, "path": p.path, "description": p.description, "github": p.gh_repo, "scratch": p.is_temp,
    })).collect::<Vec<_>>())))
}

fn session_json(s: &Session) -> Value {
    json!({
        "id": s.id, "title": s.title, "status": s.status, "needs_you": s.needs_you(), "folder_id": s.project_id, "cwd": s.cwd,
        "harness": s.harness, "profile": s.launch_spec.profile, "started_at": s.started_at, "status_since": s.status_since,
        "last_message": s.last_message, "open_question": s.open_question,
    })
}

async fn sessions(State(s): State<Server>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let list: Vec<Value> = s.app.sessions.list().iter()
        .filter(|x| !x.launch_spec.target.is_assistant())
        .filter(|x| ids.as_ref().is_none_or(|ids| x.project_id.as_ref().is_some_and(|p| ids.contains(p))))
        .map(session_json).collect();
    Ok(Json(json!(list)))
}

async fn session(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    let x = s.app.sessions.get(&id).ok_or_else(|| ApiError::not_found("no such session"))?;
    let mut v = session_json(&x);
    v["prompt"] = json!(x.launch_spec.text.chars().take(4000).collect::<String>());
    v["usage"] = json!(x.usage);
    Ok(Json(v))
}

/// Profiles without their environment (keys stay private), with the quota of each subscription login.
async fn profiles(State(s): State<Server>) -> ApiResult<Value> {
    let config = s.app.config.get();
    let status = s.app.status.get();
    Ok(Json(json!({
        "profiles": config.profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "harness": p.harness, "model": p.model, "effort": p.effort, "billing": p.billing})).collect::<Vec<_>>(),
        "quotas": status.quotas,
    })))
}

async fn presets(State(s): State<Server>) -> ApiResult<Value> {
    Ok(Json(json!(s.app.config.get().presets)))
}

async fn inbox(State(s): State<Server>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let repos: Option<Vec<String>> = match ids {
        Some(ids) => Some(launchpad_core::projects::list(&s.app).await?.into_iter().filter(|p| ids.contains(&p.id)).flat_map(|p| [p.gh_repo, p.gh_upstream]).flatten().map(|r| r.to_lowercase()).collect()),
        None => None,
    };
    let snap = s.app.inbox.snapshot(&s.app).await?;
    let threads: Vec<Value> = snap.threads.into_iter()
        .filter(|t| snap.active_ids.contains(&t.id))
        .filter(|t| repos.as_ref().is_none_or(|r| r.contains(&t.repo.to_lowercase())))
        .take(100)
        .map(|t| json!({"repo": t.repo, "number": t.number, "kind": t.kind, "title": t.title, "reason": t.reason, "unread": t.unread, "url": t.url, "updated_at": t.updated_at, "state": t.state}))
        .collect();
    Ok(Json(json!(threads)))
}

async fn schedules(State(s): State<Server>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let inside = |spec: &launchpad_core::model::LaunchSpec| match (&ids, &spec.target) {
        (None, _) => true,
        (Some(ids), launchpad_core::model::Target::Project { project_id } | launchpad_core::model::Target::Predraft { project_id }) => ids.contains(project_id),
        _ => false,
    };
    let launches: Vec<Value> = launchpad_core::schedules::list(&s.app).await?.into_iter().filter(|l| inside(&l.spec)).map(|l| json!({"id": l.id, "title": l.title, "conditions": l.conditions, "status": l.status, "resume_session_id": l.resume_session_id})).collect();
    let crons: Vec<Value> = launchpad_core::crons::list(&s.app).await?.into_iter().filter(|c| inside(&c.spec)).map(|c| json!({"id": c.id, "title": c.title, "schedule": c.schedule, "next_run_at": c.next_run_at, "paused": c.paused})).collect();
    Ok(Json(json!({"scheduled": launches, "crons": crons})))
}

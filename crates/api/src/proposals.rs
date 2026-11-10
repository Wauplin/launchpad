//! Proposals and the read-only context the Launchpad CLI tool reads (`/api/context/*`, workspace-scoped JSON
//! without secrets).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::model::Session;
use launchpad_automation::proposals::{self, Create, Proposal};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/proposals", get(list).post(create))
        .route("/api/proposals/{id}/accept", post(accept))
        .route("/api/proposals/{id}/dismiss", post(dismiss))
        .route("/api/proposals/{id}/edit", post(edited))
        .route("/api/proposals/{id}/recheck", post(recheck))
        .route("/api/proposals/{id}/ran", post(ran))
        .route("/api/proposals/{id}/reply", post(reply))
        .route("/api/context/folders", get(folders))
        .route("/api/context/bots", get(bots))
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

async fn list(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Vec<Proposal>> {
    let mut proposals = proposals::list(&s.app).await?;
    if let Some(w) = q.workspace.filter(|w| !w.is_empty() && w != "all") {
        let workspaces = launchpad_core::workspace::list(&s.app).await?;
        let ws = workspaces.iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(&w)).ok_or_else(|| ApiError::not_found(format!("no workspace {w}")))?;
        proposals.retain(|p| p.workspace_id.as_deref() == Some(&ws.id));
    }
    Ok(Json(proposals))
}
async fn create(State(s): State<Ctx>, Json(c): Json<Create>) -> ApiResult<Proposal> {
    Ok(Json(proposals::create(&s.app, c).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct AcceptQuery {
    /// The browser's IANA time zone, for routines and crons.
    time_zone: Option<String>,
}
async fn accept(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<AcceptQuery>) -> ApiResult<Proposal> {
    Ok(Json(proposals::accept_in(&s.app, &id, q.time_zone).await.map_err(bad)?))
}
async fn dismiss(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::dismiss(&s.app, &id).await.map_err(bad)?))
}
async fn edited(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::edited(&s.app, &id).await.map_err(bad)?))
}
/// A ready card's action ran from the owner's click.
async fn ran(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::ran(&s.app, &id).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct Reply {
    text: String,
}
/// The owner's reply to a report, sent to its author's thread.
async fn reply(State(s): State<Ctx>, Path(id): Path<String>, Json(r): Json<Reply>) -> ApiResult<Proposal> {
    Ok(Json(proposals::reply(&s.app, &id, &r.text).await.map_err(bad)?))
}
async fn recheck(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Proposal> {
    Ok(Json(proposals::recheck(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Scope {
    workspace: Option<String>,
}


/// The workspace (by ID or name) and its folder IDs; None means every folder.
async fn scope(s: &Ctx, workspace: Option<&str>) -> Result<Option<Vec<String>>, ApiError> {
    let Some(w) = workspace.filter(|w| !w.is_empty() && *w != "all") else { return Ok(None) };
    let all = launchpad_core::workspace::list(&s.app).await?;
    let ws = all.iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(w)).ok_or_else(|| ApiError::not_found(format!("no workspace {w}")))?;
    Ok(Some(launchpad_core::workspace::project_ids(&s.app, &ws.id).await?))
}

async fn folders(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let list = launchpad_core::projects::list(&s.app).await?;
    // Temporary folders belong to their agents: they are not folders to work on.
    Ok(Json(json!(list.into_iter().filter(|p| !p.is_temp && ids.as_ref().is_none_or(|ids| ids.contains(&p.id))).map(|p| json!({
        "id": p.id, "name": p.name, "path": p.path, "description": p.description, "github": p.gh_repo,
        "kind": if p.home_git { "git" } else if p.plain { "plain" } else { "local" },
    })).collect::<Vec<_>>())))
}

/// Bots without their instructions' history: name, job, profile, folder and the current session.
async fn bots(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ws = match q.workspace.as_deref().filter(|w| !w.is_empty() && *w != "all") {
        Some(w) => Some(launchpad_core::workspace::list(&s.app).await?.into_iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(w)).ok_or_else(|| ApiError::not_found(format!("no workspace {w}")))?.id),
        None => None,
    };
    let list = launchpad_core::bots::list(&s.app).await.map_err(bad)?;
    let folders = launchpad_core::projects::list(&s.app).await?;
    Ok(Json(json!(list.into_iter().filter(|b| ws.as_ref().is_none_or(|w| &b.workspace_id == w)).map(|b| {
        let thread = launchpad_core::bots::thread(&s.app, &b.id);
        let current = thread.iter().rev().find(|x| x.status != launchpad_core::model::SessionStatus::Closed);
        json!({
            "id": b.id, "name": b.settings.name, "workspace_id": b.workspace_id, "instructions": b.instructions.chars().take(2000).collect::<String>(),
            "profile": b.settings.profile, "folder_id": b.settings.folder, "folder": b.settings.folder.as_ref().map(|f| folders.iter().find(|p| &p.id == f).map_or("a removed folder", |p| p.name.as_str()).to_string()).unwrap_or_else(|| "a temporary folder per session".into()),
            "current_session": current.map(session_json), "sessions": thread.len(),
        })
    }).collect::<Vec<_>>())))
}

fn session_json(s: &Session) -> Value {
    json!({
        "id": s.id, "title": s.title, "status": s.status, "needs_you": s.needs_you(), "folder_id": s.project_id, "cwd": s.cwd,
        "harness": s.harness, "profile": s.launch_spec.profile, "started_at": s.started_at, "status_since": s.status_since,
        "last_message": s.last_message, "open_question": s.open_question,
        "runner_id": s.runner_id, "generation": s.generation, "ended_at": s.ended_at, "live": s.live, "bot": s.launch_spec.bot.as_ref().map(|b| &b.id),
    })
}

async fn sessions(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let list: Vec<Value> = s.app.sessions.list().iter()
        .filter(|x| ids.as_ref().is_none_or(|ids| x.project_id.as_ref().is_some_and(|p| ids.contains(p))))
        .map(session_json).collect();
    Ok(Json(json!(list)))
}

async fn session(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    let x = s.app.sessions.get(&id).ok_or_else(|| ApiError::not_found("no such session"))?;
    let mut v = session_json(&x);
    v["prompt"] = json!(x.launch_spec.text.chars().take(4000).collect::<String>());
    v["usage"] = json!(x.usage);
    Ok(Json(crate::diagnostics::session_detail(&s, &x, v).await))
}

/// Profiles without their environment (keys stay private), with the quota of each subscription login.
async fn profiles(State(s): State<Ctx>) -> ApiResult<Value> {
    let config = s.app.config.get();
    let status = s.app.status.get();
    Ok(Json(json!({
        "profiles": config.profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "harness": p.harness, "model": p.model, "effort": p.effort, "billing": p.billing})).collect::<Vec<_>>(),
        "quotas": status.quotas,
    })))
}

async fn presets(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!(s.app.config.get().presets)))
}

async fn inbox(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let repos: Option<Vec<String>> = match ids {
        Some(ids) => Some(launchpad_core::projects::list(&s.app).await?.into_iter().filter(|p| ids.contains(&p.id)).flat_map(|p| [p.gh_repo, p.gh_upstream]).flatten().map(|r| r.to_lowercase()).collect()),
        None => None,
    };
    let snap = launchpad_github::inbox::of(&s.app).snapshot(&s.app).await?;
    let threads: Vec<Value> = snap.threads.into_iter()
        .filter(|t| snap.active_ids.contains(&t.id))
        .filter(|t| repos.as_ref().is_none_or(|r| r.contains(&t.repo.to_lowercase())))
        .take(300)
        .map(|t| json!({"id": t.id, "repo": t.repo, "number": t.number, "kind": t.kind, "title": t.title, "reason": t.reason, "unread": t.unread, "url": t.url, "updated_at": t.updated_at, "state": t.state}))
        .collect();
    Ok(Json(json!(threads)))
}

async fn schedules(State(s): State<Ctx>, Query(q): Query<Scope>) -> ApiResult<Value> {
    let ids = scope(&s, q.workspace.as_deref()).await?;
    let inside = |spec: &launchpad_core::model::LaunchSpec| match (&ids, &spec.target) {
        (None, _) => true,
        (Some(ids), launchpad_core::model::Target::Project { project_id } | launchpad_core::model::Target::Predraft { project_id }) => ids.contains(project_id),
        _ => false,
    };
    let launches: Vec<Value> = launchpad_automation::schedules::list(&s.app).await?.into_iter().filter(|l| inside(&l.spec)).map(|l| json!({"id": l.id, "title": l.title, "conditions": l.conditions, "status": l.status, "resume_session_id": l.resume_session_id})).collect();
    let crons: Vec<Value> = launchpad_automation::crons::list(&s.app).await?.into_iter().filter(|c| inside(&c.spec)).map(|c| json!({"id": c.id, "title": c.title, "schedule": c.schedule, "next_run_at": c.next_run_at, "paused": c.paused})).collect();
    Ok(Json(json!({"scheduled": launches, "crons": crons})))
}

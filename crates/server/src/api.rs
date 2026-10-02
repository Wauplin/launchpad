//! Phase 1 routes: projects, launch, sessions, GitHub helpers, settings helpers.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use launchpad_core::model::*;
use launchpad_core::projects::{self, Discovered, ProjectPatch};
use launchpad_core::sessions::Preview;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/projects", get(list_projects).post(add_project))
        .route("/api/projects/discover", post(discover))
        .route("/api/projects/{id}", get(get_project).patch(patch_project).delete(delete_project))
        .route("/api/projects/{id}/branches", get(branches))
        .route("/api/projects/{id}/promote", post(promote))
        .route("/api/launch/preview", post(preview))
        .route("/api/launch/draft", get(get_draft).put(put_draft))
        .route("/api/harness/{kind}/defaults", get(defaults))
        .route("/api/sessions", get(list_sessions).post(launch))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/input", post(input))
        .route("/api/sessions/{id}/interrupt", post(interrupt))
        .route("/api/sessions/{id}/answer", post(answer))
        .route("/api/sessions/{id}/close", post(close))
        .route("/api/sessions/{id}/resume", post(resume))
        .route("/api/sessions/{id}/open-terminal", post(open_terminal))
        .route("/api/sessions/{id}/task-output", get(task_output))
        .route("/api/gh/resolve", post(gh_resolve))
        .route("/api/gh/{owner}/{repo}/suggest", get(gh_suggest))
        .route("/api/settings/terminals", get(terminals))
        .route("/api/open", post(open_url))
        .route("/api/open-editor", post(open_editor))
}

// ---------------------------------------------------------------- projects

async fn list_projects(State(s): State<Server>) -> ApiResult<Vec<Project>> {
    Ok(Json(projects::list(&s.app).await?))
}

async fn get_project(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Project> {
    Ok(Json(projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?))
}

#[derive(Deserialize)]
struct AddProject {
    path: String,
}

async fn add_project(State(s): State<Server>, Json(b): Json<AddProject>) -> ApiResult<Project> {
    let path = launchpad_core::paths::expand_home(b.path.trim());
    Ok(Json(projects::register(&s.app, &path, false).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

async fn discover(State(s): State<Server>) -> ApiResult<Vec<Discovered>> {
    Ok(Json(projects::discover(&s.app).await?))
}

async fn patch_project(State(s): State<Server>, Path(id): Path<String>, Json(patch): Json<ProjectPatch>) -> ApiResult<Project> {
    Ok(Json(projects::update(&s.app, &id, patch).await?))
}

/// Unregisters a project. For a temporary project: stops its sessions and deletes the folder.
async fn delete_project(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if p.is_temp {
        s.app.sessions.close_in(&s.app, &p.path).await?;
        projects::delete_temp(&s.app, &id).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    } else {
        projects::unregister(&s.app, &id).await?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn branches(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    let (current, list) = launchpad_core::git::branches(&s.app.env, std::path::Path::new(&p.path)).await.unwrap_or((None, vec![]));
    Ok(Json(json!({"current": current, "branches": list})))
}

#[derive(Deserialize)]
struct Promote {
    name: String,
}

async fn promote(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Promote>) -> ApiResult<Project> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if s.app.sessions.running_in(&p.path) {
        return Err(ApiError::conflict("a session is running in this folder: close it first"));
    }
    Ok(Json(projects::promote(&s.app, &id, &b.name).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

// ---------------------------------------------------------------- launch

async fn preview(State(s): State<Server>, Json(spec): Json<LaunchSpec>) -> ApiResult<Preview> {
    Ok(Json(s.app.sessions.preview(&s.app, &spec).await?))
}

async fn get_draft(State(s): State<Server>) -> ApiResult<Value> {
    Ok(Json(launchpad_core::drafts::get(&s.app).await?))
}

async fn put_draft(State(s): State<Server>, Json(draft): Json<Value>) -> ApiResult<Value> {
    launchpad_core::drafts::put(&s.app, &draft).await?;
    Ok(Json(json!({"ok": true})))
}

async fn defaults(State(s): State<Server>, Path(kind): Path<String>) -> ApiResult<HarnessDefaults> {
    let h = HarnessKind::parse(&kind).ok_or_else(|| ApiError::not_found("unknown harness"))?;
    Ok(Json(s.app.models.defaults(h)))
}

// ---------------------------------------------------------------- sessions

async fn list_sessions(State(s): State<Server>) -> Json<Vec<Session>> {
    Json(s.app.sessions.list())
}

async fn get_session(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.get(&id).ok_or_else(|| ApiError::not_found("no such session"))?))
}

async fn launch(State(s): State<Server>, Json(spec): Json<LaunchSpec>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.launch(&s.app, spec).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

#[derive(Deserialize)]
struct After {
    after: Option<i64>,
}

async fn events(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<After>) -> ApiResult<Vec<SessionEvent>> {
    Ok(Json(s.app.sessions.events(&s.app, &id, q.after.unwrap_or(0)).await?))
}

#[derive(Deserialize)]
struct Input {
    text: String,
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn input(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Input>) -> ApiResult<Value> {
    if b.text.trim().is_empty() {
        return Err(ApiError::bad_request("the message is empty"));
    }
    s.app.sessions.send(&s.app, &id, &b.text).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn interrupt(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.interrupt(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Answer {
    request_id: String,
    answers: Vec<String>,
}

async fn answer(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Answer>) -> ApiResult<Value> {
    s.app.sessions.answer(&s.app, &id, &b.request_id, b.answers).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn close(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.close(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn resume(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.resume(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn open_terminal(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.open_terminal(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct TaskQuery {
    task_id: String,
}

/// The end of a background command's output file. Only files named in the session's own events.
async fn task_output(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<TaskQuery>) -> ApiResult<Value> {
    let events = s.app.sessions.events(&s.app, &id, 0).await?;
    let file = events.iter().rev().find_map(|e| match &e.body {
        EventBody::BackgroundTask { task_id, output_file: Some(f), .. } if *task_id == q.task_id => Some(f.clone()),
        _ => None,
    });
    let Some(file) = file else { return Ok(Json(json!({"text": null}))) };
    let text = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&file)?;
        let len = f.metadata()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(4096)))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).to_string())
    })
    .await?
    .ok();
    Ok(Json(json!({"text": text})))
}

// ---------------------------------------------------------------- GitHub

#[derive(Deserialize)]
struct Resolve {
    url: String,
}

async fn gh_resolve(State(s): State<Server>, Json(b): Json<Resolve>) -> ApiResult<GitHubRef> {
    Ok(Json(s.app.github.resolve_link(&b.url).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Suggest {
    q: Option<String>,
}

async fn gh_suggest(
    State(s): State<Server>,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<Suggest>,
) -> ApiResult<Vec<launchpad_core::github::GhItem>> {
    Ok(Json(s.app.github.suggest(&format!("{owner}/{repo}"), q.q.as_deref().unwrap_or("")).await.map_err(bad)?))
}

// ---------------------------------------------------------------- settings and desktop helpers

async fn terminals(State(s): State<Server>) -> Json<Vec<launchpad_core::terminal::TerminalOption>> {
    Json(launchpad_core::terminal::options(&s.app))
}

#[derive(Deserialize)]
struct OpenUrl {
    url: String,
}

/// Opens a link in the browser (`commands.browser`, `xdg-open` by default).
async fn open_url(State(s): State<Server>, Json(b): Json<OpenUrl>) -> ApiResult<Value> {
    if !(b.url.starts_with("https://") || b.url.starts_with("http://")) {
        return Err(ApiError::bad_request("only http and https links"));
    }
    let browser = s.app.config.get().commands.browser;
    launchpad_core::procs::command(&s.app.env, &browser, None).arg(&b.url).kill_on_drop(false).spawn().map_err(|e| bad(e.into()))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct OpenEditor {
    path: String,
}

async fn open_editor(State(s): State<Server>, Json(b): Json<OpenEditor>) -> ApiResult<Value> {
    let editor = s.app.config.get().commands.editor;
    launchpad_core::procs::command(&s.app.env, &editor, None).arg(&b.path).kill_on_drop(false).spawn().map_err(|e| bad(e.into()))?;
    Ok(Json(json!({"ok": true})))
}

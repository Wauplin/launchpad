//! Routes for projects, launch, sessions, GitHub inbox and settings helpers.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use launchpad_core::model::*;
use launchpad_core::projects::{self, Discovered, ProjectPatch};
use launchpad_core::sessions::Preview;
use launchpad_core::workspace::{self, Workspace};
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
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/{id}", axum::routing::patch(patch_workspace).delete(delete_workspace))
        .route("/api/window", post(open_window))
        .route("/api/uploads", post(upload).layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024)))
        .route("/api/uploads/{hash}", get(uploaded_image))
        .route("/api/launch/preview", post(preview).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/launch/schedules", get(schedules).post(schedule).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}/follow-up", post(follow_up))
        .route("/api/launch/schedules/{id}", axum::routing::delete(cancel_schedule))
        .route("/api/launch/schedules/{id}/start", post(start_schedule))
        .route("/api/launch/crons", get(crons).post(create_cron).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/launch/crons/next", get(cron_next))
        .route("/api/launch/crons/{id}", axum::routing::patch(update_cron).delete(delete_cron))
        .route("/api/launch/crons/{id}/run", post(run_cron))
        .route("/api/launch/draft", get(get_draft).put(put_draft).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/harnesses", get(harnesses))
        .route("/api/harness/{kind}/defaults", get(defaults).post(refresh_defaults))
        .route("/api/profiles/models", post(profile_models))
        .route("/api/sessions", get(list_sessions).post(launch).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/rename", post(rename_session))
        .route("/api/sessions/{id}/seen", post(mark_seen))
        .route("/api/sessions/{id}/unread", post(mark_unread))
        .route("/api/sessions/{id}/git", get(session_git))
        .route("/api/sessions/{id}/git-diff", get(session_git_diff))
        .route("/api/sessions/search", get(search_sessions))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/events/page", get(events_page))
        .route("/api/sessions/{id}/images/{seq}/{index}", get(session_image))
        .route("/api/sessions/{id}/trace", get(trace))
        .route("/api/sessions/{id}/input", post(input).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}/interrupt", post(interrupt))
        .route("/api/sessions/{id}/processes", get(session_processes))
        .route("/api/sessions/{id}/processes/stop", post(stop_session_process))
        .route("/api/sessions/{id}/answer", post(answer))
        .route("/api/sessions/{id}/close", post(close))
        .route("/api/sessions/{id}/resume", post(resume))
        .route("/api/sessions/{id}/resume-paid", post(resume_paid))
        .route("/api/sessions/{id}/resume-subscription", post(resume_subscription))
        .route("/api/sessions/{id}/paid-usage", post(set_paid_usage))
        .route("/api/sessions/{id}/model", post(set_model))
        .route("/api/sessions/{id}/permissions", post(set_permissions))
        .route("/api/sessions/{id}/budget", post(set_budget))
        .route("/api/sessions/{id}/terminal", post(open_terminal))
        .route("/api/sessions/{id}/task-output", get(task_output))
        .route("/api/inbox", get(inbox))
        .route("/api/inbox/sync", post(inbox_sync))
        .route("/api/inbox/restore", post(inbox_restore))
        .route("/api/inbox/{id}/preview", get(inbox_preview).post(inbox_refresh))
        .route("/api/inbox/{id}/action", post(inbox_action))
        .route("/api/inbox/{id}/reply", post(inbox_reply))
        .route("/api/gh/resolve", post(gh_resolve))
        .route("/api/gh/{owner}/{repo}/suggest", get(gh_suggest))
        .route("/api/open", post(open_url))
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
    /// The SSH host of `path` (a native path on that host, `~` for its home).
    host: Option<String>,
    /// The workspace of the window that adds it.
    workspace_id: Option<String>,
}

async fn add_project(State(s): State<Server>, Json(b): Json<AddProject>) -> ApiResult<Project> {
    let path = match b.host.as_deref() {
        Some(h) => {
            let home = s.app.hosts.home(h).ok_or_else(|| ApiError::bad_request(format!("{h} is not connected")))?;
            let p = b.path.trim();
            let native = if p == "~" { home } else if let Some(rest) = p.strip_prefix("~/") { format!("{}/{rest}", home.trim_end_matches('/')) } else { p.to_string() };
            std::path::PathBuf::from(launchpad_core::hosts::loc::join(Some(h), &native))
        }
        None => launchpad_core::paths::expand_home(b.path.trim()),
    };
    let p = projects::register(&s.app, &path, false).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    if let Some(ws) = &b.workspace_id {
        workspace::add_project(&s.app, ws, &p.id).await?;
    }
    Ok(Json(p))
}

// ---------------------------------------------------------------- workspaces

async fn list_workspaces(State(s): State<Server>) -> ApiResult<Vec<Workspace>> {
    Ok(Json(workspace::list(&s.app).await?))
}

#[derive(Deserialize)]
struct NewWorkspace {
    name: String,
    #[serde(default)]
    project_ids: Vec<String>,
}

async fn create_workspace(State(s): State<Server>, Json(b): Json<NewWorkspace>) -> ApiResult<Workspace> {
    Ok(Json(workspace::create(&s.app, &b.name, b.project_ids).await.map_err(bad)?))
}

async fn patch_workspace(State(s): State<Server>, Path(id): Path<String>, Json(patch): Json<workspace::Patch>) -> ApiResult<Workspace> {
    Ok(Json(workspace::update(&s.app, &id, patch).await.map_err(bad)?))
}

async fn delete_workspace(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    workspace::delete(&s.app, &id).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct OpenWindow {
    workspace_id: Option<String>,
}

/// Opens another desktop window. The UI falls back to a browser tab when this fails.
/// Stores one attachment (raw body, its MIME type in `Content-Type`) and returns its reference.
async fn upload(headers: axum::http::HeaderMap, body: axum::body::Bytes) -> ApiResult<Value> {
    let mime = headers.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream");
    let reference = launchpad_core::attachments::store(&body, mime).await.map_err(bad)?;
    Ok(Json(json!({"data_url": reference})))
}

#[derive(Deserialize)]
struct UploadType {
    #[serde(rename = "type")]
    mime: String,
}

/// An uploaded image for previews. Other file types are never served back to the browser.
async fn uploaded_image(Path(hash): Path<String>, Query(q): Query<UploadType>) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let bytes = launchpad_core::attachments::read_image(&hash, &q.mime).map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(([(axum::http::header::CONTENT_TYPE, q.mime), (axum::http::header::CACHE_CONTROL, "private, max-age=31536000, immutable".into()), (axum::http::header::HeaderName::from_static("x-content-type-options"), "nosniff".into())], bytes).into_response())
}

async fn open_window(State(s): State<Server>, Json(b): Json<OpenWindow>) -> ApiResult<Value> {
    workspace::open_window(&s.app, b.workspace_id.as_deref()).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
struct Discover {
    /// Scan the projects root of this SSH host instead of this computer's.
    host: Option<String>,
}

async fn discover(State(s): State<Server>, body: Option<Json<Discover>>) -> ApiResult<Vec<Discovered>> {
    let host = body.map(|b| b.0).unwrap_or_default().host;
    Ok(Json(projects::discover(&s.app, host.as_deref()).await.map_err(bad)?))
}

async fn patch_project(State(s): State<Server>, Path(id): Path<String>, Json(patch): Json<ProjectPatch>) -> ApiResult<Project> {
    if s.auth.remote() && patch.worktree_setup.is_some() {
        return Err(ApiError::bad_request("Edit worktree setup commands on the host while remote access is enabled"));
    }
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

#[derive(Deserialize)]
struct DraftQuery {
    workspace: Option<String>,
}

async fn get_draft(State(s): State<Server>, Query(q): Query<DraftQuery>) -> ApiResult<Value> {
    Ok(Json(launchpad_core::drafts::get(&s.app, q.workspace.as_deref()).await?))
}

/// Body: the draft plus `base_revision` (the revision it was edited from), `force` (overwrite a newer
/// draft) and `client` (the window id echoed in the broadcast). A stale save gets 409 with the stored `revision`.
async fn put_draft(State(s): State<Server>, Query(q): Query<DraftQuery>, Json(draft): Json<Value>) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    use launchpad_core::drafts::{self, Put};
    let base = draft["base_revision"].as_u64().unwrap_or(0);
    let force = draft["force"].as_bool().unwrap_or(false);
    let client = draft["client"].as_str().map(str::to_string);
    Ok(match drafts::put(&s.app, q.workspace.as_deref(), draft, base, force, client).await? {
        Put::Saved(revision) => Json(json!({"ok": true, "revision": revision})).into_response(),
        Put::Conflict(revision) => (
            axum::http::StatusCode::CONFLICT,
            Json(json!({"error": "Another window or device saved a newer draft.", "revision": revision})),
        )
            .into_response(),
    })
}

async fn defaults(State(s): State<Server>, Path(kind): Path<String>) -> ApiResult<HarnessDefaults> {
    let h = HarnessKind::parse(&kind).ok_or_else(|| ApiError::not_found("unknown harness"))?;
    Ok(Json(s.app.models.defaults(&s.app, h)))
}

async fn harnesses(
    State(s): State<Server>,
) -> Json<Vec<launchpad_core::harness::HarnessDescriptor>> {
    Json(s.app.harnesses.descriptors())
}

async fn refresh_defaults(State(s): State<Server>, Path(kind): Path<String>) -> ApiResult<HarnessDefaults> {
    let h = HarnessKind::parse(&kind).ok_or_else(|| ApiError::not_found("unknown harness"))?;
    Ok(Json(s.app.models.refresh(&s.app, h).await?))
}

#[derive(Deserialize)]
struct ModelsUrl {
    url: String,
}

/// Models of a profile's OpenAI-compatible list; works for unsaved profiles too.
async fn profile_models(Json(b): Json<ModelsUrl>) -> ApiResult<Vec<launchpad_core::model::ModelInfo>> {
    Ok(Json(launchpad_core::profiles::models(&b.url).await?))
}

// ---------------------------------------------------------------- sessions

async fn list_sessions(State(s): State<Server>) -> Json<Vec<Session>> {
    Json(s.app.sessions.list().iter().map(Session::light).collect())
}

async fn get_session(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.get(&id).ok_or_else(|| ApiError::not_found("no such session"))?))
}

async fn launch(State(s): State<Server>, headers: axum::http::HeaderMap, Json(spec): Json<LaunchSpec>) -> ApiResult<Session> {
    let id = headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()).map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    Ok(Json(s.app.sessions.launch_with_id(&s.app, spec, id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct After {
    after: Option<i64>,
}

/// Catch-up after a reconnect or a sequence gap: every stored event after `after`.
async fn events(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<After>) -> ApiResult<Vec<SessionEvent>> {
    let mut events = s.app.sessions.events(&s.app, &id, q.after.unwrap_or(0)).await?;
    events.iter_mut().for_each(launchpad_core::session_pages::link_images);
    Ok(Json(events))
}

#[derive(Deserialize)]
struct PageQuery {
    /// The last `tail` events (default 200).
    tail: Option<i64>,
    /// Or the `limit` events before `before` (exclusive).
    before: Option<i64>,
    limit: Option<i64>,
}

/// Tail-first trace loading: `?tail=200`, then `?before=<first loaded seq>&limit=200`.
async fn events_page(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<PageQuery>) -> ApiResult<launchpad_core::session_pages::SessionEventPage> {
    let limit = q.limit.or(q.tail).unwrap_or(200);
    Ok(Json(launchpad_core::session_pages::page(&s.app, &id, q.before, limit).await?))
}

/// An image pasted into a user message, so event responses carry URLs instead of data URLs.
async fn session_image(State(s): State<Server>, Path((id, seq, index)): Path<(String, i64, usize)>) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let (bytes, mime) = launchpad_core::session_pages::image(&s.app, &id, seq, index).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok((
        [
            ("content-type", mime),
            // Stored events never change.
            ("cache-control", "private, max-age=31536000, immutable".into()),
            ("content-security-policy", "default-src 'none'; sandbox".into()),
            ("x-content-type-options", "nosniff".into()),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct Through {
    through: Option<i64>,
}

/// Plain-text trace for agents that received a trace reference as launch context.
async fn trace(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<Through>) -> Result<String, ApiError> {
    s.app.sessions.trace_text(&s.app, &id, q.through).await.map_err(bad)
}

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    text: String,
    #[serde(default)]
    images: Vec<launchpad_core::model::ImageAttachment>,
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn input(State(s): State<Server>, Path(id): Path<String>, headers: axum::http::HeaderMap, Json(b): Json<Input>) -> ApiResult<Value> {
    if b.text.trim().is_empty() && b.images.is_empty() {
        return Err(ApiError::bad_request("the message is empty"));
    }
    let mut b = b;
    launchpad_core::attachments::validate_images(&b.images).map_err(bad)?;
    launchpad_core::attachments::resolve_images(&mut b.images).map_err(bad)?;
    if let Some(client_id) = headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()) {
        s.app.sessions.send_with_id(&s.app, &id, &b.text, b.images, client_id.to_owned()).await.map_err(bad)?;
    } else {
        s.app.sessions.send(&s.app, &id, &b.text, b.images).await.map_err(bad)?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn interrupt(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.interrupt(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn session_processes(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Vec<SessionProcess>> {
    Ok(Json(s.app.sessions.processes(&id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct StopSessionProcess { thread_id: String, process_id: String }

async fn stop_session_process(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<StopSessionProcess>) -> ApiResult<Value> {
    s.app.sessions.terminate_process(&s.app, &id, &b.thread_id, &b.process_id).await.map_err(bad)?;
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
    // Archive makes clean task worktrees eligible for removal; Resume restores them.
    let app = s.app.clone();
    tokio::spawn(async move { launchpad_core::worktrees::after_archive(&app, &id).await });
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
struct Resume { #[serde(default)] close_terminal: bool, #[serde(default)] continue_turn: bool }
async fn resume(State(s): State<Server>, Path(id): Path<String>, body: Option<Json<Resume>>) -> ApiResult<Value> {
    let body = body.map(|b| b.0).unwrap_or(Resume { close_terminal: false, continue_turn: false });
    s.app.sessions.resume(&s.app, &id, body.close_terminal, body.continue_turn).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn resume_paid(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.resume_paid(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn resume_subscription(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.resume_subscription(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct PaidUsage { allow: bool }
async fn set_paid_usage(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<PaidUsage>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_paid_usage(&s.app, &id, b.allow).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionBudget { budget_usd: Option<f64> }
async fn set_budget(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<SessionBudget>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_budget(&s.app, &id, b.budget_usd).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionPermissions { mode: launchpad_core::model::PermissionMode }
async fn set_permissions(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<SessionPermissions>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_permissions(&s.app, &id, b.mode).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionModel { model: Option<String>, effort: Option<String> }
async fn set_model(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<SessionModel>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_model(&s.app, &id, b.model, b.effort).await.map_err(bad)?))
}

async fn open_terminal(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<launchpad_core::terminals::TerminalInfo> {
    Ok(Json(s.app.sessions.open_terminal(&s.app, &id).await.map_err(bad)?))
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
    // The harness reports a path on the machine of the session.
    let host = s.app.sessions.get(&id).and_then(|x| launchpad_core::hosts::loc::host_of(&x.cwd).map(str::to_string));
    let file = if file.starts_with('/') { launchpad_core::hosts::loc::join(host.as_deref(), &file) } else { file };
    let text = launchpad_core::hostfs::tail(&file, 4096).await.ok().map(|b| String::from_utf8_lossy(&b).to_string());
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
    launchpad_core::procs::command(&s.app.env, &browser, None).arg(&b.url).kill_on_drop(false).spawn().await.map_err(|e| bad(e.into()))?;
    Ok(Json(json!({"ok": true})))
}

async fn inbox(State(s): State<Server>) -> ApiResult<launchpad_core::inbox::InboxSnapshot> {
    Ok(Json(s.app.inbox.snapshot(&s.app).await?))
}
async fn inbox_sync(State(s): State<Server>) -> ApiResult<Value> {
    s.app.inbox.sync(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
async fn inbox_preview(
    State(s): State<Server>,
    Path(id): Path<String>,
) -> ApiResult<Option<launchpad_core::inbox::ThreadPreview>> {
    Ok(Json(s.app.inbox.preview(&s.app, &id).await?))
}
#[derive(Deserialize)]
struct PreviewRefresh { #[serde(default)] force: bool }
async fn inbox_refresh(State(s): State<Server>, Path(id): Path<String>, Json(b):Json<PreviewRefresh>) -> ApiResult<Value> {
    s.app.inbox.refresh_preview(&s.app, &id, b.force).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct InboxAction {
    action: String,
}
async fn inbox_action(
    State(s): State<Server>,
    Path(id): Path<String>,
    Json(b): Json<InboxAction>,
) -> ApiResult<Value> {
    s.app
        .inbox
        .action(&s.app, &id, &b.action)
        .await
        .map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
async fn inbox_reply(
    State(s): State<Server>,
    Path(id): Path<String>,
    Json(b): Json<Input>,
) -> ApiResult<Value> {
    s.app.inbox.reply(&s.app, &id, &b.text).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct InboxRestore { ids: Vec<String> }
async fn inbox_restore(State(s): State<Server>, Json(b): Json<InboxRestore>) -> ApiResult<Value> {
    s.app.inbox.restore(&s.app, b.ids).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}

// ---------------------------------------------------------------- delayed launches
use launchpad_core::schedules::{self, LaunchDelay, ScheduledLaunch};

#[derive(Deserialize)]
struct ScheduleRequest { spec: LaunchSpec, conditions: Vec<LaunchDelay> }
#[derive(Deserialize)]
struct FollowUpRequest { message: String, conditions: Vec<LaunchDelay> }
async fn follow_up(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<FollowUpRequest>) -> ApiResult<ScheduledLaunch> {
    Ok(Json(schedules::follow_up(&s.app, &id, b.message, b.conditions).await.map_err(bad)?))
}
async fn schedules(State(s): State<Server>) -> ApiResult<Vec<ScheduledLaunch>> { Ok(Json(schedules::list(&s.app).await?)) }
async fn schedule(State(s): State<Server>, Json(b): Json<ScheduleRequest>) -> ApiResult<ScheduledLaunch> { Ok(Json(schedules::create(&s.app, b.spec, b.conditions).await.map_err(bad)?)) }
async fn cancel_schedule(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    schedules::cancel(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn start_schedule(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> { Ok(Json(schedules::start_now(&s.app, &id).await.map_err(bad)?)) }

// ---------------------------------------------------------------- cron agents
use launchpad_core::crons::{self, CronAgent, CronCondition, CronPatch};

#[derive(Deserialize)]
struct CronRequest { spec: LaunchSpec, schedule: String, conditions: Vec<CronCondition> }
async fn crons(State(s): State<Server>) -> ApiResult<Vec<CronAgent>> { Ok(Json(crons::list(&s.app).await?)) }
async fn create_cron(State(s): State<Server>, Json(b): Json<CronRequest>) -> ApiResult<CronAgent> { Ok(Json(crons::create(&s.app, b.spec, b.schedule, b.conditions).await.map_err(bad)?)) }
async fn update_cron(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<CronPatch>) -> ApiResult<CronAgent> { Ok(Json(crons::update(&s.app, &id, b).await.map_err(bad)?)) }
async fn delete_cron(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    crons::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
#[derive(Deserialize)]
struct CronNext { schedule: String }
async fn cron_next(Query(q): Query<CronNext>) -> ApiResult<Vec<i64>> { Ok(Json(crons::next_runs(&q.schedule, 3).map_err(bad)?)) }
async fn run_cron(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> { Ok(Json(crons::run_now(&s.app, &id).await.map_err(bad)?)) }

#[derive(Deserialize)]
struct RenameSession { title: String }
async fn rename_session(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<RenameSession>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.rename(&s.app, &id, &b.title).await.map_err(bad)?))
}
async fn mark_seen(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.mark_seen(&s.app, &id).await.map_err(bad)?))
}
async fn mark_unread(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.mark_unread(&s.app, &id).await.map_err(bad)?))
}
async fn session_git(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<launchpad_core::session_git::GitStatus> {
    Ok(Json(launchpad_core::session_git::status(&s.app,&id).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct GitDiffQuery { path: String, #[serde(default)] main: bool }
async fn session_git_diff(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<GitDiffQuery>) -> ApiResult<launchpad_core::session_git::GitDiff> {
    Ok(Json(launchpad_core::session_git::diff(&s.app,&id,&q.path,q.main).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct SessionSearch { q: String }
async fn search_sessions(State(s): State<Server>, Query(q): Query<SessionSearch>) -> ApiResult<Vec<String>> {
    let needle = q.q.trim().to_string();
    if needle.is_empty() { return Ok(Json(vec![])); }
    let ids = s.app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT DISTINCT session_id FROM session_event WHERE instr(lower(json_extract(json, '$.text')), lower(?1)) > 0")?;
        Ok(st.query_map([needle], |r| r.get::<_,String>(0))?.collect::<Result<Vec<_>, _>>()?)
    }).await?;
    Ok(Json(ids))
}

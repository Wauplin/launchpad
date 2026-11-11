//! Routes for projects, launch, sessions, GitHub inbox and settings helpers.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use launchpad_core::model::*;
use launchpad_core::projects::{self, ProjectPatch};
use launchpad_core::sessions::Preview;
use launchpad_core::workspace::{self, Workspace};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/projects", get(list_projects))
        .route("/api/projects/remote", post(add_remote))
        .route("/api/projects/plain", post(add_plain))
        .route("/api/projects/{id}/to-git", post(plain_to_git))
        .route("/api/projects/{id}", get(get_project).patch(patch_project).delete(delete_project))
        .route("/api/projects/{id}/branches", get(branches))
        .route("/api/projects/{id}/promote", post(promote))
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/{id}", get(get_workspace).patch(patch_workspace).delete(delete_workspace))
        .route("/api/uploads", post(upload).layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024)))
        .route("/api/uploads/{hash}", get(uploaded_image))
        .route("/api/launch/preview", post(preview).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/launch/schedules", get(schedules).post(schedule).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}/follow-up", post(follow_up))
        .route("/api/launch/schedules/{id}", axum::routing::delete(cancel_schedule).patch(update_schedule).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/launch/schedules/{id}/start", post(start_schedule))
        .route("/api/launch/crons", get(crons).post(create_cron).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/launch/crons/next", get(cron_next))
        .route("/api/launch/crons/{id}", get(get_cron).patch(update_cron).delete(delete_cron))
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
        .route("/api/sessions/{id}/git-commits", get(session_git_history))
        .route("/api/sessions/{id}/git-diff", get(session_git_diff))
        .route("/api/sessions/search", get(search_sessions))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/events/page", get(events_page))
        .route("/api/sessions/{id}/machines", get(session_machines))
        .route("/api/sessions/{id}/images/{seq}/{index}", get(session_image))
        .route("/api/sessions/{id}/trace", get(trace))
        .route("/api/sessions/{id}/input", post(input).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}/interrupt", post(interrupt))
        .route("/api/sessions/{id}/notice", post(notice))
        .route("/api/sessions/{id}/processes", get(session_processes))
        .route("/api/sessions/{id}/processes/stop", post(stop_session_process))
        .route("/api/sessions/{id}/answer", post(answer))
        .route("/api/sessions/{id}/close", post(close))
        .route("/api/sessions/{id}/unarchive", post(unarchive))
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
        .route("/api/inbox/agent-done", post(inbox_agent_done))
        .route("/api/inbox/{id}/preview", get(inbox_preview).post(inbox_refresh))
        .route("/api/inbox/{id}/action", post(inbox_action))
        .route("/api/inbox/{id}/reply", post(inbox_reply))
        .route("/api/gh/resolve", post(gh_resolve))
        .route("/api/gh/{owner}/{repo}/suggest", get(gh_suggest))
}

// ---------------------------------------------------------------- projects

async fn list_projects(State(s): State<Ctx>) -> ApiResult<Vec<Project>> {
    Ok(Json(projects::list(&s.app).await?))
}

async fn get_project(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Project> {
    Ok(Json(projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?))
}

#[derive(Deserialize)]
struct AddRemote {
    name: String,
    /// Empty: a folder without a remote, imported by the runner that adds it (Keep as folder of a temporary
    /// folder that holds a Git repository).
    #[serde(default)]
    url: String,
    workspace_id: String,
}

#[derive(Deserialize)]
struct AddPlain {
    name: String,
    workspace_id: String,
}

/// A new, empty plain folder of Home.
async fn add_plain(State(s): State<Ctx>, Json(b): Json<AddPlain>) -> ApiResult<Project> {
    if s.app.paths.role == launchpad_core::role::Role::Runner {
        let v = launchpad_core::home_link::home_request("POST", "/api/projects/plain", Some(json!({"name": b.name, "workspace_id": b.workspace_id}))).await.map_err(bad)?;
        let p: Project = serde_json::from_value(v).map_err(|e| ApiError::bad_request(format!("{e}")))?;
        projects::store_home(&s.app, vec![p.clone()], false).await?;
        return Ok(Json(projects::get(&s.app, &p.id).await?.unwrap_or(p)));
    }
    Ok(Json(projects::create_plain(&s.app, &b.name, &b.workspace_id).await.map_err(bad)?))
}

/// Turn a plain folder into a Git folder (Home does it; a runner forwards the request).
async fn plain_to_git(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Project> {
    if s.app.paths.role == launchpad_core::role::Role::Runner {
        let v = launchpad_core::home_link::home_request("POST", &format!("/api/projects/{id}/to-git"), None).await.map_err(bad)?;
        let p: Project = serde_json::from_value(v).map_err(|e| ApiError::bad_request(format!("{e}")))?;
        projects::store_home(&s.app, vec![p.clone()], false).await?;
        return Ok(Json(projects::get(&s.app, &p.id).await?.unwrap_or(p)));
    }
    Ok(Json(projects::plain_to_git(&s.app, &id).await.map_err(bad)?))
}

/// Home: a Git folder by its remote (a runner keeping a temporary folder that holds a Git repository).
async fn add_remote(State(s): State<Ctx>, Json(b): Json<AddRemote>) -> ApiResult<Project> {
    Ok(Json(projects::register_remote(&s.app, &b.name, &b.url, &b.workspace_id).await.map_err(bad)?))
}

/// Runner: a change to a Git folder of Home goes to Home, and the runner keeps Home's answer.
async fn home_folder_change(s: &Ctx, p: &Project, method: &str, body: Option<Value>) -> Result<Option<Value>, ApiError> {
    if s.app.paths.role != launchpad_core::role::Role::Runner || !(p.home_git || p.plain) {
        return Ok(None);
    }
    let v = launchpad_core::home_link::home_request(method, &format!("/api/projects/{}", p.id), body).await.map_err(bad)?;
    if let Ok(updated) = serde_json::from_value::<Project>(v.clone()) {
        projects::store_home(&s.app, vec![updated], false).await?;
    }
    Ok(Some(v))
}

// ---------------------------------------------------------------- workspaces

async fn list_workspaces(State(s): State<Ctx>) -> ApiResult<Vec<Workspace>> {
    Ok(Json(workspace::list(&s.app).await?))
}

#[derive(Deserialize)]
struct NewWorkspace {
    name: String,
}

async fn create_workspace(State(s): State<Ctx>, Json(b): Json<NewWorkspace>) -> ApiResult<Workspace> {
    Ok(Json(workspace::create(&s.app, &b.name).await.map_err(bad)?))
}

async fn get_workspace(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Workspace> {
    Ok(Json(workspace::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}

async fn patch_workspace(State(s): State<Ctx>, Path(id): Path<String>, Json(patch): Json<workspace::Patch>) -> ApiResult<Workspace> {
    Ok(Json(workspace::update(&s.app, &id, patch).await.map_err(bad)?))
}

async fn delete_workspace(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    workspace::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

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

async fn patch_project(State(s): State<Ctx>, Path(id): Path<String>, Json(patch): Json<ProjectPatch>) -> ApiResult<Project> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if home_folder_change(&s, &p, "PATCH", Some(serde_json::to_value(&patch).map_err(|e| ApiError::bad_request(format!("{e}")))?)).await?.is_some() {
        return Ok(Json(projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?));
    }
    Ok(Json(projects::update(&s.app, &id, patch).await?))
}

/// Unregisters a project. For a temporary folder: stops its sessions and deletes the folder.
async fn delete_project(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if home_folder_change(&s, &p, "DELETE", None).await?.is_some() {
        return Ok(Json(json!({"ok": true})));
    }
    if p.is_temp {
        s.app.sessions.close_in(&s.app, &p.path).await?;
        if projects::get(&s.app, &id).await?.is_some() {
            projects::delete_temp(&s.app, &id).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
        }
    } else {
        projects::unregister(&s.app, &id).await?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn branches(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if p.home_git {
        // Base branches are Home's: its mirror follows the remote.
        if !launchpad_core::home_git::here(&s.app) {
            return Ok(Json(launchpad_core::home_link::home_request("GET", &format!("/api/projects/{id}/branches"), None).await.map_err(bad)?));
        }
        let (current, list) = launchpad_core::home_git::base_branches(&s.app, &id).await.unwrap_or((None, vec![]));
        return Ok(Json(json!({"current": current, "branches": list})));
    }
    let (current, list) = launchpad_core::git::branches(&s.app.env, std::path::Path::new(&p.path)).await.unwrap_or((None, vec![]));
    Ok(Json(json!({"current": current, "branches": list})))
}

#[derive(Deserialize)]
struct Promote {
    name: String,
}

async fn promote(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<Promote>) -> ApiResult<Project> {
    let p = projects::get(&s.app, &id).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    if s.app.sessions.running_in(&p.path) {
        return Err(ApiError::conflict("a session is running in this folder: close it first"));
    }
    Ok(Json(projects::keep_as_folder(&s.app, &id, &b.name).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

// ---------------------------------------------------------------- launch

async fn preview(State(s): State<Ctx>, Json(spec): Json<LaunchSpec>) -> ApiResult<Preview> {
    Ok(Json(s.app.sessions.preview(&s.app, &spec).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct DraftQuery {
    workspace: Option<String>,
}

async fn get_draft(State(s): State<Ctx>, Query(q): Query<DraftQuery>) -> ApiResult<Value> {
    Ok(Json(launchpad_core::drafts::get(&s.app, q.workspace.as_deref()).await?))
}

/// Body: the draft plus `base_revision` (the revision it was edited from), `force` (overwrite a newer
/// draft) and `client` (the window id echoed in the broadcast). A stale save gets 409 with the stored `revision`.
async fn put_draft(State(s): State<Ctx>, Query(q): Query<DraftQuery>, Json(draft): Json<Value>) -> Result<axum::response::Response, ApiError> {
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

async fn defaults(State(s): State<Ctx>, Path(kind): Path<String>) -> ApiResult<HarnessDefaults> {
    let h = HarnessKind::parse(&kind).ok_or_else(|| ApiError::not_found("unknown harness"))?;
    Ok(Json(s.app.models.defaults(&s.app, h)))
}

async fn harnesses(
    State(s): State<Ctx>,
) -> Json<Vec<launchpad_core::harness::HarnessDescriptor>> {
    Json(s.app.harnesses.descriptors())
}

async fn refresh_defaults(State(s): State<Ctx>, Path(kind): Path<String>) -> ApiResult<HarnessDefaults> {
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

async fn list_sessions(State(s): State<Ctx>) -> Json<Vec<Session>> {
    Json(s.app.sessions.list().iter().map(Session::light).collect())
}

async fn get_session(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.get(&id).ok_or_else(|| ApiError::not_found("no such session"))?))
}

async fn launch(State(s): State<Ctx>, headers: axum::http::HeaderMap, Json(spec): Json<LaunchSpec>) -> ApiResult<Session> {
    let id = headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()).map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // A runner starts only the sessions Home places on it (Home's own launches and Burst jobs arrive through the
    // tunnel); its own windows and the owner's CLI there launch on Home, which picks the runner.
    if s.app.paths.role == launchpad_core::role::Role::Runner && !headers.contains_key(launchpad_http::FROM_HOME) {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Launch from Home: a runner starts only the sessions Home places on it"));
    }
    Ok(Json(s.app.sessions.launch_with_id(&s.app, spec, id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct After {
    after: Option<i64>,
}

/// Catch-up after a reconnect or a sequence gap: every stored event after `after`.
async fn events(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<After>) -> ApiResult<Vec<SessionEvent>> {
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
async fn events_page(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<PageQuery>) -> ApiResult<launchpad_core::session_pages::SessionEventPage> {
    let limit = q.limit.or(q.tail).unwrap_or(200);
    Ok(Json(launchpad_core::session_pages::page(&s.app, &id, q.before, limit).await?))
}

/// The machines that ran the session's turns (Home's record; runners forward it).
async fn session_machines(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<launchpad_core::session_pages::MachineStint>> {
    Ok(Json(launchpad_core::session_pages::machines(&s.app, &id).await?))
}

/// An image pasted into a user message, so event responses carry URLs instead of data URLs.
async fn session_image(State(s): State<Ctx>, Path((id, seq, index)): Path<(String, i64, usize)>) -> Result<axum::response::Response, ApiError> {
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
async fn trace(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<Through>) -> Result<String, ApiError> {
    s.app.sessions.trace_text(&s.app, &id, q.through).await.map_err(bad)
}

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    text: String,
    #[serde(default)]
    images: Vec<launchpad_core::model::ImageAttachment>,
    /// Images or other files; files are copied to the session's attachment folder.
    #[serde(default)]
    files: Vec<launchpad_core::model::FileAttachment>,
    #[serde(default)]
    context: launchpad_core::model::MessageContext,
    /// A scheduled follow-up that Home placed here (`schedules::run`): "manual" or "automatic". A busy session
    /// answers 409, so Home keeps it queued.
    #[serde(default)]
    follow_up: Option<String>,
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn input(State(s): State<Ctx>, Path(id): Path<String>, headers: axum::http::HeaderMap, Json(b): Json<Input>) -> ApiResult<Value> {
    if b.text.trim().is_empty() && b.images.is_empty() && b.files.is_empty() && b.context.is_empty() {
        return Err(ApiError::bad_request("the message is empty"));
    }
    let mut b = b;
    // Only Home names a sender other than the owner (`launchpad_core::senders`), in the messages it relays.
    if !headers.contains_key(launchpad_http::FROM_HOME) {
        b.context.from = None;
    }
    // Images stay references (records and events carry no bytes); the harness gets data URLs at delivery.
    launchpad_core::attachments::fetch_missing_images(&s.app, &id, &b.images).await.map_err(bad)?;
    launchpad_core::attachments::validate_images(&b.images).map_err(bad)?;
    let (text, images) = launchpad_core::attachments::for_message(&s.app, &id, &b.text, &b.files).await.map_err(bad)?;
    b.text = text;
    b.images.extend(images);
    if let Some(mode) = &b.follow_up {
        return match s.app.sessions.send_follow_up(&s.app, &id, &b.text, mode == "automatic", b.context.from).await {
            Ok(()) => Ok(Json(json!({"ok": true}))),
            Err(e) if e.is::<launchpad_core::sessions::FollowUpBusy>() => Err(ApiError::conflict(format!("{e:#}"))),
            Err(e) => Err(bad(e)),
        };
    }
    let client_id = headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()).map(str::to_owned);
    s.app.sessions.send_with_context(&s.app, &id, &b.text, b.images, b.context, client_id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Notice {
    text: String,
}

/// A notice from Home (Home Git, Local PRs) in a session that runs here.
async fn notice(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<Notice>) -> ApiResult<Value> {
    if s.app.sessions.get(&id).is_none() {
        return Err(ApiError::not_found("no such session"));
    }
    s.app.sessions.sink(&s.app, &id).emit(launchpad_core::model::EventBody::Notice { text: b.text.chars().take(2000).collect() }).await;
    Ok(Json(json!({"ok": true})))
}

async fn interrupt(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.interrupt(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn session_processes(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<SessionProcess>> {
    Ok(Json(s.app.sessions.processes(&id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct StopSessionProcess { thread_id: String, process_id: String }

async fn stop_session_process(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<StopSessionProcess>) -> ApiResult<Value> {
    s.app.sessions.terminate_process(&s.app, &id, &b.thread_id, &b.process_id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Answer {
    request_id: String,
    answers: Vec<String>,
}

async fn answer(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<Answer>) -> ApiResult<Value> {
    s.app.sessions.answer(&s.app, &id, &b.request_id, b.answers).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn close(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    launchpad_core::sessions::archive(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn unarchive(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.unarchive(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
struct Resume { #[serde(default)] close_terminal: bool, #[serde(default)] continue_turn: bool }
async fn resume(State(s): State<Ctx>, Path(id): Path<String>, body: Option<Json<Resume>>) -> ApiResult<Value> {
    let body = body.map(|b| b.0).unwrap_or(Resume { close_terminal: false, continue_turn: false });
    s.app.sessions.resume(&s.app, &id, body.close_terminal, body.continue_turn).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn resume_paid(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.resume_paid(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn resume_subscription(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.sessions.resume_subscription(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct PaidUsage { allow: bool }
async fn set_paid_usage(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<PaidUsage>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_paid_usage(&s.app, &id, b.allow).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionBudget { budget_usd: Option<f64> }
async fn set_budget(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<SessionBudget>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_budget(&s.app, &id, b.budget_usd).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionPermissions { mode: launchpad_core::model::PermissionMode }
async fn set_permissions(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<SessionPermissions>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_permissions(&s.app, &id, b.mode).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SessionModel { model: Option<String>, effort: Option<String> }
async fn set_model(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<SessionModel>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.set_model(&s.app, &id, b.model, b.effort).await.map_err(bad)?))
}

async fn open_terminal(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<launchpad_core::terminals::TerminalInfo> {
    Ok(Json(s.app.sessions.open_terminal(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct TaskQuery {
    task_id: String,
}

/// The end of a background command's output file. Only files named in the session's own events.
async fn task_output(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<TaskQuery>) -> ApiResult<Value> {
    let events = s.app.sessions.events(&s.app, &id, 0).await?;
    let file = events.iter().rev().find_map(|e| match &e.body {
        EventBody::BackgroundTask { task_id, output_file: Some(f), .. } if *task_id == q.task_id => Some(f.clone()),
        _ => None,
    });
    let Some(file) = file else { return Ok(Json(json!({"text": null}))) };
    let text = launchpad_core::files::tail(&file, 4096).await.ok().map(|b| String::from_utf8_lossy(&b).to_string());
    Ok(Json(json!({"text": text})))
}

// ---------------------------------------------------------------- GitHub

#[derive(Deserialize)]
struct Resolve {
    url: String,
}

async fn gh_resolve(State(s): State<Ctx>, Json(b): Json<Resolve>) -> ApiResult<GitHubRef> {
    Ok(Json(s.app.github.resolve_link(&b.url).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Suggest {
    q: Option<String>,
}

async fn gh_suggest(
    State(s): State<Ctx>,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<Suggest>,
) -> ApiResult<Vec<launchpad_core::github::GhItem>> {
    Ok(Json(s.app.github.suggest(&format!("{owner}/{repo}"), q.q.as_deref().unwrap_or("")).await.map_err(bad)?))
}

async fn inbox(State(s): State<Ctx>) -> ApiResult<launchpad_github::inbox::InboxSnapshot> {
    Ok(Json(launchpad_github::inbox::of(&s.app).snapshot(&s.app).await?))
}
async fn inbox_sync(State(s): State<Ctx>) -> ApiResult<Value> {
    launchpad_github::inbox::of(&s.app).sync(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
async fn inbox_preview(
    State(s): State<Ctx>,
    Path(id): Path<String>,
) -> ApiResult<Option<launchpad_github::inbox::ThreadPreview>> {
    Ok(Json(launchpad_github::inbox::of(&s.app).preview(&s.app, &id).await?))
}
#[derive(Deserialize)]
struct PreviewRefresh { #[serde(default)] force: bool }
async fn inbox_refresh(State(s): State<Ctx>, Path(id): Path<String>, Json(b):Json<PreviewRefresh>) -> ApiResult<Value> {
    launchpad_github::inbox::of(&s.app).refresh_preview(&s.app, &id, b.force).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct InboxAction {
    action: String,
}
async fn inbox_action(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(b): Json<InboxAction>,
) -> ApiResult<Value> {
    launchpad_github::inbox::of(&s.app).action(&s.app, &id, &b.action)
        .await
        .map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}
async fn inbox_reply(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(b): Json<Input>,
) -> ApiResult<Value> {
    launchpad_github::inbox::of(&s.app).reply(&s.app, &id, &b.text).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct InboxRestore { ids: Vec<String> }
async fn inbox_restore(State(s): State<Ctx>, Json(b): Json<InboxRestore>) -> ApiResult<Value> {
    launchpad_github::inbox::of(&s.app).restore(&s.app, b.ids).await.map_err(bad)?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct AgentDone { session_id: String, ids: Vec<String>, #[serde(default)] unsubscribe: bool }
/// `launchpad inbox done|unsubscribe`: a bot session marks Inbox threads done, here and on GitHub, without the
/// owner's click; with `unsubscribe`, it also mutes them on GitHub first, like the Unsubscribe button.
/// Only for a bot whose bot.toml says `inbox_done = true` (and more than read access), checked on Home with
/// the bot's current settings; only threads still in the Inbox. Each thread is marked like the Done button.
async fn inbox_agent_done(State(s): State<Ctx>, Json(b): Json<AgentDone>) -> ApiResult<Value> {
    let forbidden = |m: &str| ApiError::new(axum::http::StatusCode::FORBIDDEN, m.to_string());
    let session = s.app.sessions.get(&b.session_id).ok_or_else(|| ApiError::not_found("no such session"))?;
    if session.status == SessionStatus::Closed {
        return Err(forbidden("this session is archived"));
    }
    let bot_id = session.launch_spec.bot.as_ref().map(|x| x.id.clone()).ok_or_else(|| forbidden("only a bot whose bot.toml says `inbox_done = true` may mark Inbox threads done; propose a ready card (`inbox.done`) instead"))?;
    let bot = launchpad_core::bots::get(&s.app, &bot_id).await.map_err(|_| ApiError::not_found("the session's bot no longer exists"))?;
    if bot.settings_error.is_some() || !bot.settings.inbox_done || bot.settings.launchpad == launchpad_core::vault::LaunchpadAccess::Read {
        return Err(forbidden("this bot may not mark Inbox threads done: the owner sets `inbox_done = true` in its bot.toml (the bot's page: About → Access); propose a ready card (`inbox.done`) instead"));
    }
    if b.ids.is_empty() || b.ids.len() > 200 {
        return Err(ApiError::bad_request("give 1 to 200 thread ids (from `launchpad inbox list`)"));
    }
    let action = if b.unsubscribe { "unsubscribe" } else { "done" };
    let inbox = launchpad_github::inbox::of(&s.app);
    let snap = inbox.snapshot(&s.app).await?;
    let active: std::collections::HashSet<String> = snap.active_ids.into_iter().collect();
    // Only threads of the repositories of the bot's workspace (`launchpad inbox list` shows them).
    let ws = launchpad_core::scope::of_session(&s.app, &b.session_id).await.ok_or_else(|| forbidden("this session has no workspace"))?;
    let repos = launchpad_core::scope::repos(&s.app, &ws).await;
    let ours: std::collections::HashSet<String> = snap.threads.into_iter().filter(|t| repos.contains(&t.repo.to_lowercase())).map(|t| t.id).collect();
    let mut results = Vec::new();
    for id in b.ids {
        let r = if !active.contains(&id) {
            Err("not in the Inbox (unknown id, or already done)".to_string())
        } else if !ours.contains(&id) {
            Err("a thread of a repository outside this workspace's folders".to_string())
        } else {
            inbox.action(&s.app, &id, action).await.map_err(|e| format!("{e:#}"))
        };
        match &r {
            Ok(()) => tracing::info!(session = %b.session_id, bot = %bot_id, thread = %id, action, "inbox thread marked done by a bot"),
            Err(e) => tracing::warn!(session = %b.session_id, bot = %bot_id, thread = %id, action, "inbox {action} by a bot failed: {e}"),
        }
        results.push(match r { Ok(()) => json!({"id": id, "done": true}), Err(e) => json!({"id": id, "done": false, "error": e}) });
    }
    Ok(Json(json!(results)))
}

// ---------------------------------------------------------------- delayed launches
use launchpad_automation::schedules::{self, LaunchDelay, ScheduledLaunch};

#[derive(Deserialize)]
struct ScheduleRequest { spec: LaunchSpec, conditions: Vec<LaunchDelay> }
#[derive(Deserialize)]
struct FollowUpRequest { message: String, conditions: Vec<LaunchDelay> }
/// The sender of a follow-up or scheduled launch: the agent whose CLI asked (Home's record, `senders`), else the
/// owner; never what the body says.
async fn sender(s: &Ctx, agent: Option<axum::Extension<launchpad_http::AgentSession>>) -> Result<Option<launchpad_core::model::MessageFrom>, ApiError> {
    launchpad_core::senders::of_request(&s.app, agent.as_ref().map(|a| a.0.0.as_str())).await.map_err(bad)
}
async fn follow_up(State(s): State<Ctx>, Path(id): Path<String>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(b): Json<FollowUpRequest>) -> ApiResult<ScheduledLaunch> {
    let from = sender(&s, agent).await?;
    Ok(Json(schedules::follow_up(&s.app, &id, b.message, b.conditions, from).await.map_err(bad)?))
}
async fn schedules(State(s): State<Ctx>) -> ApiResult<Vec<ScheduledLaunch>> { Ok(Json(schedules::list(&s.app).await?)) }
async fn schedule(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(mut b): Json<ScheduleRequest>) -> ApiResult<ScheduledLaunch> {
    b.spec.from = sender(&s, agent).await?;
    Ok(Json(schedules::create(&s.app, b.spec, b.conditions).await.map_err(bad)?))
}
#[derive(serde::Deserialize)]
struct ScheduleUpdate { spec: LaunchSpec, #[serde(default)] conditions: Option<Vec<LaunchDelay>> }
async fn update_schedule(State(s): State<Ctx>, Path(id): Path<String>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(mut b): Json<ScheduleUpdate>) -> ApiResult<ScheduledLaunch> {
    b.spec.from = sender(&s, agent).await?;
    Ok(Json(schedules::update(&s.app, &id, b.spec, b.conditions).await.map_err(bad)?))
}
async fn cancel_schedule(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    schedules::cancel(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn start_schedule(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Session> { Ok(Json(schedules::start_now(&s.app, &id).await.map_err(bad)?)) }

// ---------------------------------------------------------------- cron agents
use launchpad_automation::crons::{self, CronAgent, CronCondition, CronPatch};

#[derive(Deserialize)]
struct CronRequest { spec: LaunchSpec, #[serde(default)] schedule: String, conditions: Vec<CronCondition>, #[serde(default)] trigger: crons::Trigger, #[serde(default)] title: Option<String>, #[serde(default)] time_zone: Option<String> }
async fn crons(State(s): State<Ctx>) -> ApiResult<Vec<CronAgent>> { Ok(Json(crons::list(&s.app).await?)) }
async fn create_cron(State(s): State<Ctx>, Json(b): Json<CronRequest>) -> ApiResult<CronAgent> { Ok(Json(crons::create(&s.app, b.spec, b.schedule, b.conditions, b.trigger, b.title, b.time_zone).await.map_err(bad)?)) }
/// One routine, with a bot routine's instructions (`spec.text`) as `instructions`, the key its edits use.
async fn get_cron(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    let c = crons::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    let mut v = serde_json::to_value(&c).map_err(|e| ApiError::bad_request(format!("{e}")))?;
    if c.bot_id.is_some() {
        v["instructions"] = Value::String(c.spec.text.clone());
    }
    Ok(Json(v))
}
async fn update_cron(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<CronPatch>) -> ApiResult<CronAgent> { Ok(Json(crons::update(&s.app, &id, b).await.map_err(bad)?)) }
async fn delete_cron(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    crons::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
#[derive(Deserialize)]
struct CronNext { schedule: String, #[serde(default)] time_zone: Option<String> }
async fn cron_next(Query(q): Query<CronNext>) -> ApiResult<Vec<i64>> { Ok(Json(crons::next_runs(&q.schedule, q.time_zone.as_deref().filter(|z| !z.is_empty()), 3).map_err(bad)?)) }
async fn run_cron(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Session> { Ok(Json(crons::run_now(&s.app, &id).await.map_err(bad)?)) }

#[derive(Deserialize)]
struct RenameSession { title: String }
async fn rename_session(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<RenameSession>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.rename(&s.app, &id, &b.title).await.map_err(bad)?))
}
async fn mark_seen(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.mark_seen(&s.app, &id).await.map_err(bad)?))
}
async fn mark_unread(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Session> {
    Ok(Json(s.app.sessions.mark_unread(&s.app, &id).await.map_err(bad)?))
}
async fn session_git(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<launchpad_core::session_git::GitStatus> {
    Ok(Json(launchpad_core::session_git::status(&s.app,&id).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct GitDiffQuery { path: String, #[serde(default)] main: bool, #[serde(default)] committed: bool, commit: Option<String> }
#[derive(Deserialize)]
struct GitHistoryQuery { commit: Option<String> }
async fn session_git_history(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<GitHistoryQuery>) -> ApiResult<Option<launchpad_core::session_git::GitHistory>> {
    Ok(Json(launchpad_core::session_git::history(&s.app, &id, q.commit.as_deref()).await.map_err(bad)?))
}
async fn session_git_diff(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<GitDiffQuery>) -> ApiResult<launchpad_core::session_git::GitDiff> {
    if q.committed {
        return Ok(Json(launchpad_core::session_git::committed_diff(&s.app, &id, &q.path, q.commit.as_deref()).await.map_err(bad)?));
    }
    Ok(Json(launchpad_core::session_git::diff(&s.app,&id,&q.path,q.main).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct SessionSearch { q: String }
async fn search_sessions(State(s): State<Ctx>, Query(q): Query<SessionSearch>) -> ApiResult<Vec<String>> {
    let needle = q.q.trim().to_string();
    if needle.is_empty() { return Ok(Json(vec![])); }
    // Messages only (their partial index), on the reader connection: a long scan never
    // holds up the database thread that records events and replicas.
    let ids = s.app.state_db.read(move |c| {
        let mut st = c.prepare("SELECT DISTINCT session_id FROM session_event WHERE kind IN ('user_message', 'assistant_message') AND instr(lower(json_extract(json, '$.text')), lower(?1)) > 0")?;
        Ok(st.query_map([needle], |r| r.get::<_,String>(0))?.collect::<Result<Vec<_>, _>>()?)
    }).await?;
    Ok(Json(ids.into_iter().filter(|id| s.app.sessions.get(id).is_some()).collect()))
}

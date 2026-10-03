//! Routes for projects, launch, sessions, GitHub inbox and settings helpers.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use launchpad_core::model::*;
use launchpad_core::paths::Profile;
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
        .route("/api/launch/preview", post(preview))
        .route("/api/launch/schedules", get(schedules).post(schedule))
        .route("/api/launch/schedules/{id}", axum::routing::delete(cancel_schedule))
        .route("/api/launch/schedules/{id}/start", post(start_schedule))
        .route("/api/launch/crons", get(crons).post(create_cron))
        .route("/api/launch/crons/next", get(cron_next))
        .route("/api/launch/crons/{id}", axum::routing::patch(update_cron).delete(delete_cron))
        .route("/api/launch/crons/{id}/run", post(run_cron))
        .route("/api/launch/draft", get(get_draft).put(put_draft))
        .route("/api/harness/{kind}/defaults", get(defaults))
        .route("/api/sessions", get(list_sessions).post(launch))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/rename", post(rename_session))
        .route("/api/sessions/{id}/git", get(session_git))
        .route("/api/sessions/{id}/git-diff", get(session_git_diff))
        .route("/api/sessions/history", get(session_history).post(import_session))
        .route("/api/sessions/search", get(search_sessions))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/input", post(input).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/sessions/{id}/interrupt", post(interrupt))
        .route("/api/sessions/{id}/answer", post(answer))
        .route("/api/sessions/{id}/close", post(close))
        .route("/api/sessions/{id}/resume", post(resume))
        .route("/api/sessions/{id}/open-terminal", post(open_terminal))
        .route("/api/sessions/{id}/task-output", get(task_output))
        .route("/api/inbox", get(inbox))
        .route("/api/inbox/sync", post(inbox_sync))
        .route("/api/inbox/restore", post(inbox_restore))
        .route("/api/inbox/{id}/preview", get(inbox_preview).post(inbox_refresh))
        .route("/api/inbox/{id}/action", post(inbox_action))
        .route("/api/inbox/{id}/reply", post(inbox_reply))
        .route("/api/gh/resolve", post(gh_resolve))
        .route("/api/gh/{owner}/{repo}/suggest", get(gh_suggest))
        .route("/api/settings/terminals", get(terminals))
        .route("/api/settings/update", get(update_status).post(start_update))
        .route("/api/settings/update/install-lock", post(install_lock))
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
    /// The workspace of the window that adds it.
    workspace_id: Option<String>,
}

async fn add_project(State(s): State<Server>, Json(b): Json<AddProject>) -> ApiResult<Project> {
    let path = launchpad_core::paths::expand_home(b.path.trim());
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
async fn open_window(State(s): State<Server>, Json(b): Json<OpenWindow>) -> ApiResult<Value> {
    workspace::open_window(&s.app, b.workspace_id.as_deref()).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
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

#[derive(Deserialize)]
struct DraftQuery {
    workspace: Option<String>,
}

async fn get_draft(State(s): State<Server>, Query(q): Query<DraftQuery>) -> ApiResult<Value> {
    Ok(Json(launchpad_core::drafts::get(&s.app, q.workspace.as_deref()).await?))
}

async fn put_draft(State(s): State<Server>, Query(q): Query<DraftQuery>, Json(draft): Json<Value>) -> ApiResult<Value> {
    launchpad_core::drafts::put(&s.app, q.workspace.as_deref(), &draft).await?;
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
    #[serde(default)]
    text: String,
    #[serde(default)]
    images: Vec<launchpad_core::model::ImageAttachment>,
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn input(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Input>) -> ApiResult<Value> {
    if b.text.trim().is_empty() && b.images.is_empty() {
        return Err(ApiError::bad_request("the message is empty"));
    }
    use base64::Engine;
    if b.images.len() > 8 || b.images.iter().map(|i| i.data_url.len()).sum::<usize>() > 28_000_000 {
        return Err(ApiError::bad_request("attach at most 8 images, totaling 20 MB"));
    }
    for image in &b.images {
        let valid = image.data_url.split_once(',').is_some_and(|(header, data)| {
            matches!(header, "data:image/png;base64" | "data:image/jpeg;base64" | "data:image/webp;base64" | "data:image/gif;base64")
                && base64::engine::general_purpose::STANDARD.decode(data).is_ok_and(|bytes| !bytes.is_empty() && bytes.len() <= 5 * 1024 * 1024)
        });
        if !valid { return Err(ApiError::bad_request("images must be PNG, JPEG, WebP or GIF, up to 5 MB each")); }
    }
    s.app.sessions.send(&s.app, &id, &b.text, b.images).await.map_err(bad)?;
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

async fn update_status(State(s): State<Server>) -> ApiResult<Value> {
    let source = std::path::Path::new(launchpad_core::SOURCE);
    let latest = launchpad_core::procs::output(&s.app.env, "git", &["rev-parse", "refs/heads/main"], Some(source)).await.ok();
    let outdated = if let Some(ref latest) = latest {
        latest != launchpad_core::COMMIT && launchpad_core::procs::output(
            &s.app.env, "git", &["merge-base", "--is-ancestor", launchpad_core::COMMIT, latest], Some(source)
        ).await.is_ok()
    } else { false };
    let file = s.app.paths.data_dir.join("update-status");
    let line = std::fs::read_to_string(&file).unwrap_or_default();
    let mut fields = line.trim().splitn(3, '\t');
    let state = fields.next().filter(|v| !v.is_empty()).unwrap_or("idle");
    let state = if s.app.paths.profile == Profile::Prod { state } else { "unavailable" };
    let stage = fields.next().unwrap_or("");
    let message = fields.next().unwrap_or("");
    let message = if s.app.paths.profile == Profile::Prod { message } else { "Updates are available in the production app." };
    let log = std::fs::read_to_string(file.with_file_name("update.log")).unwrap_or_default();
    let log = log.chars().rev().take(6000).collect::<String>().chars().rev().collect::<String>();
    Ok(Json(json!({"state": state, "stage": stage, "message": message, "log": log, "commit": launchpad_core::COMMIT, "latest_commit": latest, "outdated": outdated})))
}

#[derive(Deserialize)]
struct InstallLock { #[serde(default)] release: bool }

async fn install_lock(State(s): State<Server>, Json(b): Json<InstallLock>) -> ApiResult<Value> {
    if s.app.paths.profile != Profile::Prod {
        return Err(ApiError::bad_request("Installation is production-only."));
    }
    Ok(Json(json!({"ready": s.app.sessions.prepare_install(b.release).await})))
}

async fn start_update(State(s): State<Server>) -> ApiResult<Value> {
    if s.app.paths.profile != Profile::Prod {
        return Err(ApiError::bad_request("Open the production app to update Launchpad."));
    }
    let script = std::path::Path::new(launchpad_core::SOURCE).join("packaging/update.sh");
    if !script.is_file() {
        return Err(ApiError::not_found(format!("Update script is missing: {}", script.display())));
    }
    let mut cmd = launchpad_core::procs::command(&s.app.env, "systemd-run", None);
    cmd.args(["--user", "--collect", "--unit=launchpad-update", "--property=Type=exec"]);
    if let Some(path) = s.app.env.get("PATH") {
        cmd.arg(format!("--setenv=PATH={path}"));
    }
    for key in ["DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
        if let Some(value) = s.app.env.get(key) {
            cmd.arg(format!("--setenv={key}={value}"));
        }
    }
    let out = cmd.arg("/usr/bin/bash").arg(script).output().await?;
    if !out.status.success() {
        return Err(ApiError::conflict(format!("Could not start update: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(Json(json!({"ok": true})))
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
    line: Option<u32>,
}

/// Opens a file or folder in the editor. VS Code and Cursor open a file at a line (`--goto file:line`);
/// other editors get the file alone.
async fn open_editor(State(s): State<Server>, Json(b): Json<OpenEditor>) -> ApiResult<Value> {
    let editor = s.app.config.get().commands.editor;
    let mut cmd = launchpad_core::procs::command(&s.app.env, &editor, None);
    let name = std::path::Path::new(&editor).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match b.line {
        Some(line) if matches!(name.as_str(), "code" | "code-insiders" | "codium" | "cursor" | "windsurf") => {
            cmd.arg("--goto").arg(format!("{}:{line}", b.path))
        }
        _ => cmd.arg(&b.path),
    };
    cmd.kill_on_drop(false).spawn().map_err(|e| bad(e.into()))?;
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
    s.app.inbox.reply(&s.app, &id, &b.text).await.map_err(bad)?;
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
#[derive(Deserialize)]
struct HistoryQuery { harness: HarnessKind, cursor: Option<String>, #[serde(default)] archived: bool }
async fn session_history(State(s): State<Server>, Query(q): Query<HistoryQuery>) -> ApiResult<launchpad_core::history::Page> {
    Ok(Json(launchpad_core::history::list(&s.app,q.harness,q.cursor,q.archived).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct ImportSession { harness: HarnessKind, id: String }
async fn import_session(State(s): State<Server>, Json(b): Json<ImportSession>) -> ApiResult<Session> {
    Ok(Json(launchpad_core::history::import(&s.app,b.harness,&b.id).await.map_err(bad)?))
}
async fn session_git(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<launchpad_core::session_git::GitStatus> {
    Ok(Json(launchpad_core::session_git::status(&s.app,&id).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct GitDiffQuery { path: String }
async fn session_git_diff(State(s): State<Server>, Path(id): Path<String>, Query(q): Query<GitDiffQuery>) -> ApiResult<launchpad_core::session_git::GitDiff> {
    Ok(Json(launchpad_core::session_git::diff(&s.app,&id,&q.path).await.map_err(bad)?))
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

//! Bots on Home (`launchpad_core::bots`): files, settings and the thread. Runners forward these requests to Home.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::ask_bot;
use launchpad_core::bots::{self, Bot, Create, Patch};
use launchpad_core::model::{FileAttachment, MessageContext, Session};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/bots", get(list).post(create))
        .route("/api/bots/{id}", get(one).patch(update).delete(remove))
        .route("/api/bots/{id}/message", post(message))
        .route("/api/bots/{id}/history", get(history))
        .route("/api/bots/{id}/file", get(file))
        .route("/api/bots/{id}/history/{change}", get(diff))
        .route("/api/bots/{id}/restore/{change}", post(restore))
        .route("/api/bots/{id}/routines", post(create_routine))
        .route("/api/bots/{id}/pause", post(pause))
        .route("/api/bots/templates", get(templates))
        .route("/api/bots/{id}/start-preview", get(start_preview))
        .route("/api/launch/start-preview", post(launch_start_preview).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
}

/// "What it sees at start" of the bot's next new session: its instructions and first prompt, in labelled parts.
async fn start_preview(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<launchpad_core::start::StartPreview> {
    Ok(Json(launchpad_core::start::bot_preview(&s.app, &id).await.map_err(bad)?))
}

/// "What it sees at start" of a launch spec, without starting anything.
async fn launch_start_preview(State(s): State<Ctx>, Json(spec): Json<launchpad_core::model::LaunchSpec>) -> ApiResult<launchpad_core::start::StartPreview> {
    Ok(Json(launchpad_core::start::preview(&s.app, &spec).await.map_err(bad)?))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Ctx>) -> ApiResult<Vec<Bot>> {
    Ok(Json(bots::list(&s.app).await.map_err(bad)?))
}
async fn create(State(s): State<Ctx>, Json(c): Json<Create>) -> ApiResult<Bot> {
    Ok(Json(bots::create(&s.app, c).await.map_err(bad)?))
}
async fn one(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Bot> {
    Ok(Json(bots::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}
async fn update(State(s): State<Ctx>, Path(id): Path<String>, Json(p): Json<Patch>) -> ApiResult<Bot> {
    // Routine files are checked before they are saved (a file with an error would keep its routine's old definition).
    if let Some(files) = &p.files {
        let bot = bots::get(&s.app, &id).await.map_err(bad)?;
        for (path, text) in files {
            if let Some(t) = text {
                launchpad_automation::crons::check_file(&s.app, &bot, t).await.map_err(|e| bad(e.context(path.clone())))?;
            }
        }
    }
    Ok(Json(bots::update(&s.app, &id, p).await.map_err(bad)?))
}
async fn remove(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    bots::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachments: Vec<FileAttachment>,
    /// Folders, GitHub links and agent traces, as in an agent's message box.
    #[serde(default)]
    context: MessageContext,
    /// Start a new session of the thread (with a reference to the previous one).
    #[serde(default)]
    new_session: bool,
}

async fn message(State(s): State<Ctx>, Path(id): Path<String>, Json(m): Json<Message>) -> ApiResult<Session> {
    Ok(Json(bots::message(&s.app, &id, m.text, m.attachments, m.context, m.new_session).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

/// One of the bot's files as text (its memory notes, BOT.md, bot.toml), never rendered as a document.
async fn file(State(s): State<Ctx>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<FileQuery>) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    let data = bots::read_file(&s.app, &id, &q.path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], data).into_response())
}

async fn history(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<launchpad_core::plain::Change>> {
    Ok(Json(bots::history(&s.app, &id).await.map_err(bad)?))
}
async fn diff(State(s): State<Ctx>, Path((id, change)): Path<(String, i64)>) -> ApiResult<Value> {
    Ok(Json(json!({"patch": bots::diff(&s.app, &id, change).await.map_err(bad)?})))
}
async fn restore(State(s): State<Ctx>, Path((id, change)): Path<(String, i64)>) -> ApiResult<Bot> {
    Ok(Json(bots::restore(&s.app, &id, change).await.map_err(bad)?))
}

async fn create_routine(State(s): State<Ctx>, Path(id): Path<String>, Json(r): Json<launchpad_automation::crons::BotRoutine>) -> ApiResult<launchpad_automation::crons::CronAgent> {
    Ok(Json(launchpad_automation::crons::create_for_bot(&s.app, &id, r).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Pause {
    paused: bool,
}

async fn pause(State(s): State<Ctx>, Path(id): Path<String>, Json(p): Json<Pause>) -> ApiResult<Bot> {
    Ok(Json(bots::set_paused(&s.app, &id, p.paused).await.map_err(bad)?))
}

/// Ready-made bots the New bot form starts from (Ask Launchpad, to recreate or adapt it).
async fn templates() -> Json<Value> {
    Json(json!([{ "id": "ask-launchpad", "name": ask_bot::NAME, "instructions": ask_bot::DEFAULT_INSTRUCTIONS, "starters": ask_bot::STARTERS }]))
}

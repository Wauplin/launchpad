//! Bots on Home (`launchpad_core::bots`): files, settings and the thread. Runners forward these requests to Home.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::ask_bot;
use launchpad_core::bots::{self, Bot, Create, Patch};
use launchpad_core::model::{FileAttachment, Session};
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
        .route("/api/bots/{id}/history/{change}", get(diff))
        .route("/api/bots/{id}/restore/{change}", post(restore))
        .route("/api/bots/{id}/routines", post(create_routine))
        .route("/api/bots/{id}/pause", post(pause))
        .route("/api/bots/templates", get(templates))
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
    /// Start a new session of the thread (with a reference to the previous one).
    #[serde(default)]
    new_session: bool,
}

async fn message(State(s): State<Ctx>, Path(id): Path<String>, Json(m): Json<Message>) -> ApiResult<Session> {
    Ok(Json(bots::message(&s.app, &id, m.text, m.attachments, m.new_session).await.map_err(bad)?))
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
    Json(json!([{ "id": "ask-launchpad", "name": ask_bot::NAME, "instructions": ask_bot::DEFAULT_INSTRUCTIONS }]))
}

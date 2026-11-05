//! Channels on Home (`launchpad_core::{channels, telegram, hooks}`): outgoing webhooks and the Outbox, the
//! owner's Telegram chat and incoming hooks. Runners forward `/api/channels` and `/api/outbox` to Home.
//! `POST /hooks/<id>` skips the browser sign-in: it authenticates with the hook's own secret.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use launchpad_core::channels::{self, Channel, Draft, DraftResult, Edit, NewChannel, OutboxMessage};
use launchpad_core::hooks::{self, CreatedHook, NewHook};
use launchpad_core::telegram::{self, TelegramStatus};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/channels", get(overview).post(add))
        .route("/api/channels/send", post(send_draft))
        .route("/api/channels/{id}", delete(remove))
        .route("/api/channels/telegram", patch(telegram_patch).delete(telegram_remove))
        .route("/api/channels/telegram/token", put(telegram_token))
        .route("/api/channels/telegram/code", post(telegram_code))
        .route("/api/channels/telegram/test", post(telegram_test))
        .route("/api/channels/telegram/notify", post(telegram_notify))
        .route("/api/channels/hooks", post(hook_create))
        .route("/api/channels/hooks/{id}", delete(hook_delete))
        .route("/api/outbox", get(outbox))
        .route("/api/outbox/{id}", patch(edit))
        .route("/api/outbox/{id}/send", post(send))
        .route("/api/outbox/{id}/discard", post(discard))
        .route("/api/outbox/{id}/mark-sent", post(mark_sent))
        .route("/hooks/{id}", post(receive).layer(DefaultBodyLimit::max(1 << 20)))
}

/// Paths that authenticate with their own secret instead of the browser sign-in.
pub fn hook_path(path: &str) -> bool {
    path.strip_prefix("/hooks/").is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn overview(State(s): State<Server>) -> ApiResult<Value> {
    let app = &s.app;
    let here = launchpad_core::vault::here(app);
    Ok(Json(json!({
        "here": here,
        "origin": channels::home_origin(app),
        "names": channels::names(app).await,
        "channels": if here { channels::list(app).await? } else { vec![] },
        "telegram": telegram::status(app).await,
        "hooks": if here { hooks::list(app).await? } else { vec![] },
    })))
}
async fn add(State(s): State<Server>, Json(n): Json<NewChannel>) -> ApiResult<Channel> {
    Ok(Json(channels::add(&s.app, n).await.map_err(bad)?))
}
async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    channels::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn send_draft(State(s): State<Server>, Json(d): Json<Draft>) -> ApiResult<DraftResult> {
    Ok(Json(channels::draft(&s.app, d).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Token {
    token: String,
}
async fn telegram_token(State(s): State<Server>, Json(t): Json<Token>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::set_token(&s.app, &t.token).await.map_err(bad)?))
}
async fn telegram_patch(State(s): State<Server>, Json(p): Json<telegram::Patch>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::patch(&s.app, p).await.map_err(bad)?))
}
async fn telegram_remove(State(s): State<Server>) -> ApiResult<Value> {
    telegram::remove(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn telegram_code(State(s): State<Server>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::new_code(&s.app).await.map_err(bad)?))
}
async fn telegram_test(State(s): State<Server>) -> ApiResult<Value> {
    telegram::test(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Notify {
    session_id: String,
    event: String,
}
/// A runner's notification for one of its sessions (the same events as Web Push).
async fn telegram_notify(State(s): State<Server>, Json(n): Json<Notify>) -> ApiResult<Value> {
    telegram::notification(&s.app, &n.session_id, &n.event).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn hook_create(State(s): State<Server>, Json(n): Json<NewHook>) -> ApiResult<CreatedHook> {
    Ok(Json(hooks::create(&s.app, n).await.map_err(bad)?))
}
async fn hook_delete(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    hooks::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn outbox(State(s): State<Server>) -> ApiResult<Vec<OutboxMessage>> {
    Ok(Json(channels::outbox(&s.app).await?))
}
async fn edit(State(s): State<Server>, Path(id): Path<String>, Json(e): Json<Edit>) -> ApiResult<OutboxMessage> {
    Ok(Json(channels::edit(&s.app, &id, e).await.map_err(bad)?))
}
async fn send(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(channels::send(&s.app, &id).await.map_err(bad)?))
}
async fn discard(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(channels::discard(&s.app, &id).await.map_err(bad)?))
}
async fn mark_sent(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(channels::mark_sent(&s.app, &id).await.map_err(bad)?))
}

async fn receive(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let secret = headers.get("x-launchpad-hook-secret").and_then(|v| v.to_str().ok());
    match hooks::receive(&s.app, &id, secret, &body).await {
        Ok(session) => (StatusCode::ACCEPTED, Json(json!({"ok": true, "session_id": session}))).into_response(),
        Err(hooks::Refused(status, message)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), Json(json!({"error": message}))).into_response(),
    }
}

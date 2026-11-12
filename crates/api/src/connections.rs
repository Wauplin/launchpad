//! Connections on Home (`launchpad_channels::{connections, telegram, hooks}`): outgoing webhooks and the Outbox, each
//! workspace's Telegram chat (`/api/connections/telegram/{workspace}`) and incoming hooks. Runners forward `/api/connections` and `/api/outbox` to Home.
//! `POST /hooks/<id>` skips the browser sign-in: it authenticates with the hook's own secret.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use launchpad_channels::connections::{self, Connection, Draft, DraftResult, Edit, NewConnection, OutboxMessage};
use launchpad_channels::hooks::{self, CreatedHook, NewHook};
use launchpad_channels::telegram::{self, TelegramStatus};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/connections", get(overview).post(add))
        .route("/api/connections/send", post(send_draft))
        .route("/api/connections/names", get(names))
        .route("/api/connections/{id}", delete(remove))
        .route("/api/connections/telegram/notify", post(telegram_notify))
        .route("/api/connections/telegram/{workspace}", get(telegram_status).patch(telegram_patch).delete(telegram_remove))
        .route("/api/connections/telegram/{workspace}/token", put(telegram_token))
        .route("/api/connections/telegram/{workspace}/code", post(telegram_code))
        .route("/api/connections/telegram/{workspace}/test", post(telegram_test))
        .route("/api/connections/hooks", post(hook_create))
        .route("/api/connections/hooks/{id}", delete(hook_delete))
        .route("/api/bots/{id}/queued", get(queued))
        .route("/api/bots/{id}/queued/{n}", delete(queued_cancel))
        .route("/api/outbox", get(outbox))
        .route("/api/outbox/{id}", patch(edit))
        .route("/api/outbox/{id}/send", post(send))
        .route("/api/outbox/{id}/discard", post(discard))
        .route("/api/outbox/{id}/mark-sent", post(mark_sent))
        .route("/api/context/hops", get(hops))
        .route("/hooks/{id}", post(receive).layer(DefaultBodyLimit::max(1 << 20)))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn overview(State(s): State<Ctx>) -> ApiResult<Value> {
    let app = &s.app;
    let here = launchpad_core::vault::here(app);
    Ok(Json(json!({
        "here": here,
        "origin": connections::home_origin(app),
        "names": connections::names(app).await,
        "connections": if here { connections::list(app).await? } else { vec![] },
        "hooks": if here { hooks::list(app).await? } else { vec![] },
    })))
}
/// The connection names agents can use, without the connections: what a runner reads (`home_client`'s runner list).
async fn names(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!({"names": connections::names(&s.app).await})))
}
async fn add(State(s): State<Ctx>, Json(n): Json<NewConnection>) -> ApiResult<Connection> {
    Ok(Json(connections::add(&s.app, n).await.map_err(bad)?))
}
async fn remove(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    connections::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
/// An agent's `launchpad send`: Home takes the session from its own record of the CLI token, never from the body,
/// so an agent reaches only the Telegram chat of its own workspace.
async fn send_draft(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(mut d): Json<Draft>) -> ApiResult<DraftResult> {
    if let Some(a) = agent {
        d.session_id = Some(a.0.0);
    }
    Ok(Json(connections::draft(&s.app, d).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Token {
    token: String,
}
async fn telegram_status(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::status(&s.app, &ws).await))
}
async fn telegram_token(State(s): State<Ctx>, Path(ws): Path<String>, Json(t): Json<Token>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::set_token(&s.app, &ws, &t.token).await.map_err(bad)?))
}
async fn telegram_patch(State(s): State<Ctx>, Path(ws): Path<String>, Json(p): Json<telegram::Patch>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::patch(&s.app, &ws, p).await.map_err(bad)?))
}
async fn telegram_remove(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Value> {
    telegram::remove(&s.app, &ws).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn telegram_code(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<TelegramStatus> {
    Ok(Json(telegram::new_code(&s.app, &ws).await.map_err(bad)?))
}
async fn telegram_test(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Value> {
    telegram::test(&s.app, &ws).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Notify {
    session_id: String,
    event: String,
}
/// A runner's notification for one of its sessions (`notify::session`).
async fn telegram_notify(State(s): State<Ctx>, Json(n): Json<Notify>) -> ApiResult<Value> {
    telegram::notification(&s.app, &n.session_id, &n.event).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn hook_create(State(s): State<Ctx>, Json(n): Json<NewHook>) -> ApiResult<CreatedHook> {
    Ok(Json(hooks::create(&s.app, n).await.map_err(bad)?))
}
async fn hook_delete(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    hooks::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// Hook and Telegram messages waiting for the bot's current turn to end.
async fn queued(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<hooks::QueuedMessage>> {
    Ok(Json(hooks::queued(&s.app, &id).await.map_err(bad)?))
}
async fn queued_cancel(State(s): State<Ctx>, Path((id, n)): Path<(String, i64)>) -> ApiResult<Value> {
    hooks::cancel(&s.app, &id, n).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn outbox(State(s): State<Ctx>) -> ApiResult<Vec<OutboxMessage>> {
    Ok(Json(connections::outbox(&s.app).await?))
}
async fn edit(State(s): State<Ctx>, Path(id): Path<String>, Json(e): Json<Edit>) -> ApiResult<OutboxMessage> {
    Ok(Json(connections::edit(&s.app, &id, e).await.map_err(bad)?))
}
async fn send(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(connections::send(&s.app, &id).await.map_err(bad)?))
}
async fn discard(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(connections::discard(&s.app, &id).await.map_err(bad)?))
}
async fn mark_sent(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<OutboxMessage> {
    Ok(Json(connections::mark_sent(&s.app, &id).await.map_err(bad)?))
}

/// `launchpad hops`: the hop count an agent sends with `X-Launchpad-Hops` when it calls an incoming hook, one after the
/// message its current turn answers (`senders::hops_of`), from Home's records of the CLI token's session.
async fn hops(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>) -> ApiResult<Value> {
    let Some(a) = agent else { return Err(ApiError::bad_request("only an agent session has a hop count")) };
    Ok(Json(json!({"hops": launchpad_core::senders::hops_of(&s.app, &a.0.0).await + 1})))
}

async fn receive(State(s): State<Ctx>, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let secret = headers.get("x-launchpad-hook-secret").and_then(|v| v.to_str().ok());
    let hops = headers.get("x-launchpad-hops").map(|v| v.to_str().unwrap_or("?"));
    match hooks::receive(&s.app, &id, secret, hops, &body).await {
        Ok(d) => {
            let note = if d.queued { "queued: the target is busy; the message is sent when its current turn ends" } else { "sent" };
            (StatusCode::ACCEPTED, Json(json!({"ok": true, "session_id": d.session.map(|s| s.id), "queued": d.queued, "status": note}))).into_response()
        }
        Err(hooks::Refused(status, message)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), Json(json!({"error": message}))).into_response(),
    }
}

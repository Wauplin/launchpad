//! The Android app's notifications on Home (`launchpad_channels::push`): the list a woken phone reads, the Firebase
//! key of Settings → Devices, and the session events runners forward. A phone registers its own push token with
//! `PUT /api/devices/self/push` (the server's device routes).

use axum::extract::State;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_channels::push::{self, PhoneNotifications, PushStatus};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/phone/notifications", get(notifications))
        .route("/api/phone/notifications/dismiss", post(dismiss))
        .route("/api/phone/notify", post(notify))
        .route("/api/phone/push", get(status))
        .route("/api/phone/push/key", put(set_key).delete(remove_key))
        .route("/api/phone/push/test", post(test))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn notifications(State(s): State<Ctx>) -> ApiResult<PhoneNotifications> {
    Ok(Json(push::notifications(&s.app).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Dismiss {
    id: String,
}
/// Seen on the phone, for a notification that is not an item of Today (`quota:<session id>`).
async fn dismiss(State(_): State<Ctx>, Json(d): Json<Dismiss>) -> ApiResult<Value> {
    push::dismiss(&d.id);
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Notify {
    session_id: String,
    event: String,
}
/// A runner's notification for one of its sessions (`notify::session`).
async fn notify(State(s): State<Ctx>, Json(n): Json<Notify>) -> ApiResult<Value> {
    push::record_event(&s.app, &n.session_id, &n.event).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn status(State(s): State<Ctx>) -> ApiResult<PushStatus> {
    Ok(Json(push::status(&s.app).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Key {
    key: String,
}
async fn set_key(State(s): State<Ctx>, Json(k): Json<Key>) -> ApiResult<PushStatus> {
    Ok(Json(push::set_key(&s.app, &k.key).await.map_err(bad)?))
}
async fn remove_key(State(s): State<Ctx>) -> ApiResult<PushStatus> {
    Ok(Json(push::remove_key(&s.app).await.map_err(bad)?))
}
async fn test(State(s): State<Ctx>) -> ApiResult<Value> {
    let sent = push::test(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok": true, "sent": sent})))
}

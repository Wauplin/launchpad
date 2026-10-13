//! Web Push subscriptions of paired browsers (`launchpad_core::push`).

use crate::{
    Server,
    auth::Access,
    error::{ApiError, ApiResult},
};
use axum::extract::{Query, State};
use axum::{Extension, Json, Router, routing::{get, post}};
use launchpad_core::push::{Subscription, check_subscription};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/push", get(state))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
        .route("/api/push/test", post(test))
}

fn device(access: &Access) -> Result<String, ApiError> {
    access.device.clone().ok_or_else(|| ApiError::bad_request("Notifications need a paired browser, not a bearer token"))
}

#[derive(Deserialize)]
struct StateQuery {
    endpoint: Option<String>,
}

/// The VAPID public key, and whether this browser's current subscription is registered here.
async fn state(State(s): State<Server>, Extension(access): Extension<Access>, Query(q): Query<StateQuery>) -> ApiResult<Value> {
    let public_key = s.app.push.public_key()?;
    let (Some(device), Some(endpoint)) = (access.device.clone(), q.endpoint) else {
        return Ok(Json(json!({"public_key": public_key, "subscribed": false, "titles": false})));
    };
    let titles: Option<bool> = s
        .app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT titles FROM push_subscription WHERE endpoint = ?1 AND device_id = ?2")?;
            let mut rows = st.query_map([endpoint, device], |r| r.get::<_, i64>(0).map(|t| t != 0))?;
            Ok(rows.next().transpose()?)
        })
        .await?;
    Ok(Json(json!({"public_key": public_key, "subscribed": titles.is_some(), "titles": titles.unwrap_or(false)})))
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}
#[derive(Deserialize)]
struct Subscribe {
    endpoint: String,
    keys: Keys,
    #[serde(default)]
    titles: bool,
}

async fn subscribe(State(s): State<Server>, Extension(access): Extension<Access>, Json(body): Json<Subscribe>) -> ApiResult<Value> {
    let device = device(&access)?;
    let sub = Subscription { endpoint: body.endpoint, p256dh: body.keys.p256dh, auth: body.keys.auth };
    check_subscription(&sub).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let titles = body.titles;
    s.app
        .state_db
        .call(move |c| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
            // One row per endpoint: a re-subscription replaces keys, device and the titles choice.
            c.execute(
                "INSERT INTO push_subscription (endpoint, device_id, p256dh, auth, titles, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (endpoint) DO UPDATE SET device_id = ?2, p256dh = ?3, auth = ?4, titles = ?5",
                (sub.endpoint, device, sub.p256dh, sub.auth, titles as i64, now),
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

async fn unsubscribe(State(s): State<Server>, Extension(access): Extension<Access>, Json(body): Json<Unsubscribe>) -> ApiResult<Value> {
    let device = access.device.clone();
    s.app
        .state_db
        .call(move |c| {
            c.execute("DELETE FROM push_subscription WHERE endpoint = ?1 AND (?2 IS NULL OR device_id = ?2)", (body.endpoint, device))?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok": true})))
}

async fn test(State(s): State<Server>, Extension(access): Extension<Access>) -> ApiResult<Value> {
    let device = device(&access)?;
    let sent = s.app.push.send_test(&s.app, &device).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(json!({"sent": sent})))
}

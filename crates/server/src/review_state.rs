//! Review progress shared across browsers and devices (`launchpad_core::review_state`).
//! Keys go in the query string because they contain `/` (repository names).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::review_state;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/review-state", get(read).put(write).delete(remove).layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)))
        .route("/api/review-state/import", post(import).layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024)))
}

#[derive(Deserialize)]
struct KeyQuery {
    key: String,
}

#[derive(Deserialize)]
struct Write {
    value: Value,
    origin: Option<String>,
}

#[derive(Deserialize)]
struct Remove {
    origin: Option<String>,
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// Returns `{key, value, updated_at}`, with a null value and updated_at when nothing is stored.
async fn read(State(s): State<Server>, Query(q): Query<KeyQuery>) -> ApiResult<Value> {
    let entry = review_state::get(&s.app, &q.key).await.map_err(bad)?;
    Ok(Json(match entry {
        Some(e) => json!(e),
        None => json!({"key": q.key, "value": null, "updated_at": null}),
    }))
}

async fn write(State(s): State<Server>, Query(q): Query<KeyQuery>, Json(b): Json<Write>) -> ApiResult<Value> {
    let updated_at = review_state::put(&s.app, &q.key, b.value, b.origin).await.map_err(bad)?;
    Ok(Json(json!({"ok": true, "updated_at": updated_at})))
}

async fn remove(State(s): State<Server>, Query(q): Query<KeyQuery>, b: Option<Json<Remove>>) -> ApiResult<Value> {
    let origin = b.and_then(|Json(b)| b.origin);
    review_state::put(&s.app, &q.key, Value::Null, origin).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// Imports values kept in a browser's local storage before review state moved to the daemon.
async fn import(State(s): State<Server>, Json(entries): Json<HashMap<String, Value>>) -> ApiResult<Value> {
    let imported = review_state::import(&s.app, entries).await.map_err(bad)?;
    Ok(Json(json!({"imported": imported})))
}

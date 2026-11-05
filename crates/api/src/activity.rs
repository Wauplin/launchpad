//! The activity list (`launchpad_automation::activity`): direct admin actions of sessions, with Undo. Home holds it;
//! runners forward these requests (`home_client::home_owned`). Only owner requests reach `POST /api/activity`
//! (the front door records there) and `undone`: neither is an action of the action list, so session tokens are
//! refused.
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_automation::activity::{self, Activity, Record};
use serde::Deserialize;

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/activity", get(list).post(record))
        .route("/api/activity/{id}/undone", post(undone))
}

#[derive(Deserialize)]
struct Q {
    workspace: Option<String>,
}

/// A workspace given by ID or name (the CLI takes both).
pub(crate) async fn workspace_id(s: &Ctx, w: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(w) = w.filter(|w| !w.is_empty()) else { return Ok(None) };
    let all = launchpad_core::workspace::list(&s.app).await?;
    let ws = all.iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(&w)).ok_or_else(|| ApiError::not_found(format!("no workspace {w}")))?;
    Ok(Some(ws.id.clone()))
}

async fn list(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<Vec<Activity>> {
    Ok(Json(activity::list(&s.app, workspace_id(&s, q.workspace).await?).await?))
}
async fn record(State(s): State<Ctx>, Json(r): Json<Record>) -> ApiResult<Activity> {
    Ok(Json(activity::record(&s.app, r).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}
async fn undone(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Activity> {
    Ok(Json(activity::undone(&s.app, &id).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

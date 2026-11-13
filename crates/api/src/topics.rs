//! Home-owned topic routes. Agents use their workspace-scoped CLI reads.
use crate::{
    Ctx,
    error::{ApiError, ApiResult},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use launchpad_core::topics;
use serde::Deserialize;
use serde_json::{Value, json};
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/context/topics", get(context_topics))
        .route("/api/topics", get(list).post(create))
        .route("/api/topics/{id}", get(read).patch(rename).delete(delete))
        .route("/api/topics/{id}/members", axum::routing::post(members))
}
fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}
#[derive(Deserialize)]
struct Workspace {
    workspace: String,
}
#[derive(Deserialize)]
struct Create {
    workspace_id: String,
    name: String,
}
#[derive(Deserialize)]
struct Rename {
    name: String,
}
#[derive(Deserialize)]
struct Members {
    session_ids: Vec<String>,
    present: bool,
}
async fn list(State(s): State<Ctx>, Query(q): Query<Workspace>) -> ApiResult<Vec<topics::Topic>> {
    Ok(Json(topics::list(&s.app, &q.workspace).await.map_err(bad)?))
}
async fn read(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<topics::Topic> {
    Ok(Json(topics::get(&s.app, &id).await.map_err(bad)?))
}
async fn create(State(s): State<Ctx>, Json(b): Json<Create>) -> ApiResult<topics::Topic> {
    Ok(Json(
        topics::create(&s.app, &b.workspace_id, &b.name)
            .await
            .map_err(bad)?,
    ))
}
async fn rename(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(b): Json<Rename>,
) -> ApiResult<topics::Topic> {
    Ok(Json(
        topics::rename(&s.app, &id, &b.name).await.map_err(bad)?,
    ))
}
async fn delete(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    topics::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}
async fn members(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(b): Json<Members>,
) -> ApiResult<Value> {
    topics::membership(&s.app, &id, &b.session_ids, b.present)
        .await
        .map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ContextQuery {
    workspace: String,
    id: Option<String>,
}
async fn context_topics(State(s): State<Ctx>, Query(q): Query<ContextQuery>) -> ApiResult<Value> {
    let mut topics = topics::list(&s.app, &q.workspace).await.map_err(bad)?;
    for topic in &mut topics {
        let mut visible = Vec::new();
        for id in &topic.session_ids {
            if launchpad_core::scope::of_session(&s.app, id)
                .await
                .as_deref()
                == Some(&q.workspace)
            {
                visible.push(id.clone());
            }
        }
        topic.session_ids = visible;
    }
    if let Some(id) = q.id {
        let topic = topics
            .into_iter()
            .find(|t| t.id == id || t.name.eq_ignore_ascii_case(&id))
            .ok_or_else(|| ApiError::not_found("No topic in this workspace"))?;
        let runs: Vec<_> = topic.session_ids.iter().filter_map(|id| s.app.sessions.get(id)).map(|r| json!({"session_id":r.id,"title":r.title,"status":r.status,"through_seq":r.last_seq,"read":format!("launchpad trace {} --through {}", r.id,r.last_seq)})).collect();
        return Ok(Json(json!({"topic":topic,"runs":runs})));
    }
    Ok(Json(json!(topics)))
}

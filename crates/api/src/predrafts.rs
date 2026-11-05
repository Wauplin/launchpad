//! Local PRs live on Home. A runner answers `create` and `resolve` for its own worktrees (it pushes the branch
//! first) and forwards everything else to Home.
use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use launchpad_core::predrafts::{self, Create, MergeLocal, Patch, Publish};
use serde::Deserialize;
use serde_json::{Value, json};
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/predrafts", get(list).post(create))
        .route("/api/predrafts/resolve", get(resolve))
        .route("/api/predrafts/{id}", get(detail).patch(update))
        .route("/api/predrafts/{id}/record", get(record))
        .route("/api/predrafts/{id}/{action}", post(action))
}
/// Runner: sends the request to Home and returns its answer.
async fn home(s: &Ctx, method: &str, path: &str, body: Option<Value>) -> Option<ApiResult<Value>> {
    if launchpad_core::home_git::here(&s.app) {
        return None;
    }
    Some(launchpad_core::home_link::home_request(method, path, body).await.map(Json).map_err(|e| crate::ApiError::bad_request(format!("{e:#}"))))
}
async fn list(State(s): State<Ctx>) -> ApiResult<Value> {
    if let Some(r) = home(&s, "GET", "/api/predrafts", None).await { return r; }
    Ok(Json(json!(predrafts::list(&s.app).await?)))
}
async fn create(State(s): State<Ctx>, Json(q): Json<Create>) -> ApiResult<Value> {
    if q.cwd.is_none() && let Some(r) = home(&s, "POST", "/api/predrafts", Some(serde_json::to_value(&q)?)).await { return r; }
    Ok(Json(json!(predrafts::create(&s.app, q).await?)))
}
#[derive(Deserialize)]
struct Cwd {
    cwd: String,
}
async fn resolve(State(s): State<Ctx>, Query(q): Query<Cwd>) -> ApiResult<Value> {
    Ok(Json(json!(predrafts::resolve(&s.app, &q.cwd).await?)))
}
async fn record(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    if let Some(r) = home(&s, "GET", &format!("/api/predrafts/{id}/record"), None).await { return r; }
    Ok(Json(json!(predrafts::get(&s.app, &id).await?)))
}
async fn detail(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    if let Some(r) = home(&s, "GET", &format!("/api/predrafts/{id}"), None).await { return r; }
    Ok(Json(predrafts::detail(&s.app, &id, false).await?))
}
async fn update(State(s): State<Ctx>, Path(id): Path<String>, Json(q): Json<Value>) -> ApiResult<Value> {
    if let Some(r) = home(&s, "PATCH", &format!("/api/predrafts/{id}"), Some(q.clone())).await { return r; }
    let q: Patch = serde_json::from_value(q)?;
    Ok(Json(json!(predrafts::update(&s.app, &id, q).await?)))
}
async fn action(State(s): State<Ctx>, Path((id, a)): Path<(String, String)>, Json(q): Json<Value>) -> ApiResult<Value> {
    if let Some(r) = home(&s, "POST", &format!("/api/predrafts/{id}/{a}"), Some(q.clone())).await { return r; }
    let v = match a.as_str() {
        "refresh" => predrafts::detail(&s.app, &id, true).await?,
        "feedback" => {
            predrafts::feedback(&s.app, &id).await?;
            json!({"ok":true})
        }
        "discard" => json!(predrafts::discard(&s.app, &id).await?),
        "unlock" => json!(predrafts::unlock(&s.app, &id).await?),
        "publish" => json!(predrafts::publish(&s.app, &id, serde_json::from_value::<Publish>(q)?).await?),
        "publish-set" => json!(predrafts::publish_set(&s.app, &id, q["draft"].as_bool().unwrap_or(false)).await?),
        "merge-local" => json!(predrafts::merge_local(&s.app, &id, serde_json::from_value::<MergeLocal>(q)?).await?),
        _ => return Err(anyhow::anyhow!("unknown Local PR action").into()),
    };
    Ok(Json(v))
}

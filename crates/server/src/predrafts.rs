use crate::{Server, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use launchpad_core::predrafts::{self, Create, Patch, Publish};
use serde::Deserialize;
use serde_json::{Value, json};
pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/predrafts", get(list).post(create))
        .route("/api/predrafts/resolve", get(resolve))
        .route("/api/predrafts/{id}", get(detail).patch(update))
        .route("/api/predrafts/{id}/{action}", post(action))
}
async fn list(State(s): State<Server>) -> ApiResult<Value> {
    Ok(Json(json!(predrafts::list(&s.app).await?)))
}
async fn create(State(s): State<Server>, Json(q): Json<Create>) -> ApiResult<Value> {
    Ok(Json(json!(predrafts::create(&s.app, q).await?)))
}
#[derive(Deserialize)]
struct Cwd {
    cwd: String,
}
async fn resolve(State(s): State<Server>, Query(q): Query<Cwd>) -> ApiResult<Value> {
    Ok(Json(json!(predrafts::resolve(&s.app, &q.cwd).await?)))
}
async fn detail(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(predrafts::detail(&s.app, &id, false).await?))
}
async fn update(
    State(s): State<Server>,
    Path(id): Path<String>,
    Json(q): Json<Patch>,
) -> ApiResult<Value> {
    Ok(Json(json!(predrafts::update(&s.app, &id, q).await?)))
}
async fn action(
    State(s): State<Server>,
    Path((id, a)): Path<(String, String)>,
    Json(q): Json<Value>,
) -> ApiResult<Value> {
    let v = match a.as_str() {
        "refresh" => predrafts::detail(&s.app, &id, true).await?,
        "feedback" => {
            predrafts::feedback(&s.app, &id).await?;
            json!({"ok":true})
        }
        "discard" => json!(predrafts::discard(&s.app, &id).await?),
        "publish" => {
            json!(predrafts::publish(&s.app, &id, serde_json::from_value::<Publish>(q)?).await?)
        }
        _ => return Err(anyhow::anyhow!("unknown pre-draft action").into()),
    };
    Ok(Json(v))
}

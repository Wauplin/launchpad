use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use launchpad_github::suggestions::{self, Create, Edit};
use serde_json::{Value, json};
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/suggestions", get(list).post(create))
        .route(
            "/api/suggestions/{id}",
            axum::routing::patch(edit).delete(discard),
        )
        .route("/api/suggestions/{id}/post", post(publish))
}
async fn list(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::list(&s.app).await?)))
}
async fn create(State(s): State<Ctx>, Json(q): Json<Create>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::create(&s.app, q).await?)))
}
async fn edit(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(q): Json<Edit>,
) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::edit(&s.app, &id, q).await?)))
}
async fn discard(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    suggestions::discard(&s.app, &id).await?;
    Ok(Json(json!({"ok":true})))
}
async fn publish(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::post(&s.app, &id).await?)))
}

use axum::{Extension, Json, Router, extract::{Path, State}, routing::{get, post}};
use serde_json::{Value, json};
use launchpad_core::browser;
use crate::{Server, auth::Access, error::{ApiError, ApiResult}};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/browser/grants", get(list).post(create))
        .route("/api/browser/grants/{id}", axum::routing::delete(revoke))
        .route("/api/browser/replies/{id}", post(reply))
        .route("/api/browser/sessions/{id}/grants/{grant}", post(attach))
        .route("/api/browser/sessions/{id}/call", post(call))
}

async fn list(State(s): State<Server>, Extension(access): Extension<Access>) -> ApiResult<Value> {
    Ok(Json(json!(browser::grants(&s.app).await?.into_iter().filter(|g| access.device.as_ref().is_none_or(|id| &g.device == id)).collect::<Vec<_>>())))
}

async fn create(State(s): State<Server>, Extension(access): Extension<Access>, Json(page): Json<browser::Page>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Pair the Chrome extension first"))?;
    Ok(Json(json!(browser::create(&s.app, device, page).await.map_err(|e| ApiError::bad_request(e.to_string()))?)))
}

async fn revoke(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    browser::revoke(&s.app, &id).await?;
    Ok(Json(json!({"ok":true})))
}

async fn reply(State(s): State<Server>, Extension(access): Extension<Access>, Path(id): Path<String>, Json(result): Json<Value>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Browser replies require a paired device"))?;
    s.app.browser.reply(&device, &id, result);
    Ok(Json(json!({"ok":true})))
}

async fn call(State(s): State<Server>, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let tool = body["name"].as_str().ok_or_else(|| ApiError::bad_request("Missing tool name"))?;
    Ok(Json(s.app.browser.call(&s.app, &id, tool, body["arguments"].clone()).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?))
}

async fn attach(State(s): State<Server>, Path((id, grant)): Path<(String, String)>) -> ApiResult<Value> {
    browser::attach(&s.app, &id, &grant).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

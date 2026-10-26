use axum::{Extension, Json, Router, extract::{Path, State}, routing::{get, post}};
use serde_json::{Value, json};
use launchpad_core::browser;
use crate::{Server, auth::Access, error::{ApiError, ApiResult}};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/browser/grants", get(list).post(create))
        .route("/api/browser/grants/new-tab", post(new_tab))
        .route("/api/browser/grants/{id}", axum::routing::delete(revoke))
        .route("/api/browser/replies/{id}", post(reply))
        .route("/api/browser/reviews", get(reviews))
        .route("/api/browser/dispatch/{id}", post(authorize))
        .route("/api/browser/reviews/{id}", post(decide))
        .route("/api/browser/grants/{id}/control", post(control))
        .route("/api/browser/sessions/{id}/grants/{grant}", post(attach))
        .route("/api/browser/sessions/{id}/call", post(call))
        .route("/api/browser/sessions/{id}/mode", post(mode))
        .route("/api/browser/sites", get(sites).post(allow_site))
        .route("/api/browser/sites/forget", post(forget_site))
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

async fn reviews(State(s): State<Server>, Extension(access): Extension<Access>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    Ok(Json(json!(s.app.browser.reviews(&device))))
}
async fn decide(State(s): State<Server>, Extension(access): Extension<Access>, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    // "Always allow this site" on a site review also remembers the destination for later navigation.
    let site = (body["allow"] == true && body["always"] == true).then(|| s.app.browser.reviews(&device).into_iter().find(|r| r.id == id))
        .flatten().filter(|r| matches!(r.details["tool"].as_str(), Some("browser_navigate" | "browser_open_tab")))
        .and_then(|r| r.details["target"]["url"].as_str().map(str::to_owned));
    if let Some(url) = site { browser::allow_site(&s.app, &url).await.map_err(|e| ApiError::bad_request(e.to_string()))?; }
    s.app.browser.decide(&device, &id, body["allow"] == true).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}
async fn control(State(s): State<Server>, Extension(access): Extension<Access>, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    browser::control(&s.app, &device, &id, body["action"].as_str().unwrap_or("")).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

async fn authorize(State(s): State<Server>, Extension(access): Extension<Access>, Path(id): Path<String>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    s.app.browser.authorize(&s.app, &device, &id).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

async fn mode(State(s): State<Server>, Extension(access): Extension<Access>, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    browser::set_mode(&s.app, &device, &id, body["act"] == true).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

async fn sites(State(s): State<Server>) -> ApiResult<Value> {
    Ok(Json(json!(browser::sites(&s.app).await?)))
}

async fn allow_site(State(s): State<Server>, Extension(access): Extension<Access>, Json(body): Json<Value>) -> ApiResult<Value> {
    access.device.ok_or_else(|| ApiError::bad_request("Owner browser required"))?;
    browser::allow_site(&s.app, body["url"].as_str().unwrap_or("")).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

async fn forget_site(State(s): State<Server>, Json(body): Json<Value>) -> ApiResult<Value> {
    browser::forget_site(&s.app, body["origin"].as_str().unwrap_or("")).await?;
    Ok(Json(json!({"ok":true})))
}

async fn new_tab(State(s): State<Server>, Extension(access): Extension<Access>) -> ApiResult<Value> {
    let device = access.device.ok_or_else(|| ApiError::bad_request("Pair the Chrome extension first"))?;
    Ok(Json(json!(browser::new_tab(&s.app, device).await?)))
}

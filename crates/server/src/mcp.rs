//! MCP servers of Home (Settings → Credentials and tools). Runners forward these paths to Home.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_core::mcp::{self, McpServer};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/mcp", get(list))
        .route("/api/mcp/{id}", put(save).delete(remove))
        .route("/api/mcp/{id}/refresh", post(refresh))
        .route("/api/mcp/{id}/tools/{tool}", put(set_tool))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> ApiResult<Vec<McpServer>> {
    Ok(Json(mcp::list(&s.app).await?))
}

async fn save(State(s): State<Server>, Path(id): Path<String>, Json(p): Json<mcp::Put>) -> ApiResult<McpServer> {
    Ok(Json(mcp::put(&s.app, &id, p).await.map_err(bad)?))
}

async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    mcp::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn refresh(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<McpServer> {
    Ok(Json(mcp::refresh(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct On {
    on: bool,
}

async fn set_tool(State(s): State<Server>, Path((id, tool)): Path<(String, String)>, Json(b): Json<On>) -> ApiResult<McpServer> {
    Ok(Json(mcp::set_tool(&s.app, &id, &tool, b.on).await.map_err(bad)?))
}

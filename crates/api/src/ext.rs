//! The Chrome extension's page context on Home (`launchpad_github::ext`): `GET /api/ext/context?url=&workspace=` and
//! `GET /api/ext/rows?url=&items=&workspace=` (items: comma-separated numbers or notification thread IDs). Answered
//! from Home's records and cache.db only. Runners forward them to Home; agents never reach them (`agent_auth`).
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use launchpad_github::ext::{self, ExtContext, ExtRows};
use serde::Deserialize;

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new().route("/api/ext/context", get(context)).route("/api/ext/rows", get(rows))
}

#[derive(Deserialize)]
struct Q {
    url: String,
    /// The extension's current workspace.
    workspace: Option<String>,
    #[serde(default)]
    items: String,
}

fn github_url(url: &str) -> Result<(), ApiError> {
    if url.len() > 2048 || !url.starts_with("https://github.com/") {
        return Err(ApiError::bad_request("not a github.com page"));
    }
    Ok(())
}

async fn context(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<ExtContext> {
    github_url(&q.url)?;
    Ok(Json(ext::context(&s.app, &q.url, q.workspace.as_deref()).await?))
}

async fn rows(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<ExtRows> {
    github_url(&q.url)?;
    let items: Vec<String> = q.items.split(',').map(str::trim).filter(|i| !i.is_empty() && i.len() <= 40 && i.chars().all(|c| c.is_ascii_alphanumeric())).map(str::to_string).collect();
    Ok(Json(ext::rows(&s.app, &q.url, &items, q.workspace.as_deref()).await?))
}

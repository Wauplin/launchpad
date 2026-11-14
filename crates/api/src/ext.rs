//! The Chrome extension's routes on Home (`launchpad_github::ext`): `GET /api/ext/context?url=&workspace=`,
//! `GET /api/ext/rows?url=&items=&workspace=` (items: comma-separated numbers or `owner/repo#number`) and the
//! owner's action `github.review-choice`. Reads come from Launchpad's own records only.
//! Runners forward `/api/ext/*` to Home; agents never reach them (`agent_auth`).
use axum::extract::{Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use launchpad_github::ext::{self, ExtContext, ExtReviewChoice, ExtRows};
use serde::Deserialize;

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/ext/context", get(context))
        .route("/api/ext/rows", get(rows))
        .route("/api/ext/review-choice", put(review_choice))
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

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn context(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<ExtContext> {
    github_url(&q.url)?;
    Ok(Json(ext::context(&s.app, &q.url, q.workspace.as_deref()).await?))
}

/// A row item: a number or `owner/repo#number`.
fn item_ok(i: &str) -> bool {
    !i.is_empty() && i.len() <= 140 && i.chars().all(|c| c.is_ascii_alphanumeric() || "-_./#".contains(c))
}

async fn rows(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<ExtRows> {
    github_url(&q.url)?;
    let items: Vec<String> = q.items.split(',').map(str::trim).filter(|i| item_ok(i)).map(str::to_string).collect();
    Ok(Json(ext::rows(&s.app, &q.url, &items, q.workspace.as_deref()).await?))
}

#[derive(Deserialize)]
struct Choice {
    repo: String,
    workspace: String,
    #[serde(default)]
    bot: Option<String>,
    #[serde(default)]
    preset: Option<String>,
}

/// `github.review-choice`: the bot and preset Review with agent uses on a repository's lists.
async fn review_choice(State(s): State<Ctx>, Json(b): Json<Choice>) -> ApiResult<ExtReviewChoice> {
    let choice = ExtReviewChoice { bot: b.bot.filter(|x| !x.is_empty()), preset: b.preset.filter(|x| !x.is_empty()) };
    Ok(Json(ext::set_review_choice(&s.app, &b.repo, &b.workspace, choice).await.map_err(bad)?))
}

//! The Chrome extension's routes on Home (`launchpad_github::ext`): `GET /api/ext/context?url=&workspace=`,
//! `GET /api/ext/rows?url=&items=&workspace=` (items: comma-separated numbers, notification thread IDs or
//! `owner/repo#number`), `GET /api/ext/notifications?workspace=` (the notifications strip), and the owner's actions
//! `inbox.observed`, `inbox.done-handled` and `github.review-choice`. Reads come from Home's records and cache.db only.
//! Runners forward `/api/ext/*` to Home; agents never reach them (`agent_auth`).
use axum::extract::{Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_github::ext::{self, ExtContext, ExtNotifications, ExtReviewChoice, ExtRows};
use launchpad_github::inbox::ObservedResult;
use serde::{Deserialize, Serialize};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/ext/context", get(context))
        .route("/api/ext/rows", get(rows))
        .route("/api/ext/notifications", get(notifications))
        .route("/api/ext/inbox/observed", post(observed))
        .route("/api/ext/inbox/done-handled", post(done_handled))
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

/// A row item: a number, a thread ID, or `owner/repo#number`.
fn item_ok(i: &str) -> bool {
    !i.is_empty() && i.len() <= 140 && i.chars().all(|c| c.is_ascii_alphanumeric() || "-_./#".contains(c))
}

async fn rows(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<ExtRows> {
    github_url(&q.url)?;
    let items: Vec<String> = q.items.split(',').map(str::trim).filter(|i| item_ok(i)).map(str::to_string).collect();
    Ok(Json(ext::rows(&s.app, &q.url, &items, q.workspace.as_deref()).await?))
}

#[derive(Deserialize)]
struct Ws {
    workspace: Option<String>,
}

async fn notifications(State(s): State<Ctx>, Query(q): Query<Ws>) -> ApiResult<ExtNotifications> {
    Ok(Json(ext::notifications(&s.app, q.workspace.as_deref()).await?))
}

#[derive(Deserialize)]
struct Observed {
    /// Thread IDs or `owner/repo#number`.
    ids: Vec<String>,
    /// `done`, `read` or `unsubscribe`.
    action: String,
}

/// `inbox.observed`: Done, read or Unsubscribe clicked on github.com, recorded in Launchpad's Inbox only.
async fn observed(State(s): State<Ctx>, Json(b): Json<Observed>) -> ApiResult<Vec<ObservedResult>> {
    if !b.ids.iter().all(|i| item_ok(i)) {
        return Err(ApiError::bad_request("ids are thread IDs or owner/repo#number"));
    }
    Ok(Json(launchpad_github::inbox::of(&s.app).observed(&s.app, &b.ids, &b.action).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct DoneHandled {
    /// The threads the confirmation listed.
    ids: Vec<String>,
    workspace: String,
}

#[derive(Serialize)]
struct DoneResult {
    id: String,
    error: Option<String>,
}

/// `inbox.done-handled`: marks Done, here and on GitHub (like the Done button, one thread after the other), the
/// threads the owner confirmed that a bot of the workspace still counts as handled; any other is left alone.
async fn done_handled(State(s): State<Ctx>, Json(b): Json<DoneHandled>) -> ApiResult<Vec<DoneResult>> {
    if b.ids.is_empty() || b.ids.len() > 100 {
        return Err(ApiError::bad_request("give 1 to 100 thread IDs"));
    }
    let handled = ext::notifications(&s.app, Some(&b.workspace)).await?.handled;
    let inbox = launchpad_github::inbox::of(&s.app);
    let mut out = vec![];
    for id in b.ids {
        let error = if !handled.iter().any(|h| h.thread == id) {
            Some("no longer handled by a bot (new activity, or not in the Inbox)".to_string())
        } else {
            inbox.action(&s.app, &id, "done").await.err().map(|e| format!("{e:#}"))
        };
        out.push(DoneResult { id, error });
    }
    Ok(Json(out))
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

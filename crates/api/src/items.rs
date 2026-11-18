//! The owner's ready actions about a pull request on GitHub: `github.run-ci` and `branch.push-to-pr`. Both read the
//! pull request on GitHub at the click. Also an agent's own CI approval (`launchpad ci approve`, `POST
//! /api/ci/approve`), checked by Home itself.
use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/items/{owner}/{repo}/{number}/run-ci", post(run_ci))
        .route("/api/items/{owner}/{repo}/{number}/push-branch", post(push_branch))
        .route("/api/ci/approve", post(agent_ci))
}
type ItemPath = (String, String, u64);
/// `owner/name` from a route's parts, refused unless both are names GitHub allows.
fn repo(o: &str, r: &str) -> Result<String, crate::ApiError> {
    let full = format!("{o}/{r}");
    if launchpad_core::github::valid_repo(&full) { Ok(full) } else { Err(crate::ApiError::bad_request("not a GitHub owner and repository")) }
}
#[derive(Deserialize)]
struct RunCi {
    head: String,
}
async fn run_ci(State(s): State<Ctx>, Path((o, r, n)): Path<ItemPath>, Json(b): Json<RunCi>) -> ApiResult<Value> {
    Ok(Json(launchpad_github::items::approve_runs(&s.app, &repo(&o, &r)?, n, &b.head).await?))
}
#[derive(Deserialize)]
struct PushBranch {
    branch: String,
    head: String,
}
async fn push_branch(State(s): State<Ctx>, Path((o, r, n)): Path<ItemPath>, Json(b): Json<PushBranch>) -> ApiResult<Value> {
    Ok(Json(launchpad_github::ext::push_to_pr(&s.app, &repo(&o, &r)?, n, &b.branch, &b.head).await?))
}
#[derive(Deserialize)]
struct AgentCi {
    repo: String,
    number: u64,
    head: String,
    /// The calling session (`agent_auth` checks it is the CLI token's own).
    session_id: String,
}
/// `launchpad ci approve`: an agent (propose or admin Launchpad CLI access) approves the GitHub Actions runs of a
/// pull request that wait for a maintainer, without the owner's click. Home refuses a repository of another
/// workspace's folders, a head that moved and a pull request that changes CI configuration
/// (`items::agent_approve_runs`); each approval becomes a private note on the item and an activity line.
async fn agent_ci(State(s): State<Ctx>, Json(b): Json<AgentCi>) -> ApiResult<Value> {
    let app = &s.app;
    launchpad_github::items::validate(&b.repo)?;
    let Some(ws) = launchpad_core::kb::workspace_of_session(app, &b.session_id).await else {
        return Err(crate::ApiError::new(axum::http::StatusCode::FORBIDDEN, "this session has no workspace: only the owner approves workflows here"));
    };
    if launchpad_core::scope::repo_outside(app, &ws, &b.repo).await {
        return Err(crate::ApiError::new(axum::http::StatusCode::FORBIDDEN, format!("{} belongs to another workspace's folders", b.repo)));
    }
    let head = b.head.to_ascii_lowercase();
    let result = launchpad_github::items::agent_approve_runs(app, &b.repo, b.number, &head).await.map_err(|e| crate::ApiError::new(axum::http::StatusCode::CONFLICT, format!("{e:#}")))?;
    let approved = result["approved"].as_u64().unwrap_or(0);
    let short = &head[..12];
    let body = format!("Approved {approved} GitHub Actions {} waiting for a maintainer at `{short}` (`launchpad ci approve`). Home checked that the head had not moved and that the pull request changes no workflow, action or other CI configuration.", if approved == 1 { "run" } else { "runs" });
    if let Err(e) = launchpad_github::notes::record(app, &b.session_id, &b.repo, b.number, body).await {
        tracing::warn!("CI approval of {}#{}: note: {e:#}", b.repo, b.number);
    }
    let target = format!("{}#{}", b.repo, b.number);
    if let Err(e) = launchpad_automation::activity::record_github(app, &b.session_id, "github.approve-ci", target, json!({"head": head, "approved": approved})).await {
        tracing::warn!("CI approval of {}#{}: activity: {e:#}", b.repo, b.number);
    }
    Ok(Json(result))
}

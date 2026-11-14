//! The owner's ready actions about a pull request on GitHub: `github.run-ci` and `branch.push-to-pr`. Both read the
//! pull request on GitHub at the click.
use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::Deserialize;
use serde_json::Value;
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/items/{owner}/{repo}/{number}/run-ci", post(run_ci))
        .route("/api/items/{owner}/{repo}/{number}/push-branch", post(push_branch))
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

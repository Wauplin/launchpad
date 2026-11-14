use crate::{
    Ctx,
    error::{ApiError, ApiResult},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use launchpad_core::{
    model::HarnessKind,
    prompt_context::{self, PathResults, SkillResults},
};
use serde::Deserialize;

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/prompt/files", get(files))
        .route("/api/prompt/skills", get(skills))
}
#[derive(Deserialize)]
struct Search {
    root: Option<String>,
    workspace: Option<String>,
    #[serde(default)]
    q: String,
}
async fn files(State(s): State<Ctx>, Query(q): Query<Search>) -> ApiResult<PathResults> {
    Ok(Json(
        prompt_context::paths(&s.app, q.root.as_deref(), &q.q, q.workspace.as_deref())
            .await
            .map_err(|e| ApiError::bad_request(format!("{e:#}")))?,
    ))
}
#[derive(Deserialize)]
struct Skills {
    harness: HarnessKind,
    profile: Option<String>,
    root: Option<String>,
}
async fn skills(State(s): State<Ctx>, Query(q): Query<Skills>) -> ApiResult<SkillResults> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        prompt_context::skills(&s.app, q.harness, q.profile.as_deref(), q.root.as_deref()),
    )
    .await
    .map_err(|_| ApiError::bad_request("Skill discovery timed out"))?;
    Ok(Json(
        result.map_err(|e| ApiError::bad_request(format!("{e:#}")))?,
    ))
}


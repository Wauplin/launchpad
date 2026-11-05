//! `GET /api/today`: the decision list Home computes (`launchpad_core::today`). Runners forward it to Home
//! (`home_client::home_owned`). `?workspace=<id>` keeps one workspace's items.
use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use launchpad_core::today::{self, Today};
use serde::Deserialize;

pub fn router() -> Router<Ctx> {
    Router::new().route("/api/today", get(list))
}

#[derive(Deserialize)]
struct Q {
    workspace: Option<String>,
}

async fn list(State(s): State<Ctx>, Query(q): Query<Q>) -> ApiResult<Today> {
    let mut t = today::compute(&s.app).await?;
    if let Some(w) = crate::activity::workspace_id(&s, q.workspace).await? {
        let keep = |i: &today::Item| i.workspaces.is_empty() || i.workspaces.contains(&w);
        t.items.retain(keep);
        t.done.retain(keep);
    }
    Ok(Json(t))
}

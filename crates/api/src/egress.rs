//! Exit routes (`launchpad_core::egress`, `[[egress]]` in config.toml): the owner's list and its states
//! (`GET`/`PUT /api/egress`, `egress.set`), and the states alone for agents and oracles (`GET /api/context/egress`, every
//! Launchpad CLI level, Launchpad-wide).

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use launchpad_core::egress::{self, ExitRoute};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new().route("/api/egress", get(list).put(set)).route("/api/context/egress", get(states))
}

/// The routes as configured (the body key of `egress.set`, so its undo puts them back) and their states.
fn answer(s: &Ctx) -> Value {
    json!({"routes": s.app.config.get().egress, "states": egress::states()})
}

async fn list(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(answer(&s)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Set {
    routes: Vec<ExitRoute>,
}

async fn set(State(s): State<Ctx>, Json(body): Json<Set>) -> ApiResult<Value> {
    let mut routes = body.routes;
    for r in &mut routes {
        r.name = r.name.trim().to_string();
        r.via = r.via.trim().to_string();
        r.github_org = r.github_org.take().map(|o| o.trim().to_string()).filter(|o| !o.is_empty());
        r.urls = r.urls.iter().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect();
    }
    egress::validate(&routes).map_err(ApiError::bad_request)?;
    s.app.config.update(|c| {
        c.egress = routes;
        Ok(())
    }).map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    egress::refresh(&s.app).await;
    Ok(Json(answer(&s)))
}

/// The routes with their exits' states, without anything secret (none is).
async fn states() -> Json<Vec<egress::RouteState>> {
    Json(egress::states())
}

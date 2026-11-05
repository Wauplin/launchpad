//! Burst on Home: dialing the runners of Burst jobs, handoffs, session moves and the Burst settings.
//!
//! - `/api/handoff/{id}` (runner credential): a runner sends the handoff of a session it runs (POST), and the
//!   new runner of a moved session fetches it (GET).
//! - `/api/sessions/{id}/handoff`, `/api/sessions/{id}/adopt` (runner, from Home through the tunnel).
//! - `/api/sessions/{id}/move` (Home, owner): Move to another runner or to Burst.
//! - `/api/burst` (Home, owner): settings, this month's cost and recent runs; Stop.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use launchpad_core::role::Role;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/handoff/{id}", get(adoption).post(store))
        .route("/api/sessions/{id}/handoff", post(handoff))
        .route("/api/sessions/{id}/adopt", post(adopt))
        .route("/api/sessions/{id}/move", post(move_session))
        .route("/api/burst", get(overview))
        .route("/api/burst/config", put(set_config))
        .route("/api/burst/{runner}/stop", post(stop))
}

/// Paths that authenticate with the runner credential (checked here).
pub fn runner_path(path: &str) -> bool {
    path.strip_prefix("/api/handoff/").is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn runner(s: &Server, headers: &HeaderMap) -> Result<launchpad_core::runners::Runner, ApiError> {
    let credential = headers.get(crate::runners::CREDENTIAL_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    launchpad_core::runners::authenticate(&s.app, credential)
        .await
        .filter(|r| r.state == launchpad_core::runners::RunnerState::Approved)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"))
}

async fn store(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap, Json(h): Json<launchpad_core::handoff::Handoff>) -> ApiResult<Value> {
    let r = runner(&s, &headers).await?;
    if h.session_id != id {
        return Err(ApiError::bad_request("the handoff names another session"));
    }
    launchpad_core::handoff::store(&s.app, &r.id, h).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn adoption(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<launchpad_core::handoff::Adoption> {
    let r = runner(&s, &headers).await?;
    Ok(Json(launchpad_core::handoff::adoption(&s.app, &r.id, &id).await.map_err(bad)?))
}

#[derive(Deserialize, Default)]
struct HandoffBody {
    #[serde(default, rename = "final")]
    is_final: bool,
}

/// Runner: Home asks for the session's handoff (a planned move or a finished Burst session).
async fn handoff(State(s): State<Server>, Path(id): Path<String>, body: Option<Json<HandoffBody>>) -> ApiResult<Value> {
    if s.app.paths.role != Role::Runner {
        return Err(ApiError::bad_request("only runners hand sessions off"));
    }
    let is_final = body.map(|b| b.0.is_final).unwrap_or_default();
    launchpad_core::handoff::upload(&s.app, &id, is_final).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// Runner: takes over a session Home moved here.
async fn adopt(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    launchpad_core::handoff::adopt(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct MoveBody {
    /// A runner ID, or `burst`.
    to: String,
}

async fn move_session(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<MoveBody>) -> ApiResult<Value> {
    if s.app.paths.role != Role::Home {
        return Err(ApiError::bad_request("sessions move on Home"));
    }
    if b.to == launchpad_core::burst::CHOICE {
        launchpad_core::burst::move_session(&s.app, &id).await.map_err(bad)?;
    } else {
        launchpad_core::handoff::move_to(&s.app, &id, &b.to).await.map_err(bad)?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn overview(State(s): State<Server>) -> ApiResult<launchpad_core::burst::BurstOverview> {
    Ok(Json(launchpad_core::burst::overview(&s.app).await?))
}

async fn set_config(State(s): State<Server>, Json(c): Json<launchpad_core::config::BurstConfig>) -> ApiResult<launchpad_core::burst::BurstOverview> {
    if s.app.paths.role != Role::Home {
        return Err(ApiError::bad_request("Burst is a setting of Home"));
    }
    if !(c.monthly_cap_usd.is_finite() && c.monthly_cap_usd >= 0.0) || c.flavor.trim().is_empty() {
        return Err(ApiError::bad_request("give a monthly cap of 0 or more and a hardware flavor"));
    }
    s.app.config.update(|config| {
        config.home.burst = c;
        Ok(())
    })?;
    overview(State(s)).await
}

async fn stop(State(s): State<Server>, Path(runner): Path<String>) -> ApiResult<launchpad_core::burst::BurstOverview> {
    launchpad_core::burst::stop(&s.app, &runner, None).await.map_err(bad)?;
    overview(State(s)).await
}

/// Runners Home is dialing now.
static DIALING: std::sync::LazyLock<Mutex<HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// Home dials every active Burst job's runner, and again after a disconnection, while the job runs.
pub fn spawn(s: Server) {
    if s.app.paths.role != Role::Home {
        return;
    }
    tokio::spawn(async move {
        loop {
            for (id, url) in launchpad_core::burst::to_dial(&s.app).await {
                if s.app.runners.connected(&id) || !DIALING.lock().unwrap().insert(id.clone()) {
                    continue;
                }
                let s = s.clone();
                tokio::spawn(async move {
                    if let Err(e) = dial(&s, &id, &url).await {
                        tracing::debug!("Burst {id}: {e:#}");
                    }
                    DIALING.lock().unwrap().remove(&id);
                });
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn dial(s: &Server, id: &str, url: &str) -> anyhow::Result<()> {
    let (credential, token) = launchpad_core::burst::dial_auth(&s.app, id).await?;
    let base = url.trim_end_matches('/').replacen("https://", "wss://", 1);
    let mut request = format!("{base}/api/runner/reverse").into_client_request()?;
    request.headers_mut().insert(crate::runners::CREDENTIAL_HEADER, credential.parse()?);
    request.headers_mut().insert(axum::http::header::AUTHORIZATION, format!("Bearer {token}").parse()?);
    let (socket, _) = tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(request)).await??;
    let runner = launchpad_core::runners::authenticate(&s.app, &credential).await.ok_or_else(|| anyhow::anyhow!("the Burst runner record is gone"))?;
    let (tx, rx) = crate::wire::tungstenite(socket);
    crate::runners::serve(s.clone(), tx, rx, runner).await;
    Ok(())
}

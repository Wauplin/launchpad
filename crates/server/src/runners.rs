//! Home's side of runners: pairing requests, approval, and the tunnel each runner keeps open.
//!
//! `/api/runners/request*` and `/api/runners/tunnel` skip the browser sign-in: they check the runner's own
//! credential (`X-Launchpad-Runner: <id>.<secret>`). Everything else here needs the owner.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use launchpad_core::role::{self, Compat};
use launchpad_core::runners::{self, RunnerState, RunnerStatus, proto::{FromRunner, ToRunner}};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};
use crate::Server;

pub const CREDENTIAL_HEADER: &str = "x-launchpad-runner";

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/runners", get(list))
        .route("/api/runners/request", post(request))
        .route("/api/runners/request/{id}", get(request_state))
        .route("/api/runners/tunnel", get(tunnel))
        .route("/api/runners/{id}/approve", post(approve))
        .route("/api/runners/{id}/status", get(runner_status))
        .route("/api/runners/{id}", axum::routing::delete(remove))
        .route("/api/runner/guard", get(guard).put(set_guard))
}

/// The runner guard of this machine (runner role): local settings, never synced to Home.
async fn guard(State(s): State<Server>) -> Json<runners::Guard> {
    Json(launchpad_core::home_link::guard(&s.app.paths))
}

async fn set_guard(State(s): State<Server>, Json(g): Json<runners::Guard>) -> ApiResult<runners::Guard> {
    launchpad_core::home_link::set_guard(&s.app, &g)?;
    Ok(Json(g))
}

/// Paths that authenticate with the runner credential instead of the browser sign-in.
pub fn runner_path(path: &str) -> bool {
    path == "/api/runners/request" || path == "/api/runners/tunnel" || path.strip_prefix("/api/runners/request/").is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> ApiResult<Vec<RunnerStatus>> {
    Ok(Json(runners::list(&s.app).await?))
}

#[derive(Deserialize)]
struct Request {
    name: String,
    #[serde(default)]
    os: String,
}

async fn request(State(s): State<Server>, Json(b): Json<Request>) -> ApiResult<Value> {
    let (r, secret) = runners::request(&s.app, &b.name, &b.os).await.map_err(bad)?;
    Ok(Json(json!({"id": r.id, "code": r.code, "secret": secret})))
}

async fn runner_of(s: &Server, headers: &HeaderMap) -> Result<runners::Runner, ApiError> {
    let credential = headers.get(CREDENTIAL_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    runners::authenticate(&s.app, credential).await.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner: run launchpad connect again"))
}

async fn request_state(State(s): State<Server>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Value> {
    let r = runner_of(&s, &headers).await?;
    if r.id != id {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"));
    }
    Ok(Json(json!({"state": r.state})))
}

async fn approve(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<runners::Runner> {
    Ok(Json(runners::approve(&s.app, &id).await.map_err(bad)?))
}

/// The runner's own status, through its tunnel.
async fn runner_status(State(s): State<Server>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (status, body) = s.app.runners.request(&id, "GET", "/api/status", None).await.map_err(bad)?;
    Ok((StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response())
}

async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    runners::remove(&s.app, &id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn tunnel(State(s): State<Server>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let runner = match runner_of(&s, &headers).await {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    ws.on_upgrade(move |socket| serve(s, socket, runner))
}

const PING: std::time::Duration = std::time::Duration::from_secs(25);

async fn send(tx: &mut futures_util::stream::SplitSink<WebSocket, Message>, msg: &ToRunner) -> bool {
    tx.send(Message::Text(serde_json::to_string(msg).unwrap_or_default().into())).await.is_ok()
}

async fn serve(s: Server, socket: WebSocket, runner: runners::Runner) {
    let (mut tx, mut rx) = socket.split();
    // The first message must be Hello.
    let hello = match tokio::time::timeout(std::time::Duration::from_secs(20), rx.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<FromRunner>(&t).ok(),
        _ => None,
    };
    let Some(FromRunner::Hello { protocol, commit, guard, .. }) = hello else { return };
    let refuse = if runner.state != RunnerState::Approved {
        Some("waiting for approval on Home (Settings → Home and runners)".to_string())
    } else if role::compat(role::PROTOCOL, launchpad_core::COMMIT, protocol, &commit) == Compat::Incompatible {
        Some(format!("this runner speaks protocol {protocol}, Home speaks {}: update this runner", role::PROTOCOL))
    } else {
        None
    };
    if let Some(reason) = refuse {
        let _ = send(&mut tx, &ToRunner::Refuse { reason }).await;
        return;
    }
    let compat = role::compat(role::PROTOCOL, launchpad_core::COMMIT, protocol, &commit);
    let id = runner.id.clone();
    let runner = match runners::seen(&s.app, runner, &commit, protocol).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("runner {id}: {e:#}");
            return;
        }
    };
    let acked = match launchpad_core::replica::acked_for(&s.app, &runner.id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("runner {}: {e:#}", runner.id);
            return;
        }
    };
    if !send(&mut tx, &ToRunner::Welcome { compat, commit: launchpad_core::COMMIT.into(), acked }).await {
        return;
    }
    tracing::info!("runner {} ({}) connected, {compat:?}", runner.name, runner.id);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<ToRunner>();
    let conn = s.app.runners.connect(&s.app, &runner.id, compat, guard, out_tx);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            msg = out_rx.recv() => {
                let Some(msg) = msg else { break };
                if !send(&mut tx, &msg).await { break }
            }
            _ = ping.tick() => {
                // A silent runner (sleeping laptop, dead network) is dropped after three missed pings.
                if last_heard.elapsed() > PING * 3 || tx.send(Message::Ping(Vec::new().into())).await.is_err() { break }
            }
            frame = rx.next() => {
                let Some(Ok(frame)) = frame else { break };
                last_heard = tokio::time::Instant::now();
                let Message::Text(text) = frame else { continue };
                match serde_json::from_str::<FromRunner>(&text) {
                    Ok(FromRunner::Guard { guard }) => s.app.runners.set_guard(&s.app, &runner.id, guard),
                    Ok(FromRunner::Res { id, status, body }) => s.app.runners.answered(id, status, body),
                    Ok(FromRunner::Items { items }) => {
                        let mut last = None;
                        for (n, item) in items {
                            if let Err(e) = launchpad_core::replica::ingest(&s.app, &runner.id, n, item).await {
                                // Not acknowledged: the runner sends it again after reconnecting.
                                tracing::warn!("runner {}: could not store item {n}: {e:#}", runner.id);
                                break;
                            }
                            last = Some(n);
                        }
                        if let Some(n) = last && !send(&mut tx, &ToRunner::Ack { n }).await { break }
                    }
                    Ok(FromRunner::Ping) => { if !send(&mut tx, &ToRunner::Pong).await { break } }
                    Ok(FromRunner::Hello { .. }) => {}
                    Err(e) => tracing::warn!("runner {}: bad message: {e}", runner.id),
                }
            }
        }
    }
    s.app.runners.disconnected(&s.app, &runner.id, conn);
    tracing::info!("runner {} disconnected", runner.name);
}

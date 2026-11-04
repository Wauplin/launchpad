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
        .route("/api/runner/launches", get(launches))
        .route("/api/home/backup", get(backup_info).post(backup_now))
}

async fn backup_info(State(s): State<Server>) -> Json<launchpad_core::home_backup::BackupInfo> {
    let mut info = s.app.home_backups.info();
    info.destination = Some(s.app.config.get().home.backup).filter(|d| !d.trim().is_empty());
    Json(info)
}

async fn backup_now(State(s): State<Server>) -> ApiResult<launchpad_core::home_backup::BackupInfo> {
    if s.app.paths.role != role::Role::Home {
        return Err(ApiError::bad_request("only Home backs up its database"));
    }
    launchpad_core::home_backup::run(&s.app, true).await.map_err(bad)?;
    Ok(backup_info(State(s)).await)
}

/// Launches that came from other devices through Home, newest first.
async fn launches(State(s): State<Server>) -> Json<Vec<launchpad_core::home_link::RemoteLaunch>> {
    let mut all = launchpad_core::home_link::launches(&s.app.paths);
    all.reverse();
    Json(all)
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

async fn workspaces(s: &Server) -> ToRunner {
    ToRunner::Workspaces { workspaces: launchpad_core::workspace::list(&s.app).await.unwrap_or_default() }
}

async fn folders(s: &Server) -> Option<ToRunner> {
    launchpad_core::projects::home_folders(&s.app).await.ok().map(|projects| ToRunner::Folders { projects })
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
    let mut bus = s.app.bus.subscribe(None).rx;
    if !send(&mut tx, &workspaces(&s).await).await {
        return;
    }
    if let Some(f) = folders(&s).await && !send(&mut tx, &f).await {
        return;
    }
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
            delivered = bus.recv() => {
                // Workspaces and Git folders are Home's: every runner gets each new snapshot.
                let (w, f) = match delivered {
                    Ok(d) => (d.text.contains(r#""topic":"workspaces""#), d.text.contains(r#""topic":"projects""#)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => (true, true),
                    Err(_) => (false, false),
                };
                if w && !send(&mut tx, &workspaces(&s).await).await { break }
                if f && let Some(f) = folders(&s).await && !send(&mut tx, &f).await { break }
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
                    Ok(FromRunner::Req { id, method, path, body }) => {
                        let (app, out) = (s.app.clone(), s.app.runners.clone());
                        let runner_id = runner.id.clone();
                        tokio::spawn(async move {
                            let res = match crate::ROUTER.get() {
                                Some(router) if crate::home_client::home_accepts(&path) => crate::home_client::local(&app, router, &method, &path, body, &[]).await,
                                _ => Err(anyhow::anyhow!("runners may only send requests about records Home owns")),
                            };
                            let (status, body) = res.unwrap_or_else(|e| (400, json!({"error": format!("{e:#}")}).to_string()));
                            out.send_to(&runner_id, ToRunner::Res { id, status, body });
                        });
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

/// Home: requests about a runner's session or folder go to that runner, and launches go to a runner. Home
/// serves records, events and traces itself.
pub async fn relay(State(s): State<Server>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if s.app.paths.role != role::Role::Home {
        return next.run(req).await;
    }
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let get = method == axum::http::Method::GET;
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let runner = match parts.as_slice() {
        ["api", "sessions"] if method == axum::http::Method::POST => None,
        ["api", "sessions", id, rest @ ..] if *id != "search" => {
            let served = get && matches!(rest, [] | ["events"] | ["events", "page"] | ["trace"] | ["images", ..]);
            match s.app.sessions.get(id).and_then(|x| x.runner_id) {
                Some(r) if !served => Some(r),
                _ => return next.run(req).await,
            }
        }
        ["api", "projects", id, rest @ ..] if !get || rest == ["branches"] => match launchpad_core::projects::get(&s.app, id).await.ok().flatten().and_then(|p| p.runner_id) {
            Some(r) => Some(r),
            None => return next.run(req).await,
        },
        _ => return next.run(req).await,
    };
    let headers: Vec<(String, String)> = req.headers().get("idempotency-key").and_then(|v| v.to_str().ok()).map(|v| vec![("idempotency-key".to_string(), v.to_string())]).unwrap_or_default();
    let path_and_query = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or(path);
    let body = match axum::body::to_bytes(req.into_body(), 32 << 20).await {
        Ok(b) => (!b.is_empty()).then(|| String::from_utf8_lossy(&b).into_owned()),
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };
    let runner = match runner {
        Some(r) => r,
        None => match launch_runner(&s, body.as_deref()).await {
            Ok(r) => r,
            Err(e) => return e.into_response(),
        },
    };
    if !s.app.runners.connected(&runner) {
        let name = runners::list(&s.app).await.ok().and_then(|l| l.into_iter().find(|r| r.runner.id == runner)).map(|r| r.runner.name).unwrap_or_else(|| "its runner".into());
        return ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} is offline")).into_response();
    }
    match s.app.runners.request_with(&runner, method.as_str(), &path_and_query, body, headers).await {
        Ok((status, body)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => ApiError::new(StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}

/// The runner for a new session: the one whose disk has its folder, else the first that takes launches from
/// other devices.
async fn launch_runner(s: &Server, body: Option<&str>) -> Result<String, ApiError> {
    let spec: launchpad_core::model::LaunchSpec = serde_json::from_str(body.unwrap_or("{}")).map_err(|e| ApiError::bad_request(format!("{e}")))?;
    let project = match &spec.target {
        launchpad_core::model::Target::Project { project_id } | launchpad_core::model::Target::Predraft { project_id } => Some(project_id.clone()),
        _ => None,
    };
    let available = s.app.runners.available();
    if let Some(project) = project {
        let p = launchpad_core::projects::get(&s.app, &project).await?.ok_or_else(|| ApiError::bad_request("no such folder"))?;
        // A Git folder of Home runs on any runner: it clones the folder on first use.
        if p.home_git || p.plain {
            return available.into_iter().next().ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no runner takes launches now: connect one, or check its guard in Settings → Home and runners"));
        }
        let r = p.runner_id.ok_or_else(|| ApiError::bad_request(format!("{} is not on a runner", p.name)))?;
        if !available.contains(&r) {
            let all = runners::list(&s.app).await?;
            let name = all.iter().find(|x| x.runner.id == r).map(|x| x.runner.name.clone()).unwrap_or_else(|| "Its runner".into());
            return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} does not take this launch now: it is offline, paused, refuses launches from other devices or needs an update")));
        }
        return Ok(r);
    }
    available.into_iter().next().ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no runner takes launches now: connect one, or check its guard in Settings → Home and runners"))
}

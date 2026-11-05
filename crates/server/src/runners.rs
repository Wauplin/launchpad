//! Home's side of runners: pairing requests, approval, and the tunnel each runner keeps open.
//!
//! `/api/runners/request*` and `/api/runners/tunnel` skip the browser sign-in: they check the runner's own
//! credential (`X-Launchpad-Runner: <id>.<secret>`). Everything else here needs the owner.

use axum::extract::ws::WebSocketUpgrade;
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
        .route("/api/runner/reverse", get(crate::home_client::reverse))
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
    ws.on_upgrade(move |socket| {
        let (tx, rx) = crate::wire::axum(socket);
        serve(s, tx, rx, runner)
    })
}

async fn workspaces(s: &Server) -> ToRunner {
    ToRunner::Workspaces { workspaces: launchpad_core::workspace::list(&s.app).await.unwrap_or_default() }
}

async fn folders(s: &Server) -> Option<ToRunner> {
    launchpad_core::projects::home_folders(&s.app).await.ok().map(|projects| ToRunner::Folders { projects })
}

fn profiles(s: &Server) -> ToRunner {
    ToRunner::Profiles { profiles: s.app.config.get().profiles }
}

const PING: std::time::Duration = std::time::Duration::from_secs(25);

async fn send(tx: &mut crate::wire::WireTx, msg: &ToRunner) -> bool {
    tx.send(serde_json::to_string(msg).unwrap_or_default()).await.is_ok()
}

/// The protocol with one runner, over a connection either side dialed.
pub(crate) async fn serve(s: Server, mut tx: crate::wire::WireTx, mut rx: crate::wire::WireRx, runner: runners::Runner) {
    // The first message must be Hello.
    let hello = match tokio::time::timeout(std::time::Duration::from_secs(20), rx.next()).await {
        Ok(Some(Ok(t))) => serde_json::from_str::<FromRunner>(&t).ok(),
        _ => None,
    };
    let Some(FromRunner::Hello { protocol, commit, guard, capabilities, .. }) = hello else { return };
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
    let runner = match runners::seen(&s.app, runner, &commit, protocol, capabilities).await {
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
    if !send(&mut tx, &profiles(&s)).await { return; }
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<ToRunner>();
    let conn = s.app.runners.connect(&s.app, &runner.id, compat, guard, runner.burst, out_tx);
    if runner.burst {
        launchpad_core::burst::connected(&s.app, &runner.id).await;
    }
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    let mut last_heard = tokio::time::Instant::now();
    // Request bodies of HTTP exchanges in progress.
    let mut bodies: std::collections::HashMap<u64, tokio::sync::mpsc::UnboundedSender<bytes::Bytes>> = Default::default();
    loop {
        tokio::select! {
            msg = out_rx.recv() => {
                let Some(msg) = msg else { break };
                if !send(&mut tx, &msg).await { break }
            }
            delivered = bus.recv() => {
                // Workspaces and Git folders are Home's: every runner gets each new snapshot.
                let (w, f, p) = match delivered {
                    Ok(d) => (d.text.contains(r#""topic":"workspaces""#), d.text.contains(r#""topic":"projects""#), d.text.contains(r#""topic":"config""#)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => (true, true, true),
                    Err(_) => (false, false, false),
                };
                if p && !send(&mut tx, &profiles(&s)).await { break }
                if w && !send(&mut tx, &workspaces(&s).await).await { break }
                if f && let Some(f) = folders(&s).await && !send(&mut tx, &f).await { break }
            }
            _ = ping.tick() => {
                // A silent runner (sleeping laptop, dead network) is dropped after three missed pings.
                if last_heard.elapsed() > PING * 3 { break }
            }
            frame = rx.next() => {
                let Some(Ok(text)) = frame else { break };
                last_heard = tokio::time::Instant::now();
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
                                Some(router) if crate::home_client::home_accepts(path.split('?').next().unwrap_or_default()) => crate::home_client::local(&app, router, &method, &path, body, &[]).await,
                                _ => Err(anyhow::anyhow!("runners may only send requests about records Home owns")),
                            };
                            let (status, body) = res.unwrap_or_else(|e| (400, json!({"error": format!("{e:#}")}).to_string()));
                            out.send_to(&runner_id, ToRunner::Res { id, status, body });
                        });
                    }
                    Ok(FromRunner::Call { id, op, body }) => {
                        let (app, out) = (s.app.clone(), s.app.runners.clone());
                        let runner_id = runner.id.clone();
                        tokio::spawn(async move {
                            let (status, body) = match launchpad_core::vault::runner_call(&app, Some(&runner_id), &op, body).await {
                                Ok(v) => (200, v.to_string()),
                                Err(e) => (400, json!({"error": format!("{e:#}")}).to_string()),
                            };
                            out.send_to(&runner_id, ToRunner::Res { id, status, body });
                        });
                    }
                    Ok(FromRunner::HttpOpen { id, target, method, path, headers }) => {
                        let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
                        bodies.insert(id, body_tx);
                        tokio::spawn(crate::exchange::run(s.clone(), runner.id.clone(), id, target, method, path, headers, body_rx));
                    }
                    Ok(FromRunner::HttpBody { id, data, end }) => {
                        if let Some(b) = bodies.get(&id) && !data.is_empty() {
                            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data) {
                                Ok(bytes) => { let _ = b.send(bytes.into()); }
                                Err(_) => { bodies.remove(&id); }
                            }
                        }
                        if end { bodies.remove(&id); }
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
            let served = (get && matches!(rest, [] | ["events"] | ["events", "page"] | ["trace"] | ["images", ..] | ["access"])) || rest == ["move"];
            match s.app.sessions.get(id).and_then(|x| x.runner_id) {
                Some(r) if !served => Some(r),
                _ => return next.run(req).await,
            }
        }
        ["api", "projects", id, rest @ ..] if !get || rest == ["branches"] => match launchpad_core::projects::get(&s.app, id).await.ok().flatten().and_then(|p| p.runner_id) {
            Some(r) => Some(r),
            None => return next.run(req).await,
        },
        // Task worktrees live on the runner of their session: the window names it (`?session=`). This covers
        // the worktree actions and a Local PR created from a worktree folder (`cwd`).
        ["api", "worktrees", _, _] | ["api", "predrafts"] if !get => match session_param(req.uri()).and_then(|id| s.app.sessions.get(&id)).and_then(|x| x.runner_id) {
            Some(r) => Some(r),
            None => return next.run(req).await,
        },
        _ => return next.run(req).await,
    };
    let headers: Vec<(String, String)> = req.headers().get("idempotency-key").and_then(|v| v.to_str().ok()).map(|v| vec![("idempotency-key".to_string(), v.to_string())]).unwrap_or_default();
    let path_and_query = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_else(|| path.clone());
    let body = match axum::body::to_bytes(req.into_body(), 32 << 20).await {
        Ok(b) => (!b.is_empty()).then(|| String::from_utf8_lossy(&b).into_owned()),
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };
    let runner = match runner {
        Some(r) => r,
        // A launch: a runner that takes it, else Burst (`runners::launch_anywhere`).
        None => return launch(&s, body.as_deref(), headers).await,
    };
    // A session whose Burst job stopped: a message starts a new job, Archive is Home's.
    if !s.app.runners.connected(&runner)
        && let ["api", "sessions", id, rest @ ..] = parts.as_slice()
        && let Some(session) = s.app.sessions.get(id)
        && launchpad_core::burst::needs_new_job(&s.app, &session).await
    {
        return burst_stopped(&s, id, rest, body.as_deref(), headers).await;
    }
    if !s.app.runners.connected(&runner) {
        let name = runners::list(&s.app).await.ok().and_then(|l| l.into_iter().find(|r| r.runner.id == runner)).map(|r| r.runner.name).unwrap_or_else(|| "its runner".into());
        return ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} is offline")).into_response();
    }
    match s.app.runners.request_with(&runner, method.as_str(), &path_and_query, body, headers).await {
        Ok((status, body)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => ApiError::new(StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}

/// The `session` query parameter of a request.
fn session_param(uri: &axum::http::Uri) -> Option<String> {
    let q = axum::extract::Query::<std::collections::HashMap<String, String>>::try_from_uri(uri).ok()?;
    q.0.get("session").filter(|s| !s.is_empty()).cloned()
}

/// Home: the answers of every connected runner to a task worktree list (`/api/worktrees`,
/// `/api/worktrees/sizes`). Windows show which runners are offline from `/api/runners`.
pub(crate) async fn ask_connected(s: &Server, method: &str, path: &str) -> Vec<Value> {
    let all = runners::list(&s.app).await.unwrap_or_default();
    let asks = all.iter().filter(|r| r.connected).map(|r| s.app.runners.request(&r.runner.id, method, path, None));
    futures_util::future::join_all(asks).await.into_iter().filter_map(|a| a.ok()).filter(|(status, _)| *status == 200).filter_map(|(_, body)| serde_json::from_str(&body).ok()).collect()
}

/// A launch on Home: sent once to a runner that takes it, or started on Burst.
async fn launch(s: &Server, body: Option<&str>, headers: Vec<(String, String)>) -> Response {
    let spec: launchpad_core::model::LaunchSpec = match serde_json::from_str(body.unwrap_or("{}")) {
        Ok(spec) => spec,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };
    let id = headers.into_iter().find(|(k, _)| k == "idempotency-key").map(|(_, v)| v).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    match runners::launch_anywhere(&s.app, spec, id, false).await {
        Ok(session) => Json(session).into_response(),
        Err(e) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")).into_response(),
    }
}

/// Requests for a session whose Burst job has stopped.
async fn burst_stopped(s: &Server, id: &str, rest: &[&str], body: Option<&str>, headers: Vec<(String, String)>) -> Response {
    #[derive(Deserialize)]
    struct Input {
        text: String,
        #[serde(default)]
        files: Vec<launchpad_core::model::FileAttachment>,
    }
    let result = match rest {
        ["input"] => match serde_json::from_str::<Input>(body.unwrap_or("{}")) {
            Ok(i) => {
                let key = headers.into_iter().find(|(k, _)| k == "idempotency-key").map(|(_, v)| v);
                runners::send_anywhere(&s.app, id, &i.text, i.files, key).await
            }
            Err(e) => Err(anyhow::anyhow!("{e}")),
        },
        [action] => runners::session_action(&s.app, id, action).await,
        _ => Err(anyhow::anyhow!("This session's Burst runner has stopped: send a message to continue it")),
    };
    match result {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => ApiError::bad_request(format!("{e:#}")).into_response(),
    }
}

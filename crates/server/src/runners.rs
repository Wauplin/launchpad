//! Home's side of runners: pairing requests, approval, and the tunnel each runner keeps open.
//!
//! `/api/runners/request*`, `/api/runners/tunnel` and `/api/runners/build` skip the browser sign-in: they check the
//! runner's own credential (`X-Launchpad-Runner: <id>.<secret>`). Everything else here needs the owner.

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
        .route("/api/runners/{id}/update", post(update_runner).delete(cancel_runner_update))
        .route("/api/runners/{id}/network-check", post(check_runner_network))
        .route("/api/runner/network-check", post(check_network))
        .route("/api/runners/build", get(build))
        .route("/api/runner/update", post(update_self).delete(cancel_update))
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
    path == "/api/runners/request" || path == "/api/runners/tunnel" || path == "/api/runners/build" || path.strip_prefix("/api/runners/request/").is_some_and(|id| !id.is_empty() && !id.contains('/'))
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

/// Home: runs the network check on a connected runner and keeps the result for its card.
async fn check_runner_network(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<launchpad_core::netcap::NetworkCheck> {
    let r = runners::list(&s.app).await.map_err(bad)?.into_iter().find(|r| r.runner.id == id).ok_or_else(|| ApiError::not_found("no such runner"))?;
    if !r.connected {
        return Err(ApiError::bad_request(format!("{} is offline: start it, then check it", r.runner.name)));
    }
    let (status, body) = s.app.runners.request(&id, "POST", "/api/runner/network-check", None).await.map_err(bad)?;
    if !(200..300).contains(&status) {
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        return Err(ApiError::bad_request(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{} answered {status}", r.runner.name))));
    }
    let check: launchpad_core::netcap::NetworkCheck = serde_json::from_str(&body).map_err(|e| ApiError::bad_request(format!("{e}")))?;
    launchpad_core::netcap::remember(&id, &check);
    s.app.bus.publish("runners", "changed", Value::Null);
    Ok(Json(check))
}

/// Runner: the network check of this machine's sandbox (`launchpad_core::netcap::network_check`).
async fn check_network(State(s): State<Server>) -> ApiResult<launchpad_core::netcap::NetworkCheck> {
    Ok(Json(launchpad_core::netcap::network_check(&s.app).await.map_err(bad)?))
}

/// Home: asks an outdated runner to update itself to Home's build (`launchpad_core::self_update`).
async fn update_runner(State(s): State<Server>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let r = runners::list(&s.app).await.map_err(bad)?.into_iter().find(|r| r.runner.id == id).ok_or_else(|| ApiError::not_found("no such runner"))?;
    let name = &r.runner.name;
    if !r.connected {
        return Err(ApiError::bad_request(format!("{name} is offline: start it, then update it")));
    }
    if r.compat != Some(Compat::Outdated) {
        return Err(ApiError::bad_request(format!("{name} already runs Home's build")));
    }
    if !r.runner.capabilities.self_update {
        return Err(ApiError::bad_request(format!("{name} cannot update itself: run the install command again on that machine")));
    }
    if r.runner.os != std::env::consts::OS {
        return Err(ApiError::bad_request(format!("{name} runs on {} and Home on {}: reinstall it with a build for that machine", r.runner.os, std::env::consts::OS)));
    }
    let (status, body) = s.app.runners.request(&id, "POST", "/api/runner/update", None).await.map_err(bad)?;
    Ok((StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response())
}

/// Home: cancels a runner update that still waits for agents.
async fn cancel_runner_update(State(s): State<Server>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (status, body) = s.app.runners.request(&id, "DELETE", "/api/runner/update", None).await.map_err(bad)?;
    Ok((StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response())
}

/// SHA-256 of this daemon's executable, computed once.
static BUILD_SHA: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// The running executable: `/proc/self/exe` on Linux, so a file replaced on disk since the start is never served.
fn own_exe() -> std::io::Result<std::path::PathBuf> {
    if cfg!(target_os = "linux") { Ok("/proc/self/exe".into()) } else { std::env::current_exe() }
}

/// Home: its own executable, for a runner that updates itself. Runner credential; through the tunnel.
async fn build(State(s): State<Server>, headers: HeaderMap) -> Result<Response, ApiError> {
    use launchpad_core::self_update::{ARCH_HEADER, COMMIT_HEADER, OS_HEADER, SHA256_HEADER};
    let r = runner_of(&s, &headers).await?;
    if r.state != RunnerState::Approved || s.app.paths.role != role::Role::Home {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner"));
    }
    let sha = BUILD_SHA
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(|| -> std::io::Result<String> {
                use sha2::Digest;
                let mut f = std::fs::File::open(own_exe()?)?;
                let mut h = sha2::Sha256::new();
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = std::io::Read::read(&mut f, &mut buf)?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                }
                Ok(hex::encode(h.finalize()))
            })
            .await
            .map_err(std::io::Error::other)?
        })
        .await
        .map_err(|e| ApiError::bad_request(format!("read Home's executable: {e}")))?
        .clone();
    let file = tokio::fs::File::open(own_exe().map_err(|e| ApiError::bad_request(e.to_string()))?).await.map_err(|e| ApiError::bad_request(format!("read Home's executable: {e}")))?;
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let chunks = futures_util::stream::unfold(file, |mut f| async move {
        let mut buf = vec![0u8; 256 * 1024];
        match tokio::io::AsyncReadExt::read(&mut f, &mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(bytes::Bytes::from(buf)), f))
            }
            Err(e) => Some((Err(e), f)),
        }
    });
    let body = axum::body::Body::from_stream(chunks);
    Ok((
        [
            (axum::http::header::CONTENT_TYPE.as_str(), "application/octet-stream".to_string()),
            (axum::http::header::CONTENT_LENGTH.as_str(), len.to_string()),
            (SHA256_HEADER, sha),
            (COMMIT_HEADER, launchpad_core::commit().to_string()),
            (OS_HEADER, std::env::consts::OS.to_string()),
            (ARCH_HEADER, std::env::consts::ARCH.to_string()),
        ],
        body,
    )
        .into_response())
}

/// Runner: updates itself to Home's build once no agent is in a turn.
async fn update_self(State(s): State<Server>) -> ApiResult<launchpad_core::self_update::RunnerUpdate> {
    Ok(Json(launchpad_core::self_update::start(&s.app).map_err(bad)?))
}

async fn cancel_update(State(s): State<Server>) -> ApiResult<Value> {
    launchpad_core::self_update::cancel(&s.app).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
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
    crate::wire::accept(ws).on_upgrade(move |socket| {
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
    let Some(FromRunner::Hello { protocol, commit, guard, capabilities, os, .. }) = hello else { return };
    let refuse = if let Some(why) = role::os_refusal(&os).or_else(|| role::os_refusal(&runner.os)) {
        Some(why)
    } else if runner.state != RunnerState::Approved {
        Some("waiting for approval on Home (Settings → Home and runners)".to_string())
    } else if role::compat(role::PROTOCOL, launchpad_core::commit(), protocol, &commit) == Compat::Incompatible {
        Some(format!("this runner speaks protocol {protocol}, Home speaks {}: update this runner", role::PROTOCOL))
    } else {
        None
    };
    if let Some(reason) = refuse {
        let _ = send(&mut tx, &ToRunner::Refuse { reason }).await;
        return;
    }
    let compat = role::compat(role::PROTOCOL, launchpad_core::commit(), protocol, &commit);
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
    if !send(&mut tx, &ToRunner::Welcome { compat, commit: launchpad_core::commit().into(), acked }).await {
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
    tokio::spawn({
        let (app, id) = (s.app.clone(), runner.id.clone());
        async move { runners::deliver_pending(&app, &id).await }
    });
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
                                Some(router) if crate::home_client::home_accepts(&method, path.split('?').next().unwrap_or_default()) => crate::home_client::local(&app, router, &method, &path, body, &[]).await,
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
        // Seen and unread are Home's records (the replica keeps the later seen time); the runner follows when
        // it is connected, so a sleeping laptop never keeps a session in Needs you.
        ["api", "sessions", id, "seen" | "unread"] if method == axum::http::Method::POST => {
            if let Some(r) = s.app.sessions.get(id).and_then(|x| x.runner_id).filter(|r| s.app.runners.connected(r)) {
                let (out, path) = (s.app.runners.clone(), path.clone());
                tokio::spawn(async move {
                    let _ = out.request(&r, "POST", &path, None).await;
                });
            }
            return next.run(req).await;
        }
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
        // Files of a folder on a connected runner (a session's worktree, plain copy or temporary folder) are read
        // there; Home answers for its own folders and for runners that are away (`code::open`).
        ["api", "code", ..] if get => {
            let q = axum::extract::Query::<std::collections::HashMap<String, String>>::try_from_uri(req.uri()).map(|q| q.0).unwrap_or_default();
            match launchpad_core::code::runner_for(&s.app, q.get("root").map(String::as_str), q.get("project").map(String::as_str)).await {
                Some(r) => Some(r),
                None => return next.run(req).await,
            }
        }
        _ => return next.run(req).await,
    };
    // Images and downloads are bytes: the runner sends them in an envelope (`home_client::local`).
    let binary = matches!(parts.as_slice(), ["api", "code", "raw" | "download"]);
    let mut headers: Vec<(String, String)> = req.headers().get("idempotency-key").and_then(|v| v.to_str().ok()).map(|v| vec![("idempotency-key".to_string(), v.to_string())]).unwrap_or_default();
    if binary {
        headers.push((ENVELOPE_HEADER.into(), "1".into()));
    }
    let path_and_query = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_else(|| path.clone());
    let uri = req.uri().clone();
    let body = match axum::body::to_bytes(req.into_body(), 32 << 20).await {
        Ok(b) => (!b.is_empty()).then(|| String::from_utf8_lossy(&b).into_owned()),
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };
    // A message with uploads: they are stored on Home, so the runner receives their bytes.
    let body = match (parts.as_slice(), body) {
        (["api", "sessions", _, "input"], Some(b)) => match inline_uploads(&b) {
            Ok(b) => Some(b),
            Err(e) => return ApiError::bad_request(format!("{e:#}")).into_response(),
        },
        (_, b) => b,
    };
    let runner = match runner {
        Some(r) => r,
        // A launch: a runner that takes it, else Burst (`runners::launch_anywhere`).
        None => return launch(&s, body.as_deref(), headers).await,
    };
    // A message, Resume or an answer: placed like every continuation (its runner, else another one, else Burst).
    if method == axum::http::Method::POST
        && let ["api", "sessions", id, action @ ("input" | "resume" | "answer")] = parts.as_slice()
    {
        let key = headers.into_iter().find(|(k, _)| k == "idempotency-key").map(|(_, v)| v);
        return match runners::continue_anywhere(&s.app, id, action, body, key).await {
            Ok((status, body)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response(),
            Err(e) => ApiError::bad_request(format!("{e:#}")).into_response(),
        };
    }
    let connected = s.app.runners.connected(&runner);
    // Its runner is away (offline, or a stopped Burst job), or the worktree's folder is gone from it: Home answers
    // reviews from Home Git.
    if !connected || folder_gone(&s, get, &parts, &uri, body.as_deref()).await {
        if let Some(r) = away(&s, get, &parts, &uri, body.as_deref()).await {
            return r;
        }
        if !connected {
            return offline(&s, &runner, &parts, body.as_deref()).await;
        }
    }
    let worktree_list = get && matches!(parts.as_slice(), ["api", "sessions", _, "worktrees"]);
    match s.app.runners.request_with(&runner, method.as_str(), &path_and_query, body, headers).await {
        Ok((status, body)) if binary => from_envelope(status, &body),
        // Home completes the runner's list: merges and Local PRs are Home's, and gone folders are read on Home.
        Ok((200, body)) if worktree_list => match serde_json::from_str::<Vec<Value>>(&body) {
            Ok(list) => Json(launchpad_core::away::complete(&s.app, list).await).into_response(),
            Err(_) => (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response(),
        },
        Ok((status, body)) => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => ApiError::new(StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}

/// A session input body with its `files` and `images` references replaced by data URLs.
fn inline_uploads(body: &str) -> anyhow::Result<String> {
    let mut v: Value = serde_json::from_str(body)?;
    for key in ["files", "images"] {
        if let Some(items) = v.get_mut(key) {
            let mut files: Vec<launchpad_core::model::FileAttachment> = serde_json::from_value(items.take())?;
            launchpad_core::attachments::inline_files(&mut files)?;
            *items = serde_json::to_value(files)?;
        }
    }
    Ok(v.to_string())
}

/// Asks a runner for an answer with its headers and its bytes in base64 (`Envelope`).
pub(crate) const ENVELOPE_HEADER: &str = "x-launchpad-envelope";

/// An answer that is not text, through the tunnel (whose answers are text).
#[derive(serde::Serialize, Deserialize)]
pub(crate) struct Envelope {
    pub headers: Vec<(String, String)>,
    pub body: String,
}

fn from_envelope(status: u16, body: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let Ok(e) = serde_json::from_str::<Envelope>(body) else {
        // A runner of an older build: its error text, or bytes it could not send.
        return (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body.to_string()).into_response();
    };
    let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, e.body) else {
        return ApiError::new(StatusCode::BAD_GATEWAY, "the runner sent a damaged file").into_response();
    };
    let mut res = (status, bytes).into_response();
    for (k, v) in e.headers {
        if let (Ok(k), Ok(v)) = (axum::http::HeaderName::from_bytes(k.as_bytes()), axum::http::HeaderValue::from_str(&v)) {
            res.headers_mut().insert(k, v);
        }
    }
    res
}

/// Home: reviews of a session whose runner is away, from Home Git and its last handoff (`core::away`). None:
/// not a review request.
async fn away(s: &Server, get: bool, parts: &[&str], uri: &axum::http::Uri, body: Option<&str>) -> Option<Response> {
    use launchpad_core::away;
    let app = &s.app;
    let query = axum::extract::Query::<std::collections::HashMap<String, String>>::try_from_uri(uri).map(|q| q.0).unwrap_or_default();
    let flag = |k: &str| query.get(k).is_some_and(|v| v == "true");
    let session = |id: &str| app.sessions.get(id).ok_or_else(|| anyhow::anyhow!("no such session"));
    let result: anyhow::Result<Value> = match parts {
        ["api", "sessions", id, "git"] if get => async { Ok(json!(away::status(app, &session(id)?).await?)) }.await,
        ["api", "sessions", id, "git-commits"] if get => async { Ok(json!(away::history(app, &session(id)?, query.get("commit").map(String::as_str)).await?)) }.await,
        ["api", "sessions", id, "git-diff"] if get => async {
            let path = query.get("path").ok_or_else(|| anyhow::anyhow!("path required"))?;
            Ok(json!(away::diff(app, &session(id)?, path, flag("main"), flag("committed"), query.get("commit").map(String::as_str)).await?))
        }
        .await,
        ["api", "sessions", id, "worktrees"] if get => async { Ok(json!(away::worktrees(app, &session(id)?).await?)) }.await,
        ["api", "worktrees", worktree, action] if !get => async {
            let id = session_param(uri).ok_or_else(|| anyhow::anyhow!("session required"))?;
            let b: Value = serde_json::from_str(body.filter(|b| !b.trim().is_empty()).unwrap_or("{}"))?;
            Ok(json!(away::worktree_action(app, &session(&id)?, worktree, action, b["head"].as_str()).await?))
        }
        .await,
        ["api", "predrafts"] if !get => async {
            let id = session_param(uri).ok_or_else(|| anyhow::anyhow!("session required"))?;
            let b: Value = serde_json::from_str(body.unwrap_or("{}"))?;
            let cwd = b["cwd"].as_str().ok_or_else(|| anyhow::anyhow!("cwd required"))?;
            Ok(json!(away::local_pr(app, &session(&id)?, cwd, b["title"].as_str().map(str::to_string)).await?))
        }
        .await,
        _ => return None,
    };
    Some(match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::bad_request(format!("{e:#}")).into_response(),
    })
}

/// Home: a review request about a worktree whose folder is gone from its connected runner (retention removed it,
/// or the session moved and its new runner clones it at the next message): Changes of the primary worktree,
/// Merge, or Create Local PR. Home answers them like for an away runner (`away`).
async fn folder_gone(s: &Server, get: bool, parts: &[&str], uri: &axum::http::Uri, body: Option<&str>) -> bool {
    use launchpad_core::away;
    let app = &s.app;
    match parts {
        ["api", "sessions", id, "git" | "git-commits" | "git-diff"] if get => away::primary_gone(app, id).await,
        ["api", "worktrees", worktree, "merge"] if !get => match session_param(uri) {
            Some(id) => away::worktree_gone(app, &id, worktree).await,
            None => false,
        },
        ["api", "predrafts"] if !get => {
            let cwd = body.and_then(|b| serde_json::from_str::<Value>(b).ok()).and_then(|b| b["cwd"].as_str().map(str::to_string));
            match (session_param(uri), cwd) {
                (Some(id), Some(cwd)) => away::folder_gone(app, &id, &cwd).await,
                _ => false,
            }
        }
        _ => false,
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

/// Requests Home answers itself while the runner is away: Archive (the runner follows when it is back) and its
/// undo (the pending Archive is dropped), and Keep as folder (from Home's copy of the files). Everything else waits for the runner.
async fn offline(s: &Server, runner: &str, parts: &[&str], body: Option<&str>) -> Response {
    #[derive(Deserialize)]
    struct Keep {
        name: String,
    }
    let result: anyhow::Result<Value> = match parts {
        ["api", "sessions", id, "close"] => runners::archive_away(&s.app, id).await.map(|_| json!({"ok": true})),
        ["api", "sessions", id, "unarchive"] => runners::unarchive_away(&s.app, id).await.map(|_| json!({"ok": true})),
        ["api", "projects", id, "promote"] => async {
            let b: Keep = serde_json::from_str(body.unwrap_or("{}"))?;
            Ok(json!(launchpad_core::projects::keep_away(&s.app, id, &b.name).await?))
        }
        .await,
        _ => {
            let name = runners::runner_name(&s.app, runner).await;
            return ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} is offline")).into_response();
        }
    };
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::bad_request(format!("{e:#}")).into_response(),
    }
}

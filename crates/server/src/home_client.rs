//! A runner's tunnel to Home: one outbound WebSocket, kept open with retries. No inbound port is needed.
//!
//! Home sends API requests (`Req`), which the runner answers by calling its own router in process, as its
//! local owner. What the runner sends Home is not trusted as the owner's (`runner_trust`). The runner reports its guard (local settings) on connect and when it changes. Home owns the
//! workspaces: it sends them on connect and after each change, and the runner forwards their changes to Home.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, header};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use launchpad_core::App;
use launchpad_core::home_link::{self, Link, TunnelState};
use launchpad_core::{replica, role};
use launchpad_core::runners::proto::{FromRunner, ToRunner};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const PING: Duration = Duration::from_secs(25);
/// A replica batch Home has not acknowledged after this long is sent again.
const REPLICA_ACK_WAIT: Duration = Duration::from_secs(120);
/// The pause before sending again items Home failed to store.
const REPLICA_RETRY: Duration = Duration::from_secs(3);

/// Marks requests that arrived from Home through the tunnel.
pub use launchpad_http::FROM_HOME;

type Head = (u16, Vec<(String, String)>);

/// The open tunnel, for requests from this runner to Home.
#[derive(Default)]
struct HomeHandle {
    /// The control lane (`wire::lanes`).
    tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<FromRunner>>>,
    /// The bulk lane: parts of request bodies.
    bulk: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<FromRunner>>>,
    pending: std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<(u16, String)>>>,
    next: std::sync::atomic::AtomicU64,
    /// HTTP exchanges through Home: waiting for their head, then receiving their body.
    heads: std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Head>>>,
    bodies: std::sync::Mutex<std::collections::HashMap<u64, crate::wire::InboundTx>>,
    /// Home keeps exchange bodies to their window (`ToRunner::Welcome`).
    flow: std::sync::atomic::AtomicBool,
    /// Request bodies on their way to Home: the window Home's `HttpAck`s open.
    uploads: std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<tokio::sync::Semaphore>>>,
}

impl HomeHandle {
    /// The tunnel closed: waiting requests and exchanges fail now instead of timing out.
    fn clear(&self) {
        *self.tx.lock().unwrap() = None;
        *self.bulk.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
        self.heads.lock().unwrap().clear();
        for (_, b) in self.bodies.lock().unwrap().drain() {
            b.push(Err(std::io::Error::other("Home disconnected")));
        }
        for (_, w) in self.uploads.lock().unwrap().drain() {
            w.close();
        }
    }
}

/// Sends a message in the control lane. False: Home is not connected.
fn control(msg: FromRunner) -> bool {
    HOME.tx.lock().unwrap().as_ref().is_some_and(|t| t.send(msg).is_ok())
}

/// This runner dropped exchange `id` (its client went away, or Home did not answer): Home stops it.
fn cancel_exchange(id: u64) {
    HOME.heads.lock().unwrap().remove(&id);
    HOME.bodies.lock().unwrap().remove(&id);
    if let Some(w) = HOME.uploads.lock().unwrap().remove(&id) {
        w.close();
    }
    control(FromRunner::HttpCancel { id });
}

fn b64(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}

/// Opens an HTTP exchange through Home and streams the request body there, at most `wire::WINDOW` bytes ahead
/// of Home's acknowledgements. The answer's body acknowledges each part read; dropping it early cancels the
/// exchange on Home.
async fn stream_to_home(target: launchpad_core::runners::proto::HttpTarget, method: String, path: String, headers: Vec<(String, String)>, mut body: home_link::ByteStream) -> anyhow::Result<home_link::HttpReply> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (head_tx, head_rx) = tokio::sync::oneshot::channel();
    // Created first: every early return below drops it, which cancels the exchange.
    let flow = HOME.flow.load(std::sync::atomic::Ordering::Relaxed);
    let (body_tx, reply_body) = crate::wire::Inbound::new(flow, move |n| { control(FromRunner::HttpAck { id, n }); }, move || cancel_exchange(id));
    let window = crate::wire::window(flow);
    HOME.heads.lock().unwrap().insert(id, head_tx);
    HOME.bodies.lock().unwrap().insert(id, body_tx);
    HOME.uploads.lock().unwrap().insert(id, window.clone());
    let bulk = HOME.bulk.lock().unwrap().clone();
    let Some(bulk) = bulk.filter(|_| control(FromRunner::HttpOpen { id, target, method, path, headers })) else {
        anyhow::bail!("Home is unreachable");
    };
    tokio::spawn(async move {
        while let Some(part) = body.next().await {
            let Ok(part) = part else { break };
            for c in part.chunks(crate::wire::CHUNK) {
                if !crate::wire::take_part(&window, c.len()).await || bulk.send(FromRunner::HttpBody { id, data: b64(c), end: false }).await.is_err() {
                    return;
                }
            }
        }
        let _ = bulk.send(FromRunner::HttpBody { id, data: String::new(), end: true }).await;
        HOME.uploads.lock().unwrap().remove(&id);
    });
    let (status, headers) = match tokio::time::timeout(Duration::from_secs(120), head_rx).await {
        Ok(Ok(h)) => h,
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => anyhow::bail!("Home did not answer within two minutes"),
    };
    Ok(home_link::HttpReply { status, headers, body: Box::pin(reply_body) })
}

/// Home answered an exchange: its head, a part of its body, room for more of the request body, or the end of
/// Home's interest in the request body.
fn http_from_home(msg: ToRunner) {
    match msg {
        ToRunner::HttpHead { id, status, headers } => {
            if let Some(tx) = HOME.heads.lock().unwrap().remove(&id) {
                let _ = tx.send((status, headers));
            }
        }
        ToRunner::HttpBody { id, data, end, error } => {
            let tx = HOME.bodies.lock().unwrap().get(&id).cloned();
            let mut ok = true;
            if let Some(tx) = &tx {
                if !data.is_empty() {
                    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).map_err(std::io::Error::other);
                    // A reader that is gone, or Home past its window, ends the exchange.
                    ok = tx.push(bytes.map(bytes::Bytes::from));
                }
                if let Some(e) = error {
                    tx.push(Err(std::io::Error::other(e)));
                }
            }
            if !ok {
                cancel_exchange(id);
            } else if end {
                // The exchange is over: what is left of the request body is not needed.
                HOME.bodies.lock().unwrap().remove(&id);
                if let Some(w) = HOME.uploads.lock().unwrap().remove(&id) {
                    w.close();
                }
            }
        }
        ToRunner::HttpAck { id, n } => {
            if let Some(w) = HOME.uploads.lock().unwrap().get(&id) {
                crate::wire::acked(w, n);
            }
        }
        ToRunner::HttpCancel { id } => {
            if let Some(w) = HOME.uploads.lock().unwrap().remove(&id) {
                w.close();
            }
        }
        _ => {}
    }
}

static HOME: std::sync::LazyLock<HomeHandle> = std::sync::LazyLock::new(HomeHandle::default);

/// Sends an API request to Home through the tunnel.
async fn to_home(method: &str, path: &str, body: Option<String>, session: Option<String>) -> anyhow::Result<(u16, String)> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (tx, rx) = tokio::sync::oneshot::channel();
    HOME.pending.lock().unwrap().insert(id, tx);
    let sent = HOME.tx.lock().unwrap().as_ref().is_some_and(|t| t.send(FromRunner::Req { id, method: method.into(), path: path.into(), body, session }).is_ok());
    if !sent {
        HOME.pending.lock().unwrap().remove(&id);
        anyhow::bail!("Home is unreachable; this change needs Home");
    }
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => {
            HOME.pending.lock().unwrap().remove(&id);
            anyhow::bail!("Home did not answer within a minute")
        }
    }
}

/// Sends a lease call to Home through the tunnel.
async fn call_home(op: String, body: serde_json::Value) -> anyhow::Result<(u16, String)> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (tx, rx) = tokio::sync::oneshot::channel();
    HOME.pending.lock().unwrap().insert(id, tx);
    let sent = HOME.tx.lock().unwrap().as_ref().is_some_and(|t| t.send(FromRunner::Call { id, op, body }).is_ok());
    if !sent {
        HOME.pending.lock().unwrap().remove(&id);
        anyhow::bail!("Home is unreachable");
    }
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => {
            HOME.pending.lock().unwrap().remove(&id);
            anyhow::bail!("Home did not answer within a minute")
        }
    }
}

/// Requests about records Home owns, which a runner sends to Home: workspace changes (the runner keeps a copy
/// for reading), routines and scheduled launches (only Home launches), Home Git, review drafts (`/api/review-state`, so they follow the owner to every window), and the context reads of the Launchpad CLI (only Home sees the sessions, schedules and
/// Inbox of every runner). An agent's call goes with its session and Home checks it with the agent's access; the
/// runner's own requests pass only Home's closed list (`runner_trust`).
pub(crate) fn home_owned(path: &str) -> bool {
    path == "/api/workspaces" || path.starts_with("/api/workspaces/") || path.starts_with("/api/home-git/") || path.starts_with("/api/plain/") || path.starts_with("/api/knowledge/") || path.starts_with("/api/vault") || path == "/api/mcp" || path.starts_with("/api/mcp/")
        || path == "/api/tool-defaults" || path.starts_with("/api/tool-defaults/")
        || path == "/api/proposals" || path.starts_with("/api/proposals/")
        || path == "/api/launch/crons" || (path.starts_with("/api/launch/crons/") && path != "/api/launch/crons/next")
        || path == "/api/launch/schedules" || path.starts_with("/api/launch/schedules/")
        || path == "/api/bots" || path.starts_with("/api/bots/") || path.starts_with("/api/context/") || path == "/api/launch/start-preview"
        || (path.starts_with("/api/sessions/") && (path.ends_with("/access") || path.ends_with("/machines")))
        || path == "/api/suggestions" || path.starts_with("/api/suggestions/")
        || path == "/api/channels" || path.starts_with("/api/channels/") || path == "/api/outbox" || path.starts_with("/api/outbox/")
        || path == "/api/today" || path == "/api/inbox/agent-done"
        || path == "/api/activity" || path.starts_with("/api/activity/") || path == "/api/settings/shared"
        || path == "/api/review-state"
        || path == "/api/home/update" || path == "/api/home/update/check" || path.starts_with("/api/cli-updates")
}

/// Marks a request of a session's CLI token (`auth::guard`), with its session: its launches and messages go to Home,
/// like the owner's, and Home stamps its messages as the session's (`launchpad_core::senders`).
#[derive(Clone)]
pub(crate) struct AgentCall(pub String);

/// `/api/sessions/{id}` and its sub-paths: the session's ID.
fn session_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/sessions/")?;
    let id = rest.split('/').next()?;
    (!id.is_empty() && id != "search").then_some(id)
}

/// Runner: whether a request goes to Home although Home does not own its record: a session this runner does
/// not run (Home relays it to the session's runner, or answers for Burst and runners that are away), a launch
/// by an agent (Home picks the runner, as for the owner's Launch), and log reads of other daemons.
fn to_home_too(s: &crate::Server, req: &axum::extract::Request) -> bool {
    let path = req.uri().path();
    if path == "/api/logs" {
        let local = req.uri().query().unwrap_or("").split('&').any(|kv| kv == "source=local");
        return req.method() == axum::http::Method::GET && !local;
    }
    if path == "/api/sessions" {
        return req.method() == axum::http::Method::POST && req.extensions().get::<AgentCall>().is_some();
    }
    // An agent's message to any session goes to Home, which stamps it as the agent's and places it.
    if req.method() == axum::http::Method::POST && req.extensions().get::<AgentCall>().is_some() && session_id(path).is_some_and(|id| path == format!("/api/sessions/{id}/input")) {
        return true;
    }
    // A session of another runner: reads, and an agent's admin actions (Home checks them with its access).
    (session_id(path).is_some_and(|id| s.app.sessions.get(id).is_none())
        && (req.method() == axum::http::Method::GET || launchpad_core::actions::find(req.method().as_str(), path).is_some_and(|a| a.access == launchpad_core::actions::ActionAccess::Admin)))
        || agent_to_home(req)
}

/// Runner: an agent's call whose handler here would only pass it on to Home (Local PR records, folder settings):
/// it goes to Home as the agent's, so Home checks it with the agent's access (`runner_trust`). A Local PR of a
/// worktree (`cwd`) and `resolve` stay here: they read this runner's worktrees.
/// Every admin action and every read of an agent also go to Home, which checks them against the session's
/// workspace (`agent_auth::within_workspace`), except the reads of this runner's own state (`/api/status`) and of
/// its worktrees (`resolve`).
fn agent_to_home(req: &axum::extract::Request) -> bool {
    if req.extensions().get::<AgentCall>().is_none() {
        return false;
    }
    let (method, path) = (req.method(), req.uri().path());
    let admin = launchpad_core::actions::find(method.as_str(), path).is_some_and(|a| a.access == launchpad_core::actions::ActionAccess::Admin);
    let read = method == axum::http::Method::GET && !matches!(path, "/api/status" | "/api/predrafts/resolve");
    admin || read
        || (path.starts_with("/api/projects/") && method != axum::http::Method::GET)
        || (path.starts_with("/api/predrafts/") && path != "/api/predrafts/resolve")
}

/// `/api/sessions/{id}/trace`: the session's ID.
fn trace_id(path: &str) -> Option<&str> {
    path.strip_prefix("/api/sessions/").and_then(|r| r.strip_suffix("/trace")).filter(|id| !id.is_empty() && !id.contains('/'))
}

/// Runner: requests about records Home owns go to Home; the runner's copy follows Home's snapshots.
/// Runner: whether this request goes to Home (`forward_writes`).
pub(crate) fn goes_to_home(s: &crate::Server, req: &axum::extract::Request) -> bool {
    if s.app.paths.role != role::Role::Runner {
        return false;
    }
    let path = req.uri().path();
    let get = req.method() == axum::http::Method::GET;
    // Traces always come from Home: it holds every event of every generation, while a runner that adopted a
    // session has only the turns since (a bot's earlier session, a context trace, a moved session).
    let trace = get && trace_id(path).is_some();
    // Scheduled follow-ups are Home's records, sent through placement when due.
    let follow_up = req.method() == axum::http::Method::POST && session_id(path).is_some_and(|id| path == format!("/api/sessions/{id}/follow-up"));
    (home_owned(path) && (!get || !(path == "/api/workspaces" || path.starts_with("/api/workspaces/")))) || trace || follow_up || to_home_too(s, req)
}

pub async fn forward_writes(axum::extract::State(s): axum::extract::State<crate::Server>, req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !goes_to_home(&s, &req) {
        return next.run(req).await;
    }
    let trace = if req.method() == axum::http::Method::GET { trace_id(req.uri().path()).map(str::to_string) } else { None };
    // A session of this runner: the events queued before this request reach Home first (a few seconds at most;
    // events of a running turn queued later do not hold it, nor a tunnel that is down).
    let mut local = trace.as_deref().is_some_and(|id| s.app.sessions.get(id).is_some());
    // An agent's fallback read stays in its own workspace (Home checks it otherwise).
    if local && let (Some(a), Some(id)) = (req.extensions().get::<AgentCall>(), trace.as_deref()) {
        let own = launchpad_core::scope::of_session(&s.app, &a.0).await;
        local = own.is_some() && own == launchpad_core::scope::of_session(&s.app, id).await;
    }
    if local && let Ok(newest) = replica::newest(&s.app).await {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let up = HOME.tx.lock().unwrap().is_some();
            if !up || !replica::waiting_through(&s.app, newest).await.is_ok_and(|n| n > 0) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    let (method, path) = (req.method().to_string(), req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default());
    let agent = req.extensions().get::<AgentCall>().map(|a| a.0.clone());
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 1 << 20).await {
        Ok(b) => b,
        Err(e) => return crate::error::ApiError::bad_request(format!("{e}")).into_response(),
    };
    let body = (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned());
    match to_home(&method, &path, body, agent).await {
        Ok((status, body)) => (axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::BAD_GATEWAY), [(header::CONTENT_TYPE, "application/json")], body).into_response(),
        // Home is out of reach: this runner's own copy of its session (complete unless it was adopted).
        Err(_) if local => next.run(axum::extract::Request::from_parts(parts, Body::from(bytes))).await,
        Err(e) => crate::error::ApiError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")).into_response(),
    }
}

/// A Burst runner's tunnels: Home dials `/api/runner/reverse`, which hands the socket to the tunnel loop.
static REVERSE: std::sync::LazyLock<(tokio::sync::mpsc::UnboundedSender<axum::extract::ws::WebSocket>, tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<axum::extract::ws::WebSocket>>)> =
    std::sync::LazyLock::new(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, tokio::sync::Mutex::new(rx))
    });

/// `/api/runner/reverse` on a Burst runner: Home's connection, with the runner's own credential.
pub async fn reverse(headers: axum::http::HeaderMap, ws: axum::extract::WebSocketUpgrade) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(b) = home_link::burst() else { return axum::http::StatusCode::NOT_FOUND.into_response() };
    let given = headers.get(crate::runners::CREDENTIAL_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    let expected = format!("{}.{}", b.id, b.secret);
    let same = given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    if !same {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    crate::wire::accept(ws).on_upgrade(|socket| async move {
        let _ = REVERSE.0.send(socket);
    })
}

/// A Burst runner with no Home connection for this long stops (its job ends): the same idle timeout as an HF
/// Sandbox's default.
const BURST_IDLE: Duration = Duration::from_secs(10 * 60);

/// A Burst runner: Home Git and plain-folder requests to `127.0.0.1:<port>` go through the tunnel.
async fn local_relay() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    home_link::set_local_home(listener.local_addr()?.port());
    let relay = Router::new().fallback(|req: axum::extract::Request| async move {
        use axum::response::IntoResponse;
        let (parts, body) = req.into_parts();
        let path = parts.uri.path_and_query().map(|p| p.to_string()).unwrap_or_default();
        let headers: Vec<(String, String)> = parts.headers.iter().filter(|(k, _)| *k != header::HOST).filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
        let stream = body.into_data_stream().map(|r| r.map_err(std::io::Error::other));
        match home_link::http(launchpad_core::runners::proto::HttpTarget::Home, parts.method.as_str(), &path, headers, Box::pin(stream)).await {
            Ok(reply) => {
                let mut out = axum::response::Response::builder().status(reply.status);
                for (k, v) in &reply.headers {
                    out = out.header(k, v);
                }
                out.body(Body::from_stream(reply.body)).unwrap_or_else(|e| crate::error::ApiError::bad_request(format!("{e}")).into_response())
            }
            Err(e) => crate::error::ApiError::new(axum::http::StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
        }
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, relay).await;
    });
    Ok(())
}

pub fn spawn(app: App, router: Router) {
    launchpad_core::home_link::set_requester(Box::new(|method, path, body| Box::pin(async move { to_home(&method, &path, body, None).await })));
    launchpad_core::home_link::set_caller(Box::new(|op, body| Box::pin(call_home(op, body))));
    launchpad_core::home_link::set_streamer(Box::new(|target, method, path, headers, body| Box::pin(stream_to_home(target, method, path, headers, body))));
    if let Some(b) = home_link::burst() {
        let url = b.home_url.clone();
        tokio::spawn(async move {
            if let Err(e) = local_relay().await {
                tracing::warn!("Burst relay: {e:#}");
            }
            let mut last = tokio::time::Instant::now();
            loop {
                home_link::set_state(&app, Some(url.clone()), TunnelState::Connecting, Some("waiting for Home".into()), None);
                let next = tokio::time::timeout_at(last + BURST_IDLE, async { REVERSE.1.lock().await.recv().await }).await;
                let socket = match next {
                    Ok(Some(s)) => s,
                    Ok(None) => return,
                    Err(_) => {
                        tracing::info!("no connection from Home for {} minutes: this Burst runner stops", BURST_IDLE.as_secs() / 60);
                        std::process::exit(0);
                    }
                };
                let (tx, rx) = crate::wire::axum(socket);
                match session(&app, &router, &url, tx, rx).await {
                    Ok(Some(reason)) => home_link::set_state(&app, Some(url.clone()), TunnelState::Refused, Some(reason), None),
                    Ok(None) => home_link::set_state(&app, Some(url.clone()), TunnelState::Offline, Some("Home closed the connection".into()), None),
                    Err(e) => home_link::set_state(&app, Some(url.clone()), TunnelState::Offline, Some(format!("{e:#}")), None),
                }
                last = tokio::time::Instant::now();
            }
        });
        return;
    }
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(2);
        loop {
            let Some(link) = home_link::load(&app.paths) else {
                home_link::set_state(&app, None, TunnelState::NotConnected, Some("run launchpad connect <home address>".into()), None);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            home_link::set_state(&app, Some(link.url.clone()), TunnelState::Connecting, None, None);
            match run(&app, &router, &link).await {
                Ok(Some(reason)) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Refused, Some(reason), None);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                Ok(None) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Offline, Some("Home closed the connection".into()), None);
                    backoff = Duration::from_secs(2);
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Offline, Some(format!("{e:#}")), None);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

fn tunnel_url(url: &str) -> String {
    let base = url.trim_end_matches('/');
    let base = base.strip_prefix("https://").map(|r| format!("wss://{r}")).or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}"))).unwrap_or_else(|| base.to_string());
    format!("{base}/api/runners/tunnel")
}

/// One connection. `Ok(Some(reason))`: Home refused the runner.
async fn run(app: &App, router: &Router, link: &Link) -> anyhow::Result<Option<String>> {
    let (tx, rx) = match dial(link).await? {
        Ok(wire) => wire,
        Err(reason) => return Ok(Some(reason)),
    };
    session(app, router, &link.url, tx, rx).await
}

/// Dials Home's tunnel endpoint. `Ok(Err(reason))`: Home does not know this runner.
async fn dial(link: &Link) -> anyhow::Result<Result<(crate::wire::WireTx, crate::wire::WireRx), String>> {
    let mut request = tunnel_url(&link.url).into_client_request()?;
    request.headers_mut().insert(crate::runners::CREDENTIAL_HEADER, link.credential().parse()?);
    if let Some(token) = home_link::space_token(&link.url) {
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {token}").parse()?);
    }
    match tokio::time::timeout(Duration::from_secs(30), crate::wire::dial(request)).await? {
        Ok((socket, _)) => Ok(Ok(crate::wire::tungstenite(socket))),
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) if res.status() == 401 => {
            Ok(Err("this runner is unknown to Home (removed or never approved): run launchpad connect again".into()))
        }
        Err(e) => Err(e.into()),
    }
}

/// The protocol over one connection, whichever side dialed.
async fn session(app: &App, router: &Router, url: &str, mut tx: crate::wire::WireTx, mut rx: crate::wire::WireRx) -> anyhow::Result<Option<String>> {
    let text = |m: &FromRunner| serde_json::to_string(m).unwrap_or_default();
    let guard = home_link::guard(&app.paths);
    let status = app.status.get();
    let capabilities = launchpad_core::runners::capabilities(app).await;
    tx.send(text(&FromRunner::Hello { protocol: role::PROTOCOL, commit: launchpad_core::commit().into(), name: status.host.clone(), os: std::env::consts::OS.into(), guard: guard.clone(), capabilities })).await?;
    let (compat, acked) = loop {
        match tokio::time::timeout(Duration::from_secs(30), rx.next()).await? {
            Some(Ok(t)) => match serde_json::from_str::<ToRunner>(&t)? {
                ToRunner::Welcome { compat, acked, flow, .. } => {
                    HOME.flow.store(flow, std::sync::atomic::Ordering::Relaxed);
                    break (compat, acked);
                }
                ToRunner::Refuse { reason } => return Ok(Some(reason)),
                _ => {}
            },
            Some(Err(e)) => return Err(e),
            None => return Ok(None),
        }
    };
    home_link::set_state(app, Some(url.to_string()), TunnelState::Connected, None, Some(compat));
    tracing::info!("connected to Home {url} ({compat:?})");
    // Everything Home saved can go; the rest is sent again, in order.
    replica::acked(app, acked).await?;
    // From here on one writer sends: the control lane first, request body parts when it is empty.
    let (out, bulk, mut writer) = crate::wire::lanes::<FromRunner>(tx);
    *HOME.tx.lock().unwrap() = Some(out.clone());
    *HOME.bulk.lock().unwrap() = Some(bulk);
    // Turns not saved on Home yet go now (`turn_sync`).
    launchpad_core::turn_sync::kick();
    let result = serve(app, router, &out, &mut rx, &mut writer, acked).await;
    writer.abort();
    // Requests to Home fail now instead of timing out, and terminal viewers of Home's windows end.
    HOME.clear();
    crate::terminals::home_gone();
    result
}

async fn serve(app: &App, router: &Router, out: &tokio::sync::mpsc::UnboundedSender<FromRunner>, rx: &mut crate::wire::WireRx, writer: &mut tokio::task::JoinHandle<()>, mut acked: u64) -> anyhow::Result<Option<String>> {
    let send = |m: FromRunner| out.send(m).map_err(|_| anyhow::anyhow!("the connection to Home closed"));
    let mut guard = home_link::guard(&app.paths);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    let mut last_heard = tokio::time::Instant::now();
    // One replica batch at a time: the last item of the batch Home has not acknowledged yet, and when it went. The
    // next batch starts after Home's acknowledgement, so an item Home failed to store is sent again (`replica`).
    let mut in_flight: Option<(u64, tokio::time::Instant)> = None;
    // After an acknowledgement short of the batch (Home's database was busy), the next try waits a moment.
    let mut resend_at = tokio::time::Instant::now();
    // Terminal list changes go to Home, which tells its windows.
    let mut bus = app.bus.subscribe(None).rx;
    loop {
        tokio::select! {
            _ = &mut *writer => anyhow::bail!("the connection to Home closed"),
            delivered = bus.recv() => {
                let changed = match delivered {
                    Ok(d) => d.text.contains(r#""topic":"terminals""#),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if changed {
                    send(FromRunner::Terminals)?;
                }
            }
            _ = replica::wait() => {
                let now = tokio::time::Instant::now();
                // No answer for a long time: send again from Home's mark (Home skips what it already stored).
                if in_flight.is_some_and(|(_, at)| now - at > REPLICA_ACK_WAIT) {
                    in_flight = None;
                }
                if in_flight.is_none() && now >= resend_at {
                    let items = replica::batch(app, acked).await?;
                    if let Some((n, _)) = items.last() {
                        in_flight = Some((*n, now));
                        send(FromRunner::Items { items })?;
                    }
                }
            }
            _ = home_link::guard_changed() => {
                let now = home_link::guard(&app.paths);
                if now != guard {
                    guard = now;
                    send(FromRunner::Guard { guard: guard.clone() })?;
                }
            }
            _ = ping.tick() => {
                anyhow::ensure!(last_heard.elapsed() < PING * 3, "Home stopped answering");
                send(FromRunner::Ping)?;
            }
            frame = rx.next() => {
                let Some(frame) = frame else { return Ok(None) };
                let t = frame?;
                last_heard = tokio::time::Instant::now();
                match serde_json::from_str::<ToRunner>(&t) {
                    Ok(ToRunner::Req { id, method, path, body, headers }) => {
                        let (app, router, out) = (app.clone(), router.clone(), out.clone());
                        tokio::spawn(async move {
                            let (status, body) = local(&app, &router, &method, &path, body, &headers).await.unwrap_or_else(|e| (500, serde_json::json!({"error": format!("{e:#}")}).to_string()));
                            let _ = out.send(FromRunner::Res { id, status, body });
                        });
                    }
                    Ok(ToRunner::Res { id, status, body }) => {
                        if let Some(tx) = HOME.pending.lock().unwrap().remove(&id) { let _ = tx.send((status, body)); }
                    }
                    Ok(ToRunner::Workspaces { workspaces }) => launchpad_core::workspace::replace_all(app, workspaces).await?,
                    Ok(ToRunner::Folders { projects }) => launchpad_core::projects::store_home(app, projects, true).await?,
                    Ok(ToRunner::Profiles { profiles }) => {
                        if app.config.get().profiles != profiles {
                            app.config.update(|config| {
                                config.profiles = profiles;
                                Ok(())
                            })?;
                        }
                    }
                    Ok(ToRunner::Settings { worktrees_keep }) => {
                        if app.config.get().worktrees.keep != worktrees_keep {
                            app.config.update(|config| {
                                config.worktrees.keep = worktrees_keep;
                                Ok(())
                            })?;
                        }
                    }
                    Ok(ToRunner::Ack { n }) => {
                        replica::acked(app, n).await?;
                        let (mark, complete) = replica::after_ack(acked, in_flight.map(|(sent, _)| sent), n);
                        acked = mark;
                        in_flight = None;
                        if !complete {
                            resend_at = tokio::time::Instant::now() + REPLICA_RETRY;
                        }
                        replica::wake();
                    }
                    Ok(ToRunner::Quotas { quotas }) => launchpad_core::quota::apply_home_readings(app, quotas),
                    Ok(ToRunner::Spend { entries }) => launchpad_core::spend::apply_home(app, entries),
                    Ok(ToRunner::RenewLeases { login }) => launchpad_core::contain::renew_soon(app, &login),
                    Ok(ToRunner::Refuse { reason }) => return Ok(Some(reason)),
                    Ok(msg @ (ToRunner::HttpHead { .. } | ToRunner::HttpBody { .. } | ToRunner::HttpAck { .. } | ToRunner::HttpCancel { .. })) => http_from_home(msg),
                    Ok(ToRunner::WsOpen { id, path }) => crate::terminals::open_for_home(app, id, &path, out.clone()),
                    Ok(ToRunner::WsFrame { id, text, data }) => crate::terminals::frame_from_home(id, text, data),
                    Ok(ToRunner::WsClose { id }) => crate::terminals::close_from_home(id),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Home sent a bad message: {e}"),
                }
            }
        }
    }
    Ok(None)
}

/// Home: an agent's CLI call that a runner forwarded for a session Home placed there (`runner_trust::agent`). It
/// runs through Home's router with that session's CLI credential (signed here, for its current generation), so
/// Home checks it like a call of its own (`agent_auth::authorize` with the session's access from Home's records,
/// the activity list), never as the owner.
pub(crate) async fn local_agent(app: &App, router: &Router, method: &str, path: &str, body: Option<String>, session: &str) -> anyhow::Result<(u16, String)> {
    anyhow::ensure!(path.starts_with("/api/"), "only API requests go through the tunnel");
    let s = app.sessions.get(session).ok_or_else(|| anyhow::anyhow!("no such session"))?;
    let token = launchpad_core::session_token::token(&app.paths.token()?, &s.id, s.generation)?;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.unwrap_or_default()))?;
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let res = router.clone().oneshot(req).await?;
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 64 << 20).await?;
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// Runner: Home relays a request about a session it placed here that this runner does not hold: its adoption
/// failed or never ran (`handoff::move_to`), so the runner takes it over first. Home's answer decides
/// (`handoff::adoption` refuses a session Home placed elsewhere).
async fn adopt_first(app: &App, method: &str, path: &str) -> anyhow::Result<()> {
    if app.paths.role != role::Role::Runner || method != "POST" {
        return Ok(());
    }
    let bare = path.split('?').next().unwrap_or_default();
    let Some(id) = session_id(bare) else { return Ok(()) };
    let action = bare.trim_start_matches("/api/sessions/").trim_start_matches(id).trim_start_matches('/');
    // Adoption itself, a release, an Archive of a session it no longer has, and window marks never adopt.
    if matches!(action, "adopt" | "release" | "close" | "seen" | "unread") || app.sessions.get(id).is_some() {
        return Ok(());
    }
    use anyhow::Context;
    launchpad_core::handoff::adopt_moved(app, id).await.context("this runner could not take the session over")
}

/// Runs an API request against this daemon's own router, as its local owner: Home's requests to a runner, and on
/// Home a runner's own requests that `runner_trust::check` let through.
pub(crate) async fn local(app: &App, router: &Router, method: &str, path: &str, body: Option<String>, headers: &[(String, String)]) -> anyhow::Result<(u16, String)> {
    anyhow::ensure!(path.starts_with("/api/"), "only API requests go through the tunnel");
    if let Err(e) = adopt_first(app, method, path).await {
        return Ok((409, serde_json::json!({"error": format!("{e:#}")}).to_string()));
    }
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.paths.token()?))
        .header(header::CONTENT_TYPE, "application/json")
        .header(FROM_HOME, "1")
        .body(Body::from(body.unwrap_or_default()))?;
    for (k, v) in headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("idempotency-key")) {
        req.headers_mut().insert(header::HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
    }
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let res = router.clone().oneshot(req).await?;
    let status = res.status().as_u16();
    if headers.iter().any(|(k, v)| k.eq_ignore_ascii_case(crate::runners::ENVELOPE_HEADER) && v == "1") {
        let headers = res.headers().iter().filter(|(k, _)| *k != header::CONTENT_LENGTH).filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
        let bytes = axum::body::to_bytes(res.into_body(), 64 << 20).await?;
        let body = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        return Ok((status, serde_json::to_string(&crate::runners::Envelope { headers, body })?));
    }
    let bytes = axum::body::to_bytes(res.into_body(), 64 << 20).await?;
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// `launchpad connect <home>`: asks Home for a pairing request, shows its code, and waits for the owner to
/// approve it in Settings → Home and runners. Saves the credential in the runner's folder.
pub async fn connect(paths: &launchpad_core::paths::Paths, url: &str, name: Option<String>) -> anyhow::Result<()> {
    let url = url.trim().trim_end_matches('/').to_string();
    anyhow::ensure!(url.starts_with("https://") || url.starts_with("http://"), "the Home address starts with https://");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let auth = |r: reqwest::RequestBuilder| match home_link::space_token(&url) { Some(t) => r.bearer_auth(t), None => r };
    let name = name.unwrap_or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "Runner".into()));
    let res = auth(client.post(format!("{url}/api/runners/request"))).json(&serde_json::json!({"name": name, "os": std::env::consts::OS})).send().await?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    anyhow::ensure!(status.is_success(), "Home refused the request: {}", body["error"].as_str().unwrap_or(status.as_str()));
    let link = Link { url: url.clone(), id: body["id"].as_str().unwrap_or_default().into(), secret: body["secret"].as_str().unwrap_or_default().into() };
    println!("Approve this runner on Home: the setup page or Settings → Home and runners.\nCode: {}\nWaiting…", body["code"].as_str().unwrap_or("?"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10 * 60);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let res = auth(client.get(format!("{url}/api/runners/request/{}", link.id))).header(crate::runners::CREDENTIAL_HEADER, link.credential()).send().await;
        let Ok(res) = res else { continue };
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("the request was removed on Home");
        }
        let state: serde_json::Value = res.json().await.unwrap_or_default();
        if state["state"] == "approved" {
            home_link::save(paths, &link)?;
            println!("Approved. The runner daemon connects to {url}.");
            return Ok(());
        }
    }
    anyhow::bail!("not approved within ten minutes; run launchpad connect again")
}

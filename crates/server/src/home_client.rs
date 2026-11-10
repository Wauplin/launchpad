//! A runner's tunnel to Home: one outbound WebSocket, kept open with retries. No inbound port is needed.
//!
//! Home sends API requests (`Req`), which the runner answers by calling its own router in process, as its
//! local owner. The runner reports its guard (local settings) on connect and when it changes. Home owns the
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
type BodyTx = tokio::sync::mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>;

/// The open tunnel, for requests from this runner to Home.
#[derive(Default)]
struct HomeHandle {
    tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<FromRunner>>>,
    pending: std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<(u16, String)>>>,
    next: std::sync::atomic::AtomicU64,
    /// HTTP exchanges through Home: waiting for their head, then receiving their body.
    heads: std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Head>>>,
    bodies: std::sync::Mutex<std::collections::HashMap<u64, BodyTx>>,
}

impl HomeHandle {
    /// The tunnel closed: waiting requests and exchanges fail now instead of timing out.
    fn clear(&self) {
        *self.tx.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
        self.heads.lock().unwrap().clear();
        for (_, b) in self.bodies.lock().unwrap().drain() {
            let _ = b.send(Err(std::io::Error::other("Home disconnected")));
        }
    }
}

/// Largest body part per frame, before base64.
const CHUNK: usize = 256 * 1024;

fn b64(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}

/// Opens an HTTP exchange through Home and streams the request body there.
async fn stream_to_home(target: launchpad_core::runners::proto::HttpTarget, method: String, path: String, headers: Vec<(String, String)>, mut body: home_link::ByteStream) -> anyhow::Result<home_link::HttpReply> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (head_tx, head_rx) = tokio::sync::oneshot::channel();
    let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
    HOME.heads.lock().unwrap().insert(id, head_tx);
    HOME.bodies.lock().unwrap().insert(id, body_tx);
    let tx = HOME.tx.lock().unwrap().clone();
    let Some(tx) = tx.filter(|t| t.send(FromRunner::HttpOpen { id, target, method, path, headers }).is_ok()) else {
        HOME.heads.lock().unwrap().remove(&id);
        HOME.bodies.lock().unwrap().remove(&id);
        anyhow::bail!("Home is unreachable");
    };
    tokio::spawn(async move {
        while let Some(part) = body.next().await {
            let Ok(part) = part else { break };
            for c in part.chunks(CHUNK) {
                if tx.send(FromRunner::HttpBody { id, data: b64(c), end: false }).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(FromRunner::HttpBody { id, data: String::new(), end: true });
    });
    let (status, headers) = match tokio::time::timeout(Duration::from_secs(120), head_rx).await {
        Ok(Ok(h)) => h,
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => {
            HOME.heads.lock().unwrap().remove(&id);
            HOME.bodies.lock().unwrap().remove(&id);
            anyhow::bail!("Home did not answer within two minutes")
        }
    };
    let body = futures_util::stream::unfold(body_rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    Ok(home_link::HttpReply { status, headers, body: Box::pin(body) })
}

/// Home answered an exchange: its head, or a part of its body.
fn http_from_home(msg: ToRunner) {
    match msg {
        ToRunner::HttpHead { id, status, headers } => {
            if let Some(tx) = HOME.heads.lock().unwrap().remove(&id) {
                let _ = tx.send((status, headers));
            }
        }
        ToRunner::HttpBody { id, data, end, error } => {
            let mut bodies = HOME.bodies.lock().unwrap();
            if let Some(tx) = bodies.get(&id) {
                if !data.is_empty() {
                    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).map_err(std::io::Error::other);
                    let _ = tx.send(bytes.map(bytes::Bytes::from));
                }
                if let Some(e) = error {
                    let _ = tx.send(Err(std::io::Error::other(e)));
                }
            }
            if end {
                bodies.remove(&id);
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
/// Inbox of every runner).
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

/// Home: requests a runner may send through its tunnel (as Home's owner). Sessions: reads, and the admin actions
/// of the action list (an agent's CLI acting on a session of another runner, as the owner's button would);
/// never the owner-only ones (paid usage).
pub(crate) fn home_accepts(method: &str, path: &str) -> bool {
    home_owned(path) || path.starts_with("/api/projects") || path.starts_with("/api/predrafts") || trace_id(path).is_some() || (method == "GET" && path == "/api/logs")
        || ((path == "/api/sessions" || path.starts_with("/api/sessions/"))
            && (method == "GET" || launchpad_core::actions::find(method, path).is_some_and(|a| a.access == launchpad_core::actions::ActionAccess::Admin)))
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
    session_id(path).is_some_and(|id| s.app.sessions.get(id).is_none()) && home_accepts(req.method().as_str(), path)
}

/// `/api/sessions/{id}/trace`: the session's ID.
fn trace_id(path: &str) -> Option<&str> {
    path.strip_prefix("/api/sessions/").and_then(|r| r.strip_suffix("/trace")).filter(|id| !id.is_empty() && !id.contains('/'))
}

/// Runner: requests about records Home owns go to Home; the runner's copy follows Home's snapshots.
pub async fn forward_writes(axum::extract::State(s): axum::extract::State<crate::Server>, req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    let forwarded = home_owned(path) && (req.method() != axum::http::Method::GET || !(path == "/api/workspaces" || path.starts_with("/api/workspaces/")));
    // Traces always come from Home: it holds every event of every generation, while a runner that adopted a
    // session has only the turns since (a bot's earlier session, a context trace, a moved session).
    let trace = if req.method() == axum::http::Method::GET { trace_id(path).map(str::to_string) } else { None };
    // Scheduled follow-ups are Home's records, sent through placement when due.
    let follow_up = req.method() == axum::http::Method::POST && session_id(path).is_some_and(|id| path == format!("/api/sessions/{id}/follow-up"));
    let forwarded = forwarded || trace.is_some() || follow_up || (s.app.paths.role == role::Role::Runner && to_home_too(&s, &req));
    if s.app.paths.role != role::Role::Runner || !forwarded {
        return next.run(req).await;
    }
    // A session of this runner: the events queued before this request reach Home first (a few seconds at most;
    // events of a running turn queued later do not hold it, nor a tunnel that is down).
    let local = trace.as_deref().is_some_and(|id| s.app.sessions.get(id).is_some());
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
                ToRunner::Welcome { compat, acked, .. } => break (compat, acked),
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
    let (res_tx, mut res_rx) = tokio::sync::mpsc::unbounded_channel::<FromRunner>();
    *HOME.tx.lock().unwrap() = Some(res_tx.clone());
    // Turns not saved on Home yet go now (`turn_sync`).
    launchpad_core::turn_sync::kick();
    let result = serve(app, router, &mut tx, &mut rx, res_tx, &mut res_rx, acked).await;
    // Requests to Home fail now instead of timing out, and terminal viewers of Home's windows end.
    HOME.clear();
    crate::terminals::home_gone();
    result
}

async fn serve(app: &App, router: &Router, tx: &mut crate::wire::WireTx, rx: &mut crate::wire::WireRx, res_tx: tokio::sync::mpsc::UnboundedSender<FromRunner>, res_rx: &mut tokio::sync::mpsc::UnboundedReceiver<FromRunner>, mut acked: u64) -> anyhow::Result<Option<String>> {
    let text = |m: &FromRunner| serde_json::to_string(m).unwrap_or_default();
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
            delivered = bus.recv() => {
                let changed = match delivered {
                    Ok(d) => d.text.contains(r#""topic":"terminals""#),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if changed {
                    tx.send(text(&FromRunner::Terminals)).await?;
                }
            }
            res = res_rx.recv() => {
                let Some(res) = res else { break };
                tx.send(text(&res)).await?;
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
                        tx.send(text(&FromRunner::Items { items })).await?;
                    }
                }
            }
            _ = home_link::guard_changed() => {
                let now = home_link::guard(&app.paths);
                if now != guard {
                    guard = now;
                    tx.send(text(&FromRunner::Guard { guard: guard.clone() })).await?;
                }
            }
            _ = ping.tick() => {
                anyhow::ensure!(last_heard.elapsed() < PING * 3, "Home stopped answering");
                tx.send(text(&FromRunner::Ping)).await?;
            }
            frame = rx.next() => {
                let Some(frame) = frame else { return Ok(None) };
                let t = frame?;
                last_heard = tokio::time::Instant::now();
                match serde_json::from_str::<ToRunner>(&t) {
                    Ok(ToRunner::Req { id, method, path, body, headers }) => {
                        let (app, router, res_tx) = (app.clone(), router.clone(), res_tx.clone());
                        tokio::spawn(async move {
                            let (status, body) = local(&app, &router, &method, &path, body, &headers).await.unwrap_or_else(|e| (500, serde_json::json!({"error": format!("{e:#}")}).to_string()));
                            let _ = res_tx.send(FromRunner::Res { id, status, body });
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
                    Ok(ToRunner::RenewLeases { login }) => launchpad_core::contain::renew_soon(app, &login),
                    Ok(ToRunner::Refuse { reason }) => return Ok(Some(reason)),
                    Ok(msg @ (ToRunner::HttpHead { .. } | ToRunner::HttpBody { .. })) => http_from_home(msg),
                    Ok(ToRunner::WsOpen { id, path }) => crate::terminals::open_for_home(app, id, &path, res_tx.clone()),
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

/// Runs an API request against this daemon's own router, as its local owner.
pub(crate) async fn local(app: &App, router: &Router, method: &str, path: &str, body: Option<String>, headers: &[(String, String)]) -> anyhow::Result<(u16, String)> {
    local_as(app, router, method, path, body, headers, None).await
}

/// `local`, for the agent session `agent` (Home: a runner's request that names one of its sessions).
pub(crate) async fn local_as(app: &App, router: &Router, method: &str, path: &str, body: Option<String>, headers: &[(String, String)], agent: Option<String>) -> anyhow::Result<(u16, String)> {
    anyhow::ensure!(path.starts_with("/api/"), "only API requests go through the tunnel");
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
    if let Some(a) = agent {
        req.extensions_mut().insert(launchpad_http::AgentSession(a));
    }
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

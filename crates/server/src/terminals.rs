//! Integrated terminals. Output goes over one WebSocket per viewer (`/api/terminals/{id}/ws`), never
//! over the global one: binary frames carry output and keystrokes, text frames carry JSON control messages.
//!
//! Several viewers share one PTY size. `reset` and `size` messages carry the effective rows/columns and
//! `owner` (`self`, `other` or `none`). Every viewer reports the size it would fit (`resize`); only the
//! owner's applies. A viewer becomes the owner when it is the first one, when it sends `claim` (the user
//! typed, pasted or clicked Use this size) or when it is the only one left after the owner disconnected.
//!
//! Output offsets count every byte the terminal ever produced. `reset` (clear the screen, then the
//! retained buffer) and `resume` (continue without clearing) carry the `offset` of the next binary
//! byte. A viewer that reconnects passes `?offset=<bytes it already wrote to its emulator>`; it gets
//! `resume` and only the missing bytes when the buffer still holds that offset, `reset` otherwise.
//! `{"type": "ping"}` arrives every 25 s so proxies keep idle sockets open.
//!
//! Terminals run on runners; Home runs none. Home relays its windows' calls (`home`, from `runners::relay`) to
//! the runner of the terminal, session or folder, and each viewer WebSocket over the runner's tunnel
//! (`ToRunner::WsOpen`, then `WsFrame` both ways): the runner serves it like a local viewer (`open_for_home`), so
//! the PTY, its 1 MB buffer and the per-viewer flow control stay on the runner. Home tags each answer with its
//! `runner_id` and remembers where each terminal and reported folder is (`ROUTES`), so later calls find it.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use crate::auth::Access;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use launchpad_core::App;
use launchpad_core::runners::proto::{FromRunner, ToRunner};
use launchpad_core::terminals::{Output, TerminalFolders, TerminalInfo, Terminals};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::error::{ApiError, ApiResult};
use crate::Server;

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/terminals", get(list).post(open))
        .route("/api/terminals/close-all", post(close_all))
        .route("/api/terminals/folders", get(folders))
        .route("/api/terminals/{id}/restart", post(restart))
        .route("/api/terminals/{id}", axum::routing::delete(remove))
        .route("/api/terminals/{id}/close", post(close))
        .route("/api/terminals/{id}/ws", get(ws))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> Json<Vec<TerminalInfo>> {
    Json(s.app.terminals.list())
}

#[derive(Deserialize)]
struct Open {
    #[serde(default)]
    cwd: String,
    command: Option<String>,
    language: Option<String>,
    /// A Git folder of Home, named by Home for a shell without an agent: this runner's copy of it.
    project: Option<String>,
}

async fn open(State(s): State<Server>, Json(b): Json<Open>) -> ApiResult<TerminalInfo> {
    let cwd = match &b.project {
        Some(id) => folder_copy(&s.app, id).await.map_err(bad)?,
        None => b.cwd.clone(),
    };
    Ok(Json(s.app.terminals.open_shell(&s.app, &cwd, b.command.as_deref(), b.language.as_deref()).await.map_err(bad)?))
}

/// A runner: its copy of a Git folder of Home (the repository cache, brought up to date: a read-only view of the
/// default branch), for a shell without an agent. A plain folder's files change only through its agents.
async fn folder_copy(app: &App, id: &str) -> anyhow::Result<String> {
    let p = launchpad_core::projects::get(app, id).await?.context("no such folder")?;
    anyhow::ensure!(p.home_git, "{}'s files change through its agents: open a terminal from one of its agents", p.name);
    Ok(launchpad_core::repo_cache::sync(app, &p).await?.to_string_lossy().into_owned())
}

async fn close(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.terminals.close(&id).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct FolderQuery {
    #[serde(default)]
    cwd: String,
    /// A Git folder of Home, named by Home: this runner's copy of it.
    project: Option<String>,
}

/// The folder menu of a Terminal button: the folder, its project's checkout and other worktrees.
async fn folders(State(s): State<Server>, Query(q): Query<FolderQuery>) -> ApiResult<TerminalFolders> {
    let cwd = match &q.project {
        Some(id) => folder_copy(&s.app, id).await.map_err(bad)?,
        None => q.cwd.clone(),
    };
    Ok(Json(launchpad_core::terminals::folders(&s.app, &cwd).await.map_err(bad)?))
}

async fn restart(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<TerminalInfo> {
    Ok(Json(s.app.terminals.restart(&s.app, &id).await.map_err(bad)?))
}

/// Stops every terminal in every workspace and removes all tabs.
async fn close_all(State(s): State<Server>) -> ApiResult<Value> {
    s.app.terminals.close_and_remove_all(&s.app).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.terminals.remove(&s.app, &id).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Resume {
    offset: Option<u64>,
}

async fn ws(State(s): State<Server>, Path(id): Path<String>, Query(resume): Query<Resume>, Extension(access): Extension<Access>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    let ws = ws.max_message_size(launchpad_core::terminals::INPUT_CHUNK);
    // Home: the viewer goes to the terminal's runner through its tunnel.
    if s.app.paths.role == launchpad_core::role::Role::Home {
        let runner = runner_of(&s, &id).await?;
        usable(&s, &runner).await?;
        let path = match resume.offset {
            Some(o) => format!("/api/terminals/{id}/ws?offset={o}"),
            None => format!("/api/terminals/{id}/ws"),
        };
        return Ok(ws.on_upgrade(move |socket| relay_viewer(s, runner, path, socket, access)));
    }
    if !s.app.terminals.exists(&id) {
        return Err(ApiError::not_found("no such terminal"));
    }
    Ok(ws.on_upgrade(move |socket| {
        let (tx, rx) = socket.split();
        serve(s.app.terminals.clone(), id, tx, rx, access, resume.offset)
    }))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { rows: u16, cols: u16 },
    Claim,
    Ack { epoch: u64, bytes: usize },
}

// Bounds data queued across the WebSocket and xterm parser, even when socket sends are fast.
// This is per viewer: a hidden/slow window must never stop the shared PTY or another viewer.
const OUTPUT_WINDOW: usize = 256 * 1024;
const OUTPUT_CHUNK: usize = 16 * 1024;

#[derive(Default)]
struct OutputWindow {
    epoch: u64,
    pending: usize,
}

impl OutputWindow {
    fn reset(&mut self) {
        self.epoch += 1;
        self.pending = 0;
    }

    fn available(&self) -> bool {
        self.pending <= OUTPUT_WINDOW - OUTPUT_CHUNK
    }

    fn acknowledge(&mut self, epoch: u64, bytes: usize) {
        // Old callbacks after a resync cannot grant credit for the new snapshot.
        if epoch == self.epoch && bytes <= self.pending {
            self.pending -= bytes;
        }
    }
}

fn owner_label(owner: Option<u64>, me: u64) -> &'static str {
    match owner {
        Some(o) if o == me => "self",
        Some(_) => "other",
        None => "none",
    }
}

/// Releases the viewer's size ownership however the connection ends.
struct Viewer {
    terminals: launchpad_core::terminals::Terminals,
    id: String,
    me: u64,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.terminals.leave(&self.id, self.me);
    }
}

/// One viewer of a terminal of this daemon: a window's WebSocket, or a window of Home through the tunnel.
async fn serve<T, R, E1, E2>(terminals: Terminals, id: String, mut tx: T, mut rx: R, mut access: Access, mut from: Option<u64>)
where
    T: Sink<Message, Error = E1> + Unpin,
    R: Stream<Item = Result<Message, E2>> + Unpin,
{
    let Ok(me) = terminals.join(&id) else { return };
    let _viewer = Viewer { terminals: terminals.clone(), id: id.clone(), me };
    let mut window = OutputWindow::default();
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    let ping_every = std::time::Duration::from_secs(25);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    // A viewer that falls behind resumes from the last byte sent, or gets a fresh snapshot ("reset")
    // when the buffer no longer holds it, instead of a gap.
    'attach: loop {
        if !access.valid() { return }
        let Ok(mut a) = terminals.attach(&id, from) else { return };
        window.reset();
        // The offset after the last byte sent to this viewer.
        let mut sent = a.offset;
        let kind = if a.resumed { "resume" } else { "reset" };
        let reset = json!({"type": kind, "epoch": window.epoch, "offset": a.offset, "rows": a.rows, "cols": a.cols, "owner": owner_label(a.owner, me)}).to_string();
        if tx.send(Message::Text(reset.into())).await.is_err() {
            return;
        }
        let exit = |code: Option<u32>| Message::Text(json!({"type": "exit", "code": code}).to_string().into());
        let mut snapshot = a.snapshot.chunks(OUTPUT_CHUNK);
        let mut replaying = true;
        loop {
            if !access.valid() { return }
            // Snapshot chunks obey the same credit limit as live output. Keep receiving input,
            // resize and ACK controls while either stream is waiting for renderer capacity.
            if replaying && window.available() {
                if let Some(chunk) = snapshot.next() {
                    window.pending += chunk.len();
                    sent += chunk.len() as u64;
                    if tx.send(Message::Binary(chunk.to_vec().into())).await.is_err() {
                        return;
                    }
                    continue;
                }
                replaying = false;
                if tx.send(Message::Text(json!({"type": "ready"}).to_string().into())).await.is_err() {
                    return;
                }
                if let Some(code) = a.exit
                    && tx.send(exit(code)).await.is_err() {
                    return;
                }
            }
            tokio::select! {
                _ = access.changes.changed() => { if !access.valid() { return } }
                _ = expiry.tick() => { if !access.valid() { return } }
                _ = ping.tick() => {
                    if tx.send(Message::Text(r#"{"type":"ping"}"#.into())).await.is_err() {
                        return;
                    }
                }
                out = a.rx.recv(), if !replaying && window.available() => {
                    let msg = match out {
                        Ok(Output::Data(d)) => {
                            window.pending += d.len();
                            sent += d.len() as u64;
                            Message::Binary(d.to_vec().into())
                        }
                        Ok(Output::Exit(code)) => exit(code),
                        Ok(Output::Size { rows, cols, owner }) => Message::Text(json!({"type": "size", "rows": rows, "cols": cols, "owner": owner_label(owner, me)}).to_string().into()),
                        Err(RecvError::Lagged(_)) => {
                            from = Some(sent);
                            continue 'attach;
                        }
                        // The terminal was removed.
                        Err(RecvError::Closed) => return,
                    };
                    if tx.send(msg).await.is_err() {
                        return;
                    }
                }
                msg = rx.next() => match msg {
                    Some(Ok(Message::Binary(b))) => {
                        if !access.valid() { return }
                        if terminals.write(&id, &b).is_err() {
                            let _ = tx.send(Message::Text(json!({"type": "error", "message": "Terminal input was interrupted: queue full or terminal closed. Check the terminal before retrying."}).to_string().into())).await;
                            return;
                        }
                    }
                    Some(Ok(Message::Text(t))) => {
                        if !access.valid() { return }
                        match serde_json::from_str::<Control>(&t) {
                        Ok(Control::Resize { rows, cols }) => { let _ = terminals.resize(&id, me, rows, cols); }
                        Ok(Control::Claim) => { let _ = terminals.claim(&id, me); }
                        Ok(Control::Ack { epoch, bytes }) => window.acknowledge(epoch, bytes),
                        Err(e) => tracing::debug!("terminal control message: {e}"),
                        }
                    },
                    Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return,
                },
            }
        }
    }
}

// ---------------------------------------------------------------- Home: relay to runners

/// Home: the runner of each terminal and of each folder path runners reported (terminal folders, folder
/// menus), filled from their answers. Lost on restart: the next list fills it again.
#[derive(Default)]
struct Routes {
    terminals: HashMap<String, String>,
    paths: HashMap<String, String>,
}

static ROUTES: LazyLock<Mutex<Routes>> = LazyLock::new(Default::default);

/// Sets `runner_id` on a runner's answer (a terminal or a folder menu) and remembers its terminal and paths.
fn tag(runner: &str, mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("runner_id".into(), json!(runner));
    }
    let mut routes = ROUTES.lock().unwrap();
    if v.get("kind").is_some()
        && let Some(id) = v["id"].as_str()
    {
        routes.terminals.insert(id.to_string(), runner.to_string());
    }
    let mut paths: Vec<&str> = [&v["cwd"], &v["default"], &v["current"]["path"], &v["checkout"]["path"]].into_iter().filter_map(Value::as_str).collect();
    if let Some(list) = v["worktrees"].as_array() {
        paths.extend(list.iter().filter_map(|w| w["path"].as_str()));
    }
    for p in paths.into_iter().filter(|p| !p.is_empty()) {
        routes.paths.insert(p.to_string(), runner.to_string());
    }
    drop(routes);
    v
}

/// Home: a runner's answer to Continue in TUI (`/api/sessions/{id}/terminal`), tagged like the others.
pub(crate) fn tagged(runner: &str, body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(v) => tag(runner, v).to_string(),
        Err(_) => body.to_string(),
    }
}

/// Home: a runner that can open terminals for a window now, or why not.
async fn usable(s: &Server, runner: &str) -> Result<(), ApiError> {
    let name = launchpad_core::runners::runner_name(&s.app, runner).await;
    if !s.app.runners.connected(runner) {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} is offline: its terminals open when it reconnects")));
    }
    if !launchpad_core::runners::get(&s.app, runner).await.is_some_and(|r| r.capabilities.terminals) {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, format!("{name} needs an update to open terminals from Home")));
    }
    Ok(())
}

/// Home: where a new shell or a folder menu goes, and the Git folder of Home to open there (`project`, when `cwd`
/// is Home's own path of the folder): `?runner=`, the runner of `?session=`, of a temporary folder, of a path a
/// runner reported, or of an agent working in `cwd`; for a Git folder without an agent, a runner picked like
/// for a launch (the least busy that takes new sessions, else any connected one). Never Burst.
async fn place(s: &Server, cwd: &str, query: &HashMap<String, String>) -> Result<(String, Option<String>), ApiError> {
    let app = &s.app;
    let given = |k: &str| query.get(k).filter(|v| !v.is_empty()).cloned();
    if let Some(r) = given("runner") {
        usable(s, &r).await?;
        return Ok((r, None));
    }
    if let Some(id) = given("session") {
        let r = app.sessions.get(&id).and_then(|x| x.runner_id).ok_or_else(|| ApiError::bad_request("this agent runs on no runner"))?;
        usable(s, &r).await?;
        return Ok((r, None));
    }
    if cwd.is_empty() {
        return Err(ApiError::bad_request("cwd required"));
    }
    let projects = launchpad_core::projects::list(app).await.unwrap_or_default();
    if let Some(p) = projects.iter().find(|p| p.path == cwd) {
        if p.is_temp {
            let r = p.runner_id.clone().ok_or_else(|| ApiError::bad_request(format!("{} is on no runner", p.name)))?;
            usable(s, &r).await?;
            return Ok((r, None));
        }
        if !p.home_git {
            return Err(ApiError::bad_request(format!("{}'s files change through its agents: open a terminal from one of its agents", p.name)));
        }
        let mut connected = vec![];
        for r in app.runners.connected_ids() {
            if launchpad_core::runners::get(app, &r).await.is_some_and(|x| x.capabilities.terminals && !x.burst) {
                connected.push(r);
            }
        }
        let picked = launchpad_core::runners::pick(app, None).filter(|r| connected.contains(r)).or_else(|| connected.first().cloned());
        let r = picked.ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "No runner is connected: terminals open on a runner"))?;
        return Ok((r, Some(p.id.clone())));
    }
    let known = ROUTES.lock().unwrap().paths.get(cwd).cloned();
    if let Some(r) = known.filter(|r| app.runners.connected(r)) {
        usable(s, &r).await?;
        return Ok((r, None));
    }
    if let Some(r) = launchpad_core::code::runner_for(app, Some(cwd), None).await {
        usable(s, &r).await?;
        return Ok((r, None));
    }
    // A folder of an agent (maybe gone from its runner, or on a runner that is away).
    let inside = |root: &str| cwd == root || cwd.starts_with(&format!("{}/", root.trim_end_matches('/')));
    if let Some(r) = app.sessions.list().into_iter().filter(|x| inside(&x.cwd)).find_map(|x| x.runner_id) {
        usable(s, &r).await?;
        return Ok((r, None));
    }
    Err(ApiError::bad_request(format!("No connected runner holds {cwd}: terminals open on the runner of an agent or folder")))
}

/// Home: a runner's answer as JSON, or its error with its status.
async fn ask(s: &Server, runner: &str, method: &str, path: &str, body: Option<String>) -> Result<Value, ApiError> {
    let (status, text) = s.app.runners.request(runner, method, path, body).await.map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = v["error"].as_str().map(str::to_string).unwrap_or(text);
        return Err(ApiError::new(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), message));
    }
    Ok(v)
}

/// Home: the terminals of every connected runner, tagged with their runner, oldest first.
async fn list_all(s: &Server) -> Vec<Value> {
    let ids = s.app.runners.connected_ids();
    let asks = ids.iter().map(|r| async move { (r.clone(), ask(s, r, "GET", "/api/terminals", None).await) });
    let mut all: Vec<Value> = vec![];
    for (r, answer) in futures_util::future::join_all(asks).await {
        if let Ok(Value::Array(list)) = answer {
            all.extend(list.into_iter().map(|t| tag(&r, t)));
        }
    }
    all.sort_by_key(|t| t["created_at"].as_i64().unwrap_or_default());
    all
}

/// Home: the runner of a terminal, from what Home saw, else from fresh lists.
async fn runner_of(s: &Server, id: &str) -> Result<String, ApiError> {
    let known = ROUTES.lock().unwrap().terminals.get(id).cloned();
    if let Some(r) = known.filter(|r| s.app.runners.connected(r)) {
        return Ok(r);
    }
    list_all(s).await;
    let known = ROUTES.lock().unwrap().terminals.get(id).cloned();
    known.filter(|r| s.app.runners.connected(r)).ok_or_else(|| ApiError::not_found("no such terminal (its runner may be offline)"))
}

/// `?key=value` with the value encoded.
fn with_query(path: &str, key: &str, value: &str) -> String {
    let mut url = reqwest::Url::parse("http://home/").expect("static URL");
    url.set_path(path);
    url.query_pairs_mut().append_pair(key, value);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// Home: every terminal call of a window except the viewer socket (`ws`), relayed to the right runner.
pub(crate) async fn home(s: &Server, req: axum::extract::Request) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = Query::<HashMap<String, String>>::try_from_uri(req.uri()).map(|q| q.0).unwrap_or_default();
    let body = match axum::body::to_bytes(req.into_body(), 1 << 20).await {
        Ok(b) => b,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let result: Result<Value, ApiError> = async {
        match (method.as_str(), parts.as_slice()) {
            ("GET", ["api", "terminals"]) => Ok(Value::Array(list_all(s).await)),
            ("POST", ["api", "terminals"]) => {
                let mut b: Value = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("{e}")))?;
                let cwd = b["cwd"].as_str().unwrap_or_default().to_string();
                let (runner, project) = place(s, &cwd, &query).await?;
                if let Some(p) = project {
                    b["project"] = json!(p);
                    b["cwd"] = json!("");
                }
                Ok(tag(&runner, ask(s, &runner, "POST", "/api/terminals", Some(b.to_string())).await?))
            }
            ("GET", ["api", "terminals", "folders"]) => {
                let cwd = query.get("cwd").cloned().unwrap_or_default();
                let (runner, project) = place(s, &cwd, &query).await?;
                let target = match project {
                    Some(p) => with_query("/api/terminals/folders", "project", &p),
                    None => with_query("/api/terminals/folders", "cwd", &cwd),
                };
                Ok(tag(&runner, ask(s, &runner, "GET", &target, None).await?))
            }
            ("POST", ["api", "terminals", "close-all"]) => {
                let mut errors = vec![];
                for r in s.app.runners.connected_ids() {
                    if let Err(e) = ask(s, &r, "POST", "/api/terminals/close-all", None).await {
                        errors.push(format!("{}: {}", launchpad_core::runners::runner_name(&s.app, &r).await, e.message));
                    }
                }
                if errors.is_empty() { Ok(json!({"ok": true})) } else { Err(ApiError::bad_request(errors.join("; "))) }
            }
            (_, ["api", "terminals", id, rest @ ..]) => {
                let runner = runner_of(s, id).await?;
                let body = (!body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned());
                let v = ask(s, &runner, &method, &path, body).await?;
                Ok(if rest == ["restart"] { tag(&runner, v) } else { v })
            }
            _ => Err(ApiError::not_found("no such terminal route")),
        }
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Home: viewer relays in progress, by relay ID: their runner and the sender of the frames it returns.
static RELAYS: LazyLock<Mutex<HashMap<u64, (String, mpsc::UnboundedSender<Message>)>>> = LazyLock::new(Default::default);
static NEXT_RELAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Ends a relay however the window's socket ends: the runner drops its viewer.
struct Relay {
    id: u64,
    runner: String,
    hub: launchpad_core::runners::Hub,
}

impl Drop for Relay {
    fn drop(&mut self) {
        RELAYS.lock().unwrap().remove(&self.id);
        self.hub.send_to(&self.runner, ToRunner::WsClose { id: self.id });
    }
}

fn b64(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}

/// A tunnel frame as a WebSocket message.
fn message(text: Option<String>, data: Option<String>) -> Option<Message> {
    match (text, data) {
        (Some(t), _) => Some(Message::Text(t.into())),
        (None, Some(d)) => base64::Engine::decode(&base64::engine::general_purpose::STANDARD, d).ok().map(|b| Message::Binary(b.into())),
        (None, None) => None,
    }
}

/// Home: one window's viewer of a runner's terminal, frames passed through the tunnel both ways. The window's
/// access is checked here; the runner's flow control (`ack`) passes through unchanged.
async fn relay_viewer(s: Server, runner: String, path: String, socket: WebSocket, mut access: Access) {
    let id = NEXT_RELAY.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Message>();
    RELAYS.lock().unwrap().insert(id, (runner.clone(), in_tx));
    let _relay = Relay { id, runner: runner.clone(), hub: s.app.runners.clone() };
    s.app.runners.send_to(&runner, ToRunner::WsOpen { id, path });
    let (mut tx, mut rx) = socket.split();
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        if !access.valid() {
            return;
        }
        tokio::select! {
            _ = access.changes.changed() => {}
            _ = expiry.tick() => {}
            out = in_rx.recv() => match out {
                Some(m) => if tx.send(m).await.is_err() { return },
                // The runner closed the viewer, or its tunnel ended: the window reconnects.
                None => return,
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => s.app.runners.send_to(&runner, ToRunner::WsFrame { id, text: Some(t.to_string()), data: None }),
                Some(Ok(Message::Binary(b))) => s.app.runners.send_to(&runner, ToRunner::WsFrame { id, text: None, data: Some(b64(&b)) }),
                Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Home: a frame of a relayed viewer, from its runner.
pub(crate) fn from_runner(runner: &str, id: u64, text: Option<String>, data: Option<String>) {
    let relays = RELAYS.lock().unwrap();
    if let Some((r, tx)) = relays.get(&id)
        && r == runner
        && let Some(m) = message(text, data)
    {
        let _ = tx.send(m);
    }
}

/// Home: the runner closed a relayed viewer.
pub(crate) fn closed_by_runner(runner: &str, id: u64) {
    let mut relays = RELAYS.lock().unwrap();
    if relays.get(&id).is_some_and(|(r, _)| r == runner) {
        relays.remove(&id);
    }
}

/// Home: a runner's tunnel ended: its relayed viewers close (windows reconnect once it is back).
pub(crate) fn runner_gone(runner: &str) {
    RELAYS.lock().unwrap().retain(|_, (r, _)| r != runner);
}

// ---------------------------------------------------------------- Runner: viewers for Home's windows

/// A runner: viewers Home opened through the tunnel, by relay ID: the sender of the window's frames.
static VIEWERS: LazyLock<Mutex<HashMap<u64, mpsc::UnboundedSender<Message>>>> = LazyLock::new(Default::default);

/// A runner: `ToRunner::WsOpen`. Serves a terminal viewer whose frames go to Home (`out`) as `WsFrame`s.
pub(crate) fn open_for_home(app: &App, id: u64, path: &str, out: mpsc::UnboundedSender<FromRunner>) {
    let (bare, query) = path.split_once('?').unwrap_or((path, ""));
    let terminal = bare.strip_prefix("/api/terminals/").and_then(|r| r.strip_suffix("/ws")).filter(|t| !t.is_empty() && !t.contains('/')).map(str::to_string);
    let Some(terminal) = terminal.filter(|t| app.terminals.exists(t)) else {
        let _ = out.send(FromRunner::WsClose { id, error: Some("no such terminal".into()) });
        return;
    };
    let from = query.split('&').find_map(|kv| kv.strip_prefix("offset=")).and_then(|v| v.parse::<u64>().ok());
    let (in_tx, in_rx) = mpsc::unbounded_channel::<Message>();
    VIEWERS.lock().unwrap().insert(id, in_tx);
    let rx = Box::pin(futures_util::stream::unfold(in_rx, |mut rx| async move { rx.recv().await.map(|m| (Ok::<_, anyhow::Error>(m), rx)) }));
    let sink_out = out.clone();
    let tx = Box::pin(futures_util::sink::unfold(sink_out, move |out, m: Message| async move {
        let frame = match m {
            Message::Text(t) => Some(FromRunner::WsFrame { id, text: Some(t.to_string()), data: None }),
            Message::Binary(b) => Some(FromRunner::WsFrame { id, text: None, data: Some(b64(&b)) }),
            _ => None,
        };
        if let Some(f) = frame {
            out.send(f).map_err(|_| anyhow::anyhow!("Home disconnected"))?;
        }
        Ok::<_, anyhow::Error>(out)
    }));
    let terminals = app.terminals.clone();
    tokio::spawn(async move {
        serve(terminals, terminal, tx, rx, Access::through_home(), from).await;
        VIEWERS.lock().unwrap().remove(&id);
        let _ = out.send(FromRunner::WsClose { id, error: None });
    });
}

/// A runner: a frame of a window, from Home.
pub(crate) fn frame_from_home(id: u64, text: Option<String>, data: Option<String>) {
    if let Some(tx) = VIEWERS.lock().unwrap().get(&id)
        && let Some(m) = message(text, data)
    {
        let _ = tx.send(m);
    }
}

/// A runner: the window closed its viewer.
pub(crate) fn close_from_home(id: u64) {
    VIEWERS.lock().unwrap().remove(&id);
}

/// A runner: the tunnel to Home ended, so every viewer for Home's windows ends.
pub(crate) fn home_gone() {
    VIEWERS.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_waits_for_parser_credit_and_ignores_stale_or_invalid_acks() {
        let mut window = OutputWindow::default();
        window.reset();
        window.pending = OUTPUT_WINDOW;
        assert!(!window.available());
        window.acknowledge(window.epoch, OUTPUT_CHUNK);
        assert!(window.available());
        window.reset();
        window.pending = OUTPUT_CHUNK;
        window.acknowledge(window.epoch - 1, OUTPUT_CHUNK);
        window.acknowledge(window.epoch, OUTPUT_CHUNK + 1);
        assert_eq!(window.pending, OUTPUT_CHUNK);
        window.acknowledge(window.epoch, OUTPUT_CHUNK);
        assert_eq!(window.pending, 0);
    }
}

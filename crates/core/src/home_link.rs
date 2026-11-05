//! A runner's link to its Home: the credential from `launchpad connect`, the runner guard (local settings that
//! never sync to Home), and the state of the tunnel shown in the status.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::paths::Paths;
use crate::role::Compat;
use crate::runners::Guard;

/// `<data_dir>/home.json`, mode 0600.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    /// Home's address, for example `https://wauplin-launchpad.hf.space`.
    pub url: String,
    pub id: String,
    pub secret: String,
}

impl Link {
    pub fn credential(&self) -> String {
        format!("{}.{}", self.id, self.secret)
    }
}

fn link_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir.join("home.json")
}

fn guard_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir.join("guard.json")
}

pub fn load(paths: &Paths) -> Option<Link> {
    if let Some(b) = burst() {
        // A Burst runner reaches Home only through its tunnel: Home Git and plain folders go to a local relay.
        let port = *LOCAL_HOME.get()?;
        return Some(Link { url: format!("http://127.0.0.1:{port}"), id: b.id.clone(), secret: b.secret.clone() });
    }
    serde_json::from_slice(&std::fs::read(link_file(paths)).ok()?).ok()
}

/// A Burst runner's identity, from the job's secrets: Home created the runner and dials it.
#[derive(Clone, Debug)]
pub struct Burst {
    pub id: String,
    pub secret: String,
    /// Home's address, for links only (the job cannot reach a private Home).
    pub home_url: String,
}

/// `LAUNCHPAD_BURST_CREDENTIAL` (`<runner id>.<secret>`) marks a Burst runner.
pub fn burst() -> Option<&'static Burst> {
    static BURST: std::sync::OnceLock<Option<Burst>> = std::sync::OnceLock::new();
    BURST
        .get_or_init(|| {
            let c = std::env::var("LAUNCHPAD_BURST_CREDENTIAL").ok()?;
            let (id, secret) = c.trim().split_once('.')?;
            Some(Burst { id: id.into(), secret: secret.into(), home_url: std::env::var("LAUNCHPAD_HOME_URL").unwrap_or_default() })
        })
        .as_ref()
}

static LOCAL_HOME: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// Burst runner: the port of the local relay to Home (set once at startup).
pub fn set_local_home(port: u16) {
    let _ = LOCAL_HOME.set(port);
}

pub fn save(paths: &Paths, link: &Link) -> Result<()> {
    paths.ensure_dirs()?;
    crate::paths::write_private(&link_file(paths), &serde_json::to_string_pretty(link)?).context("save the Home link")
}

pub fn guard(paths: &Paths) -> Guard {
    std::fs::read(guard_file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

static GUARD_CHANGED: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

/// Resolves after the guard changes (the tunnel reports it to Home at once).
pub async fn guard_changed() {
    GUARD_CHANGED.notified().await
}

pub fn set_guard(app: &App, guard: &Guard) -> Result<()> {
    std::fs::write(guard_file(&app.paths), serde_json::to_vec_pretty(guard)?)?;
    GUARD_CHANGED.notify_one();
    app.status.update(|s| {
        if let Some(h) = s.home.as_mut() {
            h.guard = guard.clone();
        }
    });
    app.bus.publish("runner", "guard", serde_json::to_value(guard)?);
    Ok(())
}

/// A launch that came from another device through Home, in the runner's local log.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RemoteLaunch {
    #[ts(type = "number")]
    pub at: i64,
    pub session_id: String,
    pub title: String,
}

fn launches_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir.join("launches-from-home.jsonl")
}

/// Appends to the local log of launches started elsewhere (kept to the last 200).
pub fn log_launch(paths: &Paths, s: &crate::model::Session) {
    let mut all = launches(paths);
    all.push(RemoteLaunch { at: s.started_at, session_id: s.id.clone(), title: s.title.clone() });
    let keep = all.len().saturating_sub(200);
    let text: String = all[keep..].iter().filter_map(|l| serde_json::to_string(l).ok()).map(|l| l + "\n").collect();
    if let Err(e) = std::fs::write(launches_file(paths), text) {
        tracing::warn!("launch log: {e}");
    }
}

pub fn launches(paths: &Paths) -> Vec<RemoteLaunch> {
    std::fs::read_to_string(launches_file(paths)).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    /// No `launchpad connect` yet.
    NotConnected,
    Connecting,
    Connected,
    /// Home refused the runner (pending approval, removed, incompatible): `message` says why.
    Refused,
    /// Home is unreachable; the runner retries.
    Offline,
}

/// The runner's view of its Home, in the status.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct HomeStatus {
    pub url: Option<String>,
    pub state: TunnelState,
    pub message: Option<String>,
    pub compat: Option<Compat>,
    pub guard: Guard,
}

pub fn set_state(app: &App, url: Option<String>, state: TunnelState, message: Option<String>, compat: Option<Compat>) {
    let guard = guard(&app.paths);
    app.status.update(|s| s.home = Some(HomeStatus { url, state, message, compat, guard }));
}

/// The HF token sent to a Home in a private Space (the Space's sign-in), when there is one: `HF_TOKEN`, or the
/// `hf auth login` token of this machine.
pub fn space_token(url: &str) -> Option<String> {
    let host = url.split("://").nth(1)?.split('/').next()?;
    if !host.ends_with(".hf.space") {
        return None;
    }
    if let Ok(t) = std::env::var("HF_TOKEN").map(|t| t.trim().to_string())
        && !t.is_empty()
    {
        return Some(t);
    }
    let home = std::env::var("HF_HOME").map(std::path::PathBuf::from).ok().or_else(|| dirs::home_dir().map(|h| h.join(".cache/huggingface")))?;
    std::fs::read_to_string(home.join("token")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

type Reply = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(u16, String)>> + Send>>;
type Requester = Box<dyn Fn(String, String, Option<String>) -> Reply + Send + Sync>;
static REQUESTER: std::sync::OnceLock<Requester> = std::sync::OnceLock::new();

/// Runner: how core services send API requests to Home (the tunnel, set by the server).
pub fn set_requester(r: Requester) {
    let _ = REQUESTER.set(r);
}

/// A body streamed through the tunnel.
pub type ByteStream = std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>;

/// A `ByteStream` that is also `Sync` (for hyper bodies): only polled through `&mut`, so the lock is free.
pub struct SyncStream(std::sync::Mutex<ByteStream>);

impl SyncStream {
    pub fn new(s: ByteStream) -> Self {
        SyncStream(std::sync::Mutex::new(s))
    }
}

impl futures_util::Stream for SyncStream {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let inner = self.get_mut().0.get_mut().unwrap_or_else(|e| e.into_inner());
        inner.as_mut().poll_next(cx)
    }
}

/// The answer of an HTTP exchange through Home.
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ByteStream,
}

type HttpFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<HttpReply>> + Send>>;
type Streamer = Box<dyn Fn(crate::runners::proto::HttpTarget, String, String, Vec<(String, String)>, ByteStream) -> HttpFuture + Send + Sync>;
static STREAMER: std::sync::OnceLock<Streamer> = std::sync::OnceLock::new();

/// Runner: how core services open HTTP exchanges through the tunnel (set by the server).
pub fn set_streamer(s: Streamer) {
    let _ = STREAMER.set(s);
}

/// Runner: an HTTP exchange through Home (`HttpTarget`), with both bodies streamed.
pub async fn http(target: crate::runners::proto::HttpTarget, method: &str, path: &str, headers: Vec<(String, String)>, body: ByteStream) -> Result<HttpReply> {
    let s = STREAMER.get().context("this runner is not connected to a Home: run launchpad connect")?;
    s(target, method.into(), path.into(), headers, body).await
}

type CallReply = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(u16, String)>> + Send>>;
type Caller = Box<dyn Fn(String, serde_json::Value) -> CallReply + Send + Sync>;
static CALLER: std::sync::OnceLock<Caller> = std::sync::OnceLock::new();

pub fn set_caller(c: Caller) {
    let _ = CALLER.set(c);
}

/// Runner: a call about its own sessions (`vault::runner_call` on Home), through the tunnel.
pub async fn home_call(op: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let c = CALLER.get().context("this runner is not connected to a Home: run launchpad connect")?;
    let (status, text) = c(op.into(), body).await?;
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    if !(200..300).contains(&status) {
        anyhow::bail!("{}", value["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("Home answered {status}")));
    }
    Ok(value)
}

/// Runner: an API request to Home through the tunnel. Errors carry Home's message.
pub async fn home_request(method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value> {
    let r = REQUESTER.get().context("this daemon has no Home")?;
    let (status, text) = r(method.into(), path.into(), body.map(|b| b.to_string())).await?;
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    if !(200..300).contains(&status) {
        anyhow::bail!("{}", value["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("Home answered {status}")));
    }
    Ok(value)
}

/// Home refreshes a folder's mirror from its remote (created on first use). Local on Home and `all`.
pub async fn refresh_mirror(app: &App, project_id: &str) -> Result<()> {
    if crate::home_git::here(app) {
        let p = crate::projects::get(app, project_id).await?.context("no such folder")?;
        let existed = crate::home_git::mirror(app, project_id).join("HEAD").exists();
        crate::home_git::ensure(app, &p).await?;
        if existed {
            crate::home_git::refresh(app, &p).await?;
        }
        return Ok(());
    }
    home_request("POST", &format!("/api/home-git/{project_id}/refresh"), None).await.map(|_| ())
}

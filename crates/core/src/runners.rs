//! Runners as seen from Home: approved machines that run agents, and their live tunnels.
//!
//! Pairing a runner is the only pairing. `launchpad connect <home>` asks Home for a request with a short code and
//! a secret only the runner knows; the owner compares the code and clicks Approve in Settings → Home and runners.
//! The runner then holds a credential (Home keeps its SHA-256) and keeps one outbound WebSocket to Home.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use ts_rs::TS;

use crate::App;
use crate::role::Compat;
use crate::util::now_ms;

/// A pairing request expires after ten minutes; a few may wait at once.
const REQUEST_TTL_MS: i64 = 10 * 60 * 1000;
const MAX_PENDING: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerState {
    Pending,
    Approved,
}

/// The settings a runner keeps locally (never synced to Home) and reports when they change.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Guard {
    /// Launches from other devices (phone, schedules, bots). Off: only windows on that machine launch there.
    pub accept_launches: bool,
    /// Pause runner: no new session starts there.
    pub paused: bool,
}

impl Default for Guard {
    fn default() -> Self {
        Guard { accept_launches: true, paused: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Runner {
    pub id: String,
    pub name: String,
    pub os: String,
    /// Shown by `launchpad connect` and in Settings while pending, so the owner approves the right machine.
    pub code: String,
    pub state: RunnerState,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub approved_at: Option<i64>,
    #[ts(type = "number | null")]
    pub last_seen_at: Option<i64>,
    /// Build and protocol of its last connection.
    pub commit: String,
    pub protocol: u32,
    /// What the runner reported on its last connection.
    #[serde(default)]
    pub capabilities: Capabilities,
    /// A Burst runner: an HF Job started by Home for one session, which Home dials (`crate::burst`).
    #[serde(default)]
    pub burst: bool,
}

/// What a runner offers, reported on connect.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Capabilities {
    /// A Local VM: the virtual machine contains the agents (no jail inside).
    pub vm: bool,
    /// Docker works for agents there (only on a Local VM: Docker needs root).
    pub docker: bool,
    /// A Burst runner (HF Job): the job contains the agents, Home adds every token.
    pub burst: bool,
    /// Updates itself to Home's build on Home's request (`self_update`).
    pub self_update: bool,
}

/// This runner's capabilities.
pub async fn capabilities(app: &App) -> Capabilities {
    let burst = crate::home_link::burst().is_some();
    let vm = !burst && app.config.get().runner.boundary == crate::config::Boundary::Vm;
    let docker = vm
        && tokio::time::timeout(std::time::Duration::from_secs(5), tokio::process::Command::new("docker").arg("info").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status())
            .await
            .ok()
            .and_then(|r| r.ok())
            .is_some_and(|s| s.success());
    Capabilities { vm, docker, burst, self_update: crate::self_update::supported(app) }
}

/// A runner with its live state, for Settings.
#[derive(Clone, Debug, Serialize, TS)]
pub struct RunnerStatus {
    pub runner: Runner,
    pub connected: bool,
    /// Build comparison while connected.
    pub compat: Option<Compat>,
    pub guard: Option<Guard>,
    /// The last network check the owner ran on it (`netcap::network_check`), until Home restarts.
    pub network_check: Option<crate::netcap::NetworkCheck>,
}

fn hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes: [u8; 6] = rand::random();
    let c: String = bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    format!("{}-{}", &c[..3], &c[3..])
}

pub(crate) async fn load(app: &App) -> Result<Vec<(Runner, String)>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json, secret_hash FROM runner")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|(j, h)| serde_json::from_str(&j).ok().map(|r| (r, h))).collect())
        })
        .await
}

async fn save(app: &App, r: &Runner, secret_hash: Option<String>) -> Result<()> {
    let (id, json) = (r.id.clone(), serde_json::to_string(r)?);
    app.state_db
        .call(move |c| {
            match secret_hash {
                Some(h) => c.execute("INSERT INTO runner (id, secret_hash, json) VALUES (?1, ?2, ?3) ON CONFLICT (id) DO UPDATE SET json = ?3, secret_hash = ?2", [id, h, json])?,
                None => c.execute("UPDATE runner SET json = ?2 WHERE id = ?1", [id, json])?,
            };
            Ok(())
        })
        .await?;
    app.bus.publish("runners", "changed", serde_json::Value::Null);
    Ok(())
}

fn ensure_home(app: &App) -> Result<()> {
    anyhow::ensure!(app.paths.role == crate::role::Role::Home, "runners connect to a Home (launchpad daemon --role home)");
    Ok(())
}

/// A new pairing request. Returns the runner record and the secret, which only the runner keeps.
pub async fn request(app: &App, name: &str, os: &str) -> Result<(Runner, String)> {
    ensure_home(app)?;
    let all = load(app).await?;
    let now = now_ms();
    // Expired requests go first, so they never block new ones.
    for (r, _) in all.iter().filter(|(r, _)| r.state == RunnerState::Pending && now - r.created_at > REQUEST_TTL_MS) {
        remove(app, &r.id).await?;
    }
    let pending = all.iter().filter(|(r, _)| r.state == RunnerState::Pending && now - r.created_at <= REQUEST_TTL_MS).count();
    anyhow::ensure!(pending < MAX_PENDING, "too many runners are waiting for approval; approve or remove them in Settings");
    let name: String = name.trim().chars().take(60).collect();
    let runner = Runner {
        id: uuid::Uuid::new_v4().to_string(),
        name: if name.is_empty() { "Runner".into() } else { name },
        os: os.chars().take(40).collect(),
        code: code(),
        state: RunnerState::Pending,
        created_at: now,
        approved_at: None,
        last_seen_at: None,
        commit: String::new(),
        protocol: 0,
        capabilities: Capabilities::default(),
        burst: false,
    };
    let secret = hex::encode(rand::random::<[u8; 32]>());
    save(app, &runner, Some(hash(&secret))).await?;
    Ok((runner, secret))
}

/// The runner whose credential is `<id>.<secret>`, approved or not.
pub async fn authenticate(app: &App, credential: &str) -> Option<Runner> {
    let (id, secret) = credential.split_once('.')?;
    let all = load(app).await.ok()?;
    let (runner, h) = all.into_iter().find(|(r, _)| r.id == id)?;
    let expected = hash(secret);
    // Constant-time comparison of the hashes.
    (h.len() == expected.len() && h.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0).then_some(runner)
}

pub async fn approve(app: &App, id: &str) -> Result<Runner> {
    ensure_home(app)?;
    let (mut r, _) = load(app).await?.into_iter().find(|(r, _)| r.id == id).context("no such runner")?;
    if r.state == RunnerState::Pending && now_ms() - r.created_at > REQUEST_TTL_MS {
        bail!("this request expired; run launchpad connect again");
    }
    r.state = RunnerState::Approved;
    r.approved_at = Some(now_ms());
    save(app, &r, None).await?;
    Ok(r)
}

/// The credential secret of a Burst runner: derived from Home's daemon token, so Home can dial its jobs again
/// after a restart without storing it.
fn derive_burst_secret(app: &App, id: &str) -> Result<String> {
    use hmac::{KeyInit, Mac};
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(app.paths.token()?.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    mac.update(format!("launchpad-burst:{id}").as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Home: the record of a new Burst runner, approved at once (Home starts it).
pub async fn create_burst(app: &App) -> Result<(Runner, String)> {
    ensure_home(app)?;
    let now = now_ms();
    let runner = Runner {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Burst".into(),
        os: "linux".into(),
        code: String::new(),
        state: RunnerState::Approved,
        created_at: now,
        approved_at: Some(now),
        last_seen_at: None,
        commit: String::new(),
        protocol: 0,
        capabilities: Capabilities { burst: true, ..Default::default() },
        burst: true,
    };
    let secret = derive_burst_secret(app, &runner.id)?;
    save(app, &runner, Some(hash(&secret))).await?;
    Ok((runner, secret))
}

/// Home: the secret of a Burst runner it started, None for other runners.
pub async fn burst_secret(app: &App, id: &str) -> Result<Option<String>> {
    if !is_burst(app, id).await {
        return Ok(None);
    }
    derive_burst_secret(app, id).map(Some)
}

/// Whether a runner is a Burst runner (Home started it and keeps its tokens).
pub async fn is_burst(app: &App, id: &str) -> bool {
    load(app).await.ok().and_then(|all| all.into_iter().find(|(r, _)| r.id == id)).is_some_and(|(r, _)| r.burst)
}

/// Removes a runner and closes its tunnel. Its sessions stay on Home; those it was running show Interrupted,
/// since no record will come from it again.
pub async fn remove(app: &App, id: &str) -> Result<()> {
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM runner WHERE id = ?1", [key])?)).await?;
    app.runners.disconnect(id);
    let orphans: Vec<String> = app.sessions.list().into_iter().filter(|s| s.runner_id.as_deref() == Some(id) && (s.status.is_running() || s.quota_pause.is_some())).map(|s| s.id).collect();
    for sid in orphans {
        if let Err(e) = app.sessions.end_replica(app, &sid, crate::model::SessionStatus::Interrupted, None).await {
            tracing::warn!("session {sid} of removed runner {id}: {e:#}");
        }
    }
    let _ = take_archives(app, |r, _| r == id);
    app.bus.publish("runners", "changed", serde_json::Value::Null);
    Ok(())
}

// ---------------------------------------------------------------- What a runner learns when it is back

/// Something Home did to a session while its runner was away, delivered once when the runner reconnects.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct PendingArchive {
    runner: String,
    session: String,
    /// None: the owner archived the session. Some: Home moved it on (generation on Home), so the runner saves
    /// what only it has to `lp-scratch/…` and drops its copy (`handoff::release`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    release: Option<u32>,
}

static ARCHIVES: Mutex<()> = Mutex::new(());

fn archives_file(app: &App) -> std::path::PathBuf {
    app.paths.data_dir.join("runner-archives.json")
}

fn read_archives(app: &App) -> Vec<PendingArchive> {
    std::fs::read_to_string(archives_file(app)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_archives(app: &App, list: &[PendingArchive]) -> Result<()> {
    std::fs::write(archives_file(app), serde_json::to_string(list)?)?;
    Ok(())
}

/// Removes and returns the pending items that match.
fn take_archives(app: &App, matches: impl Fn(&str, &str) -> bool) -> Result<Vec<PendingArchive>> {
    let _lock = ARCHIVES.lock().unwrap();
    let (taken, kept): (Vec<_>, Vec<_>) = read_archives(app).into_iter().partition(|a| matches(&a.runner, &a.session));
    if !taken.is_empty() {
        write_archives(app, &kept)?;
    }
    Ok(taken)
}

/// Adds an item for a runner Home knows (a removed one never comes back).
async fn deliver_later(app: &App, item: PendingArchive) -> Result<()> {
    if !load(app).await?.into_iter().any(|(r, _)| r.id == item.runner) {
        return Ok(());
    }
    let _lock = ARCHIVES.lock().unwrap();
    let mut list = read_archives(app);
    list.retain(|a| !(a.runner == item.runner && a.session == item.session && a.release.is_some() && item.release.is_some()));
    if !list.contains(&item) {
        list.push(item);
        write_archives(app, &list)?;
    }
    Ok(())
}

/// Home: Archive of a session whose runner is offline or removed. The record closes on Home now; an offline
/// runner receives the Archive when it reconnects (`deliver_pending`), a removed one never comes back.
pub async fn archive_away(app: &App, id: &str) -> Result<()> {
    let s = app.sessions.get(id).context("no such session")?;
    let runner = s.runner_id.clone().context("this session runs on Home")?;
    anyhow::ensure!(!app.runners.connected(&runner), "its runner is connected");
    deliver_later(app, PendingArchive { runner, session: id.to_string(), release: None }).await?;
    app.sessions.end_replica(app, id, crate::model::SessionStatus::Closed, None).await?;
    Ok(())
}

/// Home: the undo of `archive_away`. The pending Archive is dropped, so the runner never receives it.
pub async fn unarchive_away(app: &App, id: &str) -> Result<()> {
    let s = app.sessions.get(id).context("no such session")?;
    let runner = s.runner_id.clone().context("this session runs on Home")?;
    anyhow::ensure!(!app.runners.connected(&runner), "its runner is connected");
    {
        let _lock = ARCHIVES.lock().unwrap();
        let (dropped, kept): (Vec<_>, Vec<_>) = read_archives(app).into_iter().partition(|a| a.session == id && a.release.is_none());
        if !dropped.is_empty() {
            write_archives(app, &kept)?;
        }
    }
    app.sessions.unarchive(app, id).await?;
    Ok(())
}

/// Home: `runner` still holds an older generation of a session Home moved on while it was away; it releases
/// its copy when it reconnects.
pub async fn release_later(app: &App, runner: &str, session: &str, generation: u32) -> Result<()> {
    deliver_later(app, PendingArchive { runner: runner.to_string(), session: session.to_string(), release: Some(generation) }).await
}

/// Home: a runner that reconnected receives the Archives clicked and the moves made while it was away, once each.
pub async fn deliver_pending(app: &App, runner_id: &str) {
    let Ok(list) = take_archives(app, |r, _| r == runner_id) else { return };
    let mut failed = vec![];
    for a in list {
        let sent = match a.release {
            None => app.runners.request(runner_id, "POST", &format!("/api/sessions/{}/close", a.session), None).await,
            Some(generation) => app.runners.request(runner_id, "POST", &format!("/api/sessions/{}/release", a.session), Some(serde_json::json!({"generation": generation}).to_string())).await,
        };
        match sent {
            // A release that failed (Home did not confirm the leftovers) is tried again at the next connection.
            Ok((status, body)) if status >= 300 && a.release.is_some() => {
                tracing::warn!("Release of {} on its runner: {status} {body}", a.session);
                failed.push(a);
            }
            Ok((status, body)) if status >= 300 => tracing::warn!("Archive of {} on its runner: {status} {body}", a.session),
            Ok(_) => {}
            // The tunnel dropped again: the next connection delivers it.
            Err(_) => failed.push(a),
        }
    }
    if !failed.is_empty() {
        let _lock = ARCHIVES.lock().unwrap();
        let mut list = read_archives(app);
        list.extend(failed);
        let _ = write_archives(app, &list);
    }
}

// ---------------------------------------------------------------- Sessions of a runner that left

/// How long Home waits for a runner that left while its sessions worked or asked a question.
pub const AWAY_GRACE_MS: i64 = 10 * 60_000;

/// Home: every 30 s, sessions that still work (or ask) on a runner gone for `AWAY_GRACE_MS` become
/// Interrupted with a note, so they continue elsewhere from their last handoff. A runner that comes back
/// with the same generation replaces the record with its own.
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut away: HashMap<String, i64> = HashMap::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            interrupt_away(&app, &mut away).await;
        }
    });
}

async fn interrupt_away(app: &App, away: &mut HashMap<String, i64>) {
    let now = now_ms();
    away.retain(|r, _| !app.runners.connected(r));
    let open: Vec<crate::model::Session> =
        app.sessions.list().into_iter().filter(|s| (s.status.is_running() || s.open_question) && s.runner_id.as_deref().is_some_and(|r| !app.runners.connected(r))).collect();
    for s in open {
        let Some(r) = s.runner_id.clone() else { continue };
        let since = *away.entry(r.clone()).or_insert(now);
        if now - since < AWAY_GRACE_MS {
            continue;
        }
        // A Burst job that is starting or reconnecting: `burst::tick` watches it.
        if crate::burst::get(app, &r).await.ok().flatten().is_some_and(|run| run.active()) {
            continue;
        }
        let name = runner_name(app, &r).await;
        let note = format!("{name} went offline during this turn. Launchpad marked the session Interrupted after {} minutes: send a message to continue it on another runner from its last saved turn.", AWAY_GRACE_MS / 60_000);
        match app.sessions.end_replica(app, &s.id, crate::model::SessionStatus::Interrupted, Some(note)).await {
            Ok(_) => tracing::info!("session {} interrupted: runner {r} away", s.id),
            Err(e) => tracing::warn!("session {} of away runner {r}: {e:#}", s.id),
        }
    }
}

/// A runner's name for messages: Burst, its name, or "its runner".
async fn runner_name(app: &App, id: &str) -> String {
    if is_burst(app, id).await || crate::burst::get(app, id).await.ok().flatten().is_some() {
        return "Burst".into();
    }
    load(app).await.ok().and_then(|l| l.into_iter().find(|(r, _)| r.id == id)).map(|(r, _)| r.name).unwrap_or_else(|| "Its runner".into())
}

pub async fn list(app: &App) -> Result<Vec<RunnerStatus>> {
    let mut out: Vec<RunnerStatus> = load(app).await?.into_iter().filter(|(r, _)| !r.burst).map(|(runner, _)| {
        let live = app.runners.live(&runner.id);
        RunnerStatus { connected: live.is_some(), compat: live.as_ref().map(|l| l.0), guard: live.map(|l| l.1), network_check: crate::netcap::last(&runner.id), runner }
    }).collect();
    out.sort_by_key(|r| (r.runner.state != RunnerState::Pending, r.runner.name.to_lowercase()));
    Ok(out)
}

/// Records a connection: build, protocol and time.
pub async fn seen(app: &App, mut r: Runner, commit: &str, protocol: u32, capabilities: Capabilities) -> Result<Runner> {
    r.capabilities = capabilities;
    r.commit = commit.into();
    r.protocol = protocol;
    r.last_seen_at = Some(now_ms());
    save(app, &r, None).await?;
    Ok(r)
}

/// Messages over the tunnel, one JSON object per WebSocket text frame.
pub mod proto {
    use serde::{Deserialize, Serialize};

    use super::Guard;
    use crate::role::Compat;

    /// Where an HTTP exchange through the tunnel goes. Bodies follow in base64 `HttpBody` frames.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum HttpTarget {
        /// Home's own runner-authenticated paths (Home Git, plain folders), for a runner without HTTP access to
        /// Home (Burst).
        Home,
        /// `https://<host><path>` with the token of a relayed lease, added by Home (Burst).
        Lease { lease_id: String, host: String },
        /// An MCP server of Home, through its gateway, for any runner.
        Mcp { lease_id: String, server: String },
        /// A TCP connection Home opens to a public address for a Burst runner (its agents' traffic): status 200
        /// once connected, then the request body goes to the connection and its bytes come back as the answer.
        Tcp { host: String, port: u16 },
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub enum FromRunner {
        /// First message.
        Hello { protocol: u32, commit: String, name: String, os: String, guard: Guard, #[serde(default)] capabilities: super::Capabilities },
        /// The runner's local settings changed.
        Guard { guard: Guard },
        /// Answer to `ToRunner::Req`.
        Res { id: u64, status: u16, body: String },
        /// Outbox items (`replica::Item`) with their numbers, oldest first.
        Items { items: Vec<(u64, serde_json::Value)> },
        /// A change made in a window of the runner that Home owns (workspaces), for Home's API.
        Req { id: u64, method: String, path: String, body: Option<String> },
        /// A request about the runner's own sessions (leases, ssh-agent), answered with `ToRunner::Res`.
        /// Home knows the runner from the tunnel; windows can never send it.
        Call { id: u64, op: String, body: serde_json::Value },
        /// Starts an HTTP exchange through Home; Home answers with `ToRunner::HttpHead` and `HttpBody` frames.
        HttpOpen { id: u64, target: HttpTarget, method: String, path: String, headers: Vec<(String, String)> },
        /// A part of a request body (base64); `end` closes it.
        HttpBody { id: u64, data: String, end: bool },
        Ping,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub enum ToRunner {
        /// Answer to `Hello`, with the last outbox number Home saved for this runner.
        Welcome { compat: Compat, commit: String, acked: u64 },
        /// Home saved every outbox item up to `n`.
        Ack { n: u64 },
        /// The connection is refused (unknown, pending or incompatible runner); the runner shows the reason.
        Refuse { reason: String },
        /// An API request for the runner's own daemon. `headers`: only `Idempotency-Key`, and `x-launchpad-envelope`
        /// for an answer that is not text (headers and base64 bytes in JSON).
        Req { id: u64, method: String, path: String, body: Option<String>, #[serde(default)] headers: Vec<(String, String)> },
        /// Answer to `FromRunner::Req`.
        Res { id: u64, status: u16, body: String },
        /// Every workspace (Home owns them), on connect and after each change.
        Workspaces { workspaces: Vec<crate::workspace::Workspace> },
        /// Every Git folder of Home, on connect and after each change.
        Folders { projects: Vec<crate::model::Project> },
        /// Profiles configured on Home, including their vault login IDs.
        Profiles { profiles: Vec<crate::config::Profile> },
        /// Quota readings of Home's logins (`vault:<id>`), on connect and when they change.
        Quotas { quotas: std::collections::BTreeMap<String, crate::status::QuotaStatus> },
        /// The status and headers of an exchange opened with `FromRunner::HttpOpen`.
        HttpHead { id: u64, status: u16, headers: Vec<(String, String)> },
        /// A part of a response body (base64); `end` closes it, with `error` when it ended early.
        HttpBody { id: u64, data: String, end: bool, error: Option<String> },
        Pong,
    }
}

struct Link {
    tx: mpsc::UnboundedSender<proto::ToRunner>,
    compat: Compat,
    guard: Guard,
    /// A Burst runner: it runs only the session it was started for.
    dedicated: bool,
    /// Distinguishes reconnections: an old socket ending never removes the new one.
    conn: u64,
}

/// The live tunnels of a Home.
#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<HubInner>,
}

/// The status and body of a relayed API request.
type Reply = oneshot::Sender<(u16, String)>;

#[derive(Default)]
struct HubInner {
    links: Mutex<HashMap<String, Link>>,
    /// Requests waiting for an answer, with their runner.
    pending: Mutex<HashMap<u64, (String, Reply)>>,
    next: std::sync::atomic::AtomicU64,
}

impl Hub {
    /// Registers a tunnel; a previous one of the same runner is replaced (and closes when its sender drops).
    pub fn connect(&self, app: &App, id: &str, compat: Compat, guard: Guard, dedicated: bool, tx: mpsc::UnboundedSender<proto::ToRunner>) -> u64 {
        let conn = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.inner.links.lock().unwrap().insert(id.into(), Link { tx, compat, guard, dedicated, conn });
        app.bus.publish("runners", "changed", serde_json::Value::Null);
        conn
    }

    pub fn disconnected(&self, app: &App, id: &str, conn: u64) {
        let mut links = self.inner.links.lock().unwrap();
        if links.get(id).is_some_and(|l| l.conn == conn) {
            links.remove(id);
            drop(links);
            // Waiting requests fail now (their senders drop) instead of timing out.
            self.inner.pending.lock().unwrap().retain(|_, (runner, _)| runner != id);
            app.bus.publish("runners", "changed", serde_json::Value::Null);
        }
    }

    fn disconnect(&self, id: &str) {
        self.inner.links.lock().unwrap().remove(id);
    }

    /// Closes a runner's tunnel now (a stopped Burst job).
    pub fn disconnect_now(&self, app: &App, id: &str) {
        if self.inner.links.lock().unwrap().remove(id).is_some() {
            self.inner.pending.lock().unwrap().retain(|_, (runner, _)| runner != id);
            app.bus.publish("runners", "changed", serde_json::Value::Null);
        }
    }

    pub fn set_guard(&self, app: &App, id: &str, guard: Guard) {
        if let Some(l) = self.inner.links.lock().unwrap().get_mut(id) {
            l.guard = guard;
        }
        app.bus.publish("runners", "changed", serde_json::Value::Null);
    }

    fn live(&self, id: &str) -> Option<(Compat, Guard)> {
        self.inner.links.lock().unwrap().get(id).map(|l| (l.compat, l.guard.clone()))
    }

    /// Sends a message to one connected runner.
    pub fn send_to(&self, id: &str, msg: proto::ToRunner) {
        if let Some(l) = self.inner.links.lock().unwrap().get(id) {
            let _ = l.tx.send(msg);
        }
    }

    /// Sends a message to every connected runner.
    pub fn broadcast(&self, msg: proto::ToRunner) {
        for l in self.inner.links.lock().unwrap().values() {
            let _ = l.tx.send(msg.clone());
        }
    }

    pub fn connected(&self, id: &str) -> bool {
        self.inner.links.lock().unwrap().contains_key(id)
    }

    /// Connected runners that take new sessions from other devices: same build, accepting, not paused.
    pub fn available(&self) -> Vec<String> {
        let links = self.inner.links.lock().unwrap();
        let mut ids: Vec<String> = links.iter().filter(|(_, l)| !l.dedicated && l.compat == Compat::Same && l.guard.accept_launches && !l.guard.paused).map(|(id, _)| id.clone()).collect();
        ids.sort();
        ids
    }

    /// Sends an API request to a runner's daemon and waits for its answer.
    pub async fn request(&self, id: &str, method: &str, path: &str, body: Option<String>) -> Result<(u16, String)> {
        self.request_with(id, method, path, body, vec![]).await
    }

    pub async fn request_with(&self, id: &str, method: &str, path: &str, body: Option<String>, headers: Vec<(String, String)>) -> Result<(u16, String)> {
        let n = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(n, (id.to_string(), tx));
        let sent = self.inner.links.lock().unwrap().get(id).map(|l| l.tx.send(proto::ToRunner::Req { id: n, method: method.into(), path: path.into(), body, headers }).is_ok());
        if sent != Some(true) {
            self.inner.pending.lock().unwrap().remove(&n);
            bail!("the runner is not connected");
        }
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(res)) => Ok(res),
            Ok(Err(_)) => bail!("the runner disconnected before answering"),
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&n);
                bail!("the runner did not answer within two minutes")
            }
        }
    }

    pub fn answered(&self, id: u64, status: u16, body: String) {
        if let Some((_, tx)) = self.inner.pending.lock().unwrap().remove(&id) {
            let _ = tx.send((status, body));
        }
    }
}

// ---------------------------------------------------------------- Placement

/// Where a session's next request goes (`place`).
#[derive(Debug, PartialEq)]
pub enum Place {
    /// This daemon holds the session (a runner, or a session without a runner).
    Here,
    /// A connected runner holds it now: its own, or the one it just moved to.
    Runner(String),
    /// No runner takes it: a new Burst job continues it (the caller starts the job with its request).
    Burst,
}

/// Among `available` runners (in any order), `prefer` when it is one of them, else the least busy (fewest
/// running sessions), then the first by ID.
fn choose(available: &[String], prefer: Option<&str>, busy: impl Fn(&str) -> usize) -> Option<String> {
    if let Some(p) = prefer.filter(|p| available.iter().any(|a| a == p)) {
        return Some(p.to_string());
    }
    available.iter().min_by_key(|r| (busy(r.as_str()), r.to_string())).cloned()
}

/// Home: the runner for new work among those that take launches: `prefer` (the previous runner) when it
/// does, else the least busy.
pub fn pick(app: &App, prefer: Option<&str>) -> Option<String> {
    let sessions = app.sessions.list();
    choose(&app.runners.available(), prefer, |r| sessions.iter().filter(|s| s.runner_id.as_deref() == Some(r) && s.status.is_running()).count())
}

/// Whether a session's runner holds it now: connected, and not a Burst job that has stopped or is stopping.
async fn holds(app: &App, s: &crate::model::Session) -> bool {
    s.runner_id.as_deref().is_some_and(|r| app.runners.connected(r)) && !crate::burst::needs_new_job(app, s).await
}

/// Home: where a request that continues a session goes (a message, Resume, an answer), in the owner's order:
/// its runner when connected; else, between turns, a move to another runner that takes launches (`pick`);
/// else Burst; else a clear error. Burst is always last. The move happens here; Burst is only named (the
/// caller starts the job with its request).
pub async fn place(app: &App, id: &str) -> Result<Place> {
    if app.paths.role != crate::role::Role::Home {
        return Ok(Place::Here);
    }
    // One placement at a time per session: a second message waits and finds the session moved.
    let lock = app.suggestion_locks.get(&format!("place:{id}"));
    let _lock = lock.lock().await;
    let s = app.sessions.get(id).context("no such session")?;
    let Some(r) = s.runner_id.clone() else { return Ok(Place::Here) };
    if holds(app, &s).await {
        return Ok(Place::Runner(r));
    }
    let stopped = crate::burst::needs_new_job(app, &s).await;
    if !stopped && is_burst(app, &r).await {
        bail!("This session's Burst runner is reconnecting: try again in a minute.");
    }
    let name = runner_name(app, &r).await;
    if s.status.is_running() || s.open_question {
        bail!(
            "{name} is offline and this session's turn is still open there. It continues elsewhere once {name} is back or after Launchpad marks it Interrupted ({} minutes after its runner left).",
            AWAY_GRACE_MS / 60_000
        );
    }
    if let Some(reason) = crate::handoff::blocked(&s) {
        bail!("{reason}");
    }
    if let Some(to) = pick(app, None) {
        crate::handoff::move_to(app, id, &to).await?;
        return Ok(Place::Runner(to));
    }
    if let Some(why) = crate::burst::unavailable(app).await {
        bail!("{name} is offline, no other runner takes this session, and Burst cannot: {why}.");
    }
    Ok(Place::Burst)
}

/// Home: a request that continues a session (`input`, `resume`, `answer`), placed first (`place`). Returns
/// the runner's answer; a Burst start answers at once (the job sends the request once it is up).
pub async fn continue_anywhere(app: &App, id: &str, action: &str, body: Option<String>, key: Option<String>) -> Result<(u16, String)> {
    match place(app, id).await? {
        Place::Here => bail!("this session runs on no runner"),
        Place::Runner(r) => {
            let headers = key.map(|k| vec![("idempotency-key".to_string(), k)]).unwrap_or_default();
            app.runners.request_with(&r, "POST", &format!("/api/sessions/{id}/{action}"), body, headers).await
        }
        Place::Burst => {
            crate::burst::continue_session(app, id, action, body, key).await?;
            Ok((200, serde_json::json!({"ok": true}).to_string()))
        }
    }
}

/// The runner a launch must continue on: the session whose folder it continues in (New agent here, Run in),
/// or the runner of its temporary folder.
async fn pinned_runner(app: &App, spec: &crate::model::LaunchSpec) -> Result<Option<String>> {
    use crate::model::Target;
    Ok(match &spec.target {
        Target::SessionFolder { session_id } => app.sessions.get(session_id).and_then(|s| s.runner_id),
        Target::Project { project_id } | Target::Predraft { project_id } => crate::projects::get(app, project_id).await?.filter(|p| p.is_temp).and_then(|p| p.runner_id),
        _ => None,
    })
}

/// Home: a launch pinned to a runner that is away (offline, or a stopped Burst job): the sessions it
/// continues from move first to the chosen runner, else the least busy one (their worktrees come back from
/// Home Git, a temporary folder from its files on Home).
async fn move_pinned(app: &App, spec: &crate::model::LaunchSpec) -> Result<()> {
    use crate::model::Target;
    let Some(from) = pinned_runner(app, spec).await? else { return Ok(()) };
    let sessions: Vec<crate::model::Session> = match &spec.target {
        Target::SessionFolder { session_id } => app.sessions.get(session_id).into_iter().collect(),
        Target::Project { project_id } | Target::Predraft { project_id } => app
            .sessions
            .list()
            .into_iter()
            .filter(|s| s.project_id.as_deref() == Some(project_id.as_str()) && s.runner_id.as_deref() == Some(from.as_str()) && s.status != crate::model::SessionStatus::Closed)
            .collect(),
        _ => vec![],
    };
    let away = match sessions.first() {
        Some(s) => !holds(app, s).await,
        None => !app.runners.connected(&from),
    };
    if !away {
        return Ok(());
    }
    let name = runner_name(app, &from).await;
    let to = match &spec.runner_id {
        Some(c) => Some(c.clone()).filter(|c| app.runners.available().contains(c)).context("The chosen runner does not take this launch now: it is offline, paused, refuses launches from other devices or needs an update")?,
        None => pick(app, None).with_context(|| format!("{name} has this folder and is offline, and no other runner takes launches now: connect a runner, or send its agent a message to continue it on Burst"))?,
    };
    for s in &sessions {
        crate::handoff::move_to(app, &s.id, &to).await?;
    }
    // A temporary folder without an open agent follows the launch.
    if sessions.is_empty()
        && let Target::Project { project_id } | Target::Predraft { project_id } = &spec.target
    {
        crate::projects::move_temp(app, project_id, &to).await?;
    }
    Ok(())
}

/// Home: the runner for a new session. The one chosen in the composer (never another); else the runner the
/// launch is pinned to (a session's folder, a temporary folder); else the bot's previous runner or the least
/// busy one that takes launches from other devices: every folder is stored by Home, so any runner opens it.
pub async fn launch_runner(app: &App, spec: &crate::model::LaunchSpec) -> Result<String> {
    use crate::model::Target;
    let project = match &spec.target {
        Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.clone()),
        _ => None,
    };
    let available = app.runners.available();
    let refused = |name: String| anyhow::anyhow!("{name} does not take this launch now: it is offline, paused, refuses launches from other devices or needs an update");
    if let Some(r) = pinned_runner(app, spec).await? {
        let what = if matches!(spec.target, Target::SessionFolder { .. }) { "this agent continues in another session's folder" } else { "this temporary folder" };
        anyhow::ensure!(spec.runner_id.as_ref().is_none_or(|c| c == &r), "{what} is on {}: choose that runner", runner_name(app, &r).await);
        anyhow::ensure!(available.contains(&r), refused(runner_name(app, &r).await));
        return Ok(r);
    }
    if let Some(id) = project {
        let p = crate::projects::get(app, &id).await?.context("no such folder")?;
        anyhow::ensure!(p.home_git || p.plain || p.is_temp, "{} is not stored on Home: add it again in Folders", p.name);
    }
    if let Some(r) = &spec.runner_id {
        anyhow::ensure!(available.contains(r), refused("The chosen runner".into()));
        return Ok(r.clone());
    }
    let previous = spec.bot.as_ref().and_then(|b| b.previous.as_ref()).and_then(|p| app.sessions.get(&p.session_id)).and_then(|s| s.runner_id);
    pick(app, previous.as_deref()).context("no runner takes launches now: connect one, or check its guard in Settings → Home and runners")
}

fn answer<T: serde::de::DeserializeOwned>(status: u16, body: &str) -> Result<T> {
    anyhow::ensure!((200..300).contains(&status), "{}", serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_else(|| body.to_string()));
    Ok(serde_json::from_str(body)?)
}

/// Starts a session where agents run: here, or on Home through a runner (sent once, never retried). Home
/// places it like a message (`place`): the chosen runner (no fallback), else the runner it is pinned to (its
/// sessions move first when that runner is away), else another runner, else Burst.
pub async fn launch_anywhere(app: &App, mut spec: crate::model::LaunchSpec, id: String, automatic: bool) -> Result<crate::model::Session> {
    if app.paths.role != crate::role::Role::Home {
        return if automatic { app.sessions.launch_automatic(app, spec, id).await } else { app.sessions.launch_with_id(app, spec, id).await };
    }
    // Uploads are stored on Home: the runner or Burst job receives their bytes.
    crate::attachments::inline_files(&mut spec.attachments)?;
    if spec.runner_id.as_deref() == Some(crate::burst::CHOICE) {
        return crate::burst::launch(app, spec, id, automatic).await;
    }
    move_pinned(app, &spec).await?;
    let pinned = pinned_runner(app, &spec).await?.is_some();
    let runner = match launch_runner(app, &spec).await {
        Ok(r) => r,
        // No runner takes it (the laptop is off): Burst, unless a runner was chosen or the launch is pinned.
        Err(e) if spec.runner_id.is_none() && !pinned && app.runners.available().is_empty() => match crate::burst::unavailable(app).await {
            None => return crate::burst::launch(app, spec, id, automatic).await,
            Some(why) => bail!("{e:#}. {why}."),
        },
        Err(e) => return Err(e),
    };
    let (status, body) = app.runners.request_with(&runner, "POST", "/api/sessions", Some(serde_json::to_string(&spec)?), vec![("idempotency-key".into(), id)]).await?;
    answer(status, &body)
}

/// A session action (`close`, `stop`, …) wherever the session runs. Archive of a session whose runner is
/// away (offline, a stopped Burst job) is Home's alone (`archive_away`).
pub async fn session_action(app: &App, id: &str, action: &str) -> Result<()> {
    match app.sessions.get(id).and_then(|s| s.runner_id).filter(|_| app.paths.role == crate::role::Role::Home) {
        Some(r) if !app.runners.connected(&r) => match action {
            "close" => archive_away(app, id).await,
            "unarchive" => unarchive_away(app, id).await,
            _ => bail!("{} is offline: only Archive and its undo work until it is back", runner_name(app, &r).await),
        },
        Some(r) => {
            let (status, body) = app.runners.request(&r, "POST", &format!("/api/sessions/{id}/{action}"), None).await?;
            answer::<serde_json::Value>(status, &body).map(|_| ())
        }
        None => match action {
            "close" => crate::sessions::archive(app, id).await,
            "unarchive" => app.sessions.unarchive(app, id).await.map(|_| ()),
            _ => bail!("unknown session action {action}"),
        },
    }
}

/// A message to a session wherever it runs, placed like the owner's messages (`place`). `files` are
/// attachments the session's machine stores.
pub async fn send_anywhere(app: &App, id: &str, text: &str, mut files: Vec<crate::model::FileAttachment>, key: Option<String>) -> Result<()> {
    if app.paths.role == crate::role::Role::Home && app.sessions.get(id).is_some_and(|s| s.runner_id.is_some()) {
        crate::attachments::inline_files(&mut files)?;
        let body = serde_json::json!({"text": text, "files": files}).to_string();
        let (status, body) = continue_anywhere(app, id, "input", Some(body), key).await?;
        return answer::<serde_json::Value>(status, &body).map(|_| ());
    }
    let (text, images) = crate::attachments::for_message(app, id, text, &files).await?;
    match key {
        Some(k) => app.sessions.send_with_id(app, id, &text, images, k).await,
        None => app.sessions.send(app, id, &text, images).await,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn choose_prefers_the_previous_runner_then_the_least_busy() {
        let available = vec!["b".to_string(), "a".to_string(), "c".to_string()];
        let busy = |r: &str| match r {
            "a" => 2,
            "b" => 1,
            _ => 1,
        };
        assert_eq!(super::choose(&available, Some("a"), busy).as_deref(), Some("a"));
        assert_eq!(super::choose(&available, Some("gone"), busy).as_deref(), Some("b"));
        assert_eq!(super::choose(&available, None, |_| 0).as_deref(), Some("a"));
        assert_eq!(super::choose(&[], Some("a"), busy), None);
    }
}

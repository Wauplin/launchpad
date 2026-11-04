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
}

/// A runner with its live state, for Settings.
#[derive(Clone, Debug, Serialize, TS)]
pub struct RunnerStatus {
    pub runner: Runner,
    pub connected: bool,
    /// Build comparison while connected.
    pub compat: Option<Compat>,
    pub guard: Option<Guard>,
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

async fn load(app: &App) -> Result<Vec<(Runner, String)>> {
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

/// Removes a runner and closes its tunnel. Its sessions stay on Home.
pub async fn remove(app: &App, id: &str) -> Result<()> {
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM runner WHERE id = ?1", [key])?)).await?;
    app.runners.disconnect(id);
    app.bus.publish("runners", "changed", serde_json::Value::Null);
    Ok(())
}

pub async fn list(app: &App) -> Result<Vec<RunnerStatus>> {
    let mut out: Vec<RunnerStatus> = load(app).await?.into_iter().map(|(runner, _)| {
        let live = app.runners.live(&runner.id);
        RunnerStatus { connected: live.is_some(), compat: live.as_ref().map(|l| l.0), guard: live.map(|l| l.1), runner }
    }).collect();
    out.sort_by_key(|r| (r.runner.state != RunnerState::Pending, r.runner.name.to_lowercase()));
    Ok(out)
}

/// Records a connection: build, protocol and time.
pub async fn seen(app: &App, mut r: Runner, commit: &str, protocol: u32) -> Result<Runner> {
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

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub enum FromRunner {
        /// First message.
        Hello { protocol: u32, commit: String, name: String, os: String, guard: Guard },
        /// The runner's local settings changed.
        Guard { guard: Guard },
        /// Answer to `ToRunner::Req`.
        Res { id: u64, status: u16, body: String },
        /// Outbox items (`replica::Item`) with their numbers, oldest first.
        Items { items: Vec<(u64, serde_json::Value)> },
        /// A change made in a window of the runner that Home owns (workspaces), for Home's API.
        Req { id: u64, method: String, path: String, body: Option<String> },
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
        /// An API request for the runner's own daemon. `headers`: only `Idempotency-Key`.
        Req { id: u64, method: String, path: String, body: Option<String>, #[serde(default)] headers: Vec<(String, String)> },
        /// Answer to `FromRunner::Req`.
        Res { id: u64, status: u16, body: String },
        /// Every workspace (Home owns them), on connect and after each change.
        Workspaces { workspaces: Vec<crate::workspace::Workspace> },
        /// Every Git folder of Home, on connect and after each change.
        Folders { projects: Vec<crate::model::Project> },
        Pong,
    }
}

struct Link {
    tx: mpsc::UnboundedSender<proto::ToRunner>,
    compat: Compat,
    guard: Guard,
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
    pub fn connect(&self, app: &App, id: &str, compat: Compat, guard: Guard, tx: mpsc::UnboundedSender<proto::ToRunner>) -> u64 {
        let conn = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.inner.links.lock().unwrap().insert(id.into(), Link { tx, compat, guard, conn });
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
        let mut ids: Vec<String> = links.iter().filter(|(_, l)| l.compat == Compat::Same && l.guard.accept_launches && !l.guard.paused).map(|(id, _)| id.clone()).collect();
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

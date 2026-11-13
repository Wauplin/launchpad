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
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Guard {
    /// Takes new sessions off ("Paused"): Home starts no new session there; running sessions continue.
    pub paused: bool,
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
    /// Serves its terminals to Home's windows through the tunnel (`ToRunner::WsOpen`).
    pub terminals: bool,
    /// Updates its agent CLIs on Home's request (`launchpad_setup::cli_update`).
    pub cli_update: bool,
    /// Keeps exchange bodies to their window and sends `HttpAck` and `HttpCancel` (`server::wire`). Home keeps
    /// no window with an older runner, so it can still download Home's build.
    pub flow: bool,
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
    Capabilities { vm, docker, burst, self_update: crate::self_update::supported(app), terminals: true, cli_update: crate::self_update::supported(app), flow: true }
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
    /// Why Home refuses its connection (`role::os_refusal`): agents run only on Linux.
    pub refused: Option<String>,
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
    anyhow::ensure!(app.paths.role == crate::role::Role::Home, "runners connect to a Home (launchpad daemon)");
    Ok(())
}

/// A new pairing request. Returns the runner record and the secret, which only the runner keeps.
pub async fn request(app: &App, name: &str, os: &str) -> Result<(Runner, String)> {
    ensure_home(app)?;
    if let Some(why) = crate::role::os_refusal(os) {
        bail!("Home refuses this runner: {why}");
    }
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
    if let Some(why) = crate::role::os_refusal(&r.os) {
        bail!("{why}");
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
    app.state_db
        .call(move |c| {
            c.execute("DELETE FROM runner WHERE id = ?1", [&key])?;
            // What waited for it never goes.
            c.execute("DELETE FROM runner_delivery WHERE runner_id = ?1", [&key])?;
            Ok(())
        })
        .await?;
    app.runners.disconnect(id);
    let orphans: Vec<String> = app.sessions.list().into_iter().filter(|s| s.runner_id.as_deref() == Some(id) && (s.status.is_running() || s.open_question || s.quota_pause.is_some())).map(|s| s.id).collect();
    for sid in orphans {
        if let Err(e) = app.sessions.end_replica(app, &sid, crate::model::SessionStatus::Interrupted, None).await {
            tracing::warn!("session {sid} of removed runner {id}: {e:#}");
        }
    }
    app.bus.publish("runners", "changed", serde_json::Value::Null);
    Ok(())
}

// ---------------------------------------------------------------- What a runner learns when it is back

/// Something Home did to a session while its runner was away, delivered once when the runner reconnects. Kept in
/// `state.db` (`runner_delivery`, backed up with it) until the runner confirmed it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct PendingArchive {
    runner: String,
    session: String,
    /// None: the owner archived the session. Some: Home moved it on (generation on Home), so the runner saves
    /// what only it has to `lp-scratch/…` and drops its copy (`handoff::release`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    release: Option<u32>,
}

impl PendingArchive {
    fn kind(&self) -> &'static str {
        if self.release.is_some() { "release" } else { "archive" }
    }
}

/// The file older Homes kept these items in; imported once into `runner_delivery` (`import_archives_file`).
fn archives_file(app: &App) -> std::path::PathBuf {
    app.paths.data_dir.join("runner-archives.json")
}

/// Home start: items of the former `runner-archives.json` (also what a restore of an older backup wrote) move
/// into `state.db`, then the file goes. A damaged file stays, with a warning.
async fn import_archives_file(app: &App) {
    let file = archives_file(app);
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    let list: Vec<PendingArchive> = match serde_json::from_str(&text) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("{} is damaged, its pending Archives and releases are not delivered: {e}", file.display());
            return;
        }
    };
    for item in list {
        if let Err(e) = store_pending(app, &item).await {
            tracing::warn!("{}: {e:#}", file.display());
            return;
        }
    }
    let _ = std::fs::remove_file(&file);
}

async fn store_pending(app: &App, item: &PendingArchive) -> Result<()> {
    let (runner, session, kind, generation) = (item.runner.clone(), item.session.clone(), item.kind(), item.release);
    app.state_db
        .call(move |c| {
            // A newer release replaces an older one; an Archive is stored once.
            c.execute(
                "INSERT INTO runner_delivery (runner_id, session_id, kind, generation) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (runner_id, session_id, kind) DO UPDATE SET generation = ?4",
                rusqlite::params![runner, session, kind, generation],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

async fn pending_of(app: &App, runner: &str) -> Result<Vec<PendingArchive>> {
    let runner = runner.to_string();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT session_id, generation FROM runner_delivery WHERE runner_id = ?1 ORDER BY rowid")?;
            let rows = st.query_map([&runner], |r| Ok(PendingArchive { runner: runner.clone(), session: r.get(0)?, release: r.get::<_, Option<u32>>(1)? }))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
}

/// Drops a delivered item, unless it changed meanwhile (a newer release of the same session waits).
async fn delivered(app: &App, item: &PendingArchive) -> Result<()> {
    let (runner, session, kind, generation) = (item.runner.clone(), item.session.clone(), item.kind(), item.release);
    app.state_db
        .call(move |c| {
            c.execute(
                "DELETE FROM runner_delivery WHERE runner_id = ?1 AND session_id = ?2 AND kind = ?3 AND generation IS ?4",
                rusqlite::params![runner, session, kind, generation],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Adds an item for a runner Home knows (a removed one never comes back).
async fn deliver_later(app: &App, item: PendingArchive) -> Result<()> {
    if !load(app).await?.into_iter().any(|(r, _)| r.id == item.runner) {
        return Ok(());
    }
    store_pending(app, &item).await
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
    let session = id.to_string();
    app.state_db
        .call(move |c| {
            c.execute("DELETE FROM runner_delivery WHERE session_id = ?1 AND kind = 'archive'", [session])?;
            Ok(())
        })
        .await?;
    app.sessions.unarchive(app, id).await?;
    Ok(())
}

/// Home: `runner` still holds an older generation of a session Home moved on (it was away, or the move took
/// the session from it); it releases its copy when it is next connected (`deliver_pending`).
pub async fn release_later(app: &App, runner: &str, session: &str, generation: u32) -> Result<()> {
    deliver_later(app, PendingArchive { runner: runner.to_string(), session: session.to_string(), release: Some(generation) }).await
}

/// Whether a delivery attempt settles its item (it is dropped): the runner accepted it, or refused an Archive (a
/// session it no longer has). A refused release (Home did not confirm the leftovers) and a dropped tunnel wait
/// for the next connection.
fn settled(release: bool, sent: &Result<(u16, String)>) -> bool {
    match sent {
        Ok((status, _)) => *status < 300 || !release,
        Err(_) => false,
    }
}

/// Home: a runner that (re)connected receives the Archives clicked and the moves made while it was away. Each
/// item stays in `state.db` until the runner answered it, so a Home restart meanwhile delivers it again.
pub async fn deliver_pending(app: &App, runner_id: &str) {
    let lock = app.suggestion_locks.get(&format!("deliver:{runner_id}"));
    let _lock = lock.lock().await;
    let list = match pending_of(app, runner_id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("pending deliveries of runner {runner_id}: {e:#}");
            return;
        }
    };
    for a in list {
        let sent = match a.release {
            None => app.runners.request(runner_id, "POST", &format!("/api/sessions/{}/close", a.session), None).await,
            Some(generation) => app.runners.request(runner_id, "POST", &format!("/api/sessions/{}/release", a.session), Some(serde_json::json!({"generation": generation}).to_string())).await,
        };
        if let Ok((status, body)) = &sent
            && *status >= 300
        {
            tracing::warn!("{} of {} on its runner: {status} {body}", if a.release.is_some() { "Release" } else { "Archive" }, a.session);
        }
        if settled(a.release.is_some(), &sent) {
            if let Err(e) = delivered(app, &a).await {
                tracing::warn!("pending delivery of {}: {e:#}", a.session);
            }
        } else if sent.is_err() && !app.runners.connected(runner_id) {
            // The tunnel dropped again: the next connection delivers the rest. A request that only timed out (a
            // release pushing large leftovers) stays for the next round, and the others go on.
            return;
        }
    }
}

/// Home: delivers again what connected runners still have pending (each runner in its own task).
async fn retry_pending(app: &App) {
    let runners: Vec<String> = match app.state_db.call(|c| {
        let mut st = c.prepare("SELECT DISTINCT runner_id FROM runner_delivery")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("pending deliveries: {e:#}");
            return;
        }
    };
    for r in runners.into_iter().filter(|r| app.runners.connected(r)) {
        let app = app.clone();
        tokio::spawn(async move { deliver_pending(&app, &r).await });
    }
}

// ---------------------------------------------------------------- Sessions of a runner that left

/// How long Home waits for a runner that left while its sessions worked or asked a question.
pub const AWAY_GRACE_MS: i64 = 10 * 60_000;

/// Home: every 30 s, sessions that still work, ask or wait for quota on a runner gone for `AWAY_GRACE_MS`
/// become Interrupted with a note, so they continue elsewhere from their last handoff (the note names the
/// messages a quota pause held, which are not sent). A runner that comes back
/// with the same generation replaces the record with its own.
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        import_archives_file(&app).await;
        let mut away: HashMap<String, i64> = HashMap::new();
        let mut round = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            interrupt_away(&app, &mut away).await;
            // Every 10 minutes, items a connected runner refused or did not answer in time are offered again.
            round += 1;
            if round.is_multiple_of(20) {
                retry_pending(&app).await;
            }
        }
    });
}

async fn interrupt_away(app: &App, away: &mut HashMap<String, i64>) {
    let now = now_ms();
    away.retain(|r, _| !app.runners.connected(r));
    let open = |s: &crate::model::Session| s.status.is_running() || s.open_question || s.quota_pause.is_some();
    let sessions: Vec<crate::model::Session> =
        app.sessions.list().into_iter().filter(|s| open(s) && s.runner_id.as_deref().is_some_and(|r| !app.runners.connected(r))).collect();
    for s in sessions {
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
        // Messages that waited for quota are in its trace; Home's record cannot deliver them.
        let during = match s.quota_pause.as_ref().map(|p| p.messages.len()) {
            Some(0) => "while this session was paused for quota".to_string(),
            Some(n) => format!("while this session was paused for quota, with {n} message{} waiting that {} not sent (send {} again)", if n == 1 { "" } else { "s" }, if n == 1 { "was" } else { "were" }, if n == 1 { "it" } else { "them" }),
            None => "during this turn".to_string(),
        };
        let note = format!("{name} went offline {during}. Launchpad marked the session Interrupted after {} minutes: send a message to continue it on another runner from its last saved turn.", AWAY_GRACE_MS / 60_000);
        // The runner may have come back (and sent its record) while this check ran: then its record stands.
        let unchanged = |c: &crate::model::Session| c.generation == s.generation && c.runner_id == s.runner_id && open(c);
        if app.runners.connected(&r) || !app.sessions.get(&s.id).is_some_and(|c| unchanged(&c)) {
            continue;
        }
        match app.sessions.end_replica(app, &s.id, crate::model::SessionStatus::Interrupted, Some(note)).await {
            Ok(_) => tracing::info!("session {} interrupted: runner {r} away", s.id),
            Err(e) => tracing::warn!("session {} of away runner {r}: {e:#}", s.id),
        }
    }
}

/// A runner's record (Burst runners included).
pub async fn get(app: &App, id: &str) -> Option<Runner> {
    load(app).await.ok()?.into_iter().find(|(r, _)| r.id == id).map(|(r, _)| r)
}

/// A runner's name for messages: Burst, its name, or "Its runner".
pub async fn runner_name(app: &App, id: &str) -> String {
    if is_burst(app, id).await || crate::burst::get(app, id).await.ok().flatten().is_some() {
        return "Burst".into();
    }
    load(app).await.ok().and_then(|l| l.into_iter().find(|(r, _)| r.id == id)).map(|(r, _)| r.name).unwrap_or_else(|| "Its runner".into())
}

pub async fn list(app: &App) -> Result<Vec<RunnerStatus>> {
    let mut out: Vec<RunnerStatus> = load(app).await?.into_iter().filter(|(r, _)| !r.burst).map(|(runner, _)| {
        let live = app.runners.live(&runner.id);
        RunnerStatus { connected: live.is_some(), compat: live.as_ref().map(|l| l.0), guard: live.map(|l| l.1), network_check: crate::netcap::last(&runner.id), refused: crate::role::os_refusal(&runner.os), runner }
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
        /// An API request to Home: the runner's own work (`server::runner_trust`'s closed list), or with
        /// `session` an agent's CLI call that Home checks with that session's access.
        Req {
            id: u64,
            method: String,
            path: String,
            body: Option<String>,
            /// The agent session whose CLI sent it (Home checks that it placed the session on this runner).
            #[serde(default, skip_serializing_if = "Option::is_none")]
            session: Option<String>,
        },
        /// A request about the runner's own sessions (leases, ssh-agent), answered with `ToRunner::Res`.
        /// Home knows the runner from the tunnel; windows can never send it.
        Call { id: u64, op: String, body: serde_json::Value },
        /// Starts an HTTP exchange through Home; Home answers with `ToRunner::HttpHead` and `HttpBody` frames.
        HttpOpen { id: u64, target: HttpTarget, method: String, path: String, headers: Vec<(String, String)> },
        /// A part of a request body (base64); `end` closes it. Parts travel in the bulk lane, at most
        /// `server::wire::WINDOW` bytes ahead of Home's `ToRunner::HttpAck`.
        HttpBody { id: u64, data: String, end: bool },
        /// The runner consumed `n` bytes of the response body of exchange `id`: Home may send `n` more.
        HttpAck { id: u64, n: u32 },
        /// The runner dropped exchange `id` (its client went away): Home stops it and drops its upstream.
        HttpCancel { id: u64 },
        /// A frame of a WebSocket Home opened with `ToRunner::WsOpen`: `text`, or binary `data` in base64.
        WsFrame { id: u64, #[serde(default)] text: Option<String>, #[serde(default)] data: Option<String> },
        /// That WebSocket ended; `error` when it could not open (no such terminal).
        WsClose { id: u64, #[serde(default)] error: Option<String> },
        /// The runner's terminals changed (opened, exited, removed): Home tells its windows.
        Terminals,
        Ping,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub enum ToRunner {
        /// Answer to `Hello`, with the last outbox number Home saved for this runner. `flow`: Home keeps exchange
        /// bodies to their window and sends `HttpAck` and `HttpCancel` (an older Home does not).
        Welcome { compat: Compat, commit: String, acked: u64, #[serde(default)] flow: bool },
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
        /// Home's settings a runner applies to its own work: how many idle task worktrees it keeps (`[worktrees] keep`,
        /// Settings → Disk cleanup), on connect and after each change.
        Settings { worktrees_keep: u32 },
        /// Entries of Home's spending ledger (`spend.rs`): all that runners need on connect and after a
        /// configuration change, each new one as Home stores it.
        Spend { entries: Vec<crate::spend::Entry> },
        /// Quota readings of Home's logins (`vault:<id>`), on connect and when they change.
        Quotas { quotas: std::collections::BTreeMap<String, crate::status::QuotaStatus> },
        /// Home refreshed or replaced this vault login: renew the leases that hold it now (`contain::renew_soon`).
        RenewLeases { login: String },
        /// The status and headers of an exchange opened with `FromRunner::HttpOpen`.
        HttpHead { id: u64, status: u16, headers: Vec<(String, String)> },
        /// A part of a response body (base64); `end` closes it, with `error` when it ended early. Parts travel in
        /// the bulk lane, at most `server::wire::WINDOW` bytes ahead of the runner's `FromRunner::HttpAck`.
        HttpBody { id: u64, data: String, end: bool, error: Option<String> },
        /// Home consumed `n` bytes of the request body of exchange `id`: the runner may send `n` more.
        HttpAck { id: u64, n: u32 },
        /// Home stopped reading the request body of exchange `id`: the runner stops sending it (the response,
        /// if any, still comes).
        HttpCancel { id: u64 },
        /// Opens a viewer WebSocket on the runner for a window of Home. Only a terminal's
        /// (`/api/terminals/{id}/ws[?offset=…]`): the PTY and its output stay on the runner, frames pass through.
        WsOpen { id: u64, path: String },
        /// A frame from the window: `text`, or binary `data` in base64.
        WsFrame { id: u64, #[serde(default)] text: Option<String>, #[serde(default)] data: Option<String> },
        /// The window's WebSocket closed.
        WsClose { id: u64 },
        Pong,
    }
}

struct Link {
    /// The control lane: requests, answers, acknowledgements, snapshots (small, sent first).
    tx: mpsc::UnboundedSender<proto::ToRunner>,
    /// The bulk lane: parts of exchange bodies, bounded (`send_bulk` waits for room).
    bulk: mpsc::Sender<proto::ToRunner>,
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
    /// What Home let a runner do on its own credential because of a request Home sent it (`grant`).
    grants: Mutex<HashMap<(String, String), i64>>,
}

/// How long a grant (`Hub::grant`) holds.
const GRANT_MS: i64 = 24 * 3600 * 1000;

/// The agents whose traces a request body names as context (`agent_traces` at any depth: a launch spec, a
/// message's context).
pub fn traced_agents(body: &serde_json::Value) -> Vec<String> {
    let mut out = vec![];
    match body {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                if k == "agent_traces" && let Some(list) = v.as_array() {
                    out.extend(list.iter().filter_map(|t| t["session_id"].as_str().map(str::to_string)));
                } else {
                    out.extend(traced_agents(v));
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|v| out.extend(traced_agents(v))),
        _ => {}
    }
    out
}

impl Hub {
    /// Registers a tunnel; a previous one of the same runner is replaced (and closes when its sender drops).
    /// `lanes`: the connection's control lane and bulk lane.
    pub fn connect(&self, app: &App, id: &str, compat: Compat, guard: Guard, dedicated: bool, lanes: (mpsc::UnboundedSender<proto::ToRunner>, mpsc::Sender<proto::ToRunner>)) -> u64 {
        let (tx, bulk) = lanes;
        let conn = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.inner.links.lock().unwrap().insert(id.into(), Link { tx, bulk, compat, guard, dedicated, conn });
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

    /// Sends a part of an exchange body to a connected runner in the bulk lane, waiting while the lane is full.
    /// False: the runner is not connected (or its connection changed).
    pub async fn send_bulk(&self, id: &str, msg: proto::ToRunner) -> bool {
        let bulk = self.inner.links.lock().unwrap().get(id).map(|l| l.bulk.clone());
        match bulk {
            Some(b) => b.send(msg).await.is_ok(),
            None => false,
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

    /// Every connected runner, Burst jobs included, by ID.
    pub fn connected_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.links.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Connected runners that take new sessions: same build, not paused.
    pub fn available(&self) -> Vec<String> {
        let links = self.inner.links.lock().unwrap();
        let mut ids: Vec<String> = links.iter().filter(|(_, l)| !l.dedicated && l.compat == Compat::Same && !l.guard.paused).map(|(id, _)| id.clone()).collect();
        ids.sort();
        ids
    }

    /// Sends an API request to a runner's daemon and waits for its answer.
    pub async fn request(&self, id: &str, method: &str, path: &str, body: Option<String>) -> Result<(u16, String)> {
        self.request_with(id, method, path, body, vec![]).await
    }

    pub async fn request_with(&self, id: &str, method: &str, path: &str, body: Option<String>, headers: Vec<(String, String)>) -> Result<(u16, String)> {
        // The context traces a launch or a message names: the runner reads them from Home on its own credential.
        if let Some(v) = body.as_deref().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()) {
            for agent in traced_agents(&v) {
                self.grant(id, &format!("trace:{agent}"));
            }
        }
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

    /// Home: lets runner `id` do `key` on its own credential for a day, because Home asked it to (the owner's
    /// Merge or Keep as folder that Home relayed, a context trace Home sent with a launch or a message).
    pub fn grant(&self, id: &str, key: &str) {
        let now = crate::util::now_ms();
        let mut grants = self.inner.grants.lock().unwrap();
        grants.retain(|_, at| now - *at < GRANT_MS);
        grants.insert((id.to_string(), key.to_string()), now);
    }

    /// Whether runner `id` holds the grant `key` (`grant`).
    pub fn granted(&self, id: &str, key: &str) -> bool {
        let now = crate::util::now_ms();
        self.inner.grants.lock().unwrap().get(&(id.to_string(), key.to_string())).is_some_and(|at| now - *at < GRANT_MS)
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
    /// This daemon holds the session (a runner).
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

/// Home: the lock every placement and move of a session takes (`place`, `handoff::move_to`).
pub(crate) fn place_lock(app: &App, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    app.suggestion_locks.get(&format!("place:{id}"))
}

/// Home: where a request that continues a session goes (a message, Resume, an answer), in the owner's order:
/// its runner when connected; else, between turns, a move to another runner that takes launches (`pick`);
/// else Burst; else a clear error. Burst is always last. The move happens here; Burst is only named (the
/// caller starts the job with its request).
pub async fn place(app: &App, id: &str) -> Result<Place> {
    if app.paths.role != crate::role::Role::Home {
        return Ok(Place::Here);
    }
    // One placement or move at a time per session: a second message waits and finds the session moved.
    let lock = place_lock(app, id);
    let _lock = lock.lock().await;
    let s = app.sessions.get(id).context("no such session")?;
    let r = s.runner_id.clone().context("this session runs on no runner")?;
    // A Burst job is starting for it: the job moves the session and sends the first request, so nothing else
    // may reach the old runner (if it comes back) or start a second job meanwhile. Later requests wait in the
    // job's record and follow the first one (`burst::continue_session`).
    if crate::burst::starting_for(app, id).await {
        return Ok(Place::Burst);
    }
    if holds(app, &s).await {
        return Ok(Place::Runner(r));
    }
    let stopped = crate::burst::needs_new_job(app, &s).await;
    if !stopped && is_burst(app, &r).await {
        bail!("This session's Burst runner is reconnecting: try again in a minute.");
    }
    let name = runner_name(app, &r).await;
    if (s.status.is_running() || s.open_question) && app.runners.connected(&r) {
        bail!("{name} reaches HF Jobs' time limit soon and this session's turn is still open there: Launchpad ends it, then send this again (it continues elsewhere).");
    }
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
        crate::handoff::move_locked(app, id, &to).await?;
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
        Place::Here => bail!("only Home places sessions"),
        Place::Runner(r) => deliver(app, &r, id, action, body, key).await,
        // The job delivers it once it is up (`deliver`, then the queued ones through here).
        Place::Burst => {
            crate::burst::continue_session(app, id, action, body, key).await?;
            Ok((200, serde_json::json!({"ok": true}).to_string()))
        }
    }
}

/// Home: a selection of a pull request's diff (`DiffSelection`) gets the lines Home reads, replacing whatever the
/// request said; an invalid one is refused. `agent`: the workspace the message or launch of another bot or agent goes
/// to (Some(None) when it has none): Home reads with its own GitHub login, so a repository of another workspace's
/// folder is refused, like every record of another workspace.
async fn resolve_selection(app: &App, selection: &mut Option<crate::model::DiffSelection>, agent: Option<Option<String>>) -> Result<()> {
    if let Some(sel) = selection {
        sel.check()?;
        if let Some(ws) = agent {
            anyhow::ensure!(!crate::scope::repo_outside(app, ws.as_deref().unwrap_or_default(), &sel.repo).await, "{} belongs to another workspace's folder: agents act only in their own workspace", sel.repo);
        }
        sel.lines = crate::ext::extensions().selection_lines(app, sel).await;
    }
    Ok(())
}

/// A session input body with its selection's lines read by Home (`resolve_selection`).
async fn selection_read(app: &App, id: &str, body: String) -> Result<String> {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&body) else { return Ok(body) };
    let Some(raw) = v.get("context").and_then(|c| c.get("selection")).filter(|s| !s.is_null()).cloned() else { return Ok(body) };
    let mut sel = Some(serde_json::from_value::<crate::model::DiffSelection>(raw)?);
    let agent = match v["context"].get("from").is_some_and(|f| !f.is_null()) {
        true => Some(crate::scope::of_session(app, id).await),
        false => None,
    };
    resolve_selection(app, &mut sel, agent).await?;
    v["context"]["selection"] = serde_json::to_value(sel)?;
    Ok(v.to_string())
}

/// Home: sends a continuation to the runner that has the session. A message from another bot or agent
/// (`context.from`, stamped by Home) arrives under its sender's header, fenced (`senders::delivered`). A message to a
/// bot session whose BOT.md changed gets the new one after its typed text, recorded in the session once the runner
/// accepted it.
pub async fn deliver(app: &App, runner: &str, id: &str, action: &str, body: Option<String>, key: Option<String>) -> Result<(u16, String)> {
    let mut inherited_membership = None;
    let body = if action == "input" {
        if let Some(raw) = body {
            let mut value: serde_json::Value = serde_json::from_str(&raw)?;
            if let Some(context) = value.get_mut("context") {
                let mut ctx: crate::model::MessageContext = serde_json::from_value(context.clone())?;
                if !ctx.topics.is_empty() || !ctx.agent_traces.is_empty() {
                    let ws = crate::scope::of_session(app, id).await.context("Context needs a workspace")?;
                    let inherited = crate::topics::resolve(app, &ws, &mut ctx.topics, &mut ctx.agent_traces).await?;
                    ctx.agent_traces.retain(|t| t.session_id != id);
                    for topic in &mut ctx.topics { if let Some(ref mut refs) = topic.snapshot { refs.retain(|t| t.session_id != id); } }
                    inherited_membership = Some((ws, inherited));
                    *context = serde_json::to_value(ctx)?;
                }
            }
            Some(value.to_string())
        } else { None }
    } else { body };
    let body = match (action, body) {
        ("input", Some(b)) => Some(crate::senders::delivered(&selection_read(app, id, b).await?)?),
        (_, b) => b,
    };
    let (body, received) = match action {
        "input" => crate::bots::with_new_instructions(app, id, body).await,
        _ => (body, None),
    };
    let headers = key.map(|k| vec![("idempotency-key".to_string(), k)]).unwrap_or_default();
    let answer = app.runners.request_with(runner, "POST", &format!("/api/sessions/{id}/{action}"), body, headers).await?;
    if (200..300).contains(&answer.0) && let Some((ws, inherited)) = inherited_membership {
        crate::topics::inherit(app, id, &ws, inherited).await?;
    }
    if let Some(r) = received.filter(|_| (200..300).contains(&answer.0)) {
        crate::bots::record_instructions(app, id, r).await;
    }
    Ok(answer)
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

/// The sessions a launch pinned to `from` continues from: the session whose folder it continues in, or the open
/// sessions of its temporary folder there.
fn pinned_sessions(app: &App, spec: &crate::model::LaunchSpec, from: &str) -> Vec<crate::model::Session> {
    use crate::model::Target;
    match &spec.target {
        Target::SessionFolder { session_id } => app.sessions.get(session_id).into_iter().collect(),
        Target::Project { project_id } | Target::Predraft { project_id } => app
            .sessions
            .list()
            .into_iter()
            .filter(|s| s.project_id.as_deref() == Some(project_id.as_str()) && s.runner_id.as_deref() == Some(from) && s.status != crate::model::SessionStatus::Closed)
            .collect(),
        _ => vec![],
    }
}

/// Home: a launch pinned to a runner that does not take it (offline, a stopped Burst job, or connected but
/// paused, outdated or refusing launches from other devices): the sessions it continues from move first to the
/// chosen runner, else the least busy one (their worktrees come back from Home Git, a temporary folder from its
/// files on Home). Ok(true): no runner takes it and the launch goes to Burst, whose job takes the folder first
/// (`move_pinned_to`).
async fn move_pinned(app: &App, spec: &crate::model::LaunchSpec) -> Result<bool> {
    let Some(from) = pinned_runner(app, spec).await? else { return Ok(false) };
    let sessions = pinned_sessions(app, spec, &from);
    let away = match sessions.first() {
        Some(s) => !holds(app, s).await,
        None => !app.runners.connected(&from),
    };
    if !away && app.runners.available().contains(&from) {
        return Ok(false);
    }
    let name = runner_name(app, &from).await;
    // None: no runner takes it, Burst does.
    let to = match &spec.runner_id {
        Some(c) => Some(Some(c.clone()).filter(|c| c != &from && app.runners.available().contains(c)).context("The chosen runner does not take this launch now: it is offline, paused, refuses launches from other devices or needs an update")?),
        None => pick(app, None),
    };
    if to.is_none()
        && let Some(why) = crate::burst::unavailable(app).await
    {
        bail!("{name} has this folder and does not take launches now (offline, paused, refusing launches from other devices or needing an update), no other runner does, and Burst cannot: {why}");
    }
    // All or none: a temporary folder never ends up split between two runners by a refused move.
    if let Some(reason) = sessions.iter().find_map(crate::handoff::blocked) {
        bail!("{name} has this folder and does not take launches now, and its agent cannot move: {reason}");
    }
    let Some(to) = to else { return Ok(true) };
    move_pinned_to(app, spec, &to).await?;
    Ok(false)
}

/// Home: moves what a pinned launch continues from (`pinned_sessions`, or a temporary folder without an open
/// agent) to `to`. Also called by a Burst job once it is up, before it starts the launch.
pub(crate) async fn move_pinned_to(app: &App, spec: &crate::model::LaunchSpec, to: &str) -> Result<()> {
    use crate::model::Target;
    let Some(from) = pinned_runner(app, spec).await?.filter(|f| f != to) else { return Ok(()) };
    let sessions = pinned_sessions(app, spec, &from);
    for s in &sessions {
        crate::handoff::move_to(app, &s.id, to).await?;
    }
    // A temporary folder without an open agent follows the launch.
    if sessions.is_empty()
        && let Target::Project { project_id } | Target::Predraft { project_id } = &spec.target
    {
        crate::projects::move_temp(app, project_id, to).await?;
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

/// Home: starts a session on a runner (sent once, never retried). Home places it like a message (`place`): the
/// chosen runner (no fallback), else the runner it is pinned to (its sessions move first to another runner, else to
/// Burst, when that runner does not take launches), else another runner, else Burst. Runners never pick their own launches: they start only what Home places.
pub async fn launch_anywhere(app: &App, mut spec: crate::model::LaunchSpec, id: String, automatic: bool) -> Result<crate::model::Session> {
    anyhow::ensure!(app.paths.role == crate::role::Role::Home, "only Home launches sessions: a runner starts the ones Home places on it");
    if !spec.topics.is_empty() || !spec.agent_traces.is_empty() {
        let ws = crate::scope::of_spec(app, &spec).await.context("Context needs a workspace")?;
        let inherited = crate::topics::resolve(app, &ws, &mut spec.topics, &mut spec.agent_traces).await?;
        spec.inherited_topics = inherited;
    }
    crate::ref_use::record(app, &id, spec.bot.as_ref().map(|b| b.id.as_str()), &spec.github_refs).await;
    let agent = match spec.from.is_some() {
        true => Some(crate::scope::of_spec(app, &spec).await),
        false => None,
    };
    resolve_selection(app, &mut spec.selection, agent).await?;
    // Uploads are stored on Home: the runner or Burst job receives references and fetches the bytes.
    crate::attachments::to_refs(&id, &mut spec.attachments).await?;
    if spec.runner_id.as_deref() == Some(crate::burst::CHOICE) {
        return crate::burst::launch(app, spec, id, automatic).await;
    }
    // A pinned launch whose runner does not take it, and no other runner does: Burst takes the folder, then the launch.
    if move_pinned(app, &spec).await? {
        return crate::burst::launch(app, spec, id, automatic).await;
    }
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
    // Home records the session on this runner before the runner has it: leases go only to sessions Home placed.
    record_placement(app, &spec, &id, &runner).await?;
    match app.runners.request_with(&runner, "POST", "/api/sessions", Some(serde_json::to_string(&spec)?), vec![("idempotency-key".into(), id.clone())]).await {
        Ok((status, body)) => {
            if !(200..300).contains(&status) {
                unplace(app, &id, &runner).await;
            }
            answer(status, &body)
        }
        // Sent once and never retried: whether the runner started it is unknown. Its own record replaces this one.
        Err(e) => {
            if app.sessions.get(&id).is_some_and(|s| is_placeholder(&s, &runner)) {
                let name = runner_name(app, &runner).await;
                let _ = app.sessions.fail_replica(app, &id, format!("Launchpad could not tell whether {name} started this agent: {e:#}")).await;
            }
            Err(e)
        }
    }
}

/// Home: the record of a launch placed on `runner_id`, before that runner (or Burst job) has it. Its own
/// record, same ID and generation, replaces it (`Sessions::apply_replica`).
pub(crate) fn placed_record(spec: &crate::model::LaunchSpec, id: &str, runner_id: &str) -> Result<crate::model::Session> {
    use crate::model::{Session, SessionStatus, Target};
    let harness = spec.harness.context("pick a harness")?;
    let now = now_ms();
    let title: String = spec.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New agent").chars().take(80).collect();
    Ok(Session {
        id: id.to_string(),
        harness,
        runner_id: Some(runner_id.to_string()),
        generation: 1,
        harness_session_id: None,
        project_id: match &spec.target {
            Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.clone()),
            _ => None,
        },
        cwd: String::new(),
        title,
        title_pending: true,
        title_refreshed: false,
        status: SessionStatus::Starting,
        archived_done: false,
        status_since: now,
        live: false,
        open_question: false,
        model: spec.model.clone(),
        effort: spec.effort.clone(),
        prompt: spec.text.clone(),
        usage: Default::default(),
        started_at: now,
        ended_at: None,
        last_seq: 0,
        launch_spec: spec.clone(),
        quota_pause: None,
        capacity_retry_count: 0,
        capacity_retry_at: None,
        seen_at: now,
        last_message: None,
        unsaved: None,
    })
}

/// A record Home placed on `runner` that the runner has not replaced yet.
fn is_placeholder(s: &crate::model::Session, runner: &str) -> bool {
    s.runner_id.as_deref() == Some(runner) && s.generation == 1 && s.last_seq == 0 && s.status == crate::model::SessionStatus::Starting
}

/// Home: records a new session on `runner_id` before the runner starts it (`vault::check_owner` gives leases
/// only for such sessions). Every launch calls it before relaying: runners start only what Home placed.
pub(crate) async fn record_placement(app: &App, spec: &crate::model::LaunchSpec, id: &str, runner_id: &str) -> Result<()> {
    anyhow::ensure!(app.paths.role == crate::role::Role::Home, "only Home places launches");
    match app.sessions.get(id) {
        Some(s) => anyhow::ensure!(is_placeholder(&s, runner_id), "a session with this id already exists"),
        None => app.sessions.placeholder(app, placed_record(spec, id, runner_id)?).await?,
    }
    if let Some(ws) = crate::scope::of_spec(app, spec).await {
        crate::topics::inherit(app, id, &ws, spec.inherited_topics.clone()).await?;
    }
    Ok(())
}

/// Home: a placed launch the runner refused: its record goes.
async fn unplace(app: &App, id: &str, runner: &str) {
    if app.sessions.get(id).is_some_and(|s| is_placeholder(&s, runner))
        && let Err(e) = app.sessions.forget_local(app, id).await
    {
        tracing::warn!("refused launch {id}: {e:#}");
    }
}

/// A session action (`close`, `stop`, …) wherever the session runs. Archive of a session whose runner is
/// away (offline, a stopped Burst job) is Home's alone (`archive_away`); a runner, and Home for a record
/// without a runner, archive their own record.
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
pub async fn send_anywhere(app: &App, id: &str, text: &str, files: Vec<crate::model::FileAttachment>, key: Option<String>) -> Result<()> {
    send_anywhere_with(app, id, text, files, crate::model::MessageContext::default(), key).await
}

/// `send_anywhere` with the message's context (folders, GitHub links, agent traces), rendered on the session's
/// runner by `Sessions::send_with_context`, like the owner's messages from the message box.
pub async fn send_anywhere_with(app: &App, id: &str, text: &str, mut files: Vec<crate::model::FileAttachment>, context: crate::model::MessageContext, key: Option<String>) -> Result<()> {
    if app.paths.role == crate::role::Role::Home {
        crate::attachments::to_refs(id, &mut files).await?;
        let body = serde_json::json!({"text": text, "files": files, "context": context}).to_string();
        let (status, body) = continue_anywhere(app, id, "input", Some(body), key).await?;
        return answer::<serde_json::Value>(status, &body).map(|_| ());
    }
    let (text, images) = crate::attachments::for_message(app, id, text, &files).await?;
    app.sessions.send_with_context(app, id, &text, images, context, key).await
}

#[cfg(test)]
mod tests {
    /// A launch spec and a message's context name the agents whose traces the runner may then read from Home.
    #[test]
    fn traced_agents_are_found_at_any_depth() {
        let launch = serde_json::json!({"spec": {"agent_traces": [{"session_id": "a", "through_seq": 3}]}, "text": "x"});
        assert_eq!(super::traced_agents(&launch), vec!["a".to_string()]);
        let message = serde_json::json!({"text": "x", "context": {"agent_traces": [{"session_id": "b"}, {"through_seq": 1}]}});
        assert_eq!(super::traced_agents(&message), vec!["b".to_string()]);
        assert!(super::traced_agents(&serde_json::json!({"agent_traces": "a"})).is_empty());
    }

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

    #[test]
    fn a_pending_delivery_is_dropped_only_once_the_runner_answered_it() {
        let ok = Ok((200, String::new()));
        let refused = Ok((400, "no such session".to_string()));
        let dropped: anyhow::Result<(u16, String)> = Err(anyhow::anyhow!("the runner disconnected"));
        assert!(super::settled(false, &ok));
        assert!(super::settled(true, &ok));
        // A refused Archive (the runner no longer has the session) is not sent again; a refused release is.
        assert!(super::settled(false, &refused));
        assert!(!super::settled(true, &refused));
        assert!(!super::settled(false, &dropped));
        assert!(!super::settled(true, &dropped));
    }
}

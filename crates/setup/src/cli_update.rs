//! Agent CLI updates on the runners. Home reads the newest version of each harness's npm package (the
//! `latest` dist-tag, `CliPackage`) every six hours and on Check now, asks each connected runner which version it
//! has, and decides with `plan`: a newer version of the same major version (semver; for 0.x, the same minor) is
//! installed without asking; a new major version waits for the owner's Approve on Today. Home records each
//! update (`cli_update` in `state.db`, one record per harness and version, with one entry per runner) and claims
//! a runner's entry before it asks the runner, so an update is never asked twice after a crash or a failure: only
//! Retry (the owner's click) or a newer version asks again. Settings → Home and runners turns automatic updates
//! off per harness (`[home.cli_updates]` in config.toml).
//!
//! The runner daemon installs (`start_job`), never an agent: it finds how the CLI was installed (`detect`: npm, the
//! CLI's own installer, or something else), waits until no agent of the runner is in a turn (like `self_update`),
//! holds `CLI_INSTALL` so no harness starts meanwhile, runs the update and reads the version again. A CLI in a
//! folder the runner cannot write, or installed another way (a system package), is not touched: the entry says so,
//! with the command to run. Burst jobs and Home's Space use the versions pinned in `deploy/hf-home/release.json`
//! (bumped with `deploy/hf-home/bump_tools.py`), so Burst starts fast and runs what Home's image was built with.
//!
//! Today shows a major version to approve, the newest completed update of each harness (until Mark read) and
//! updates that did not install (Fix).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::harness::CliPackage;
use launchpad_core::role::Role;
use launchpad_core::today::{Filter, Item, Kind, Place};
use launchpad_core::util::now_ms;

const EVERY: Duration = Duration::from_secs(6 * 3600);
/// Follows the updates in progress and sends approved ones to runners that came back.
const FOLLOW: Duration = Duration::from_secs(60);
/// Records handled for this long are deleted.
const KEEP_MS: i64 = 30 * 86_400_000;

// ------------------------------------------------------------------ semver

/// `major.minor.patch`, with an optional pre-release (`-beta.1`); build metadata is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: String,
}

impl Semver {
    /// Reads a version, also with a leading `v`.
    pub fn parse(s: &str) -> Option<Semver> {
        let s = s.trim().trim_start_matches('v');
        let s = s.split('+').next()?;
        let (core, pre) = s.split_once('-').unwrap_or((s, ""));
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let v = Semver { major: parts.next()??, minor: parts.next()??, patch: parts.next()??, pre: pre.to_string() };
        parts.next().is_none().then_some(v)
    }

    /// The versions that stay compatible with this one (semver): the same major version, for 0.x the same minor
    /// version, for 0.0.x only this one.
    fn line(&self) -> (u64, u64, u64) {
        match (self.major, self.minor) {
            (0, 0) => (0, 0, self.patch),
            (0, m) => (0, m, 0),
            (m, _) => (m, 0, 0),
        }
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Semver {
    /// A pre-release comes before its release; pre-releases compare as text.
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(o.major, o.minor, o.patch))
            .then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (false, false) => self.pre.cmp(&o.pre),
            })
    }
}

/// What a runner with `installed` gets, given npm's `latest` and (when fetched) every published version.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Installed without asking: the newest release compatible with `installed`.
    pub auto: Option<String>,
    /// Waits for the owner: `latest`, when it is a new major version.
    pub major: Option<String>,
}

/// The update decision. `latest` is npm's `latest` dist-tag; `versions` (empty when not fetched) lets a runner
/// behind a new major version still get the newest release of its own major version. Pre-releases are never
/// installed, and nothing older than `installed` is.
pub fn plan(installed: &str, latest: &str, versions: &[String]) -> Plan {
    let (Some(i), Some(l)) = (Semver::parse(installed), Semver::parse(latest)) else { return Plan::default() };
    if l <= i || !l.pre.is_empty() {
        return Plan::default();
    }
    if l.line() == i.line() {
        return Plan { auto: Some(latest.to_string()), major: None };
    }
    let auto = versions
        .iter()
        .filter_map(|v| Semver::parse(v).map(|s| (s, v)))
        .filter(|(s, _)| s.pre.is_empty() && s.line() == i.line() && *s > i)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v.clone());
    Plan { auto, major: Some(latest.to_string()) }
}

// ------------------------------------------------------------------ records (Home)

/// Where one runner is with one update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "CliUpdateState")]
pub enum State {
    /// A new major version: waits for the owner's Approve.
    Approval,
    /// Approved or retried: Home asks the runner once it is connected.
    Queued,
    /// Asked; the runner waits until no agent of it is in a turn, then installs.
    Requested,
    Installing,
    Done,
    Failed,
    /// The runner cannot update this CLI (a folder it cannot write, or another installer): `command` says how.
    Manual,
}

impl State {
    fn in_progress(self) -> bool {
        matches!(self, State::Queued | State::Requested | State::Installing)
    }
    fn problem(self) -> bool {
        matches!(self, State::Failed | State::Manual)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "CliUpdateRunner")]
pub struct RunnerEntry {
    pub runner_id: String,
    /// Its name when the entry was written.
    pub runner: String,
    /// The version it had.
    pub from: Option<String>,
    pub state: State,
    /// While it waits: agents in a turn there.
    #[serde(default)]
    pub running: u32,
    pub error: Option<String>,
    /// The command to run on that machine (Manual), or what Launchpad ran.
    pub command: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

/// One version of one harness's CLI and where it was installed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CliUpdate {
    /// `<harness>@<version>`.
    pub id: String,
    pub harness: String,
    pub version: String,
    /// The release notes.
    pub changelog: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub approved_at: Option<i64>,
    /// Mark read (or, for a major version that waits, Skip).
    #[ts(type = "number | null")]
    pub dismissed_at: Option<i64>,
    pub runners: Vec<RunnerEntry>,
}

impl CliUpdate {
    fn entry(&mut self, runner_id: &str) -> Option<&mut RunnerEntry> {
        self.runners.iter_mut().find(|e| e.runner_id == runner_id)
    }
    fn last_at(&self) -> i64 {
        self.runners.iter().map(|e| e.at).max().unwrap_or(self.created_at)
    }
    fn unread(&self, e: &RunnerEntry) -> bool {
        self.dismissed_at.is_none_or(|d| e.at > d)
    }
}

/// The newest version of one harness's CLI on npm.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
pub struct CliLatest {
    pub harness: String,
    pub name: String,
    pub npm: String,
    pub latest: Option<String>,
    /// Automatic updates are on (Settings → Home and runners).
    pub auto: bool,
    pub error: Option<String>,
}

/// Settings → Home and runners: the newest versions, the last check and the updates of the last 30 days.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
pub struct CliUpdates {
    pub harnesses: Vec<CliLatest>,
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
    pub updates: Vec<CliUpdate>,
}

static LATEST: Mutex<Option<(Vec<CliLatest>, i64)>> = Mutex::new(None);
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// One check or follow-up at a time on Home.
static ROUND: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn load(app: &App) -> Result<Vec<CliUpdate>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM cli_update")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|j| serde_json::from_str(&j).ok()).collect())
        })
        .await
}

async fn save(app: &App, u: &CliUpdate) -> Result<()> {
    let (id, json) = (u.id.clone(), serde_json::to_string(u)?);
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO cli_update (id, json) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET json = excluded.json", [id, json])?;
            Ok(())
        })
        .await?;
    app.bus.publish("cli_updates", "changed", Value::Null);
    Ok(())
}

async fn delete(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM cli_update WHERE id = ?1", [id])?)).await?;
    Ok(())
}

fn package(app: &App, harness: &str) -> Option<(launchpad_core::model::HarnessKind, CliPackage)> {
    let id = launchpad_core::model::HarnessKind::parse(harness)?;
    Some((id, app.harnesses.get(id).cli_package()?))
}

fn auto_on(app: &App, harness: &str) -> bool {
    app.config.get().home.cli_updates.get(harness).copied().unwrap_or(true)
}

/// The overview for Settings, without a network call.
pub async fn overview(app: &App) -> Result<CliUpdates> {
    let (harnesses, checked_at) = match LATEST.lock().unwrap().clone() {
        Some((list, at)) => (list, Some(at)),
        None => (vec![], None),
    };
    let mut harnesses = if harnesses.is_empty() { known(app) } else { harnesses };
    for h in &mut harnesses {
        h.auto = auto_on(app, &h.harness);
    }
    let mut updates = load(app).await?;
    updates.sort_by_key(|u| std::cmp::Reverse(u.last_at()));
    Ok(CliUpdates { harnesses, checked_at, updates })
}

/// The harnesses with a CLI package, before any check.
fn known(app: &App) -> Vec<CliLatest> {
    launchpad_core::harness::registrations()
        .iter()
        .filter_map(|r| {
            let p = app.harnesses.get(r.descriptor.id).cli_package()?;
            Some(CliLatest { harness: r.descriptor.id.as_str().into(), name: r.descriptor.name.into(), npm: p.npm.into(), latest: None, auto: auto_on(app, r.descriptor.id.as_str()), error: None })
        })
        .collect()
}

// ------------------------------------------------------------------ npm

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("launchpad-home").build().expect("HTTP client")
});

fn registry() -> String {
    std::env::var("LAUNCHPAD_NPM_REGISTRY").unwrap_or_else(|_| "https://registry.npmjs.org".into()).trim_end_matches('/').to_string()
}

/// npm's `latest` dist-tag of a package (one small manifest).
async fn npm_latest(npm: &str) -> Result<String> {
    let res = CLIENT.get(format!("{}/{}/latest", registry(), npm.replace('/', "%2F"))).send().await.context("reach the npm registry")?;
    anyhow::ensure!(res.status().is_success(), "the npm registry answered {} for {npm}", res.status());
    let v: Value = res.json().await?;
    v["version"].as_str().map(str::to_string).context("the npm registry sent no version")
}

/// Every published version of a package (the abbreviated document), read only when a runner is behind a new
/// major version.
async fn npm_versions(npm: &str) -> Result<Vec<String>> {
    let res = CLIENT
        .get(format!("{}/{}", registry(), npm.replace('/', "%2F")))
        .header("Accept", "application/vnd.npm.install-v1+json")
        .send()
        .await
        .context("reach the npm registry")?;
    anyhow::ensure!(res.status().is_success(), "the npm registry answered {} for {npm}", res.status());
    let v: Value = res.json().await?;
    Ok(v["versions"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default())
}

// ------------------------------------------------------------------ Home: check and follow

/// What a runner answers on `GET /api/runner/clis`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct RunnerClis {
    /// The version of each harness's CLI there; None: not installed.
    pub installed: BTreeMap<String, Option<String>>,
    pub jobs: Vec<Job>,
}

/// A connected runner that updates its CLIs.
struct Target {
    id: String,
    name: String,
}

async fn targets(app: &App) -> Vec<Target> {
    launchpad_core::runners::list(app)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.connected && !r.runner.burst && r.runner.capabilities.cli_update && r.runner.state == launchpad_core::runners::RunnerState::Approved)
        .map(|r| Target { id: r.runner.id, name: r.runner.name })
        .collect()
}

async fn runner_clis(app: &App, id: &str) -> Result<RunnerClis> {
    let (status, body) = app.runners.request(id, "GET", "/api/runner/clis", None).await?;
    anyhow::ensure!(status == 200, "the runner answered {status}");
    Ok(serde_json::from_str(&body)?)
}

/// Reads npm now, then updates the runners. Home only.
pub async fn check(app: &App) -> Result<CliUpdates> {
    anyhow::ensure!(app.paths.role != Role::Runner, "Home checks the agent CLIs");
    {
        let _round = ROUND.lock().await;
        let mut latest = vec![];
        let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for mut h in known(app) {
            if h.auto {
                match npm_latest(&h.npm).await {
                    Ok(v) => h.latest = Some(v),
                    Err(e) => h.error = Some(format!("{e:#}")),
                }
            }
            latest.push(h);
        }
        *LATEST.lock().unwrap() = Some((latest.clone(), now_ms()));
        let mut records = load(app).await?;
        for t in targets(app).await {
            let clis = match runner_clis(app, &t.id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("CLI updates: {}: {e:#}", t.name);
                    continue;
                }
            };
            for h in latest.iter().filter(|h| h.auto) {
                let (Some(l), Some(Some(installed))) = (&h.latest, clis.installed.get(&h.harness)) else { continue };
                let mut p = plan(installed, l, &[]);
                if p.major.is_some() {
                    if !versions.contains_key(&h.npm) {
                        let list = npm_versions(&h.npm).await.unwrap_or_else(|e| {
                            tracing::warn!("CLI updates: {}: {e:#}", h.npm);
                            vec![]
                        });
                        versions.insert(h.npm.clone(), list);
                    }
                    p = plan(installed, l, &versions[&h.npm]);
                }
                if let Some(v) = p.auto {
                    add(app, &mut records, &t, &h.harness, &v, installed, State::Queued).await?;
                }
                if let Some(v) = p.major {
                    add(app, &mut records, &t, &h.harness, &v, installed, State::Approval).await?;
                }
            }
        }
        prune(app, &mut records).await?;
    }
    follow(app, true).await?;
    overview(app).await
}

/// Adds a runner's entry to the record of `harness@version`, unless it has one (an update is asked once).
async fn add(app: &App, records: &mut Vec<CliUpdate>, t: &Target, harness: &str, version: &str, from: &str, state: State) -> Result<()> {
    let id = format!("{harness}@{version}");
    let Some((_, pkg)) = package(app, harness) else { return Ok(()) };
    if !records.iter().any(|u| u.id == id) {
        records.push(CliUpdate { id: id.clone(), harness: harness.into(), version: version.into(), changelog: pkg.changelog.replace("{version}", version), created_at: now_ms(), approved_at: None, dismissed_at: None, runners: vec![] });
    }
    let u = records.iter_mut().find(|u| u.id == id).expect("record");
    if u.entry(&t.id).is_some() {
        return Ok(());
    }
    // An approved major version installs on runners that come later too.
    let state = if state == State::Approval && u.approved_at.is_some() { State::Queued } else { state };
    u.runners.push(RunnerEntry { runner_id: t.id.clone(), runner: t.name.clone(), from: Some(from.into()), state, running: 0, error: None, command: None, at: now_ms() });
    let u = u.clone();
    save(app, &u).await?;
    // A newer major version replaces this runner's older ones that still wait for approval.
    if state == State::Approval {
        for old in records.iter_mut().filter(|o| o.harness == harness && o.id != id) {
            let before = old.runners.len();
            old.runners.retain(|e| !(e.runner_id == t.id && e.state == State::Approval));
            if old.runners.len() != before {
                if old.runners.is_empty() { delete(app, &old.id).await?; } else { save(app, old).await?; }
            }
        }
        records.retain(|o| !o.runners.is_empty());
    }
    Ok(())
}

/// Deletes records handled more than 30 days ago.
async fn prune(app: &App, records: &mut Vec<CliUpdate>) -> Result<()> {
    let old = now_ms() - KEEP_MS;
    for u in records.iter().filter(|u| u.last_at() < old && !u.runners.iter().any(|e| e.state.in_progress() || e.state == State::Approval)) {
        delete(app, &u.id).await?;
    }
    records.retain(|u| u.last_at() >= old || u.runners.iter().any(|e| e.state.in_progress() || e.state == State::Approval));
    Ok(())
}

/// Sends queued entries to their runners (claimed first: Requested is saved before the request) and copies the
/// runners' progress into the records. `all`: also looks again at updates that did not install (the owner may
/// have run the command), at each check.
async fn follow(app: &App, all: bool) -> Result<()> {
    let _round = ROUND.lock().await;
    let mut records = load(app).await?;
    let open = |e: &RunnerEntry| e.state.in_progress() || (all && e.state.problem());
    if !records.iter().any(|u| u.runners.iter().any(open)) {
        return Ok(());
    }
    for t in targets(app).await {
        if !records.iter().any(|u| u.runners.iter().any(|e| e.runner_id == t.id && open(e))) {
            continue;
        }
        let Ok(clis) = runner_clis(app, &t.id).await else { continue };
        // Harnesses asked in this pass: one update per CLI at a time.
        let mut sent: Vec<String> = vec![];
        for u in records.iter_mut() {
            let (harness, version) = (u.harness.clone(), u.version.clone());
            let installed = clis.installed.get(&harness).cloned().flatten();
            let job = clis.jobs.iter().find(|j| j.harness == harness && j.version == version).cloned();
            let busy = sent.contains(&harness) || clis.jobs.iter().any(|j| j.harness == harness && !j.state.finished());
            let Some(i) = u.runners.iter().position(|e| e.runner_id == t.id) else { continue };
            let before = u.runners[i].clone();
            let has = installed.as_deref().and_then(Semver::parse).zip(Semver::parse(&version)).is_some_and(|(i, v)| i >= v);
            match (before.state, job) {
                // Installed meanwhile (by the owner, or a newer update): nothing left to do.
                (s, None) if (s.problem() || s.in_progress()) && has => {
                    u.runners[i].state = State::Done;
                    u.runners[i].error = None;
                }
                (State::Queued, _) if !busy => {
                    // Claimed before the request: never asked twice.
                    u.runners[i].state = State::Requested;
                    u.runners[i].at = now_ms();
                    sent.push(harness.clone());
                    save(app, u).await?;
                    let body = serde_json::json!({"harness": harness, "version": version}).to_string();
                    let answer = app.runners.request(&t.id, "POST", "/api/runner/clis/update", Some(body)).await;
                    let e = &mut u.runners[i];
                    match answer {
                        Ok((200, body)) => {
                            if let Ok(j) = serde_json::from_str::<Job>(&body) {
                                j.copy_to(e);
                            }
                        }
                        Ok((status, body)) => {
                            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                            e.state = State::Failed;
                            e.error = Some(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("the runner answered {status}")));
                        }
                        Err(err) => {
                            e.state = State::Failed;
                            e.error = Some(format!("{err:#}"));
                        }
                    }
                }
                (State::Requested | State::Installing, Some(j)) => j.copy_to(&mut u.runners[i]),
                // The runner restarted before it finished: never asked again on its own.
                (State::Requested | State::Installing, None) => {
                    u.runners[i].state = State::Failed;
                    u.runners[i].error = Some("The runner restarted before the update finished. Retry asks it again.".into());
                }
                _ => {}
            }
            if u.runners[i] != before {
                if u.runners[i].state != before.state {
                    u.runners[i].at = now_ms();
                }
                save(app, u).await?;
            }
        }
    }
    Ok(())
}

/// Home: the first check a few minutes after start, then every six hours; follows updates in progress every
/// minute.
pub fn spawn(app: &App) {
    if app.paths.role == Role::Runner {
        return;
    }
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(180)).await;
        loop {
            if let Err(e) = check(&a).await {
                tracing::warn!("CLI updates: {e:#}");
            }
            tokio::time::sleep(EVERY).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(FOLLOW) => {}
                _ = WAKE.notified() => {}
            }
            if let Err(e) = follow(&a, false).await {
                tracing::warn!("CLI updates: {e:#}");
            }
        }
    });
}

async fn change(app: &App, id: &str, f: impl FnOnce(&mut CliUpdate) -> Result<()>) -> Result<CliUpdate> {
    let _round = ROUND.lock().await;
    let mut u = load(app).await?.into_iter().find(|u| u.id == id).context("no such CLI update")?;
    f(&mut u)?;
    save(app, &u).await?;
    WAKE.notify_one();
    Ok(u)
}

/// Approve: the runners that wait for this major version install it.
pub async fn approve(app: &App, id: &str) -> Result<CliUpdate> {
    change(app, id, |u| {
        let waiting: Vec<_> = u.runners.iter_mut().filter(|e| e.state == State::Approval).collect();
        anyhow::ensure!(!waiting.is_empty(), "no runner waits for this version");
        for e in waiting {
            e.state = State::Queued;
            e.at = now_ms();
        }
        u.approved_at = Some(now_ms());
        Ok(())
    })
    .await
}

/// Retry: asks the runners where it did not install again.
pub async fn retry(app: &App, id: &str) -> Result<CliUpdate> {
    change(app, id, |u| {
        let failed: Vec<_> = u.runners.iter_mut().filter(|e| e.state.problem()).collect();
        anyhow::ensure!(!failed.is_empty(), "nothing to retry");
        for e in failed {
            e.state = State::Queued;
            e.error = None;
            e.at = now_ms();
        }
        Ok(())
    })
    .await
}

pub async fn dismiss(app: &App, id: &str) -> Result<CliUpdate> {
    change(app, id, |u| {
        u.dismissed_at = Some(now_ms());
        Ok(())
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auto {
    pub harness: String,
    pub on: bool,
}

/// Turns automatic updates of one harness's CLI on or off (config.toml).
pub async fn set_auto(app: &App, b: Auto) -> Result<CliUpdates> {
    anyhow::ensure!(package(app, &b.harness).is_some(), "no CLI updates for {}", b.harness);
    app.config.update(|c| {
        if b.on { c.home.cli_updates.remove(&b.harness); } else { c.home.cli_updates.insert(b.harness.clone(), false); }
        Ok(())
    })?;
    app.bus.publish("cli_updates", "changed", Value::Null);
    overview(app).await
}

// ------------------------------------------------------------------ Today

fn name(harness: &str) -> String {
    launchpad_core::model::HarnessKind::parse(harness).map(|h| h.display().to_string()).unwrap_or_else(|| harness.to_string())
}

fn names(entries: &[&RunnerEntry]) -> String {
    entries.iter().map(|e| e.runner.as_str()).collect::<Vec<_>>().join(", ")
}

/// The Today items of the records: a major version to approve, updates that did not install, and the newest
/// completed update of each harness until Mark read (older ones and read ones go to the Done list).
pub fn items(records: &[CliUpdate]) -> Vec<Item> {
    let mut out = vec![];
    let mut newest_done: BTreeMap<&str, (Semver, &str)> = BTreeMap::new();
    for u in records {
        if u.runners.iter().any(|e| e.state == State::Done)
            && let Some(v) = Semver::parse(&u.version)
            && newest_done.get(u.harness.as_str()).is_none_or(|(n, _)| v > *n)
        {
            newest_done.insert(&u.harness, (v, &u.id));
        }
    }
    for u in records {
        let name = name(&u.harness);
        let approval: Vec<_> = u.runners.iter().filter(|e| e.state == State::Approval).collect();
        let problems: Vec<_> = u.runners.iter().filter(|e| e.state.problem() && u.unread(e)).collect();
        let done: Vec<_> = u.runners.iter().filter(|e| e.state == State::Done).collect();
        let waiting: Vec<_> = u.runners.iter().filter(|e| e.state.in_progress()).collect();
        let item = |filter, title: String, at| Item::new(Kind::CliUpdate, filter, &u.id, title, Place::Everywhere, at);
        if !approval.is_empty() && u.dismissed_at.is_none() {
            let from: Vec<String> = approval.iter().map(|e| format!("{} {}", e.runner, e.from.as_deref().unwrap_or("?"))).collect();
            out.push(item(Filter::Approve, format!("Update {name} to {}: new major version", u.version), u.created_at).detail(format!("{} · read the release notes, then approve", from.join(", "))).action("cli-update.approve"));
        } else if !problems.is_empty() {
            let first = problems[0];
            let why = first.error.clone().unwrap_or_default();
            let detail = match &first.command { Some(c) if first.state == State::Manual => format!("{why} Run: {c}"), _ => why };
            let action = if problems.iter().any(|e| e.state == State::Failed) { "cli-update.retry" } else { "cli-update.dismiss" };
            out.push(item(Filter::Fix, format!("{name} {} did not install on {}", u.version, names(&problems)), problems.iter().map(|e| e.at).max().unwrap_or(u.created_at)).detail(detail.trim().to_string()).action(action));
        } else if !done.is_empty() {
            let at = done.iter().map(|e| e.at).max().unwrap_or(u.created_at);
            let from: Vec<String> = done.iter().map(|e| format!("{} {} → {}", e.runner, e.from.as_deref().unwrap_or("?"), u.version)).collect();
            let mut detail = from.join(", ");
            if !waiting.is_empty() {
                detail.push_str(&format!(" · {} waits for its agents", names(&waiting)));
            }
            let newest = newest_done.get(u.harness.as_str()).is_some_and(|(_, id)| *id == u.id);
            let unread = done.iter().any(|e| u.unread(e));
            let i = item(Filter::Review, format!("{name} updated to {}", u.version), at).detail(detail).action("cli-update.dismiss");
            out.push(if newest && unread { i } else { i.handled(format!("Updated on {}", names(&done))) });
        } else if !approval.is_empty() {
            out.push(item(Filter::Approve, format!("{name} {}: skipped", u.version), u.dismissed_at.unwrap_or(u.created_at)).handled("Skipped"));
        }
    }
    out
}

pub async fn today_items(app: &App) -> Vec<Item> {
    if app.paths.role == Role::Runner {
        return vec![];
    }
    match load(app).await {
        Ok(records) => items(&records),
        Err(e) => {
            tracing::warn!("Today: CLI updates: {e:#}");
            vec![]
        }
    }
}

// ------------------------------------------------------------------ runner: install

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "CliJobState")]
pub enum JobState {
    /// Waits until no agent of this runner is in a turn.
    Waiting,
    Installing,
    Done,
    Failed,
    Manual,
}

impl JobState {
    fn finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Manual)
    }
}

/// An update on this runner, kept in memory (a restart forgets it; Home then says so).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "CliJob")]
pub struct Job {
    pub harness: String,
    pub version: String,
    pub from: Option<String>,
    pub state: JobState,
    pub running: u32,
    pub error: Option<String>,
    pub command: Option<String>,
}

impl Job {
    fn copy_to(&self, e: &mut RunnerEntry) {
        e.state = match self.state {
            JobState::Waiting => State::Requested,
            JobState::Installing => State::Installing,
            JobState::Done => State::Done,
            JobState::Failed => State::Failed,
            JobState::Manual => State::Manual,
        };
        e.running = self.running;
        e.error = self.error.clone();
        e.command = self.command.clone();
    }
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
/// One install at a time: npm installs of several CLIs share a prefix.
static INSTALLING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Seconds without any agent in a turn before an install starts (the turn-end push and sync finish first).
const QUIET: u32 = 4;
const CHECK: Duration = Duration::from_secs(5);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

fn set_job(j: Job) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.retain(|x| x.harness != j.harness);
    jobs.push(j);
}

fn job(harness: &str) -> Option<Job> {
    JOBS.lock().unwrap().iter().find(|j| j.harness == harness).cloned()
}

/// `GET /api/runner/clis`: this runner's CLI versions and updates.
pub async fn runner_state(app: &App) -> Result<RunnerClis> {
    anyhow::ensure!(launchpad_core::self_update::supported(app), "only a paired runner updates its agent CLIs");
    launchpad_core::status::refresh_versions(app).await;
    let status = app.status.get();
    let installed = launchpad_core::harness::registrations()
        .iter()
        .filter(|r| app.harnesses.get(r.descriptor.id).cli_package().is_some())
        .map(|r| (r.descriptor.id.as_str().to_string(), status.harnesses.get(r.descriptor.id.as_str()).and_then(|h| h.version.clone())))
        .collect();
    Ok(RunnerClis { installed, jobs: JOBS.lock().unwrap().clone() })
}

/// How a CLI was installed, from the resolved path of its executable.
#[derive(Debug, PartialEq)]
pub enum Method {
    /// `npm install --global` into this prefix.
    Npm { prefix: PathBuf },
    /// Its own installer: run the executable with these arguments.
    Native { args: Vec<String> },
    /// Another installer (a system package, Homebrew…): Launchpad leaves it alone.
    Other,
}

pub fn detect(resolved: &Path, pkg: &CliPackage, version: &str) -> Method {
    let path = resolved.to_string_lossy();
    let marker = format!("/lib/node_modules/{}/", pkg.npm);
    if let Some(at) = path.find(&marker) {
        return Method::Npm { prefix: PathBuf::from(&path[..at]) };
    }
    if let Some((mark, args)) = pkg.native
        && path.contains(mark)
    {
        return Method::Native { args: args.iter().map(|a| a.replace("{version}", version)).collect() };
    }
    Method::Other
}

/// Whether the runner's user can create files in `dir`.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".launchpad-write-check-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn shell_words(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ")
}

/// `POST /api/runner/clis/update`: Home asks this runner to install `version` of a harness's CLI. It answers at
/// once; the install waits until no agent here is in a turn.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub harness: String,
    pub version: String,
}

pub async fn start_job(app: &App, b: Request) -> Result<Job> {
    anyhow::ensure!(launchpad_core::self_update::supported(app), "only a paired runner updates its agent CLIs");
    let (id, pkg) = package(app, &b.harness).context("this harness has no CLI package")?;
    anyhow::ensure!(Semver::parse(&b.version).is_some_and(|v| v.pre.is_empty()), "not a release version: {}", b.version);
    if let Some(j) = job(&b.harness)
        && !j.state.finished()
    {
        anyhow::ensure!(j.version == b.version, "{} {} is already being installed here", id.display(), j.version);
        return Ok(j);
    }
    let from = app.status.get().harnesses.get(id.as_str()).and_then(|h| h.version.clone());
    let mut j = Job { harness: b.harness.clone(), version: b.version.clone(), from, state: JobState::Waiting, running: 0, error: None, command: None };
    let spec = format!("{}@{}", pkg.npm, b.version);
    let (program, args) = match app.env.which(pkg.program) {
        None => {
            j.state = JobState::Manual;
            j.error = Some(format!("{} is not on this runner's PATH.", pkg.program));
            j.command = Some(format!("npm install --global {spec}"));
            (String::new(), vec![])
        }
        Some(exe) => {
            let resolved = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
            match detect(&resolved, &pkg, &b.version) {
                Method::Npm { prefix } => {
                    let args = vec!["install".into(), "--global".into(), "--prefix".into(), prefix.to_string_lossy().into_owned(), spec.clone()];
                    if writable(&prefix.join("lib/node_modules")) && writable(&prefix.join("bin")) {
                        ("npm".to_string(), args)
                    } else {
                        j.state = JobState::Manual;
                        j.error = Some(format!("{} is installed in {}, which this runner's user cannot write.", pkg.program, prefix.display()));
                        j.command = Some(format!("sudo {}", shell_words("npm", &args)));
                        (String::new(), vec![])
                    }
                }
                Method::Native { args } => {
                    let dir = resolved.parent().map(Path::to_path_buf).unwrap_or_default();
                    if writable(&dir) {
                        (exe.to_string_lossy().into_owned(), args)
                    } else {
                        j.state = JobState::Manual;
                        j.error = Some(format!("{} is installed in {}, which this runner's user cannot write.", pkg.program, dir.display()));
                        j.command = Some(shell_words(&exe.to_string_lossy(), &args));
                        (String::new(), vec![])
                    }
                }
                Method::Other => {
                    j.state = JobState::Manual;
                    j.error = Some(format!("{} comes from {}, not from npm or its own installer: update it the way you installed it.", pkg.program, resolved.display()));
                    (String::new(), vec![])
                }
            }
        }
    };
    if j.state == JobState::Waiting {
        j.command = Some(shell_words(&program, &args));
    }
    set_job(j.clone());
    if j.state == JobState::Waiting {
        let app = app.clone();
        tokio::spawn(async move {
            let result = install(&app, &program, &args, &b.harness, &b.version).await;
            if let Some(mut j) = job(&b.harness).filter(|j| j.version == b.version) {
                match result {
                    Ok(()) => j.state = JobState::Done,
                    Err(e) => {
                        tracing::warn!("CLI update {}@{}: {e:#}", b.harness, b.version);
                        j.state = JobState::Failed;
                        j.error = Some(format!("{e:#}"));
                    }
                }
                j.running = 0;
                set_job(j);
            }
        });
    }
    Ok(j)
}

/// Waits until no agent is in a turn, then installs with `CLI_INSTALL` held, and checks the new version.
async fn install(app: &App, program: &str, args: &[String], harness: &str, version: &str) -> Result<()> {
    let _one = INSTALLING.lock().await;
    let _gate = loop {
        let mut quiet = 0;
        while quiet < QUIET {
            tokio::time::sleep(CHECK).await;
            let n = launchpad_core::self_update::running(app);
            quiet = if n == 0 { quiet + 1 } else { 0 };
            if let Some(mut j) = job(harness).filter(|j| j.version == version && j.running != n) {
                j.running = n;
                set_job(j);
            }
        }
        // No harness starts while the CLI is replaced; a session that started meanwhile makes it wait again.
        let gate = launchpad_core::harness::CLI_INSTALL.write().await;
        if launchpad_core::self_update::running(app) == 0 {
            break gate;
        }
    };
    if let Some(mut j) = job(harness).filter(|j| j.version == version) {
        j.state = JobState::Installing;
        j.running = 0;
        set_job(j);
    }
    tracing::info!("CLI update: {program} {}", args.join(" "));
    let mut cmd = launchpad_core::procs::command(&app.env, program, None);
    cmd.args(args).stdin(launchpad_core::exec::Io::Null);
    let out = tokio::time::timeout(INSTALL_TIMEOUT, cmd.output()).await.context("the install took more than 10 minutes")?.with_context(|| format!("run {program}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).into_owned();
        let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
        anyhow::bail!("{} exited with {}: {}", program.rsplit('/').next().unwrap_or(program), out.status, tail.into_iter().rev().collect::<Vec<_>>().join(" "));
    }
    launchpad_core::status::refresh_versions(app).await;
    let now = app.status.get().harnesses.get(harness).and_then(|h| h.version.clone());
    let ok = now.as_deref().and_then(Semver::parse).zip(Semver::parse(version)).is_some_and(|(n, v)| n == v);
    anyhow::ensure!(ok, "installed, but the CLI on PATH still reports {}: another copy comes first on PATH", now.as_deref().unwrap_or("no version"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn same_major_installs_without_asking() {
        assert_eq!(plan("2.1.311", "2.1.315", &[]), Plan { auto: Some("2.1.315".into()), major: None });
        assert_eq!(plan("1.18.35", "1.19.0", &[]), Plan { auto: Some("1.19.0".into()), major: None });
        // Up to date, older or unreadable: nothing.
        assert_eq!(plan("2.1.315", "2.1.315", &[]), Plan::default());
        assert_eq!(plan("2.1.316", "2.1.315", &[]), Plan::default());
        assert_eq!(plan("garbage", "2.1.315", &[]), Plan::default());
        assert_eq!(plan("2.1.315", "", &[]), Plan::default());
    }

    #[test]
    fn a_major_version_waits_for_approval() {
        assert_eq!(plan("2.1.311", "3.0.0", &[]), Plan { auto: None, major: Some("3.0.0".into()) });
        // Behind a new major version, the newest release of its own major version still installs.
        let all = s(&["2.1.311", "2.1.320", "2.2.0-beta.1", "2.1.319", "3.0.0", "1.9.9"]);
        assert_eq!(plan("2.1.311", "3.0.0", &all), Plan { auto: Some("2.1.320".into()), major: Some("3.0.0".into()) });
    }

    #[test]
    fn zero_minor_bumps_are_major() {
        // Semver: 0.x minors may break (Codex is 0.x; its app-server protocol changes between minors).
        assert_eq!(plan("0.166.0", "0.166.2", &[]), Plan { auto: Some("0.166.2".into()), major: None });
        assert_eq!(plan("0.166.0", "0.167.0", &[]), Plan { auto: None, major: Some("0.167.0".into()) });
        assert_eq!(plan("0.166.0", "0.167.0", &s(&["0.166.1", "0.166.3", "0.167.0"])), Plan { auto: Some("0.166.3".into()), major: Some("0.167.0".into()) });
        // 0.0.x: every release may break.
        assert_eq!(plan("0.0.3", "0.0.4", &[]), Plan { auto: None, major: Some("0.0.4".into()) });
        assert_eq!(plan("0.9.1", "1.0.0", &[]), Plan { auto: None, major: Some("1.0.0".into()) });
    }

    #[test]
    fn pre_releases_never_install() {
        assert_eq!(plan("2.1.311", "2.1.312-beta.1", &[]), Plan::default());
        // A pre-release installed by hand gets its release.
        assert_eq!(plan("2.1.312-beta.1", "2.1.312", &[]), Plan { auto: Some("2.1.312".into()), major: None });
    }

    #[test]
    fn reads_versions() {
        assert_eq!(Semver::parse("v1.2.3+build").map(|v| (v.major, v.minor, v.patch)), Some((1, 2, 3)));
        assert!(Semver::parse("1.2").is_none() && Semver::parse("1.2.3.4").is_none() && Semver::parse("1.x.3").is_none());
        assert!(Semver::parse("1.10.0").unwrap() > Semver::parse("1.9.9").unwrap());
        assert!(Semver::parse("1.0.0-rc.1").unwrap() < Semver::parse("1.0.0").unwrap());
    }

    #[test]
    fn finds_how_the_cli_was_installed() {
        let claude = CliPackage { npm: "@anthropic-ai/claude-code", program: "claude", native: Some(("/.local/share/claude/", &["install", "{version}"])), changelog: "" };
        assert_eq!(detect(Path::new("/home/u/.nvm/versions/node/v24/lib/node_modules/@anthropic-ai/claude-code/cli.js"), &claude, "2.1.3"), Method::Npm { prefix: "/home/u/.nvm/versions/node/v24".into() });
        assert_eq!(detect(Path::new("/usr/lib/node_modules/@anthropic-ai/claude-code/bin/claude"), &claude, "2.1.3"), Method::Npm { prefix: "/usr".into() });
        assert_eq!(detect(Path::new("/home/u/.local/share/claude/versions/2.1.0"), &claude, "2.1.3"), Method::Native { args: s(&["install", "2.1.3"]) });
        assert_eq!(detect(Path::new("/opt/homebrew/Cellar/claude/2.1.0/bin/claude"), &claude, "2.1.3"), Method::Other);
    }

    fn record(id: &str, entries: Vec<(State, i64)>, dismissed: Option<i64>) -> CliUpdate {
        let (harness, version) = id.split_once('@').unwrap();
        CliUpdate {
            id: id.into(), harness: harness.into(), version: version.into(), changelog: String::new(), created_at: 1, approved_at: None, dismissed_at: dismissed,
            runners: entries.into_iter().enumerate().map(|(i, (state, at))| RunnerEntry { runner_id: format!("r{i}"), runner: format!("r{i}"), from: Some("1.0.0".into()), state, running: 0, error: Some("why".into()), command: None, at }).collect(),
        }
    }

    #[test]
    fn today_shows_the_newest_completed_update_and_problems() {
        let list = [
            record("h@1.0.1", vec![(State::Done, 10)], None),
            record("h@1.0.2", vec![(State::Done, 20), (State::Requested, 20)], None),
            record("h@2.0.0", vec![(State::Approval, 5)], None),
            record("h@1.0.3", vec![(State::Failed, 30)], None),
            record("h@1.0.4", vec![(State::Manual, 30)], Some(40)),
        ];
        let items = items(&list);
        let waiting: Vec<_> = items.iter().filter(|i| i.outcome.is_none()).map(|i| (i.record.as_str(), i.filter, i.action.as_deref())).collect();
        assert_eq!(waiting, vec![("h@1.0.2", Filter::Review, Some("cli-update.dismiss")), ("h@2.0.0", Filter::Approve, Some("cli-update.approve")), ("h@1.0.3", Filter::Fix, Some("cli-update.retry"))]);
        // The older completed update goes to the Done list; a problem marked read leaves Today.
        assert!(items.iter().any(|i| i.record == "h@1.0.1" && i.outcome.is_some()));
        assert!(!items.iter().any(|i| i.record == "h@1.0.4"));
        assert!(items.iter().find(|i| i.record == "h@1.0.2").unwrap().detail.contains("waits for its agents"));
    }
}

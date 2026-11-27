//! The indexer (Home): gives finished turns to Funes, one memory per workspace.
//!
//! - After a turn (`turn_ended`, `turn_saved`): the session is marked, and the worker indexes what is new in it.
//! - Backfill (`recall.backfill`): every stored session of a workspace, in the background. The ledger makes it
//!   idempotent (a session's cursor moves only after Funes accepted its turns) and resumable; the workspace then
//!   stays in `home.recall.backfill`, so Home continues after a restart and indexes again when the memory is gone
//!   (the memory is derived and not in Home's backups).
//! - One writer at a time for all of Home (`Service::writer`): Funes allows one per memory, and Home's CPU is small.
//!
//! Nothing here can fail a turn, a backup or Home's start: every error ends in the log, the ledger or the report.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::model::{Session, SessionStatus};
use launchpad_core::util::now_ms;
use serde::{Deserialize, Serialize};

use crate::convert::{self, CONVERTER_VERSION};
use crate::funes;
use crate::ledger::{Ledger, TurnRow};

/// A session that does not run is final once quiet for this long: its end without a `turn_end` (stopped, failed)
/// is indexed too.
const SETTLE_MS: i64 = 10 * 60 * 1000;
/// Turns per `funes index` call.
const FILE_TURNS: usize = 200;
const INDEX_LIMIT: Duration = Duration::from_secs(30 * 60);
/// How often a session marked after a turn is looked at again while its events are not all on Home.
const MAX_ATTEMPTS: u32 = 5;
/// A session that failed this many passes in a row no longer makes the worker start a catch-up pass.
const GIVE_UP: i64 = 3;

#[derive(Default)]
pub struct Service {
    /// Sessions with a finished turn to index, with how often they were tried.
    dirty: Mutex<HashMap<String, u32>>,
    wake: tokio::sync::Notify,
    writer: tokio::sync::Mutex<()>,
    /// One search at a time for all of Home: each loads both models (about 0.9 GB), and Home's CPU is small.
    pub(crate) reader: tokio::sync::Mutex<()>,
    /// Workspaces whose backfill runs now.
    running: Mutex<HashSet<String>>,
}

pub fn service(app: &App) -> std::sync::Arc<Service> {
    app.extras.get::<Service>()
}

pub fn enabled(app: &App) -> bool {
    app.paths.role == launchpad_core::role::Role::Home && app.config.get().home.recall.enabled
}

/// A workspace's folder: `<data>/recall/<workspace>` with `funes/` (its `FUNES_HOME`), `ledger.db` and `spool/`.
pub fn workspace_dir(app: &App, ws: &str) -> Result<PathBuf> {
    ensure!(!ws.is_empty() && ws.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "not a workspace ID: {ws}");
    Ok(funes::root(app).join("ws").join(ws))
}

/// The sessions of a workspace, oldest first, archived ones included.
pub async fn workspace_sessions(app: &App, ws: &str) -> Result<Vec<Session>> {
    let projects: HashSet<String> = launchpad_core::workspace::project_ids(app, ws).await?.into_iter().collect();
    let mut all: Vec<Session> = app
        .sessions
        .list()
        .into_iter()
        .filter(|s| match &s.project_id {
            Some(p) => projects.contains(p),
            None => s.launch_spec.workspace_id.as_deref() == Some(ws),
        })
        .collect();
    all.sort_by_key(|s| s.started_at);
    Ok(all)
}

/// Why a session is left out of recall (None: it is indexed).
pub fn left_out(config: &launchpad_core::config::RecallConfig, s: &Session) -> Option<&'static str> {
    if config.hidden.iter().any(|h| h == &s.id) {
        Some("hidden")
    } else if s.status == SessionStatus::Closed && !config.include_archived {
        Some("archived")
    } else {
        None
    }
}

/// Opens a workspace's ledger for writing. A memory built by another converter, or whose Funes files are gone,
/// starts again from nothing: its turn IDs would not match what this converter emits.
fn open_for_writing(app: &App, ws: &str) -> Result<(PathBuf, Ledger)> {
    let dir = workspace_dir(app, ws)?;
    let version = CONVERTER_VERSION.to_string();
    if dir.join("ledger.db").exists() {
        let ledger = Ledger::open(&dir)?;
        let stale = ledger.meta("converter")?.is_some_and(|v| v != version);
        let lost = ledger.totals()?.turns > 0 && !dir.join("funes").join("memory").exists();
        if stale || lost {
            tracing::info!("recall: the memory of workspace {ws} starts again ({})", if stale { "another converter version" } else { "its Funes files are gone" });
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
    }
    let ledger = Ledger::open(&dir)?;
    if ledger.meta("converter")?.is_none() {
        ledger.set_meta("converter", &version)?;
        ledger.set_meta("created_at", &now_ms().to_string())?;
    }
    Ok((dir, ledger))
}

/// The new turns of one session, ready for Funes.
struct Unit {
    session_id: String,
    /// The cursor once these turns are in.
    cursor: i64,
    rows: Vec<TurnRow>,
    lines: Vec<String>,
    blocks: i64,
    truncated: i64,
}

enum Prepared {
    /// Nothing new, or its turn is not finished.
    Nothing { lagging: bool },
    Skipped(&'static str),
    Units(Vec<Unit>),
}

async fn prepare(app: &App, ledger: &Ledger, s: &Session) -> Result<Prepared> {
    let config = app.config.get().home.recall;
    let row = ledger.session(&s.id)?;
    if let Some(reason) = left_out(&config, s) {
        if row.skip.as_deref() != Some(reason) {
            ledger.set_skip(&s.id, Some(reason), now_ms())?;
        }
        return Ok(Prepared::Skipped(reason));
    }
    if s.last_seq <= row.cursor {
        // Unarchived or shown again with nothing new: it counts as indexed again.
        if matches!(row.skip.as_deref(), Some("archived" | "hidden")) {
            ledger.commit(&s.id, row.cursor, &[], 0, 0, now_ms())?;
        }
        // Counted like `plan` counts it: left out, not up to date.
        return Ok(match row.skip.as_deref() {
            Some("empty") => Prepared::Skipped("empty"),
            Some("unconvertible") => Prepared::Skipped("unconvertible"),
            _ => Prepared::Nothing { lagging: false },
        });
    }
    let events = app.sessions.events(app, &s.id, row.cursor).await?;
    let settled = !s.status.is_running() && now_ms() - s.status_since > SETTLE_MS;
    let through = convert::complete_through(&events, settled);
    if through <= row.cursor {
        if settled && events.is_empty() {
            if row.cursor == 0 {
                // It has stored events, and none of them reads. The cursor moves, so it is not counted as left to
                // index (and the worker starts no catch-up pass for it); a new event makes it tried again.
                ledger.commit(&s.id, s.last_seq, &[], 0, 0, now_ms())?;
                ledger.set_skip(&s.id, Some("unconvertible"), now_ms())?;
                return Ok(Prepared::Skipped("unconvertible"));
            }
            ledger.commit(&s.id, s.last_seq, &[], 0, 0, now_ms())?;
            return Ok(Prepared::Nothing { lagging: false });
        }
        return Ok(Prepared::Nothing { lagging: true });
    }
    let converted = convert::convert(&events, through);
    if converted.turns.is_empty() {
        ledger.commit(&s.id, through, &[], 0, 0, now_ms())?;
        return Ok(if ledger.session(&s.id)?.turns == 0 { Prepared::Skipped("empty") } else { Prepared::Nothing { lagging: false } });
    }
    let mut units = vec![];
    let mut fseq = row.next_fseq;
    let mut truncated = converted.truncated as i64;
    let count = converted.turns.chunks(FILE_TURNS).len();
    for (i, turns) in converted.turns.chunks(FILE_TURNS).enumerate() {
        let mut unit = Unit { session_id: s.id.clone(), cursor: if i + 1 == count { through } else { turns.last().map_or(through, |t| t.lp_end) }, rows: vec![], lines: vec![], blocks: 0, truncated: std::mem::take(&mut truncated) };
        for t in turns {
            unit.lines.push(convert::jsonl_line(&s.id, s.harness.as_str(), fseq, t, s.started_at));
            unit.rows.push(TurnRow { fseq, lp_seq: t.lp_seq, lp_end: t.lp_end, ts: if t.ts > 0 { t.ts } else { s.started_at } });
            unit.blocks += t.blocks.len() as i64;
            fseq += 1;
        }
        units.push(unit);
    }
    Ok(Prepared::Units(units))
}

enum Flush {
    Done,
    /// Another Funes writer holds the memory: try again later.
    Busy,
    Failed(String),
}

/// Gives the turns of `units` to Funes as one turns file, then records them.
async fn flush(app: &App, dir: &std::path::Path, ledger: &Ledger, units: &[&Unit]) -> Result<Flush> {
    let bin = funes::ready(app, false)?;
    let spool = dir.join("spool");
    tokio::fs::create_dir_all(&spool).await?;
    let file = spool.join("batch.funes.jsonl");
    let mut text = String::new();
    for u in units {
        for l in &u.lines {
            text.push_str(l);
            text.push('\n');
        }
    }
    tokio::fs::write(&file, text).await.with_context(|| format!("writing {}", file.display()))?;
    let out = funes::run(app, &bin, &dir.join("funes"), &["index".into(), file.to_string_lossy().into_owned(), "--yes".into()], INDEX_LIMIT, true).await?;
    let _ = tokio::fs::remove_file(&file).await;
    if out.code == Some(funes::BUSY) {
        return Ok(Flush::Busy);
    }
    if !out.ok() {
        return Ok(Flush::Failed(format!("funes index: {}", out.tail())));
    }
    let now = now_ms();
    for u in units {
        ledger.commit(&u.session_id, u.cursor, &u.rows, u.blocks, u.truncated, now)?;
    }
    Ok(Flush::Done)
}

/// What a pass over some sessions did.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Counts {
    /// Sessions looked at.
    pub sessions_seen: i64,
    /// Sessions that got new turns in the memory, and those turns.
    pub sessions_indexed: i64,
    pub turns_indexed: i64,
    /// Sessions already indexed through their last event (skipped: nothing to do).
    pub sessions_up_to_date: i64,
    /// Sessions whose last turn is not finished yet (running, or stopped a moment ago): indexed at their next turn
    /// end, or by the next pass.
    pub sessions_pending: i64,
    /// Sessions left out, by reason: `archived`, `hidden`, `empty` (no text, tool call or result to index),
    /// `unconvertible` (stored events that no longer read).
    pub sessions_skipped: BTreeMap<String, i64>,
    /// Blocks cut to the size limit (too large): their start and end are indexed.
    pub blocks_truncated: i64,
    /// Sessions Funes refused or failed on; `launchpad recall status` lists them with the error.
    pub sessions_failed: i64,
}

/// Indexes what is new in `sessions` (all of one workspace). Returns false when it stopped early because the memory
/// was busy. `progress` is called after each file.
async fn index_sessions(app: &App, ws: &str, sessions: &[Session], counts: &mut Counts, mut progress: impl FnMut(&Counts)) -> Result<bool> {
    let svc = service(app);
    let mut i = 0;
    while i < sessions.len() {
        if !enabled(app) || launchpad_core::maintenance::active() {
            bail!("recall is off or Home is in maintenance");
        }
        let _writer = svc.writer.lock().await;
        let (dir, ledger) = open_for_writing(app, ws)?;
        // One file's worth of sessions, prepared and given to Funes under the same lock.
        let mut units: Vec<Unit> = vec![];
        while i < sessions.len() && units.iter().map(|u| u.rows.len()).sum::<usize>() < FILE_TURNS {
            let s = &sessions[i];
            i += 1;
            counts.sessions_seen += 1;
            match prepare(app, &ledger, s).await {
                Ok(Prepared::Units(u)) => units.extend(u),
                Ok(Prepared::Nothing { lagging: true }) => counts.sessions_pending += 1,
                Ok(Prepared::Nothing { lagging: false }) => counts.sessions_up_to_date += 1,
                Ok(Prepared::Skipped(reason)) => *counts.sessions_skipped.entry(reason.to_string()).or_default() += 1,
                Err(e) => {
                    counts.sessions_failed += 1;
                    let _ = ledger.set_error(&s.id, &format!("{e:#}"), now_ms());
                }
            }
        }
        // Files of at most FILE_TURNS turns, in order: a session's turns go in in the order of its counter.
        let mut failed: HashSet<String> = HashSet::new();
        let mut indexed: HashSet<String> = HashSet::new();
        let mut at = 0;
        while at < units.len() {
            let mut end = at + 1;
            while end < units.len() && units[at..end].iter().map(|u| u.rows.len()).sum::<usize>() + units[end].rows.len() <= FILE_TURNS {
                end += 1;
            }
            let group: Vec<&Unit> = units[at..end].iter().filter(|u| !failed.contains(&u.session_id)).collect();
            at = end;
            if group.is_empty() {
                continue;
            }
            let mut tries: Vec<Vec<&Unit>> = vec![group];
            while let Some(group) = tries.pop() {
                match flush(app, &dir, &ledger, &group).await? {
                    Flush::Done => {
                        for u in &group {
                            counts.turns_indexed += u.rows.len() as i64;
                            counts.blocks_truncated += u.truncated;
                            indexed.insert(u.session_id.clone());
                        }
                    }
                    Flush::Busy => return Ok(false),
                    // Funes refuses a file whole: each session alone then, so one bad session does not hold the others.
                    Flush::Failed(_) if group.len() > 1 => {
                        // Each unit alone, popped in order.
                        tries.extend(group.into_iter().rev().map(|u| vec![u]));
                    }
                    Flush::Failed(why) => {
                        let id = &group[0].session_id;
                        if failed.insert(id.clone()) {
                            counts.sessions_failed += 1;
                            tracing::warn!("recall: session {id} of workspace {ws} is not indexed: {why}");
                            let _ = ledger.set_error(id, &why, now_ms());
                        }
                        // Its later turns wait: the counter must stay dense.
                        tries.retain(|g| g[0].session_id != *id);
                    }
                }
            }
            progress(counts);
        }
        counts.sessions_indexed += indexed.difference(&failed).count() as i64;
        progress(counts);
    }
    Ok(true)
}

// ---------------------------------------------------------------- after a turn

/// A turn of this session ended, or its work reached Home: the worker indexes it.
pub fn mark(app: &App, session: &Session) {
    if !enabled(app) {
        return;
    }
    let svc = service(app);
    svc.dirty.lock().unwrap().entry(session.id.clone()).or_insert(0);
    svc.wake.notify_one();
}

async fn index_marked(app: &App) {
    let svc = service(app);
    let marked: Vec<(String, u32)> = svc.dirty.lock().unwrap().drain().collect();
    let mut by_ws: BTreeMap<String, Vec<(Session, u32)>> = BTreeMap::new();
    for (id, attempts) in marked {
        let Some(s) = app.sessions.get(&id) else { continue };
        if let Some(ws) = launchpad_core::kb::workspace_of_session(app, &id).await {
            by_ws.entry(ws).or_default().push((s, attempts));
        }
    }
    for (ws, list) in by_ws {
        let sessions: Vec<Session> = list.iter().map(|(s, _)| s.clone()).collect();
        let mut counts = Counts::default();
        let done = match index_sessions(app, &ws, &sessions, &mut counts, |_| {}).await {
            Ok(done) => done,
            Err(e) => {
                tracing::info!("recall: indexing after a turn in workspace {ws}: {e:#}");
                continue;
            }
        };
        // Its events were not all on Home yet, or the memory was busy: look again at the next pass, a few times.
        if !done || counts.sessions_pending > 0 {
            let mut dirty = svc.dirty.lock().unwrap();
            for (s, attempts) in list {
                let lags = workspace_dir(app, &ws).ok().and_then(|d| Ledger::open(&d).ok()).and_then(|l| l.session(&s.id).ok()).is_none_or(|r| r.skip.is_none() && r.cursor < s.last_seq);
                if lags && attempts + 1 < MAX_ATTEMPTS {
                    dirty.entry(s.id).or_insert(attempts + 1);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- backfill

/// The report of a workspace's last backfill, kept in its ledger.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Backfill {
    /// `running`, `done`, `interrupted` (Home stopped, recall was switched off or the memory was busy: it continues
    /// on its own) or `failed`.
    pub state: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub counts: Counts,
    pub error: Option<String>,
}

/// What a backfill of the workspace would do now, from Home's records and the ledger alone (no event is read).
pub async fn plan(app: &App, ws: &str) -> Result<Counts> {
    let config = app.config.get().home.recall;
    let dir = workspace_dir(app, ws)?;
    let rows = if dir.join("ledger.db").exists() { Ledger::open(&dir)?.sessions()? } else { Default::default() };
    let mut counts = Counts::default();
    for s in workspace_sessions(app, ws).await? {
        counts.sessions_seen += 1;
        let row = rows.get(&s.id);
        if let Some(reason) = left_out(&config, &s) {
            *counts.sessions_skipped.entry(reason.to_string()).or_default() += 1;
        } else if row.is_some_and(|r| r.cursor >= s.last_seq) || s.last_seq == 0 {
            match row.and_then(|r| r.skip.clone()).filter(|r| r == "empty" || r == "unconvertible") {
                Some(reason) => *counts.sessions_skipped.entry(reason).or_default() += 1,
                None => counts.sessions_up_to_date += 1,
            }
        } else if s.status.is_running() {
            counts.sessions_pending += 1;
        } else if row.is_some_and(|r| r.failures >= GIVE_UP) {
            // Still tried by every backfill, but no reason for the worker to start one.
            counts.sessions_failed += 1;
        } else {
            // To index: a dry run counts them as indexed.
            counts.sessions_indexed += 1;
        }
    }
    Ok(counts)
}

fn save_report(ledger: &Ledger, report: &Backfill) {
    if let Ok(json) = serde_json::to_string(report) {
        let _ = ledger.set_meta("backfill", &json);
    }
}

pub fn last_backfill(ledger: &Ledger) -> Option<Backfill> {
    ledger.meta("backfill").ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
}

/// Starts the backfill of a workspace in the background, unless it runs already. `catch_up`: the worker's own
/// pass, which does nothing (and leaves the last report as it is) when every session is indexed.
pub fn start_backfill(app: &App, ws: &str, catch_up: bool) -> bool {
    let svc = service(app);
    if !svc.running.lock().unwrap().insert(ws.to_string()) {
        return false;
    }
    let (app, ws) = (app.clone(), ws.to_string());
    tokio::spawn(async move {
        if let Err(e) = backfill(&app, &ws, catch_up).await {
            tracing::warn!("recall: backfill of workspace {ws}: {e:#}");
        }
        service(&app).running.lock().unwrap().remove(&ws);
    });
    true
}

pub fn backfill_running(app: &App, ws: &str) -> bool {
    service(app).running.lock().unwrap().contains(ws)
}

async fn backfill(app: &App, ws: &str, catch_up: bool) -> Result<()> {
    funes::ready(app, false)?;
    if catch_up {
        let todo = plan(app, ws).await?;
        let dir = workspace_dir(app, ws)?;
        let interrupted = dir.join("ledger.db").exists() && last_backfill(&Ledger::open(&dir)?).is_some_and(|b| b.state != "done");
        if todo.sessions_indexed == 0 && !interrupted {
            return Ok(());
        }
    }
    let sessions = workspace_sessions(app, ws).await?;
    let mut report = Backfill { state: "running".into(), started_at: now_ms(), updated_at: now_ms(), ..Default::default() };
    let svc = service(app);
    let ledger = {
        let _writer = svc.writer.lock().await;
        open_for_writing(app, ws)?.1
    };
    save_report(&ledger, &report);
    let mut counts = Counts::default();
    let result = index_sessions(app, ws, &sessions, &mut counts, |c| {
        report.counts = c.clone();
        report.updated_at = now_ms();
        save_report(&ledger, &report);
    })
    .await;
    report.counts = counts;
    report.updated_at = now_ms();
    match result {
        Ok(true) => {
            report.state = "done".into();
            report.finished_at = Some(now_ms());
        }
        Ok(false) => {
            report.state = "interrupted".into();
            report.error = Some("the memory was busy; Home continues on its own".into());
        }
        Err(e) => {
            report.state = if enabled(app) && !launchpad_core::maintenance::active() { "failed".into() } else { "interrupted".into() };
            report.error = Some(format!("{e:#}"));
        }
    }
    // The memory may have started again meanwhile (a rebuild): the report goes to the ledger that is there now.
    save_report(&Ledger::open(&workspace_dir(app, ws)?)?, &report);
    Ok(())
}

/// Deletes a workspace's memory and ledger. The caller starts the backfill.
pub async fn reset(app: &App, ws: &str) -> Result<()> {
    let dir = workspace_dir(app, ws)?;
    let svc = service(app);
    let _writer = svc.writer.lock().await;
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.with_context(|| format!("removing {}", dir.display()))?;
    }
    Ok(())
}

/// Keeps `ws` in `home.recall.backfill`: its whole history stays indexed.
pub fn keep_indexed(app: &App, ws: &str) -> Result<()> {
    app.config.update(|c| {
        if !c.home.recall.backfill.iter().any(|w| w == ws) {
            c.home.recall.backfill.push(ws.to_string());
        }
        Ok(())
    })
}

/// Home: the worker. Indexes marked sessions shortly after their turn, and every quarter of an hour catches up the
/// workspaces of `home.recall.backfill` (an interrupted backfill, a memory lost in a restart, a missed turn).
pub fn spawn(app: &App) {
    if app.paths.role != launchpad_core::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let svc = service(&app);
        let mut catch_up = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            let _ = tokio::time::timeout(Duration::from_secs(60), svc.wake.notified()).await;
            // A turn's last events and its end arrive together: leave them a moment.
            tokio::time::sleep(Duration::from_secs(5)).await;
            // Off, or the program or its weights are missing: nothing is indexed, quietly (`recall status` says why).
            if !enabled(&app) || launchpad_core::maintenance::active() || funes::ready(&app, false).is_err() {
                svc.dirty.lock().unwrap().clear();
                continue;
            }
            index_marked(&app).await;
            // Two minutes after Home's start, then every 15 minutes.
            if tokio::time::Instant::now() >= catch_up {
                catch_up = tokio::time::Instant::now() + Duration::from_secs(15 * 60);
                let existing: HashSet<String> = launchpad_core::workspace::list(&app).await.map(|l| l.into_iter().map(|w| w.id).collect()).unwrap_or_default();
                for ws in app.config.get().home.recall.backfill {
                    if existing.contains(&ws) {
                        start_backfill(&app, &ws, true);
                    }
                }
            }
        }
    });
}

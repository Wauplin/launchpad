//! Session manager: the registry of sessions, their event log, and the
//! routing of input to the harness adapters.
//!
//! Every session is kept in memory (a few hundred records) and written through to `state.db`.
//! Adapters report through a [`Sink`], which numbers events, stores them, pushes them over the
//! WebSocket and keeps the session record (status, usage, cost) up to date.

/// Wait between capacity failures and automatic continuation attempts.
pub const CAPACITY_RETRY_DELAY_MS: i64 = 5 * 60_000;
pub const MAX_CAPACITY_RETRIES: u8 = 5;

fn is_capacity_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("model is at capacity") || message.contains("overloaded")
}

mod records;
mod observation;
mod lifecycle;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::App;
use crate::compose::{ContextInfo, render_prompt, title_from};
use crate::events::Event;
use crate::model::*;
use crate::util::now_ms;

/// A running harness process (or thread) attached to a session.
#[async_trait]
pub trait LiveSession: Send + Sync {
    /// A new turn, or a message read during the running turn.
    async fn send(&self, text: &str, client_id: &str, images: &[ImageAttachment]) -> Result<()>;
    async fn interrupt(&self) -> Result<()>;
    /// Safe to begin another turn, including commands that outlive interruption.
    fn ready_for_turn(&self) -> bool;
    fn turn_active(&self) -> bool;
    /// Work that interruption cannot end, even during an active turn.
    fn has_background_work(&self) -> bool { false }
    async fn processes(&self) -> Result<Vec<SessionProcess>> { Ok(vec![]) }
    async fn terminate_process(&self, _thread: &str, _process: &str) -> Result<()> {
        bail!("This harness does not support stopping individual background commands")
    }
    fn has_open_questions(&self) -> bool { false }
    /// This explicit choice cancels the current turn (a silent owner Stop).
    fn answer_stops_turn(&self, _request_id: &str, _answers: &[String]) -> bool { false }
    /// Answers an `ask_user` request: one answer per question.
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()>;
    /// Stops the process; the conversation stays resumable.
    async fn stop(&self) -> Result<()>;
    /// Uses this model and effort from the next turn. Returns false when the process must restart to apply them.
    fn set_model(&self, _model: Option<&str>, _effort: Option<&str>) -> bool { false }
    /// After this long between turns with nothing running or asked, the core releases the process
    /// (`Sessions::release`, like before a move); the next message resumes the conversation. `None`: kept.
    fn idle_release(&self) -> Option<std::time::Duration> { None }
}

/// A session idle long enough for its process to be released (`LiveSession::idle_release`).
fn idle_release_due(s: &Session, now: i64, after: std::time::Duration) -> bool {
    matches!(s.status, SessionStatus::Waiting | SessionStatus::Stopped | SessionStatus::Interrupted | SessionStatus::Error)
        && !s.open_question && s.quota_pause.is_none() && s.capacity_retry_at.is_none()
        && now - s.status_since >= after.as_millis() as i64
}

/// Input delivery errors do not prove that the live turn itself failed.
#[derive(Debug)]
pub enum InputDeliveryError {
    Uncertain(anyhow::Error),
    Rejected(&'static str),
}
impl std::fmt::Display for InputDeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uncertain(error) => write!(f, "Message delivery is uncertain; inspect the trace before sending again: {error:#}"),
            Self::Rejected(reason) => write!(f, "Message was not sent: {reason}"),
        }
    }
}
impl std::error::Error for InputDeliveryError {}

/// Everything an adapter needs to start or resume a session.
pub struct StartSpec {
    pub permission_mode: PermissionMode,
    pub session_id: String,
    pub harness_session_id: Option<String>,
    pub resume: bool,
    pub cwd: PathBuf,
    pub title: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub add_dirs: Vec<String>,
    /// Profile environment overrides; adapters add them to every native process of the session.
    pub env: Vec<(String, String)>,
    /// The session's jail (`contain::apply`); adapters start every native process of the session in it.
    pub jail: Option<std::sync::Arc<crate::jail::Jail>>,
}

impl StartSpec {
    /// The environment of the session's native processes: profile overrides and the jail.
    pub fn process_env(&self, base: &crate::shellenv::UserEnv) -> crate::shellenv::UserEnv {
        base.with(&self.env).jailed(self.jail.clone())
    }
}

fn unanswered_requests(events: &[EventBody]) -> Vec<String> {
    let mut pending = std::collections::BTreeSet::new();
    for event in events {
        match event {
            EventBody::AskUser { request_id, .. } => { pending.insert(request_id.clone()); }
            EventBody::AskAnswered { request_id, .. } => { pending.remove(request_id); }
            _ => {}
        }
    }
    pending.into_iter().collect()
}

/// Raw harness lines kept per session (`Sink::raw`): at most this many lines and bytes, each line cut to
/// `RAW_LINE_BYTES`, deleted at Archive and after `RAW_KEEP_MS` (the server's `retention.rs`).
const RAW_LINES_CAP: i64 = 20_000;
const RAW_BYTES_CAP: i64 = 16 * 1024 * 1024;
const RAW_LINE_BYTES: usize = 64 * 1024;
pub const RAW_KEEP_MS: i64 = 14 * 24 * 3600 * 1000;

/// A raw line cut to at most `max` bytes, on a character boundary.
fn raw_cut(line: &str, max: usize) -> &str {
    if line.len() <= max {
        return line;
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// Deletes a session's raw lines (Archive, forget).
fn drop_raw(app: &App, sessions: &Sessions, id: &str) {
    sessions.inner.raw_counts.lock().unwrap().remove(id);
    let id = id.to_string();
    // Queued, not awaited: nothing reads these lines.
    drop(app.state_db.enqueue(move |c| Ok(c.execute("DELETE FROM session_raw WHERE session_id = ?1", [id])?)));
}

#[derive(Debug)]
pub struct FollowUpBusy;
impl std::fmt::Display for FollowUpBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("The session is busy (running, in a terminal, awaiting an answer or paused for quota) or its quota is not confirmed.") }
}
impl std::error::Error for FollowUpBusy {}

/// `FollowUpHeld`: an automatic follow-up that a quota reading the runner lacks holds (`hold_input`) instead of
/// refusing (Burst's first request, `burst::first_request`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode { Manual, FollowUpManual, FollowUpAutomatic, FollowUpHeld }

/// What an automatic follow-up does while its quota is not confirmed: `FollowUpBusy` (the scheduler keeps it
/// queued), or held until a reading confirms quota (true). None: it goes on.
fn unconfirmed_quota(mode: InputMode, protected: bool, available: bool) -> Option<bool> {
    match mode {
        InputMode::FollowUpAutomatic | InputMode::FollowUpHeld if protected && !available => Some(mode == InputMode::FollowUpHeld),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuotaResume { Automatic, ResetFallback, Manual, Paid }

#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Inner>,
}

type LiveSessions = HashMap<String, (u64, Arc<dyn LiveSession>)>;

struct Inner {
    records: Mutex<HashMap<String, Session>>,
    stop_requests: Mutex<HashSet<String>>,
    quota_stops: Mutex<HashSet<String>>,
    title_requests: crate::locks::Locks,
    quota_interrupts: Mutex<HashMap<String, i64>>,
    /// Subscription-only retries (manual or reset fallback) ignore the old snapshot until new evidence.
    manual_quota_retries: Mutex<HashMap<String, i64>>,
    /// Running turns interrupted at their cost limit, with the time of the last attempt.
    budget_interrupts: Mutex<HashMap<String, i64>>,
    /// Reported cost increases of profile sessions, in time order (see `spend.rs`).
    spend: Mutex<crate::spend::Ledger>,
    /// Sessions whose process must restart to apply model, effort or permission changes.
    process_restarts: Mutex<HashSet<String>>,
    /// New sessions running their worktree setup command before the agent starts.
    setups: Mutex<HashMap<String, SetupTask>>,
    /// Detached shutdowns still own their folders until exit is confirmed.
    shutdowns: Mutex<HashMap<String, usize>>,
    /// One action lock per session (`action_lock`): its user actions, quota decisions and worktree operations
    /// serialize, other sessions never wait for it. Adapter events never take it.
    actions: crate::locks::Locks,
    /// One status change per session at a time (`Sink::set_status`), from its read to its side effects. Taken
    /// by adapters and actions alike, never held across a call to Home or a harness write.
    statuses: crate::locks::Locks,
    /// Quota paused sessions whose harness started a turn of its own (a background command's end): the status
    /// stays QuotaPaused, and that turn's end is still saved on Home (`Sink::set_status_when`).
    paused_turns: Mutex<HashSet<String>>,
    /// Attached processes, with an instance number: the exit of an old process never detaches a newer one.
    live: Mutex<LiveSessions>,
    next_instance: std::sync::atomic::AtomicU64,
    /// Per session: the next raw line number and the bytes stored so far.
    raw_counts: Mutex<HashMap<String, (i64, i64)>>,
    /// Sessions shown in a focused window, per window connection: no notification for them.
    watched: Mutex<HashMap<u64, Presence>>,
}

/// Acknowledge setup cancellation; unconfirmed process exits stay tracked in shutdowns.
struct SetupTask {
    cancel: tokio::sync::watch::Sender<bool>,
    stopped: tokio::sync::oneshot::Receiver<()>,
}

impl SetupTask {
    async fn stop(self) {
        let _ = self.cancel.send(true);
        let _ = self.stopped.await;
    }
}

/// A window's focus, the sessions it shows, when it said so, and the time of its last user input.
type Presence = (bool, Vec<String>, i64, Option<i64>);

/// A window only watches a session while someone uses it: an unattended focused window does not.
const WATCH_INPUT_MS: i64 = 60_000;

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

/// What a status change does to a session (`Sink::set_status`), decided from its record under its status lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    /// Nothing changes: the same status, a late update during a quota pause, or a cancelled quota continuation.
    Ignore,
    /// The session takes `status`; `turn_end`: it leaves a running status, so the turn is saved on Home and
    /// notified, and routines that wait for it run.
    To { status: SessionStatus, turn_end: bool },
}

/// The status change `requested` for a session now `current`, with its quota pause phase. A quota pause
/// survives late adapter updates, and a turn the quota guard interrupts ends only when the harness says so: while
/// it pauses (`Pausing`) and the turn runs, the guard's own QuotaPaused waits, and the harness's turn end
/// (Done, Stopped, Interrupted, Failed) becomes QuotaPaused. `quota_stop`: the owner cancelled the continuation.
pub(crate) fn transition(current: SessionStatus, pause: Option<QuotaPausePhase>, quota_stop: bool, requested: SessionStatus) -> Transition {
    use SessionStatus::*;
    let status = match pause {
        Some(QuotaPausePhase::Resuming) | None => requested,
        // The turn still runs: only its real end pauses it.
        Some(QuotaPausePhase::Pausing) if current.is_running() && requested == QuotaPaused => return Transition::Ignore,
        Some(_) if requested == QuotaPaused || (current.is_running() && !requested.is_running()) => QuotaPaused,
        Some(_) => return Transition::Ignore,
    };
    if (quota_stop && status != Closed && status != Waiting) || status == current {
        return Transition::Ignore;
    }
    Transition::To { status, turn_end: current.is_running() && !status.is_running() }
}

/// How far a session's event numbers jump when another runner (or a new generation) takes it over (`handoff::adopt`):
/// late events of the previous generation keep their own numbers below.
pub const MOVE_SEQ_GAP: i64 = 1_000_000;

impl Sessions {
    pub fn new() -> Sessions {
        Sessions {
            inner: Arc::new(Inner {
                records: Mutex::new(HashMap::new()),
                stop_requests: Mutex::new(HashSet::new()),
                quota_stops: Mutex::new(HashSet::new()),
                title_requests: crate::locks::Locks::default(),
                quota_interrupts: Mutex::new(HashMap::new()),
                manual_quota_retries: Mutex::new(HashMap::new()),
                budget_interrupts: Mutex::new(HashMap::new()),
                spend: Mutex::new(crate::spend::Ledger::default()),
                process_restarts: Mutex::new(HashSet::new()),
                setups: Mutex::new(HashMap::new()),
                shutdowns: Mutex::new(HashMap::new()),
                actions: crate::locks::Locks::default(),
                statuses: crate::locks::Locks::default(),
                paused_turns: Mutex::new(HashSet::new()),
                live: Mutex::new(HashMap::new()),
                next_instance: std::sync::atomic::AtomicU64::new(1),
                raw_counts: Mutex::new(HashMap::new()),
                watched: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Loads sessions from `state.db`. Sessions that were mid-turn when the daemon stopped become
    /// `interrupted`; done ones stay done without a process (sending a message resumes them).
    pub(crate) fn spend_entries<R>(&self, f: impl FnOnce(&mut crate::spend::Ledger) -> R) -> R {
        f(&mut self.inner.spend.lock().unwrap())
    }

    pub async fn load(&self, app: &App) -> Result<()> {
        let spend = crate::spend::load(app).await?;
        let now = now_ms();
        self.spend_entries(|ledger| {
            *ledger = crate::spend::Ledger::default();
            for e in spend {
                ledger.insert(e, now);
            }
        });
        let rows: Vec<Session> = app
            .state_db
            .call(|c| {
                let mut st = c.prepare(
                    "SELECT id, harness, harness_session_id, project_id, cwd, title, status, status_since, open_question, model,
                            effort, prompt, usage_json, cost_usd, started_at, ended_at, last_seq, launch_spec_json, quota_pause_json, title_pending,
                            seen_at, last_message, archived_done, capacity_retry_count, capacity_retry_at, runner_id, generation, unsaved_json, title_refreshed, sleep_json
                     FROM session",
                )?;
                let rows = st
                    .query_map([], |r| {
                        let usage: Option<String> = r.get(12)?;
                        let mut usage: Usage = usage.and_then(|u| serde_json::from_str(&u).ok()).unwrap_or_default();
                        usage.cost_usd = r.get(13)?;
                        let spec: String = r.get(17)?;
                        let spec: LaunchSpec = serde_json::from_str(&spec).unwrap_or_default();
                        let title: String = r.get(5)?;
                        // Older records have no naming state: retry only launch-derived titles.
                        let title_pending = r.get::<_, Option<bool>>(19)?.unwrap_or_else(||
                            spec.harness.is_some() && !spec.text.trim().is_empty() && title == title_from(&spec.text, ""));
                        Ok(Session {
                            runner_id: r.get(25)?,
                            generation: r.get(26)?,
                            id: r.get(0)?,
                            harness: HarnessKind::parse(&r.get::<_, String>(1)?).ok_or_else(|| rusqlite::Error::InvalidParameterName("unregistered stored harness".into()))?,
                            harness_session_id: r.get(2)?,
                            project_id: r.get(3)?,
                            cwd: r.get(4)?,
                            title,
                            title_pending,
                            title_refreshed: r.get(28)?,
                            status: SessionStatus::parse(&r.get::<_, String>(6)?),
                            archived_done: r.get(22)?,
                            capacity_retry_count: r.get(23)?,
                            capacity_retry_at: r.get(24)?,
                            status_since: r.get(7)?,
                            live: false,
                            open_question: r.get::<_, i64>(8)? != 0,
                            model: r.get(9)?,
                            effort: r.get(10)?,
                            prompt: r.get(11)?,
                            usage,
                            started_at: r.get(14)?,
                            ended_at: r.get(15)?,
                            last_seq: r.get(16)?,
                            launch_spec: spec,
                            quota_pause: r.get::<_, Option<String>>(18)?.and_then(|v| serde_json::from_str(&v).ok()),
                            seen_at: r.get(20)?,
                            last_message: r.get::<_, Option<String>>(21)?.and_then(|m| message_preview(&m)),
                            unsaved: r.get::<_, Option<String>>(27)?.and_then(|v| serde_json::from_str(&v).ok()),
                            sleep: r.get::<_, Option<String>>(29)?.and_then(|v| serde_json::from_str(&v).ok()),
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        // (record, status before) of the sessions whose status changed here.
        let mut interrupted: Vec<(Session, SessionStatus)> = vec![];
        {
            let mut recs = self.inner.records.lock().unwrap();
            for mut s in rows {
                // Replicas: another instance or a runner owns their processes.
                if s.runner_id.is_some() { recs.insert(s.id.clone(), s); continue; }
                let mut changed = false;
                let before = s.status;
                if !crate::profiles::supports_cost(app, s.harness, &s.launch_spec, s.model.as_deref()) && crate::harness::registration(s.harness).descriptor.capabilities.cost {
                    changed = !s.usage.cost_unavailable;
                    s.usage.cost_unavailable = true;
                }
                if s.usage.cost_unavailable {
                    changed |= s.usage.cost_usd.take().is_some();
                }
                if let Some(pause) = &mut s.quota_pause {
                    pause.phase = if pause.phase == QuotaPausePhase::Resuming {
                        pause.error = Some("Daemon stopped during continuation; inspect the trace before resuming.".into());
                        QuotaPausePhase::NeedsAttention
                    } else if pause.phase == QuotaPausePhase::Pausing { QuotaPausePhase::Paused } else { pause.phase };
                    s.change_status(SessionStatus::QuotaPaused);
                    changed = true;
                } else if s.status.is_running() {
                    s.change_status(SessionStatus::Interrupted);
                    s.status_since = now_ms();
                    changed = true;
                }
                if changed { interrupted.push((s.clone(), before)); }
                recs.insert(s.id.clone(), s);
            }
        }
        for (mut s, before) in interrupted {
            // A turn the stop cut short ends like any turn: its branch tips first, then the status with the
            // `unsaved` mark (`turn_sync` saves it once Home is reachable), and a notification once it is saved.
            if before.is_running() && !s.status.is_running() {
                crate::worktrees::record_heads(app, &s.id).await;
                s.unsaved = crate::turn_sync::mark(app).or(s.unsaved);
                if let Some(r) = self.inner.records.lock().unwrap().get_mut(&s.id) {
                    r.unsaved = s.unsaved.clone();
                }
                if let Some(event) = crate::notify::status_notification(before, s.status, s.open_question, false, false) {
                    crate::turn_sync::notify_when_saved(&s.id, s.status, event);
                }
            }
            Self::persist(&app.state_db, &s).await?;
        }
        // Read only decision events, not transcripts or attachment bytes, during startup.
        let mut decisions = app.state_db.call(|c| {
            let mut st = c.prepare("SELECT session_id, json FROM session_event WHERE kind IN ('ask_user', 'ask_answered') ORDER BY session_id, seq")?;
            let mut by_session: HashMap<String, Vec<EventBody>> = HashMap::new();
            for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (id, json) = row?;
                by_session.entry(id).or_default().push(serde_json::from_str(&json)?);
            }
            Ok(by_session)
        }).await?;
        for s in self.list() {
            if s.runner_id.is_some() { continue; }
            let pending = unanswered_requests(&decisions.remove(&s.id).unwrap_or_default());
            let sink = self.sink(app, &s.id);
            for request_id in pending {
                sink.emit(EventBody::AskAnswered { request_id, answers: vec!["Request ended".into()] }).await;
            }
            if s.open_question { sink.set_open_question(false).await; }
        }
        self.publish_counts(app);
        Ok(())
    }

    /// The action lock of one session: its launch, messages, Resume, Stop, Archive, answers, quota decisions and
    /// the worktree operations that detach its process or move its folders. Other sessions never wait for it.
    /// Never hold it across a call to Home, a download, an install or a push that can wait on the network,
    /// except the session's own start (see `launch_inner`) and resume (`reattach`), which only that session waits for.
    /// Code that holds one session's lock never takes another one: several are taken at once with `action_locks`.
    pub(crate) async fn action_lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.inner.actions.get(id).lock_owned().await
    }

    /// The action locks of several sessions, always taken in ID order, so two callers never wait for each other.
    pub(crate) async fn action_locks<S: AsRef<str>>(&self, ids: &[S]) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut ids: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut guards = Vec::with_capacity(ids.len());
        for id in ids {
            guards.push(self.action_lock(id).await);
        }
        guards
    }

    /// The action lock of a session when nobody holds it: the 1-second guards skip a busy session until the next check.
    fn try_action_lock(&self, id: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.inner.actions.get(id).try_lock_owned().ok()
    }

    /// Launch preparation (remote sync, worktree setup) runs before the agent of this new session starts.
    pub fn setup_running(&self, id: &str) -> bool {
        self.inner.setups.lock().unwrap().contains_key(id)
    }

    fn ensure_local(&self, id: &str) -> Result<()> {
        self.get(id).context("no such session")?;
        Ok(())
    }

    /// Home: stores an item replicated from a runner, with its outbox number, then shows it. Items of an older
    /// generation than the stored record (the session moved) only advance the acknowledged number.
    pub(crate) async fn apply_replica(&self, app: &App, runner_id: &str, n: u64, item: crate::replica::Item) -> Result<()> {
        use crate::replica::{Item, set_acked};
        let runner = runner_id.to_string();
        match item {
            Item::Session { session } => {
                let mut s = *session;
                let current = self.get(&s.id);
                // A session Home no longer knows because it restored a backup taken before the launch: the runner's
                // record carries Home's own proof of the placement (`runners::sign_placement`), which Home checks.
                let recovered = current.is_none()
                    && crate::replica::restored_before(s.started_at)
                    && crate::runners::placement_valid(app, &s, runner_id).await;
                // A record Home cannot take back after a restore (no proof: launched before a verified backup held
                // the placement key, by an older build, or moved after the checkpoint) is discarded on purpose: kept
                // in `replica_orphan` for inspection, never applied.
                let orphan = current.is_none() && !recovered && crate::replica::restored_before(s.started_at);
                if recovered {
                    tracing::warn!("runner {runner_id}: session {} was launched after the backup this Home restored: its record comes back from the runner (placement proof checked)", s.id);
                } else if orphan {
                    tracing::error!("runner {runner_id}: session {} is unknown after a restore and carries no placement proof Home can check: its record is kept in replica_orphan, not applied", s.id);
                } else if current.is_none() {
                    tracing::info!("runner {runner_id}: record of unknown session {} ignored (no placement Home can prove)", s.id);
                }
                let orphan_json = orphan.then(|| serde_json::to_string(&s.without_bytes())).transpose()?;
                // Only sessions Home placed on this runner are its records (`runners::record_placement`, a move):
                // a record of an unknown session or of another runner's is ignored.
                let stale = !recovered && current.as_ref().is_none_or(|c| c.runner_id.as_deref() != Some(runner_id) || c.generation > s.generation);
                let mut topic_name = None;
                if !stale {
                    s.runner_id = Some(runner.clone());
                    if recovered {
                        // Home's own numbers start over: its events come back from the runner's outbox, and a
                        // wake-up is Home's (`replica::Item::Sleep` brings a lost one back).
                        s.last_seq = 0;
                        s.sleep = None;
                    }
                    // Either side may have shown it to the owner.
                    if let Some(c) = &current {
                        s.seen_at = s.seen_at.max(c.seen_at);
                        // A wake-up is Home's (`launchpad sleep`): runners never know of it.
                        s.sleep = c.sleep.clone();
                        // Home's number is its last stored event (raised by `Item::Event`), so a trace through it
                        // always exists here, also right after a move (`MOVE_SEQ_GAP`) or before the events arrive.
                        s.last_seq = c.last_seq;
                        // What the lease holds stays as Home placed it (`vault::grant` reads it from this record).
                        s.harness = c.harness;
                        s.launch_spec.profile = c.launch_spec.profile.clone();
                        s.launch_spec.workspace_id = c.launch_spec.workspace_id.clone();
                        s.launch_spec.bot = c.launch_spec.bot.clone();
                        s.launch_spec.tools_off = c.launch_spec.tools_off.clone();
                        s.launch_spec.tools_on = c.launch_spec.tools_on.clone();
                        // Who started it is Home's stamp (`senders`, `task_archive::bot_origin`), never a runner's.
                        s.launch_spec.from = c.launch_spec.from.clone();
                        s.launch_spec.routine = c.launch_spec.routine.clone();
                        // Home created the launch's topic: the runner writes only its generated name, once.
                        let generated = s.launch_spec.topic_naming.as_ref().and_then(|n| n.name.clone());
                        s.launch_spec.topic_naming = c.launch_spec.topic_naming.clone().map(|mut n| {
                            if n.name.as_deref().is_none_or(str::is_empty) {
                                topic_name = generated.clone().filter(|v| !v.is_empty());
                                n.name = generated.clone();
                            }
                            n
                        });
                    }
                }
                let stored = s.clone();
                let orphan_runner = runner.clone();
                app.state_db.call(move |c| {
                    let tx = c.transaction()?;
                    if let Some(json) = orphan_json {
                        tx.execute("INSERT INTO replica_orphan (runner_id, session_id, at, json) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (runner_id, session_id) DO UPDATE SET at = excluded.at, json = excluded.json",
                            params![orphan_runner, stored.id, crate::util::now_ms(), json])?;
                    }
                    if !stale {
                        Self::store(&tx, &stored)?;
                        tx.execute("UPDATE session SET last_message = ?2, prompt = ?3 WHERE id = ?1", params![stored.id, stored.last_message, stored.prompt])?;
                    }
                    set_acked(&tx, &runner, n)?;
                    tx.commit()?;
                    Ok(())
                }).await?;
                if !stale {
                    // Home may have changed its own fields meanwhile (the BOT.md a message carried,
                    // `bots::record_instructions`): keep them, and store them again.
                    let again = {
                        let mut recs = self.inner.records.lock().unwrap();
                        let newer = recs.get(&s.id).map(|now| now.launch_spec.bot.clone()).filter(|b| *b != s.launch_spec.bot);
                        if let Some(b) = &newer {
                            s.launch_spec.bot = b.clone();
                        }
                        // The same for a wake-up set or cleared meanwhile (`launchpad sleep`).
                        let slept = recs.get(&s.id).map(|now| now.sleep.clone()).filter(|z| *z != s.sleep);
                        if let Some(z) = &slept {
                            s.sleep = z.clone();
                        }
                        recs.insert(s.id.clone(), s.clone());
                        (newer.is_some() || slept.is_some()).then(|| Self::persist(&app.state_db, &s))
                    };
                    if let Some(stored) = again {
                        stored.await?;
                    }
                    app.bus.publish("sessions", "updated", serde_json::to_value(s.light())?);
                    self.publish_counts(app);
                    if let Some(name) = topic_name
                        && let Err(e) = crate::topics::named(app, &s.id, &name).await
                    {
                        tracing::info!("generated topic name of {}: {e:#}", s.id);
                    }
                    let ended = current.as_ref().is_some_and(|c| c.status.is_running() && !s.status.is_running());
                    // A message woke a sleeping agent before its wake-up: it sleeps no more, and its wake-up goes.
                    if let Some(z) = s.sleep.clone().filter(|_| current.as_ref().is_some_and(|c| !c.status.is_running()) && s.status.is_running()) {
                        if let Err(e) = self.set_sleep(app, &s.id, None).await {
                            tracing::warn!("wake-up of {}: {e:#}", s.id);
                        }
                        crate::ext::extensions().woken(app, &s.id, &z.schedule_id);
                    }
                    if ended {
                        self.replica_turn_ended(app, &s);
                    }
                    // A merge the owner asked for waits for this turn's rebase on Home (`rebase_merge`).
                    if ended || current.as_ref().is_some_and(|c| c.unsaved.is_some()) {
                        crate::rebase_merge::after_turn(app, &s);
                        // When done = Draft PR or Open PR: the agent's Local PR goes to GitHub now.
                        crate::pr_when_done::after_turn(app, &s);
                    }
                    // The runner saved the last turn's work on Home: routines that wait for it start now.
                    if current.as_ref().is_some_and(|c| c.unsaved.is_some()) && s.unsaved.is_none() && !s.status.is_running() {
                        crate::ext::extensions().turn_saved(app, &s);
                    }
                    if s.status == SessionStatus::Closed
                        && let Err(e) = crate::away::forget_archived(app, &s.id).await
                    {
                        tracing::warn!("worktree records of archived session {}: {e:#}", s.id);
                    }
                }
            }
            Item::Event { generation, event } => {
                let current = self.get(&event.session_id);
                let fresh = current.as_ref().is_some_and(|c| c.runner_id.as_deref() == Some(runner_id) && c.generation <= generation);
                // Late events of an older generation (its runner was offline when the session moved): kept below the
                // new generation's numbers (`MOVE_SEQ_GAP`), never over an event Home has.
                let late = !fresh && current.as_ref().is_some_and(|c| c.generation > generation);
                let ev = event.clone();
                // The summary lists the machines that ran the turns; runner records go away (Burst), so keep the name.
                let machine = match &event.body {
                    EventBody::TurnEnd { .. } if fresh => Some(crate::runners::load(app).await?.into_iter().find(|(r, _)| r.id == runner_id).map_or_else(|| runner_id.to_string(), |(r, _)| r.name)),
                    _ => None,
                };
                let inserted = app.state_db.call(move |c| {
                    let tx = c.transaction()?;
                    let mut inserted = false;
                    if fresh || late {
                        inserted = tx.execute("INSERT INTO session_event (session_id, seq, ts, kind, json, machine) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (session_id, seq) DO NOTHING",
                            params![ev.session_id, ev.seq, ev.ts, ev.body.kind(), serde_json::to_string(&ev.body)?, machine])? > 0;
                    }
                    if fresh {
                        tx.execute("UPDATE session SET last_seq = max(last_seq, ?1) WHERE id = ?2", params![ev.seq, ev.session_id])?;
                    }
                    set_acked(&tx, &runner, n)?;
                    tx.commit()?;
                    Ok(inserted)
                }).await?;
                if inserted && fresh {
                    if let Some(s) = self.inner.records.lock().unwrap().get_mut(&event.session_id) { s.last_seq = s.last_seq.max(event.seq); }
                    app.bus.send(crate::events::Event {
                        seq: event.seq as u64,
                        topic: format!("session/{}", event.session_id),
                        kind: "event".into(),
                        payload: serde_json::to_value(&event).unwrap_or(Value::Null),
                    });
                }
            }
            // Handled by `replica::ingest` before it gets here.
            Item::Sleep { .. } => app.state_db.call(move |c| set_acked(c, &runner, n)).await?,
            Item::Forget { id } => {
                let owned = self.get(&id).is_some_and(|s| s.runner_id.as_deref() == Some(runner_id));
                if owned { self.inner.records.lock().unwrap().remove(&id); }
                app.state_db.call(move |c| {
                    let tx = c.transaction()?;
                    if owned {
                        tx.execute("DELETE FROM session_event WHERE session_id = ?1", [&id])?;
                        tx.execute("DELETE FROM session WHERE id = ?1", [&id])?;
                    }
                    set_acked(&tx, &runner, n)?;
                    tx.commit()?;
                    Ok(())
                }).await?;
                if owned { self.publish_counts(app); app.bus.publish("sessions", "changed", Value::Null); }
            }
            Item::Project { .. } | Item::ForgetProject { .. } => return crate::projects::apply_replica(app, runner_id, n, item).await,
            Item::Access { access } => return crate::replica::apply_access(app, runner_id, n, access).await,
            Item::Worktree { worktree } => return crate::away::apply_replica(app, runner_id, n, *worktree).await,
            Item::Spend { entry } => return crate::spend::apply_replica(app, runner_id, n, entry).await,
        }
        Ok(())
    }

    /// Home: a runner's session left Running, so routines settle and trigger. The runner sends its own
    /// notification (forwarded to Home, for the Telegram chat of the session's workspace).
    fn replica_turn_ended(&self, app: &App, s: &Session) {
        crate::ext::extensions().turn_ended(app, s);
    }

    /// Applies a change to a session record, stores it, and pushes it to the UI.
    pub(crate) async fn update(&self, app: &App, id: &str, f: impl FnOnce(&mut Session)) -> Result<Session> {
        let (s, stored) = {
            let mut recs = self.inner.records.lock().unwrap();
            let s = recs.get_mut(id).context("no such session")?;
            f(s);
            (s.clone(), Self::persist(&app.state_db, s))
        };
        stored.await?;
        // Another update may have completed while this task waited for the database.
        if let Some(current) = self.inner.records.lock().unwrap().get(id) {
            app.bus.publish("sessions", "updated", serde_json::to_value(current.light())?);
        }
        Ok(s)
    }

    /// Runner: records a session Home moved here (`handoff::adopt`). Its history stays on Home; new events
    /// continue its numbering.
    pub(crate) async fn adopt(&self, app: &App, s: Session) -> Result<()> {
        let stored = s.clone();
        app.state_db.call(move |c| Self::store(c, &stored)).await?;
        self.inner.records.lock().unwrap().insert(s.id.clone(), s.clone());
        app.bus.publish("sessions", "updated", serde_json::to_value(s.light())?);
        self.publish_counts(app);
        Ok(())
    }

    /// Home: the record of a launch that waits for its Burst runner (`burst::launch`); the runner's own record
    /// replaces it.
    pub(crate) async fn placeholder(&self, app: &App, s: Session) -> Result<()> {
        let stored = s.clone();
        app.state_db.call(move |c| Self::store(c, &stored)).await?;
        self.inner.records.lock().unwrap().insert(s.id.clone(), s.clone());
        app.bus.publish("sessions", "updated", serde_json::to_value(s.light())?);
        self.publish_counts(app);
        Ok(())
    }

    /// Home: a status Home sets itself for a session no runner holds (a stopped Burst job, a failed start).
    pub(crate) async fn set_replica_status(&self, app: &App, id: &str, status: SessionStatus) -> Result<Session> {
        let s = self.update(app, id, |s| {
            s.change_status(status);
            s.status_since = now_ms();
            if status == SessionStatus::Closed {
                s.ended_at = Some(now_ms());
            }
        })
        .await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Home: a replicated session shows Failed with `reason` as its last message, when Home could not continue
    /// it and no runner holds it. Only the record changes, like `end_replica`.
    pub(crate) async fn fail_replica(&self, app: &App, id: &str, reason: String) -> Result<Session> {
        let _action = self.action_lock(id).await;
        let s = self.update(app, id, |s| {
            s.change_status(SessionStatus::Error);
            s.status_since = now_ms();
            s.live = false;
            s.last_message = Some(reason);
        })
        .await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Home: ends a replicated session on Home alone, when its runner cannot do it (offline or removed). Only
    /// the record changes: Home adds no event, so the runner's sequence stays its own; `note` becomes its last
    /// message.
    pub(crate) async fn end_replica(&self, app: &App, id: &str, status: SessionStatus, note: Option<String>) -> Result<Session> {
        let _action = self.action_lock(id).await;
        let s = self.update(app, id, |s| {
            if let Some(n) = note {
                s.last_message = Some(n);
            }
            s.change_status(status);
            s.status_since = now_ms();
            s.live = false;
            s.open_question = false;
            s.quota_pause = None;
            s.capacity_retry_at = None;
            if status == SessionStatus::Closed {
                s.ended_at = Some(now_ms());
            }
        })
        .await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Runner: forgets a session Home moved on while this runner was away (`handoff::release`). Its record and
    /// events leave this disk only, without a replica item: Home holds the newer generation.
    pub(crate) async fn forget_local(&self, app: &App, id: &str) -> Result<()> {
        self.inner.records.lock().unwrap().remove(id);
        drop_raw(app, self, id);
        let key = id.to_string();
        app.state_db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute("DELETE FROM session_event WHERE session_id = ?1", [&key])?;
                tx.execute("DELETE FROM session WHERE id = ?1", [&key])?;
                tx.execute("DELETE FROM topic_session WHERE session_id = ?1", [&key])?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.publish_counts(app);
        app.bus.publish("sessions", "changed", Value::Null);
        app.bus.publish("topics", "changed", Value::Null);
        Ok(())
    }

    /// Home: the session moves to another runner, in a new generation. Records and events of the older one are
    /// ignored from now on.
    pub(crate) async fn reassign(&self, app: &App, id: &str, runner_id: &str) -> Result<Session> {
        let _action = self.action_lock(id).await;
        self.update(app, id, |s| {
            s.generation += 1;
            s.runner_id = Some(runner_id.to_string());
        })
        .await
    }

    /// A focused window showed the session: it no longer needs you until its status changes again.
    pub async fn mark_seen(&self, app: &App, id: &str) -> Result<Session> {
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        if s.seen_at >= s.status_since { return Ok(s); }
        let s = self.update(app, id, |s| s.seen_at = now_ms().max(s.status_since)).await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Home: sets or clears the wake-up an agent asked for (`launchpad sleep`, `schedules::sleep`). A sleeping agent
    /// does not need you.
    /// Home: sets a session's wake-up and runs `with` (the scheduled follow-up's row, `schedules::sleep`) in the same
    /// transaction: both are stored or neither, so no restart or failure leaves a sleeping session without its
    /// wake-up, or a wake-up of a session that does not sleep.
    pub async fn set_sleep_with(
        &self,
        app: &App,
        id: &str,
        sleep: Option<crate::model::Sleep>,
        with: impl FnOnce(&rusqlite::Connection) -> Result<()> + Send + 'static,
    ) -> Result<Session> {
        let (before, stored) = {
            let mut recs = self.inner.records.lock().unwrap();
            let s = recs.get_mut(id).context("no such session")?;
            let before = std::mem::replace(&mut s.sleep, sleep.clone());
            let record = s.clone();
            // Queued now, under the records' lock, like `update`: writes of this session stay in order.
            let stored = app.state_db.enqueue(move |c| Self::store_with(c, &record, with));
            (before, stored)
        };
        if let Err(e) = stored.await {
            let mut recs = self.inner.records.lock().unwrap();
            if let Some(s) = recs.get_mut(id).filter(|s| s.sleep == sleep) {
                s.sleep = before;
            }
            return Err(e);
        }
        let s = self.get(id).context("no such session")?;
        app.bus.publish("sessions", "updated", serde_json::to_value(s.light())?);
        self.publish_counts(app);
        Ok(s)
    }

    /// One transaction: `with`, then the session's record; nothing is stored when either fails.
    pub(crate) fn store_with(c: &mut rusqlite::Connection, record: &Session, with: impl FnOnce(&rusqlite::Connection) -> Result<()>) -> Result<()> {
        let tx = c.transaction()?;
        with(&tx)?;
        Self::store(&tx, record)?;
        tx.commit()?;
        Ok(())
    }

    pub async fn set_sleep(&self, app: &App, id: &str, sleep: Option<crate::model::Sleep>) -> Result<Session> {
        let s = self.update(app, id, |s| s.sleep = sleep).await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Returns a completed session to Needs you without changing its run state.
    pub async fn mark_unread(&self, app: &App, id: &str) -> Result<Session> {
        let _action = self.action_lock(id).await;
        let s = self.update(app, id, |s| {
            if matches!(s.status, SessionStatus::Waiting | SessionStatus::Error | SessionStatus::Interrupted) {
                s.seen_at = s.status_since.saturating_sub(1);
            }
        }).await?;
        self.publish_counts(app);
        Ok(s)
    }

    fn publish_counts(&self, app: &App) {
        let (mut running, mut done, mut quota_paused) = (0, 0, 0);
        for s in self.inner.records.lock().unwrap().values() {
            if s.status.is_running() {
                running += 1;
            } else if s.status == SessionStatus::QuotaPaused {
                quota_paused += 1;
            } else if s.needs_you() {
                done += 1;
            }
        }
        app.status.update(|st| {
            st.sessions.running = running;
            st.sessions.done = done;
            st.sessions.quota_paused = quota_paused;
        });
    }

    pub async fn events(&self, app: &App, id: &str, after: i64) -> Result<Vec<SessionEvent>> {
        self.events_through(app, id, after, i64::MAX).await
    }

    async fn events_through(&self, app: &App, id: &str, after: i64, through: i64) -> Result<Vec<SessionEvent>> {
        let id = id.to_string();
        app.state_db
            .call(move |c| {
                let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq > ?2 AND seq <= ?3 ORDER BY seq")?;
                let rows = st
                    .query_map(params![id, after, through], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?
                    .filter_map(|r| r.ok())
                    .filter_map(|(seq, ts, json)| {
                        serde_json::from_str::<EventBody>(&json).ok().map(|body| SessionEvent { session_id: id.clone(), seq, ts, body })
                    })
                    .collect();
                Ok(rows)
            })
            .await
    }

    /// The stored trace of a session through a fixed event, as text for another agent.
    pub async fn trace_text(&self, app: &App, id: &str, through_seq: Option<i64>) -> Result<String> {
        let source = self.get(id).with_context(|| format!("no agent {id}"))?;
        let through_seq = through_seq.unwrap_or(source.last_seq);
        anyhow::ensure!(through_seq > 0 && through_seq <= source.last_seq, "invalid trace sequence: agent {id} has events through {} only, not {through_seq}", source.last_seq);
        let events = self.events_through(app, &source.id, 0, through_seq).await?;
        anyhow::ensure!(events.last().is_some_and(|e| e.seq == through_seq), "the trace of agent {id} through event {through_seq} is unavailable");
        crate::compose::render_trace(&source, through_seq, &events)
    }

    /// The source agents of context traces. Home holds every session and every event of every generation; a
    /// runner holds only the sessions it ran (a Burst runner none), so it asks Home for the others and for those
    /// whose local record ends before the requested event (a session that moved away).
    async fn context_traces(&self, app: &App, refs: &[crate::model::AgentTraceRef]) -> Result<Vec<(Session, i64)>> {
        let runner = app.paths.role == crate::role::Role::Runner;
        resolve_traces(refs, |id| self.get(id), |id| {
            let sessions = self.clone();
            async move {
                if !runner {
                    return sessions.get(&id).context("no such session");
                }
                let v = crate::home_link::home_request("GET", &format!("/api/sessions/{id}"), None).await?;
                Ok(serde_json::from_value(v)?)
            }
        })
        .await
    }

    pub fn sink(&self, app: &App, id: &str) -> Sink {
        Sink { app: app.clone(), sessions: self.clone(), id: id.to_string(), instance: 0, agent_id: None }
    }

    // ---------------------------------------------------------------- operations

    /// Resolves a launch spec: cwd, prompt, title and context directories. No side effect.
    pub async fn preview(&self, app: &App, spec: &LaunchSpec) -> Result<Preview> {
        let mut spec = spec.clone();
        if app.paths.role == crate::role::Role::Home && (!spec.topics.is_empty() || !spec.agent_traces.is_empty()) {
            let ws = crate::scope::of_spec(app, &spec).await.context("Context needs a workspace")?;
            crate::topics::resolve(app, &ws, &mut spec.topics, &mut spec.agent_traces).await?;
        }
        if app.paths.role == crate::role::Role::Home && !spec.folios.is_empty() {
            let ws = crate::scope::of_spec(app, &spec).await.context("Folio context needs a workspace")?;
            crate::folio_ref::resolve(app, &ws, &mut spec.folios).await?;
        }
        if app.paths.role == crate::role::Role::Home && let Some(ws) = crate::scope::of_spec(app, &spec).await {
            let text = spec.text.clone();
            crate::refs::resolve_context(app, &ws, &mut spec.refs, &text).await?;
        }
        self.resolve(app, &spec, false).await
    }

    pub async fn resume_paid(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.action_lock(id).await;
        self.continue_quota(app, id, QuotaResume::Paid).await
    }

    pub async fn resume_subscription(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.action_lock(id).await;
        self.continue_quota(app, id, QuotaResume::Manual).await
    }

    fn quota_blockers(&self, app: &App, s: &Session) -> Vec<crate::model::QuotaBlock> {
        let reading = crate::quota::reading_for(app, s.harness, &s.launch_spec);
        let mut retries = self.inner.manual_quota_retries.lock().unwrap();
        if let Some(&baseline) = retries.get(&s.id) {
            if reading.quota_at.unwrap_or(0) <= baseline { return vec![]; }
            retries.remove(&s.id);
        }
        crate::quota::exhausted(&reading)
    }

    /// Toggles paid usage for one session. Turning it off puts the session back under quota protection at the next guard check.
    pub async fn set_paid_usage(&self, app: &App, id: &str, allow: bool) -> Result<Session> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(crate::harness::registration(s.harness).descriptor.capabilities.quota,
            "This integration does not monitor subscription quotas; native provider billing settings apply.");
        anyhow::ensure!(!crate::profiles::pay_as_you_go(app, &s.launch_spec),
            "This pay-as-you-go profile has no subscription quota protection. Cost limits are available only with reliable model cost reporting.");
        if s.launch_spec.allow_paid_usage == allow { return Ok(s); }
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        let s = self.update(app, id, |s| s.launch_spec.allow_paid_usage = allow).await?;
        self.sink(app, id).emit(EventBody::Notice { text: if allow { "Paid usage allowed for this session; additional credits may be used after included quota runs out." } else { "Paid usage disabled; this session pauses at included quota limits." }.into() }).await;
        Ok(s)
    }

    /// Sets or removes the cost limit of one session. A running turn above the new limit is interrupted by the guard.
    pub async fn set_budget(&self, app: &App, id: &str, budget_usd: Option<f64>) -> Result<Session> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(budget_usd.is_none() || (!s.usage.cost_unavailable && crate::profiles::supports_cost(app, s.harness, &s.launch_spec, s.model.as_deref())), "{}", crate::harness::COST_UNAVAILABLE);
        check_budget(s.harness, budget_usd)?;
        if s.launch_spec.budget_usd == budget_usd { return Ok(s); }
        let s = self.update(app, id, |s| s.launch_spec.budget_usd = budget_usd).await?;
        let spent = s.usage.cost_usd.unwrap_or(0.0);
        self.sink(app, id).emit(EventBody::Notice { text: match budget_usd {
            Some(b) => format!("Cost limit set to ${b:.2} (${spent:.2} reported so far).{}", if s.budget_reached() { " The limit is reached: a running turn stops and no new turn starts." } else { "" }),
            None => "Cost limit removed.".into(),
        } }).await;
        Ok(s)
    }

    /// Reconfigure the same native conversation at an idle boundary, then continue an interrupted task.
    pub async fn set_permissions(&self, app: &App, id: &str, mode: PermissionMode) -> Result<Session> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        self.ensure_shutdown_finished(id)?;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(crate::harness::registration(s.harness).descriptor.capabilities.permission_modes,
            "This harness manages permissions in its native configuration.");
        anyhow::ensure!(s.status != SessionStatus::InTerminal, "Re-attach the session before changing permissions.");
        anyhow::ensure!(!self.setup_running(id), "Wait for the agent to start before changing permissions.");
        if s.launch_spec.permission_mode == mode { return Ok(s); }
        if let Some(live) = self.live(id) {
            let continued = live.turn_active() || s.open_question;
            anyhow::ensure!(!live.has_background_work(), "Wait for background commands to finish before changing permissions.");
            anyhow::ensure!(continued || live.ready_for_turn(),
                "Wait for background commands to finish before changing permissions.");
            if continued {
                // Treat this as an owner interruption: no failure/completion notification and no
                // approval of the pending operation. The resumed agent reconsiders it itself.
                self.inner.stop_requests.lock().unwrap().insert(id.to_string());
                if let Err(e) = live.interrupt().await {
                    self.inner.stop_requests.lock().unwrap().remove(id);
                    return Err(e);
                }
                drop(_action);
                tokio::time::timeout(std::time::Duration::from_secs(20), async {
                    while !live.ready_for_turn() || self.get(id).is_some_and(|s| s.status.is_running()) {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }).await.context("The turn has not stopped or background commands are still running. Permissions are unchanged; wait for them to finish and try again.")?;
                let _action = self.action_lock(id).await;
                let current = self.get(id).context("no such session")?;
                anyhow::ensure!(self.live(id).is_some_and(|now| Arc::ptr_eq(&now, &live))
                    && current.status == SessionStatus::Stopped && current.quota_pause.is_none()
                    && current.launch_spec.permission_mode == s.launch_spec.permission_mode
                    && live.ready_for_turn(), "The session changed while stopping; permissions are unchanged. Try again.");
                return self.apply_permissions(app, id, mode, true).await;
            }
        }
        self.apply_permissions(app, id, mode, false).await
    }

    /// Caller holds the session's action lock and has validated the native attachment.
    async fn apply_permissions(&self, app: &App, id: &str, mode: PermissionMode, continued: bool) -> Result<Session> {
        let sink = self.sink(app, id);
        self.update(app, id, |s| s.launch_spec.permission_mode = mode).await?;
        // Reuse model/effort replacement so every adapter applies its verified startup/resume
        // settings, including native Auto support checks, without granting a waiting request.
        self.inner.process_restarts.lock().unwrap().insert(id.to_string());
        let label = match mode { PermissionMode::Yolo => "YOLO", PermissionMode::Approvals => "Ask for approval", PermissionMode::Auto => "Auto approval" };
        sink.emit(EventBody::Notice { text: format!("Permissions changed to {label}.{}", if continued { " Resuming the same conversation; pending requests were cancelled." } else { " Applies when the session next runs." }) }).await;
        if continued {
            self.send_locked(app, id, "Continue the task from where you stopped. The owner changed this session's permission mode; reconsider any cancelled operation under the current native permissions.", vec![], InputMode::Manual, None, None).await.context("Permissions saved, but the session could not continue")?;
        }
        self.get(id).context("no such session")
    }

    /// Changes model and effort through the live adapter, or resumes a replacement process once idle.
    pub async fn set_model(&self, app: &App, id: &str, model: Option<String>, effort: Option<String>) -> Result<Session> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.status != SessionStatus::InTerminal, "The session is open in a terminal: re-attach it first.");
        let defaults = app.models.defaults(app, s.harness);
        anyhow::ensure!(effort.is_none() || !defaults.efforts.is_empty() || defaults.models.iter().any(|m| !m.efforts.is_empty()), "This harness does not support an effort override.");
        // A profile with its own model list (a provider such as the Hugging Face router) cannot serve the
        // harness's base models: every later turn would fail.
        if let Some(m) = &model {
            let config = app.config.get();
            let own = s.launch_spec.profile.as_deref().and_then(|p| config.profile(p)).filter(|p| p.models_url.is_some());
            if let Some(p) = own {
                anyhow::ensure!(p.model.as_deref() == Some(m.as_str()) || !defaults.models.iter().any(|b| &b.id == m), "{} uses its own model list: {m} is not one of its models.", p.name);
            }
        }
        if s.model == model && s.effort == effort { return Ok(s); }
        let (m, e) = (model.clone(), effort.clone());
        let s = self.update(app, id, |s| { s.model = m; s.effort = e; }).await?;
        let label = [model.as_deref().unwrap_or("default model"), effort.as_deref().unwrap_or("default effort")].join(" · ");
        let sink = self.sink(app, id);
        let Some(live) = self.live(id) else {
            sink.emit(EventBody::Notice { text: format!("Model set to {label}; it applies when the session resumes.") }).await;
            return Ok(s);
        };
        if live.set_model(model.as_deref(), effort.as_deref()) {
            sink.emit(EventBody::Notice { text: format!("Model set to {label} from the next turn.") }).await;
            return Ok(s);
        }
        self.inner.process_restarts.lock().unwrap().insert(id.to_string());
        if s.status.is_running() || s.open_question || !live.ready_for_turn() {
            sink.emit(EventBody::Notice { text: format!("Model set to {label}; the agent restarts with it after the current turn.") }).await;
            return Ok(s);
        }
        self.reattach(app, id).await?;
        sink.emit(EventBody::Notice { text: format!("Restarted with {label}.") }).await;
        self.get(id).context("no such session")
    }

    async fn continue_quota(&self, app: &App, id: &str, mode: QuotaResume) -> Result<()> {
        let paid = mode == QuotaResume::Paid;
        let manual = mode == QuotaResume::Manual;
        let fallback = mode == QuotaResume::ResetFallback;
        let s = self.get(id).context("no such session")?;
        let p = s.quota_pause.clone().context("The session is not quota paused")?;
        ensure_budget(app, &s)?;
        anyhow::ensure!(!s.open_question && self.live(id).is_none_or(|l| l.ready_for_turn()), "Wait for the interrupted turn and outstanding commands to finish.");
        if manual {
            anyhow::ensure!(matches!(p.phase, QuotaPausePhase::Paused | QuotaPausePhase::ProviderLimited), "Wait for the interrupted turn to finish; a claimed continuation needs inspection before retrying.");
            anyhow::ensure!(!s.launch_spec.allow_paid_usage, "Paid usage is already allowed for this session.");
        } else if !paid {
            anyhow::ensure!(p.phase == QuotaPausePhase::Paused && {
                let reading = crate::quota::reading_for(app, s.harness, &s.launch_spec);
                crate::quota::available(&reading, now_ms()) || (fallback && crate::quota::reset_fallback(&reading, &p, now_ms()))
            }, "Waiting for a fresh quota reading");
        }
        self.update(app, id, |s| {
            if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Resuming; p.error = None; }
            if paid { s.launch_spec.allow_paid_usage = true; }
        }).await?;
        if manual || fallback {
            // Ignore only the old snapshot until new evidence arrives. A fresh exhausted
            // reading pauses again and cannot reuse the already passed reset fallback.
            let baseline = crate::quota::reading_for(app, s.harness, &s.launch_spec).quota_at.unwrap_or(0);
            self.inner.manual_quota_retries.lock().unwrap().insert(id.to_string(), baseline);
        }
        // Preserve the claim until the send has returned. Recovery never retries a claimed send.
        let result = async {
            let live = self.reattach(app, id).await?;
            let sink = self.sink(app, id);
            let client_id = p.client_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if p.client_id.is_none() {
                sink.emit(EventBody::UserMessage { text: p.message.clone(), client_id: client_id.clone(), queued: false, images: p.images.clone(), from: None }).await;
            }
            sink.emit(EventBody::Notice { text: if paid { "Continuing with paid usage allowed" } else if fallback { "Last known 5-hour reset passed by one minute; retrying automatically with subscription-only protection while quota is unconfirmed. Paid credits may still be used if included quota is exhausted." } else if manual { "Continuing with subscription-only protection despite an unconfirmed quota reading; paid credits may still be used if the included quota is exhausted." } else { "Quota available; continuing automatically" }.into() }).await;
            sink.set_status(SessionStatus::Working).await;
            let mut text = p.message.clone();
            let mut images = p.images.clone();
            for message in &p.messages {
                text.push_str("\n\n");
                text.push_str(&message.text);
                images.extend(message.images.iter().cloned());
            }
            live.send(&text, &client_id, &crate::attachments::for_harness(app, id, &images).await?).await?;
            for message in &p.messages {
                sink.emit(EventBody::MessageDelivered { client_id: message.client_id.clone() }).await;
            }
            if p.client_id.is_some() { sink.emit(EventBody::MessageDelivered { client_id }).await; }
            Ok::<(), anyhow::Error>(())
        }.await;
        match result {
            Ok(()) => {
                let resumed = self.update(app, id, |s| {
                    // A fast provider rejection may already have paused this new turn.
                    if s.quota_pause.as_ref().is_some_and(|p| p.phase == QuotaPausePhase::Resuming) { s.quota_pause = None; }
                }).await?;
                if resumed.quota_pause.is_none() && !paid && !manual && !fallback && !self.is_watched(id) {
                    crate::notify::session(app, &resumed, crate::notify::SessionNotification::QuotaResumed);
                }
                Ok(())
            }
            Err(e) => { self.inner.manual_quota_retries.lock().unwrap().remove(id); self.quota_attention(app, id, &format!("Continuation outcome needs inspection: {e:#}")).await?; Err(e) }
        }
    }

    async fn quota_attention(&self, app: &App, id: &str, message: &str) -> Result<()> {
        self.update(app, id, |s| { s.change_status(SessionStatus::QuotaPaused); if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::NeedsAttention; p.error = Some(message.into()); } }).await?;
        self.publish_counts(app);
        Ok(())
    }

    pub async fn send(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>) -> Result<()> {
        self.send_inner(app, id, text, images, InputMode::Manual, None, None).await
    }

    /// Resolve follow-up context under the same lock as delivery. No launch-only instructions
    /// or worktree creation: additional folders remain read-only references.
    pub async fn send_with_context(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, context: MessageContext, client_id: Option<String>) -> Result<()> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        if context.is_empty() {
            return self.send_locked(app, id, text, images, InputMode::Manual, client_id, context.from).await;
        }
        let session = self.get(id).context("no such session")?;
        let mut folders = Vec::new();
        for cp in &context.context_projects {
            anyhow::ensure!(!cp.editable, "Follow-up context folders must be read-only");
            let project = crate::projects::resolve(app, &cp.project_id).await.context("Context folder unavailable; remove it and select it again")?;
            if project.plain { crate::plain::reference(app, &project.id, std::path::Path::new(&project.path)).await?; }
            if project.home_git && !crate::files::is_dir(&project.path).await { crate::repo_cache::sync(app, &project).await?; }
            folders.push(ContextInfo { path: project.path, description: project.description, branch: cp.branch.clone(), editable: false, own_worktree: false });
        }
        let traces = self.context_traces(app, &context.agent_traces).await?;
        crate::prompt_context::validate_mentions(text, &context.mentions)?;
        let spec = LaunchSpec { topics: context.topics.clone(), text: text.into(), github_refs: context.github_refs, selection: context.selection, mentions: context.mentions, skills: context.skills, profile: session.launch_spec.profile.clone(), ..Default::default() };
        let mut prompt = render_prompt(&spec, &folders, &[]);
        prompt.push_str(&crate::prompt_context::render_skills(app, session.harness, &spec, Some(&session.cwd)).await?);
        if !traces.is_empty() {
            if !prompt.is_empty() { prompt.push_str("\n\n"); }
            prompt.push_str(&crate::compose::trace_references(&traces, &app.paths.cli_command()));
        }
        prompt.push_str(&crate::compose::topic_notes(&context.topics));
        prompt.push_str(&crate::folio_ref::notes(&context.folios, &app.paths.cli_command()));
        prompt.push_str(&crate::refs::notes(&context.refs, &app.paths.cli_command()));
        self.send_locked(app, id, &prompt, images, InputMode::Manual, client_id, context.from).await
    }

    /// `from`: an agent scheduled it (stamped by Home, `senders`); its text arrives already under its header.
    pub async fn send_follow_up(&self, app: &App, id: &str, text: &str, automatic: bool, from: Option<crate::model::MessageFrom>) -> Result<()> {
        self.ensure_local(id)?;
        self.send_inner(app, id, text, vec![], if automatic { InputMode::FollowUpAutomatic } else { InputMode::FollowUpManual }, None, from).await
    }

    /// An automatic follow-up that waits in a quota pause when this runner's reading does not confirm quota yet,
    /// instead of being refused (`unconfirmed_quota`).
    pub async fn send_held_follow_up(&self, app: &App, id: &str, text: &str, from: Option<crate::model::MessageFrom>) -> Result<()> {
        self.ensure_local(id)?;
        self.send_inner(app, id, text, vec![], InputMode::FollowUpHeld, None, from).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_inner(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, mode: InputMode, client_id: Option<String>, from: Option<crate::model::MessageFrom>) -> Result<()> {
        let _action = self.action_lock(id).await;
        self.send_locked(app, id, text, images, mode, client_id, from).await
    }

    /// Caller holds the session's action lock, including during permission changes.
    /// `from`: the sender Home stamped when it is not the owner (`senders`), recorded on the `user_message` event.
    #[allow(clippy::too_many_arguments)]
    async fn send_locked(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, mode: InputMode, client_id: Option<String>, from: Option<crate::model::MessageFrom>) -> Result<()> {
        crate::role::ensure_runs_agents(app.paths.role)?;
        self.ensure_local(id)?;
        self.ensure_shutdown_finished(id)?;
        self.update(app, id, |s| s.capacity_retry_at = None).await?;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(!self.setup_running(id), "The agent is still starting; send your message once it is ready.");
        if mode != InputMode::Manual && (s.status.is_running() || s.open_question || s.status == SessionStatus::InTerminal || s.quota_pause.is_some()) {
            return Err(FollowUpBusy.into());
        }
        let capabilities = crate::harness::registration(s.harness)
            .descriptor
            .capabilities;
        anyhow::ensure!(
            images.is_empty() || capabilities.images,
            "This harness does not support images."
        );
        anyhow::ensure!(
            !s.status.is_running() || capabilities.messages_while_running,
            "Wait for this harness to finish its current turn before sending another message."
        );
        if s.status == SessionStatus::InTerminal {
            bail!("the session is open in a terminal: re-attach it first");
        }
        if let Some(pause) = &s.quota_pause {
            anyhow::ensure!(matches!(pause.phase, QuotaPausePhase::Pausing | QuotaPausePhase::Paused | QuotaPausePhase::ProviderLimited),
                "The quota continuation needs inspection or is already resuming. Wait or Stop before sending input.");
            let client_id = client_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            self.update(app, id, |s| {
                s.quota_pause.as_mut().unwrap().messages.push(QuotaMessage {
                    text: text.into(), images: images.clone(), client_id: client_id.clone(),
                });
            }).await?;
            self.sink(app, id).emit(EventBody::UserMessage {
                text: text.into(), client_id, queued: true, images, from,
            }).await;
            return Ok(());
        }
        ensure_budget(app, &s)?;
        let protected = crate::quota::protected(app, s.harness, &s.launch_spec);
        match unconfirmed_quota(mode, protected, crate::quota::available(&crate::quota::reading_for(app, s.harness, &s.launch_spec), now_ms())) {
            Some(true) => return self.hold_input(app, id, text, images, from, client_id).await,
            Some(false) => return Err(FollowUpBusy.into()),
            None => {}
        }
        if protected && !self.quota_blockers(app, &s).is_empty() {
            if s.status.is_running() || s.open_question { bail!("Included quota exhausted; wait for quota to reset before sending input or answering this question."); }
            self.hold_input(app, id, text, images, from, client_id).await?;
            return Ok(());
        }
        anyhow::ensure!(
            s.open_question
                || s.status.is_running()
                || self.live(id).is_none_or(|l| l.ready_for_turn()),
            "Wait for outstanding commands to finish before starting another turn."
        );
        if protected && !crate::quota::available(&crate::quota::reading_for(app, s.harness, &s.launch_spec), now_ms()) {
            self.sink(app, id).emit(EventBody::Notice { text: "Quota reading unavailable or stale; continuing with best-effort protection.".into() }).await;
        }
        let resumed = self.live(id).is_none();
        let live = self.reattach(app, id).await?;
        let sink = self.sink(app, id);
        if resumed {
            sink.set_open_question(false).await;
            sink.emit(EventBody::Notice { text: "Resumed".into() }).await;
        }
        self.inner.stop_requests.lock().unwrap().remove(id);
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.inner.budget_interrupts.lock().unwrap().remove(id);
        let client_id = client_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let queued = s.status.is_running() && !resumed;
        sink.emit(EventBody::UserMessage { text: text.to_string(), client_id: client_id.clone(), queued, images: images.clone(), from }).await;
        if !queued {
            sink.set_status(SessionStatus::Working).await;
        }
        let sent = match crate::attachments::for_harness(app, id, &images).await {
            Ok(images) => live.send(text, &client_id, &images).await,
            Err(e) => Err(e),
        };
        if let Err(e) = sent {
            if e.is::<InputDeliveryError>() {
                sink.emit(EventBody::Notice { text: format!("{e:#}") }).await;
            } else {
                sink.fail(&format!("{e:#}")).await;
            }
            return Err(e);
        }
        // The turn this message joined ended while it was on its way: the harness works on it in a turn of its own,
        // which is Working again so that its end is a turn end (saved, notified) too.
        if queued {
            sink.set_status_when(SessionStatus::Working, |s| s.status == SessionStatus::Waiting && !s.open_question && s.quota_pause.is_none() && live.turn_active()).await;
        }
        Ok(())
    }

    pub async fn processes(&self, id: &str) -> Result<Vec<SessionProcess>> {
        anyhow::ensure!(self.get(id).is_some(), "no such session");
        match self.live(id) {
            Some(live) => live.processes().await,
            None => Ok(vec![]),
        }
    }

    pub async fn terminate_process(&self, app: &App, id: &str, thread: &str, process: &str) -> Result<()> {
        self.ensure_local(id)?;
        let live = {
            let _action = self.action_lock(id).await;
            self.live(id).context("the session has no running process")?
        };
        // A host/RPC wait must not hold the session's action lock. The attachment keeps
        // ownership until command completion even if this request times out.
        tokio::time::timeout(std::time::Duration::from_secs(5), live.terminate_process(thread, process)).await
            .context("Stop is not confirmed yet. Check Processes and Trace before trying again")??;
        self.sink(app, id).emit(EventBody::Notice { text: format!("Stop requested by you for background command {process}.") }).await;
        Ok(())
    }

    pub async fn interrupt(&self, app: &App, id: &str) -> Result<()> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        self.update(app, id, |s| s.capacity_retry_at = None).await?;
        // An owner Stop takes precedence over a pending cost-limit interruption.
        self.inner.budget_interrupts.lock().unwrap().remove(id);
        let setup = self.inner.setups.lock().unwrap().remove(id);
        if let Some(task) = setup {
            self.wait_for_shutdown(id, async move { task.stop().await; Ok(()) }).await?;
            self.inner.manual_quota_retries.lock().unwrap().remove(id);
            self.update(app, id, |s| s.quota_pause = None).await?;
            let sink = self.sink(app, id);
            sink.emit(EventBody::Notice { text: "Launch cancelled by you; the agent was not started.".into() }).await;
            sink.set_status(SessionStatus::Stopped).await;
            return Ok(());
        }
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        if self.get(id).is_some_and(|s| s.quota_pause.is_some()) {
            if let Some(live) = self.live(id).filter(|l| l.turn_active()) { live.interrupt().await?; }
            self.inner.quota_stops.lock().unwrap().insert(id.to_string());
            self.update(app, id, |s| { s.quota_pause = None; }).await?;
            // The owner's Stop: a turn still pausing (Working) ends here without a Done notification.
            self.inner.stop_requests.lock().unwrap().insert(id.to_string());
            self.sink(app, id).set_status(SessionStatus::Waiting).await;
            self.inner.stop_requests.lock().unwrap().remove(id);
            self.sink(app, id).emit(EventBody::Notice { text: "Automatic quota continuation cancelled by you. Held input will not be sent.".into() }).await;
            return Ok(());
        }
        let live = self.live(id).context("the session has no running process")?;
        let sink = self.sink(app, id);
        self.inner.stop_requests.lock().unwrap().insert(id.to_string());
        if let Err(e) = live.interrupt().await {
            self.inner.stop_requests.lock().unwrap().remove(id);
            return Err(e);
        }
        sink.set_open_question(false).await;
        Ok(())
    }

    pub async fn answer(&self, app: &App, id: &str, request_id: &str, answers: Vec<String>) -> Result<()> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.quota_pause.is_none(), "The session is quota paused.");
        anyhow::ensure!(!crate::quota::protected(app, s.harness, &s.launch_spec) || self.quota_blockers(app, &s).is_empty(), "Included quota exhausted; wait for the quota pause.");
        ensure_budget(app, &s)?;
        let live = self.live(id).context("the session has no running process")?;
        let stopping = live.answer_stops_turn(request_id, &answers);
        if stopping { self.inner.stop_requests.lock().unwrap().insert(id.into()); }
        if let Err(e) = live.answer(request_id, answers.clone()).await {
            if stopping { self.inner.stop_requests.lock().unwrap().remove(id); }
            return Err(e);
        }
        let sink = self.sink(app, id);
        sink.emit(EventBody::AskAnswered { request_id: request_id.to_string(), answers }).await;
        let open = live.has_open_questions();
        sink.set_open_question(open).await;
        if open { sink.set_status(SessionStatus::Waiting).await; }
        else if live.turn_active() { sink.set_status(SessionStatus::Working).await; }
        Ok(())
    }

    pub(crate) async fn detach(&self, app: &App, id: &str) -> Result<()> {
        self.ensure_shutdown_finished(id)?;
        let live = self.inner.live.lock().unwrap().get(id).cloned();
        if let Some((instance, live)) = live {
            let (this, app, session_id) = (self.clone(), app.clone(), id.to_string());
            self.wait_for_shutdown(id, async move {
                // A failed Codex stop must retain the attachment and its busy state even
                // after the background shutdown marker clears. Never forget live commands.
                live.stop().await?;
                crate::contain::release(&app, &session_id).await;
                {
                    let mut attached = this.inner.live.lock().unwrap();
                    if attached.get(&session_id).is_some_and(|(current, _)| *current == instance) {
                        attached.remove(&session_id);
                    }
                }
                // Settle the record before releasing folder ownership, including when
                // the caller timed out and this shutdown continued in the background.
                this.update(&app, &session_id, |s| { s.live = false; s.open_question = false; }).await?;
                if this.get(&session_id).is_some_and(|s| s.status.is_running()) {
                    this.sink(&app, &session_id).set_status(SessionStatus::Stopped).await;
                }
                Ok(())
            }).await?;
        }
        Ok(())
    }

    /// Stops the process. The conversation stays resumable.
    pub async fn close(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.action_lock(id).await;
        self.close_locked(app, id).await
    }

    /// Caller holds the session action lock (for atomic worktree discard).
    pub(crate) async fn close_locked(&self, app: &App, id: &str) -> Result<()> {
        self.update(app, id, |s| s.capacity_retry_at = None).await?;
        let setup = self.inner.setups.lock().unwrap().remove(id);
        if let Some(task) = setup { self.wait_for_shutdown(id, async move { task.stop().await; Ok(()) }).await?; }
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.update(app, id, |s| s.quota_pause = None).await?;
        // Keep the live attachment during shutdown, but native completion of Archive is silent.
        self.inner.budget_interrupts.lock().unwrap().remove(id);
        self.inner.stop_requests.lock().unwrap().insert(id.to_string());
        self.detach(app, id).await?;
        crate::contain::release(app, id).await;
        let sink = self.sink(app, id);
        sink.set_status(SessionStatus::Closed).await;
        self.update(app, id, |s| {
            s.live = false;
            s.open_question = false;
            s.ended_at = Some(now_ms());
            // Archive does its own last sync (`plain::after_archive`, worktree cleanup, `handoff::clean_archived`).
            s.unsaved = None;
        })
        .await?;
        drop_raw(app, self, id);
        self.publish_counts(app);
        Ok(())
    }

    /// Archive only while the session is still exactly as an automatic archive saw it (`task_archive`): Done since
    /// `since`, no question, sleep, quota pause or capacity retry, and nothing running (`is_busy`), checked under its
    /// action lock, so a message that started a turn in between is never stopped. False when it changed.
    pub async fn close_if_done(&self, app: &App, id: &str, since: i64) -> Result<bool> {
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        if !still_done(&s, since) || self.is_busy(&s) {
            return Ok(false);
        }
        self.close_locked(app, id).await?;
        Ok(true)
    }

    /// The undo of Archive: the session comes back Done (Stopped when it was not Done when archived) and seen,
    /// without a process. Its next message resumes it and brings back what Archive cleaned up (`reattach`).
    pub async fn unarchive(&self, app: &App, id: &str) -> Result<Session> {
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.status == SessionStatus::Closed, "This agent is not archived");
        let status = if s.archived_done { SessionStatus::Waiting } else { SessionStatus::Stopped };
        let s = self.update(app, id, |s| {
            let now = now_ms();
            s.change_status(status);
            s.status_since = now;
            s.seen_at = now;
            s.ended_at = None;
        })
        .await?;
        self.publish_counts(app);
        Ok(s)
    }

    /// Re-attaches an idle session, optionally continuing its unfinished turn.
    /// A session without a native conversation receives its original task and launch images.
    /// With `close_terminal`, an integrated terminal still running the session's TUI is closed first.
    pub async fn resume(&self, app: &App, id: &str, close_terminal: bool, continue_turn: bool) -> Result<()> {
        crate::role::ensure_runs_agents(app.paths.role)?;
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        self.update(app, id, |s| s.capacity_retry_at = None).await?;
        self.resume_locked(app, id, close_terminal, continue_turn).await
    }

    /// Caller holds the session's action lock, just like manual Resume.
    async fn resume_locked(&self, app: &App, id: &str, close_terminal: bool, continue_turn: bool) -> Result<()> {
        self.ensure_local(id)?;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(!s.status.is_running() && !s.open_question && self.live(id).is_none_or(|l| l.ready_for_turn()), "Wait for the current turn, question and outstanding commands to finish.");
        anyhow::ensure!(s.quota_pause.is_none(), "Use Resume with paid usage to continue a quota-paused session.");
        anyhow::ensure!(!self.setup_running(id), "Wait for the agent to start.");
        if let Some(t) = app.terminals.tui_of(id) {
            if !close_terminal {
                bail!("the session is still open in an integrated terminal: close it first");
            }
            app.terminals.close_and_wait(&[t]).await?;
        }
        if s.status == SessionStatus::InTerminal && crate::terminals::tui_running(app, &s) {
            bail!("the session is still open in another terminal: close it there first");
        }
        if s.harness_session_id.is_none() {
            let images = s.launch_spec.attachments.iter()
                .filter(|a| crate::attachments::is_image(&a.data_url))
                .map(|a| ImageAttachment { name: a.name.clone(), data_url: a.data_url.clone() })
                .collect::<Vec<_>>();
            return self.send_locked(app, id, &s.prompt, images, InputMode::Manual, None, None).await;
        }
        if continue_turn {
            return self.send_locked(app, id, "Continue the unfinished turn.", vec![], InputMode::Manual, None, None).await;
        }
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.reattach(app, id).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::Notice {
            text: if s.status == SessionStatus::InTerminal {
                "Re-attached. Messages exchanged in the terminal are not shown here.".into()
            } else {
                "Resumed".into()
            },
        })
        .await;
        sink.set_status(SessionStatus::Waiting).await;
        self.update(app, id, |s| s.ended_at = None).await?;
        Ok(())
    }

    /// Hands the session to its harness TUI in an integrated terminal.
    pub async fn open_terminal(&self, app: &App, id: &str) -> Result<crate::terminals::TerminalInfo> {
        self.ensure_local(id)?;
        let _action = self.action_lock(id).await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(crate::harness::registration(s.harness).descriptor.capabilities.terminal, "This harness does not support a terminal TUI.");
        anyhow::ensure!(!self.setup_running(id), "Wait for the agent to start.");
        if let Some(t) = app.terminals.tui_of(id) {
            return app.terminals.list().into_iter().find(|i| i.id == t).context("no such terminal");
        }
        if s.status.is_running() || s.quota_pause.is_some() {
            bail!("stop the session before opening it in a terminal; terminal sessions are outside quota protection");
        }
        if s.harness_session_id.is_none() {
            bail!("the session has no conversation yet");
        }
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.detach(app, id).await?;
        self.update(app, id, |s| {
            s.live = false;
            s.open_question = false;
        })
        .await?;
        let terminal = app.terminals.open_tui(app, &s).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::Notice { text: "Opened in the terminal".into() }).await;
        sink.set_status(SessionStatus::InTerminal).await;
        Ok(terminal)
    }

    /// Stops and forgets the sessions and terminals running in a folder (temporary project clean-up).
    pub async fn close_in(&self, app: &App, path: &str) -> Result<()> {
        let ids: Vec<String> =
            self.list().into_iter().filter(|s| std::path::Path::new(&s.cwd).starts_with(path)).map(|s| s.id).collect();
        let _action = self.action_locks(&ids).await;
        app.terminals.close_in(path).await?;
        for id in ids {
            if self.get(&id).is_some_and(|s| s.live || s.status.is_running() || s.status.is_done() || s.quota_pause.is_some()) {
                self.close_locked(app, &id).await?;
            }
        }
        Ok(())
    }

    /// After a folder rename (promote): sessions in `old` now run in `new`.
    /// Terminals there are closed by the caller before the rename.
    pub async fn move_cwd(&self, app: &App, old: &str, new: &str) -> Result<()> {
        let ids: Vec<String> = self.list().into_iter().filter(|s| s.cwd == old || s.cwd.starts_with(&format!("{old}/"))).map(|s| s.id).collect();
        for id in ids {
            let (old, new) = (old.to_string(), new.to_string());
            self.update(app, &id, move |s| s.cwd = format!("{new}{}", &s.cwd[old.len()..])).await?;
        }
        Ok(())
    }

    pub fn is_busy(&self, s: &Session) -> bool {
        self.setup_running(&s.id) || self.inner.shutdowns.lock().unwrap().contains_key(&s.id) || s.status.is_running() || self.live(&s.id).is_some_and(|l| !l.ready_for_turn())
    }

    pub fn running_in(&self, path: &str) -> bool {
        self.list().iter().any(|s| (s.cwd == path || s.cwd.starts_with(&format!("{path}/"))) && (s.live || self.is_busy(s)))
    }

    /// Called by adapters when their process ends.
    pub async fn process_exited(&self, app: &App, id: &str, instance: u64, error: Option<String>) {
        let was_live = {
            let mut live = self.inner.live.lock().unwrap();
            match live.get(id) {
                Some((i, _)) if *i == instance => live.remove(id).is_some(),
                // A newer process is attached (or none): this exit is old news.
                _ => false,
            }
        };
        if !was_live {
            return;
        }
        let Some(s) = self.get(id) else { return };
        let sink = self.sink(app, id);
        let _ = self.update(app, id, |s| s.live = false).await;
        if s.status.is_running() {
            match error {
                Some(e) => sink.fail(&e).await,
                None => sink.set_status(if sink.stop_requested() { SessionStatus::Stopped } else { SessionStatus::Interrupted }).await,
            }
        } else if let Some(e) = error {
            sink.emit(EventBody::Error { message: e }).await;
        }
    }
}

/// Resolves context traces: the local record when it already covers the requested events, else Home's
/// (`home`). Errors name the agent and what Home has.
async fn resolve_traces<F>(refs: &[crate::model::AgentTraceRef], local: impl Fn(&str) -> Option<Session>, home: impl Fn(String) -> F) -> Result<Vec<(Session, i64)>>
where
    F: std::future::Future<Output = Result<Session>>,
{
    let mut out = Vec::new();
    for t in refs {
        let id = &t.session_id;
        anyhow::ensure!(!id.is_empty() && !id.contains(['/', '?', '#']), "invalid context agent id {id:?}");
        let source = match local(id).filter(|s| t.through_seq <= s.last_seq) {
            Some(s) => s,
            None => home(id.clone()).await.with_context(|| format!("Could not read context agent {id} from Home"))?,
        };
        anyhow::ensure!(
            t.through_seq > 0 && t.through_seq <= source.last_seq,
            "The context trace of agent {id} goes through event {}, but Home has its events through {} only",
            t.through_seq,
            source.last_seq
        );
        out.push((source, t.through_seq));
    }
    Ok(out)
}

/// The presets a launch can name. Presets are shared settings of Home: a runner's own config.toml does not
/// hold them, so a runner reads Home's, and a launch that names presets fails visibly when Home cannot answer.
async fn launch_presets(app: &App, spec: &LaunchSpec, config: &crate::config::Config) -> Result<Vec<crate::config::Preset>> {
    if spec.presets.is_empty() || app.paths.role != crate::role::Role::Runner {
        return Ok(config.presets.clone());
    }
    let presets = crate::home_link::home_request("GET", "/api/context/presets", None).await.context("could not read the presets from Home")?;
    Ok(serde_json::from_value(presets)?)
}

fn check_budget(harness: HarnessKind, budget_usd: Option<f64>) -> Result<()> {
    let Some(b) = budget_usd else { return Ok(()) };
    anyhow::ensure!(b.is_finite() && b > 0.0, "The cost limit must be a positive amount.");
    anyhow::ensure!(crate::harness::registration(harness).descriptor.capabilities.cost,
        "{} does not report costs, so a cost limit cannot apply.", harness.display());
    Ok(())
}

fn ensure_budget(app: &App, s: &Session) -> Result<()> {
    anyhow::ensure!(!s.budget_reached(), "Cost limit reached: ${:.2} reported of ${:.2}. Raise or remove the limit to continue.",
        s.usage.cost_usd.unwrap_or(0.0), s.launch_spec.budget_usd.unwrap_or(0.0));
    crate::spend::ensure(app, &s.launch_spec)
}

#[derive(Clone, Debug, serde::Serialize, ts_rs::TS)]
pub struct Preview {
    pub title: String,
    pub prompt: String,
    /// The prompt in labelled parts (`start::slices`), for "What it sees at start".
    pub parts: Vec<crate::start::StartPart>,
    /// None for a new temporary folder (created at launch).
    pub cwd: Option<String>,
    pub add_dirs: Vec<String>,
}

/// Where adapters report what happens in a session.
#[derive(Clone)]
pub struct Sink {
    app: App,
    sessions: Sessions,
    pub id: String,
    /// The attached process this sink belongs to (0: no process).
    instance: u64,
    pub agent_id: Option<String>,
}

impl Sink {
    pub fn child(&self, id: &str) -> Self {
        Self { agent_id: Some(id.into()), ..self.clone() }
    }
    pub async fn subagent(&self, id: &str, name: Option<String>, status: Option<String>, model: Option<String>) {
        self.root().emit(EventBody::Subagent { agent_id: id.into(), parent_id: self.agent_id.clone(), name, status, model }).await;
    }
    pub fn root(&self) -> Self { Self { agent_id: None, ..self.clone() } }
    pub async fn emit(&self, body: EventBody) {
        let body = if let Some(id) = &self.agent_id {
            EventBody::SubagentEvent { agent_id: id.clone(), body: Box::new(body) }
        } else { body };
        let ts = now_ms();
        if body.is_ephemeral() {
            let ev = SessionEvent { session_id: self.id.clone(), seq: 0, ts, body };
            self.publish(&ev);
            return;
        }
        let preview = match &body { EventBody::AssistantMessage { text, .. } => message_preview(text), _ => None };
        let stored = {
            let mut recs = self.sessions.inner.records.lock().unwrap();
            let Some(s) = recs.get_mut(&self.id) else { return };
            s.last_seq += 1;
            if preview.is_some() { s.last_message = preview.clone(); }
            let seq = s.last_seq;
            let generation = s.generation;
            let ev = SessionEvent { session_id: self.id.clone(), seq, ts, body };
            let (id, kind, json) = (self.id.clone(), ev.body.kind(), serde_json::to_string(&ev.body).unwrap_or_default());
            let sink = self.clone();
            // Enqueue under the record lock, then publish in database order after commit.
            self.app.state_db.enqueue(move |c| {
                let tx = c.transaction()?;
                tx.execute("INSERT INTO session_event (session_id, seq, ts, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, seq, ts, kind, json])?;
                tx.execute("UPDATE session SET last_seq = max(last_seq, ?1), last_message = coalesce(?3, last_message) WHERE id = ?2", params![seq, id, preview])?;
                crate::replica::queue(&tx, &crate::replica::Item::Event { generation, event: ev.clone() })?;
                tx.commit()?;
                sink.publish(&ev);
                Ok(())
            })
        };
        if let Err(e) = stored.await {
            tracing::error!("store event: {e:#}");
        }
    }

    fn publish(&self, ev: &SessionEvent) {
        // Windows load pasted images by URL instead of receiving them inline.
        let mut linked;
        let ev = if matches!(&ev.body, EventBody::UserMessage { images, .. } if !images.is_empty()) {
            linked = ev.clone();
            crate::session_pages::link_images(&mut linked);
            &linked
        } else { ev };
        self.app.bus.send(Event {
            seq: ev.seq as u64,
            topic: format!("session/{}", self.id),
            kind: "event".into(),
            payload: serde_json::to_value(ev).unwrap_or(Value::Null),
        });
    }

    /// Stores a raw harness line (capped per session in lines and bytes), so the normalizer can be improved and
    /// replayed. Nothing reads them in the daemon; they go at Archive and after `RAW_KEEP_MS`.
    pub async fn raw(&self, dir: &str, line: &str) {
        let known = self.sessions.inner.raw_counts.lock().unwrap().contains_key(&self.id);
        if !known {
            // First line since the daemon started: continue the numbering and the size of earlier processes.
            let id = self.id.clone();
            let next = self
                .app
                .state_db
                .call(move |c| Ok(c.query_row("SELECT coalesce(max(n) + 1, 0), coalesce(sum(length(CAST(line AS BLOB))), 0) FROM session_raw WHERE session_id = ?1", [id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?))
                .await
                .unwrap_or((0, 0));
            self.sessions.inner.raw_counts.lock().unwrap().entry(self.id.clone()).or_insert(next);
        }
        let line = raw_cut(line, RAW_LINE_BYTES);
        let n = {
            let mut counts = self.sessions.inner.raw_counts.lock().unwrap();
            let Some((next, bytes)) = counts.get_mut(&self.id) else { return };
            if *next >= RAW_LINES_CAP || *bytes + line.len() as i64 > RAW_BYTES_CAP {
                return;
            }
            *next += 1;
            *bytes += line.len() as i64;
            *next - 1
        };
        let (id, dir, line) = (self.id.clone(), dir.to_string(), line.to_string());
        // Queued, not awaited: the harness's output loop never waits for the database.
        drop(self.app.state_db.enqueue(move |c| Ok(c.execute("INSERT OR IGNORE INTO session_raw (session_id, n, ts, dir, line) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, n, now_ms(), dir, line])?)));
    }

    pub fn stop_requested(&self) -> bool {
        self.sessions.inner.stop_requests.lock().unwrap().contains(&self.id)
    }

    pub fn status(&self) -> Option<SessionStatus> {
        if self.agent_id.is_some() { return None; }
        self.sessions.get(&self.id).map(|s| s.status)
    }

    pub async fn set_status(&self, status: SessionStatus) {
        if let Some(id) = &self.agent_id {
            let status = if status.is_running() { "running" } else { match status {
                SessionStatus::Error => "failed", SessionStatus::Interrupted => "interrupted",
                SessionStatus::Stopped | SessionStatus::Closed => "stopped", _ => "completed",
            }};
            self.root().subagent(id, None, Some(status.into()), None).await;
            return;
        }
        self.set_status_when(status, |_| true).await;
    }

    /// `set_status` when `when` holds for the record as it is under the session's status lock.
    pub(crate) async fn set_status_when(&self, status: SessionStatus, when: impl FnOnce(&Session) -> bool) {
        // Subagents (`agent_id`) go through `set_status`, which never calls this for them.
        if self.agent_id.is_some() { return; }
        // One change at a time: each one decides from the record the previous one left, and the Git and save
        // mark awaits between its read and its write let no other change in (`transition`).
        let lock = self.sessions.inner.statuses.get(&self.id);
        let _status = lock.lock().await;
        // Detached adapters can still report a final result after Archive or reattachment.
        if self.instance != 0 && self.sessions.inner.live.lock().unwrap().get(&self.id).is_none_or(|(instance, _)| *instance != self.instance) { return; }
        if !self.sessions.get(&self.id).is_some_and(|s| when(&s)) { return; }
        let requested = status;
        let budget_stop = self.sessions.inner.budget_interrupts.lock().unwrap().contains_key(&self.id);
        let manual_stop = self.stop_requested() && !budget_stop;
        if matches!(status, SessionStatus::Stopped | SessionStatus::Closed | SessionStatus::Interrupted | SessionStatus::Error) {
            self.sessions.inner.stop_requests.lock().unwrap().remove(&self.id);
        }
        let Some(before) = self.sessions.get(&self.id) else { return };
        let quota_stop = self.sessions.inner.quota_stops.lock().unwrap().contains(&self.id);
        let decided = transition(before.status, before.quota_pause.as_ref().map(|p| p.phase), quota_stop, requested);
        let Transition::To { status, turn_end } = decided else {
            // A turn the harness ran on its own while quota paused keeps the status, but its end is saved like any.
            if before.status == SessionStatus::QuotaPaused {
                let mut turns = self.sessions.inner.paused_turns.lock().unwrap();
                if requested.is_running() {
                    turns.insert(self.id.clone());
                    return;
                }
                if !turns.remove(&self.id) { return; }
            } else { return; }
            crate::worktrees::record_heads(&self.app, &self.id).await;
            if let Some(mark) = crate::turn_sync::mark(&self.app)
                && self.sessions.update(&self.app, &self.id, |s| s.unsaved = Some(mark)).await.is_ok() {
                let (app, id) = (self.app.clone(), self.id.clone());
                tokio::spawn(async move { crate::turn_sync::save(&app, &id).await });
            }
            return;
        };
        let capacity_failure = if status == SessionStatus::Error && before.status != SessionStatus::Closed && !manual_stop && !budget_stop {
            let id = self.id.clone();
            self.app.state_db.call(move |c| {
                let json: Option<String> = c.query_row(
                    "SELECT json FROM session_event WHERE session_id = ?1 AND kind IN ('error', 'turn_end', 'status') ORDER BY seq DESC LIMIT 1",
                    [id], |r| r.get(0)).optional()?;
                Ok(json.and_then(|json| serde_json::from_str::<EventBody>(&json).ok())
                    .is_some_and(|body| matches!(body, EventBody::Error { message } | EventBody::TurnEnd { error: Some(message), .. } if is_capacity_error(&message))))
            }).await.unwrap_or(false)
        } else { false };
        // A turn end stores the branch tips first, then the new status with an `unsaved` mark: Home receives
        // both in that order and shows the turn as saving until `turn_sync` cleared the mark.
        let unsaved = if turn_end {
            crate::worktrees::record_heads(&self.app, &self.id).await;
            crate::turn_sync::mark(&self.app)
        } else { None };
        let mut applied = false;
        let _ = self
            .sessions
            .update(&self.app, &self.id, |s| {
                // Compare and set: an action that changed the record meanwhile (Archive, Resume, a quota pause)
                // without this lock wins, and this change does nothing.
                if s.status != before.status || transition(s.status, s.quota_pause.as_ref().map(|p| p.phase), quota_stop, requested) != decided { return; }
                applied = true;
                if unsaved.is_some() { s.unsaved = unsaved; }
                s.capacity_retry_at = if capacity_failure && s.capacity_retry_count < MAX_CAPACITY_RETRIES {
                    Some(now_ms() + CAPACITY_RETRY_DELAY_MS)
                } else { None };
                s.change_status(status);
                s.status_since = now_ms();
                if status.is_running() { s.ended_at = None; }
            })
            .await;
        if !applied { return; }
        self.sessions.inner.paused_turns.lock().unwrap().remove(&self.id);
        self.emit(EventBody::Status { status }).await;
        // Every turn end saves the session's branches, plain folders and conversation on Home; the first
        // turn end also renames the session once (an LLM call) beside it and never delays it.
        if turn_end {
            let (app, id) = (self.app.clone(), self.id.clone());
            let completed = self.sessions.get(&self.id);
            tokio::spawn(async move {
                tokio::join!(crate::turn_sync::save(&app, &id), async {
                    if matches!(status, SessionStatus::Waiting | SessionStatus::Error | SessionStatus::Interrupted | SessionStatus::Stopped)
                        && let Some(session) = completed
                    {
                        crate::titles::refresh(&app, &session).await;
                    }
                });
            });
        }
        // A turn that ends with `no_update` (a routine with nothing to report) never notifies; Home's routine
        // records decide whether it leaves Needs you (`crons::turn_ended`).
        let silent = status == SessionStatus::Waiting && self.sessions.get(&self.id).is_some_and(|s| crate::routines::is_no_update(s.last_message.as_deref()));
        self.sessions.publish_counts(&self.app);
        if let Some(event) = crate::notify::status_notification(before.status, status, before.open_question, manual_stop, budget_stop)
            && !silent
            && !self.sessions.is_watched(&self.id) {
            let s = self.sessions.get(&self.id).unwrap();
            crate::notify::session(&self.app, &s, event);
        }
    }

    pub async fn set_open_question(&self, open: bool) {
        if self.agent_id.is_some() { return; }
        let _ = self.sessions.update(&self.app, &self.id, |s| s.open_question = open).await;
    }

    pub async fn set_harness_id(&self, harness_id: &str) {
        if self.agent_id.is_some() { return; }
        let h = harness_id.to_string();
        let _ = self.sessions.update(&self.app, &self.id, |s| s.harness_session_id = Some(h)).await;
    }

    pub async fn set_model(&self, model: &str) {
        if let Some(id) = &self.agent_id {
            self.root().subagent(id, None, None, Some(model.into())).await;
            return;
        }
        let m = model.to_string();
        let _ = self.sessions.update(&self.app, &self.id, |s| {
            if s.model.is_none() {
                s.model = Some(m)
            }
        })
        .await;
    }

    pub async fn update_usage(&self, f: impl FnOnce(&mut Usage)) {
        if self.agent_id.is_some() { return; }
        let mut crossed = false;
        let mut unavailable = false;
        let mut added = 0.0;
        let mut profile = None;
        let _ = self.sessions.update(&self.app, &self.id, |s| {
            let before = s.budget_reached();
            let cost = s.usage.cost_usd.unwrap_or(0.0);
            let was_unavailable = s.usage.cost_unavailable;
            f(&mut s.usage);
            if !crate::profiles::supports_cost(&self.app, s.harness, &s.launch_spec, s.model.as_deref()) && crate::harness::registration(s.harness).descriptor.capabilities.cost {
                s.usage.cost_unavailable = true;
            }
            if s.usage.cost_unavailable { s.usage.cost_usd = None; }
            unavailable = !was_unavailable && s.usage.cost_unavailable;
            crossed = !before && s.budget_reached();
            added = s.usage.cost_usd.unwrap_or(0.0) - cost;
            profile = s.launch_spec.profile.clone();
        }).await;
        if unavailable {
            crate::spend::publish(&self.app);
            self.emit(EventBody::Notice { text: crate::harness::COST_UNAVAILABLE.into() }).await;
        }
        if let Some(v) = profile.filter(|_| added > 0.0) {
            let spec = self.sessions.get(&self.id).map(|s| s.launch_spec).unwrap_or_default();
            let before = crate::spend::reached(&self.app, &spec).len();
            crate::spend::record(&self.app, &v, added).await;
            let reached = crate::spend::reached(&self.app, &spec);
            if reached.len() > before {
                let name = self.app.config.get().profile(&v).map(|v| v.name.clone()).unwrap_or_default();
                self.emit(EventBody::Notice { text: format!("{} A running turn is stopped and no new turn starts.", crate::spend::message(&name, &reached[0])) }).await;
            }
        }
        if crossed && let Some(s) = self.sessions.get(&self.id) {
            self.emit(EventBody::Notice { text: format!(
                "Cost limit reached: ${:.2} reported of ${:.2}. A running turn is stopped and no new turn starts until you raise or remove the limit.",
                s.usage.cost_usd.unwrap_or(0.0), s.launch_spec.budget_usd.unwrap_or(0.0),
            ) }).await;
        }
    }

    pub async fn fail(&self, message: &str) {
        self.emit(EventBody::Error { message: message.to_string() }).await;
        self.set_status(SessionStatus::Error).await;
    }

    /// A terminal provider limit is a recoverable pause, even when the usage monitor
    /// still reports capacity (individual/organization spending limits are separate).
    pub async fn quota_rejected(&self, message: &str) {
        self.emit(EventBody::Notice { text: format!("Provider quota limit reached: {message}") }).await;
        if self.agent_id.is_some() {
            self.set_status(SessionStatus::Interrupted).await;
            return;
        }
        if self.instance != 0 && self.sessions.inner.live.lock().unwrap().get(&self.id).is_none_or(|(instance, _)| *instance != self.instance) { return; }
        if self.sessions.inner.quota_stops.lock().unwrap().contains(&self.id) { return; }
        if self.stop_requested() {
            self.set_status(SessionStatus::Stopped).await;
            return;
        }
        let _ = self.sessions.update(&self.app, &self.id, |s| {
            if let Some(pause) = s.quota_pause.as_mut().filter(|p| matches!(p.phase, QuotaPausePhase::Pausing | QuotaPausePhase::Paused | QuotaPausePhase::ProviderLimited)) {
                // A provider rejection can arrive after the quota guard interrupted
                // the turn and the owner already queued steering. Keep unsent input.
                pause.phase = QuotaPausePhase::ProviderLimited;
                pause.error = Some(message.to_string());
            } else {
                // This turn's input was delivered. Retry with a continuation, rather
                // than replaying a previously held prompt or its attachments.
                s.quota_pause = Some(QuotaPause {
                    phase: QuotaPausePhase::ProviderLimited,
                    blockers: crate::quota::blockers(&self.app, s.harness, &s.launch_spec),
                    message: "Continue the interrupted task from where you stopped; check the current state before repeating actions.".into(),
                    images: vec![], client_id: None, error: Some(message.to_string()), messages: vec![],
                });
            }
            s.capacity_retry_at = None;
        }).await;
        self.set_status(SessionStatus::QuotaPaused).await;
    }

    pub async fn exited(&self, error: Option<String>) {
        self.sessions.process_exited(&self.app, &self.id, self.instance, error).await;
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    /// The subscription login of the session (`profiles::account`), for quota readings its process reports.
    pub fn quota_account(&self) -> Option<String> {
        let s = self.sessions.get(&self.id)?;
        Some(crate::profiles::account(&self.app, s.harness, &s.launch_spec))
    }
}

/// The runner of a session that Home serves as a replica.
fn runner_of(app: &App, id: &str) -> Option<String> {
    (app.paths.role == crate::role::Role::Home).then(|| app.sessions.get(id).and_then(|s| s.runner_id)).flatten()
}

/// A notice in a session's history, wherever it runs: here, or on its runner (from Home).
pub async fn notice_anywhere(app: &App, id: &str, text: &str) {
    match runner_of(app, id) {
        Some(r) => {
            if let Err(e) = app.runners.request(&r, "POST", &format!("/api/sessions/{id}/notice"), Some(serde_json::json!({"text": text}).to_string())).await {
                tracing::warn!("notice for {id}: {e:#}");
            }
        }
        None => {
            if app.sessions.get(id).is_some() {
                app.sessions.sink(app, id).emit(EventBody::Notice { text: text.to_string() }).await;
            }
        }
    }
}

/// JSON helper used by adapters.
pub fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|i| i.get("text").and_then(|t| t.as_str()).map(str::to_string).unwrap_or_else(|| text_of(i)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Keeps the beginning and the end of a long tool output.
pub fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let head_end = s.floor_char_boundary(max * 2 / 3);
    let tail_start = s.ceil_char_boundary(s.len() - max / 3);
    format!("{}\n… {} bytes omitted …\n{}", &s[..head_end], tail_start - head_end, &s[tail_start..])
}

pub fn json_line(v: &Value) -> String {
    let mut s = v.to_string();
    s.push('\n');
    s
}

/// The start of an assistant message as one line of plain text, for list rows.
pub fn message_preview(text: &str) -> Option<String> {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['#', '>', '-', '*', '|', ' ']).replace("**", "").replace(['`', '|'], "");
        let line = line.trim();
        if line.is_empty() || line.chars().all(|c| c == '-' || c == ':') { continue; }
        if !out.is_empty() { out.push(' '); }
        out.push_str(line);
        if out.chars().count() > 160 { break; }
    }
    if out.is_empty() { return None; }
    Some(if out.chars().count() > 160 { format!("{}…", out.chars().take(160).collect::<String>().trim_end()) } else { out })
}

pub fn shorten(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(max).collect::<String>())
    }
}

/// Archive: stops the session, then cleans up after it in the background (clean worktrees become eligible for
/// removal, a temporary folder whose agents are all archived is deleted). Resume restores worktrees.
pub async fn archive(app: &App, id: &str) -> Result<()> {
    app.sessions.close(app, id).await?;
    after_close(app, id);
    Ok(())
}

/// `archive` only while the session is still Done since `since` with nothing running (`Sessions::close_if_done`).
pub async fn archive_if_done(app: &App, id: &str, since: i64) -> Result<bool> {
    if !app.sessions.close_if_done(app, id, since).await? {
        return Ok(false);
    }
    after_close(app, id);
    Ok(true)
}

/// Pure: the session is Done since `since` and nothing waits in it (no question, sleep, quota pause, capacity retry).
pub fn still_done(s: &Session, since: i64) -> bool {
    s.status == SessionStatus::Waiting && s.status_since == since && !s.open_question && s.sleep.is_none() && s.quota_pause.is_none() && s.capacity_retry_at.is_none()
}

fn after_close(app: &App, id: &str) {
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        crate::plain::after_archive(&app, &id).await;
        crate::worktrees::after_archive(&app, &id).await;
        crate::projects::after_archive(&app, &id).await;
    });
}

/// The first-prompt note of a session in a temporary folder.
pub fn temp_instruction(cli: &str, session_id: &str, workspace_id: &str, cwd: &str) -> String {
    format!(
        "Launchpad temporary folder: your working directory `{cwd}` is temporary. Launchpad saves its files on Home at the end of every turn (except files listed in a `.gitignore` and nested Git repositories: list large or generated files there), so they follow this session to another machine, and deletes them 30 days after the owner archives this session, unless they keep it as a folder. \
To work on a registered folder, find it with `{cli} folders list --workspace {workspace_id}`, then run `{cli} work add-project --session-id {session_id} --project <name>`: it prints a dedicated worktree folder of that Git folder, which you edit and commit there (Launchpad pushes its branch to Home at the end of each turn for the owner's review). Never edit the owner's checkouts."
    )
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn raw_lines_are_cut_on_a_character_boundary() {
        assert_eq!(raw_cut("abc", 5), "abc");
        assert_eq!(raw_cut("abcdef", 3), "abc");
        assert_eq!(raw_cut("aé", 2), "a");
    }

    #[test]
    fn startup_retires_only_unanswered_root_decisions() {
        let ask = |id: &str| EventBody::AskUser { request_id: id.into(), questions: vec![] };
        let events = vec![
            ask("answered"), ask("pending"),
            EventBody::AskAnswered { request_id: "answered".into(), answers: vec!["Allow once".into()] },
            EventBody::AskAnswered { request_id: "unknown".into(), answers: vec![] },
            EventBody::SubagentEvent { agent_id: "child".into(), body: Box::new(ask("child-only")) },
        ];
        assert_eq!(unanswered_requests(&events), vec!["pending"]);
        let mut recovered = events;
        recovered.push(EventBody::AskAnswered { request_id: "pending".into(), answers: vec!["Request ended".into()] });
        assert!(unanswered_requests(&recovered).is_empty());
    }
}


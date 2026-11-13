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

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode { Manual, FollowUpManual, FollowUpAutomatic }

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
                            seen_at, last_message, archived_done, capacity_retry_count, capacity_retry_at, runner_id, generation, unsaved_json, title_refreshed
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

    /// Stops an idle process before its folder moves or disappears; sending a message resumes it.
    /// The caller holds the session's action lock.
    pub(crate) async fn release(&self, app: &App, id: &str) -> Result<()> {
        self.ensure_shutdown_finished(id)?;
        if self.live(id).is_none() { return Ok(()); }
        self.detach(app, id).await?;
        self.update(app, id, |s| s.live = false).await?;
        Ok(())
    }

    fn ensure_shutdown_finished(&self, id: &str) -> Result<()> {
        anyhow::ensure!(!self.inner.shutdowns.lock().unwrap().contains_key(id), "The previous process is still stopping. Wait for its exit.");
        Ok(())
    }

    /// Keep ownership in a background task when a process cannot acknowledge shutdown promptly.
    pub(crate) async fn wait_for_shutdown<F>(&self, id: &str, wait: F) -> Result<()>
    where F: std::future::Future<Output = Result<()>> + Send + 'static {
        self.wait_for_shutdown_timeout(id, wait, std::time::Duration::from_secs(5)).await
    }

    async fn wait_for_shutdown_timeout<F>(&self, id: &str, wait: F, timeout: std::time::Duration) -> Result<()>
    where F: std::future::Future<Output = Result<()>> + Send + 'static {
        *self.inner.shutdowns.lock().unwrap().entry(id.into()).or_default() += 1;
        let (this, id) = (self.clone(), id.to_string());
        let task = tokio::spawn(async move {
            let result = wait.await;
            let mut shutdowns = this.inner.shutdowns.lock().unwrap();
            if let Some(count) = shutdowns.get_mut(&id) {
                *count -= 1;
                if *count == 0 { shutdowns.remove(&id); }
            }
            result
        });
        tokio::time::timeout(timeout, task).await
            .context("The process is still stopping in the background; its folder remains busy until exit is confirmed")??
    }

    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<Session> = self.inner.records.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        v
    }

    /// Profiles whose ledger cannot represent all of their usage reliably.
    pub(crate) fn profiles_without_cost(&self) -> std::collections::HashSet<String> {
        self.inner.records.lock().unwrap().values().filter(|s| s.usage.cost_unavailable)
            .filter_map(|s| s.launch_spec.profile.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.inner.records.lock().unwrap().get(id).cloned()
    }

    fn live(&self, id: &str) -> Option<Arc<dyn LiveSession>> {
        self.inner.live.lock().unwrap().get(id).map(|(_, l)| l.clone())
    }

    pub fn find_by_harness_id(&self, harness_id: &str) -> Option<Session> {
        self.inner.records.lock().unwrap().values().find(|s| s.harness_session_id.as_deref() == Some(harness_id)).cloned()
    }

    pub async fn rename(&self, app: &App, id: &str, title: &str) -> Result<Session> {
        self.ensure_local(id)?;
        let title = title.trim();
        anyhow::ensure!(!title.is_empty() && title.chars().count() <= 100 && !title.contains(['\n', '\r']), "Use a title of 1 to 100 characters on one line.");
        self.update(app, id, |s| { s.title = title.to_string(); s.title_pending = false; }).await
    }

    pub(crate) fn title_request_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner.title_requests.get(id)
    }

    /// Each window reports which sessions it shows, whether it has the focus, and how long ago it last
    /// received user input (`idle_ms`, relative so device clocks do not matter). `None` when it closes.
    pub fn set_watched(&self, window: u64, presence: Option<(bool, Vec<String>, Option<i64>)>) {
        let mut w = self.inner.watched.lock().unwrap();
        let now = now_ms();
        match presence {
            Some((focused, sessions, idle_ms)) => w.insert(window, (focused, sessions, now, idle_ms.map(|idle| now - idle.max(0)))),
            None => w.remove(&window),
        };
    }

    /// Shown in a focused window that received user input in the last minute (the same rule as "seen" in the UI).
    pub(crate) fn is_watched(&self, id: &str) -> bool {
        let w = self.inner.watched.lock().unwrap();
        let now = now_ms();
        w.values().any(|(focused, sessions, at, input)| {
            *focused && now - at < 10 * 60_000 && input.is_some_and(|i| now - i < WATCH_INPUT_MS) && sessions.iter().any(|s| s == id)
        })
    }

    fn ensure_local(&self, id: &str) -> Result<()> {
        self.get(id).context("no such session")?;
        Ok(())
    }

    fn persist(db: &crate::db::Db, s: &Session) -> impl std::future::Future<Output = Result<()>> + use<> {
        let s = s.clone();
        db.enqueue(move |c| Self::store(c, &s))
    }

    pub(crate) fn store(c: &rusqlite::Connection, s: &Session) -> Result<()> {
        c.execute(
            "INSERT INTO session (id, harness, harness_session_id, project_id, cwd, title, status, status_since,
                open_question, model, effort, launch_spec_json, prompt, cost_usd, usage_json, started_at, ended_at, last_seq, quota_pause_json, title_pending, seen_at, last_message, archived_done, capacity_retry_count, capacity_retry_at, runner_id, generation, unsaved_json, title_refreshed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
             ON CONFLICT (id) DO UPDATE SET harness_session_id = ?3, project_id = ?4, cwd = ?5, title = ?6, status = ?7,
                status_since = ?8, open_question = ?9, model = ?10, effort = ?11, cost_usd = ?14, usage_json = ?15,
                ended_at = ?17, launch_spec_json = ?12, quota_pause_json = ?19, title_pending = ?20,
                seen_at = ?21, archived_done = ?23, capacity_retry_count = ?24, capacity_retry_at = ?25,
                runner_id = ?26, generation = ?27, unsaved_json = ?28, title_refreshed = ?29",
            params![
                s.id,
                s.harness.as_str(),
                s.harness_session_id,
                s.project_id,
                s.cwd,
                s.title,
                s.status.as_str(),
                s.status_since,
                s.open_question as i64,
                s.model,
                s.effort,
                serde_json::to_string(&s.launch_spec)?,
                s.prompt,
                s.usage.cost_usd,
                serde_json::to_string(&s.usage)?,
                s.started_at,
                s.ended_at,
                s.last_seq,
                s.quota_pause.as_ref().map(serde_json::to_string).transpose()?,
                s.title_pending,
                s.seen_at,
                s.last_message,
                s.archived_done,
                s.capacity_retry_count,
                s.capacity_retry_at,
                s.runner_id,
                s.generation,
                s.unsaved.as_ref().map(serde_json::to_string).transpose()?,
                s.title_refreshed,
            ],
        )?;
        crate::replica::queue(c, &crate::replica::Item::Session { session: Box::new(s.without_bytes()) })?;
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
                // Only sessions Home placed on this runner are its records (`runners::record_placement`, a move):
                // a record of an unknown session or of another runner's is ignored.
                let stale = current.as_ref().is_none_or(|c| c.runner_id.as_deref() != Some(runner_id) || c.generation > s.generation);
                if !stale {
                    s.runner_id = Some(runner.clone());
                    // Either side may have shown it to the owner.
                    if let Some(c) = &current {
                        s.seen_at = s.seen_at.max(c.seen_at);
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
                    }
                }
                let stored = s.clone();
                app.state_db.call(move |c| {
                    let tx = c.transaction()?;
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
                        recs.insert(s.id.clone(), s.clone());
                        newer.map(|_| Self::persist(&app.state_db, &s))
                    };
                    if let Some(stored) = again {
                        stored.await?;
                    }
                    app.bus.publish("sessions", "updated", serde_json::to_value(s.light())?);
                    self.publish_counts(app);
                    let ended = current.as_ref().is_some_and(|c| c.status.is_running() && !s.status.is_running());
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
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.publish_counts(app);
        app.bus.publish("sessions", "changed", Value::Null);
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
        self.resolve(app, spec, false).await
    }

    /// At launch, worktree folders are not known yet: the launch appends their exact instructions.
    async fn resolve(&self, app: &App, spec: &LaunchSpec, for_launch: bool) -> Result<Preview> {

        crate::attachments::validate(&spec.attachments)?;
        let (cwd, project) = match &spec.target {
            Target::Project { project_id } | Target::Predraft { project_id } => {
                let p = crate::projects::resolve(app, project_id).await?;
                (Some(p.path.clone()), Some(p))
            }
            Target::ExistingPredraft { predraft_id } => {
                let p = crate::predrafts::get(app, predraft_id).await?;
                anyhow::ensure!(p.agents_can_work(), "this Local PR is archived or merged");
                let project = crate::projects::resolve(app, &p.project_id).await?;
                (Some(project.path.clone()), Some(project))
            }
            Target::SessionFolder { session_id } => {
                let source = self.get(session_id).context("source session no longer exists")?;
                let project = match &source.project_id {
                    Some(id) => Some(crate::projects::get(app, id).await?.context("the project no longer exists")?),
                    None => None,
                };
                (Some(source.cwd), project)
            }
            Target::Temp => (None, None),
            Target::None => (None, None),
        };
        let cli = app.paths.cli_command();
        let config = app.config.get();
        let mut context = Vec::new();
        let mut add_dirs = Vec::new();
        let project_target = matches!(spec.target, Target::Project { .. });
        // Git folders of Home always run in fresh worktrees: there is no checkout to share.
        let home_git = project.as_ref().is_some_and(|p| p.home_git);
        let in_checkout = project_target && spec.worktree.is_none() && !home_git;
        let worktree_launch = project_target && (spec.worktree.is_some() || home_git);
        for cp in &spec.context_projects {
            let p = crate::projects::resolve(app, &cp.project_id).await.context("Context folder unavailable; remove it and select it again")?;
            add_dirs.push(p.path.clone());
            context.push(ContextInfo { path: p.path, description: p.description, branch: cp.branch.clone(), editable: cp.editable, own_worktree: cp.editable && !in_checkout });
        }
        crate::prompt_context::validate_mentions(&spec.text, &spec.mentions)?;
        // Where each labelled part of the prompt starts ("What it sees at start").
        let mut marks: Vec<(&str, &str, usize)> = vec![("message", "Your message, its context and presets", 0)];
        let mut prompt = render_prompt(spec, &context, &launch_presets(app, spec, &config).await?);
        marks.push(("skills", "Skills", prompt.len()));
        if !spec.skills.is_empty() {
            let harness = spec.harness.context("Pick a harness for selected skills")?;
            prompt.push_str(&crate::prompt_context::render_skills(app, harness, spec, cwd.as_deref()).await?);
        }
        let traces = self.context_traces(app, &spec.agent_traces).await?;
        marks.push(("traces", "Agent traces", prompt.len()));
        if !traces.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&crate::compose::trace_references(&traces, &cli));
        }
        marks.push(("folder", "Folder and worktrees", prompt.len()));
        if !for_launch && !prompt.trim().is_empty() && matches!(spec.target, Target::Predraft { .. } | Target::ExistingPredraft { .. }) {
            prompt.push_str("\n\nWork on the Local PR's branch in a fresh worktree. Commit your changes, update the Local PR title and description with launchpad pr update. Do not open a pull request on GitHub.");
        }
        if !for_launch && !prompt.trim().is_empty() && (worktree_launch || context.iter().any(|c| c.own_worktree)) {
            let base = match spec.worktree.as_ref() {
                Some(WorktreeOptions { pull_request: Some(pr), .. }) => format!("the head of pull request {}#{}", pr.repo, pr.number),
                w => {
                    let from = w.and_then(|w| w.base.clone()).map(|b| format!("`{b}`")).unwrap_or_else(|| "the default branch".into());
                    match w.and_then(|w| w.into.clone()).filter(|i| w.and_then(|w| w.base.as_ref()) != Some(i)) {
                        Some(into) => format!("{from}, for `{into}`"),
                        None => from,
                    }
                }
            };
            prompt.push_str(&format!("\n\nLaunchpad worktree: Launchpad creates dedicated Git worktrees for this task at launch{}, and lists their exact folders and branches here.", if worktree_launch { format!(", starting from {base}") } else { String::new() }));
        }
        marks.push(("routine", "Routine", prompt.len()));
        if let Some(r) = &spec.routine {
            prompt.push_str("\n\n");
            prompt.push_str(&crate::routines::instruction(r));
        }
        marks.push(("bot", "The bot and its BOT.md", prompt.len()));
        if let Some(bot) = &spec.bot {
            prompt.push_str("\n\n");
            prompt.push_str(&crate::bots::instruction(bot, &cli));
        }
        marks.push(("final", "Final message", prompt.len()));
        crate::compose::append_initial_summary_instruction(&mut prompt);
        let parts = crate::start::slices(&prompt, &marks);
        Ok(Preview {
            title: title_from(&spec.text, project.as_ref().map(|p| p.name.as_str()).unwrap_or("New session")),
            prompt,
            parts,
            cwd,
            add_dirs,
        })
    }

    pub async fn launch(&self, app: &App, spec: LaunchSpec) -> Result<Session> {
        self.launch_with_id(app, spec, uuid::Uuid::new_v4().to_string()).await
    }

    pub async fn launch_with_id(&self, app: &App, spec: LaunchSpec, id: String) -> Result<Session> {
        self.launch_inner(app, spec, id, false).await
    }

    async fn launch_inner(&self, app: &App, spec: LaunchSpec, id: String, automatic: bool) -> Result<Session> {
        crate::role::ensure_runs_agents(app.paths.role)?;
        // Home sends references: their bytes come from Home before the checks read them.
        crate::attachments::fetch_missing(app, &id, &spec.attachments).await?;
        anyhow::ensure!(app.paths.role != crate::role::Role::Runner || !crate::home_link::guard(&app.paths).paused, "This runner takes no new sessions: switch Takes new sessions back on in its own window");
        // This session's lock (a second launch with the same ID waits, then finds the record), and the lock of the
        // session whose folder it continues in, so that one cannot start a turn meanwhile.
        let mut locked = vec![id.clone()];
        if let Target::SessionFolder { session_id } = &spec.target { locked.push(session_id.clone()); }
        let _action = self.action_locks(&locked).await;
        anyhow::ensure!(self.get(&id).is_none(), "a session with this id already exists");
        let mut spec = spec;
        // A pull request on GitHub needs the owner's own launch: others open a Local PR the owner publishes.
        let when_done = spec.when_done();
        if let Some(w) = spec.worktree.as_mut() { w.when_done = when_done; }
        if let Target::SessionFolder { session_id } = &spec.target {
            let source = self.get(session_id).context("source session no longer exists")?;
            anyhow::ensure!(!self.is_busy(&source) && source.status != SessionStatus::InTerminal, "stop the source agent or re-attach its terminal before continuing with another agent");
        }
        let harness = spec.harness.context("pick a harness")?;
        if spec.target == Target::None {
            bail!("pick a project or a temporary folder");
        }
        crate::attachments::validate(&spec.attachments)?;
        anyhow::ensure!(spec.attachments.iter().all(|f| !crate::attachments::is_image(&f.data_url)) || crate::harness::registration(harness).descriptor.capabilities.images, "This harness does not support images.");
        let profile = crate::profiles::resolve(app, harness, &spec)?;
        if crate::harness::registration(harness).descriptor.capabilities.cost && !crate::profiles::supports_cost(app, harness, &spec, spec.model.as_deref()) {
            spec.budget_usd = None;
        }
        check_budget(harness, spec.budget_usd)?;
        crate::spend::ensure(app, &spec)?;
        let (model, effort) = (
            spec.model.clone().or_else(|| profile.as_ref().and_then(|v| v.model.clone())),
            spec.effort.clone().or_else(|| profile.as_ref().and_then(|v| v.effort.clone())),
        );
        let mut preview = self.resolve(app, &spec, true).await?;
        let env = crate::profiles::env(app, harness, &spec, model.as_deref())?;
        if preview.prompt.trim().is_empty() {
            bail!("the prompt is empty");
        }
        let mut worktrees: Vec<(crate::worktrees::TaskWorktree, String)> = vec![];
        let mut setups: Vec<(Project, String)> = vec![];
        // Worktrees of this launch: cloned from Home in the background, before the agent starts.
        let mut created: Vec<String> = vec![];
        // The Local PR this session works on (an existing one), or opens once its branch exists.
        let mut local_pr: Option<(Option<crate::predrafts::Predraft>, String)> = None;
        // Read-only context folders of Home: their clones are refreshed (plain ones brought when missing) before the agent starts.
        let mut views: Vec<Project> = vec![];
        // A runner starts only launches Home placed on it (`runners::record_placement`, before Home relayed the
        // launch): Home gives leases only to sessions it placed (`vault::check_owner`). A failed launch below
        // sends `Forget`, which removes Home's record.
        let prepared = async {
            let (mut cwd, project_id) = match &spec.target {
                Target::Temp => {
                    let workspace = spec.workspace_id.as_deref().context("a temporary folder needs a workspace: launch it from a workspace")?;
                    let p = crate::projects::create_temp(app, &spec.text, workspace).await?;
                    preview.prompt.push_str(&format!("\n\n{}", temp_instruction(&app.paths.cli_command(), &id, workspace, &p.path)));
                    (p.path, Some(p.id))
                }
                Target::Project { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
                Target::Predraft { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
                Target::ExistingPredraft { predraft_id } => {
                    let p = crate::predrafts::get(app, predraft_id).await?;
                    anyhow::ensure!(p.agents_can_work(), "this Local PR is archived or merged");
                    anyhow::ensure!(
                        !p.session_id.as_ref().and_then(|s| self.get(s)).is_some_and(|s| self.is_busy(&s) || s.status == SessionStatus::InTerminal),
                        "the Local PR already has an active agent"
                    );
                    let project = crate::projects::get(app, &p.project_id).await?.context("folder not found")?;
                    let opts = WorktreeOptions { base: Some(p.base_branch.clone()), ..Default::default() };
                    let (w, folder) = crate::worktrees::plan(app, &id, &project, &opts, true, &preview.title, Some(&p.branch)).await?;
                    created.push(w.id.clone());
                    preview.prompt.push_str(&format!("\n\n{}", crate::predrafts::instruction(app, &p)));
                    worktrees.push((w, folder.clone()));
                    setups.push((project, folder.clone()));
                    let project_id = p.project_id.clone();
                    local_pr = Some((Some(p), String::new()));
                    (folder, Some(project_id))
                }
                Target::SessionFolder { session_id } => {
                    let source = self.get(session_id).context("source session no longer exists")?;
                    let into = spec.worktree.as_ref().and_then(|w| w.into.clone()).filter(|b| !b.is_empty());
                    let joined = crate::worktrees::join(app, session_id, &id, into.as_deref()).await?;
                    // Removed folders are cloned again in the background.
                    created.extend(joined.iter().filter(|w| w.state != crate::worktrees::WorktreeState::Active).map(|w| w.id.clone()));
                    worktrees.extend(joined.into_iter().filter(|w| w.state != crate::worktrees::WorktreeState::Discarded).map(|w| {
                        let path = crate::worktrees::folder(&w);
                        (w, path)
                    }));
                    (preview.cwd.clone().context("source session folder missing")?, source.project_id)
                }
                Target::None => unreachable!(),
            };
            // Task worktrees for the project and the editable context projects, before any agent starts.
            let project_target = match &spec.target {
                Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.as_str()),
                _ => None,
            };
            let target_project = match project_target { Some(id) => crate::projects::get(app, id).await?, None => None };
            // Git folders of Home always run in a fresh worktree.
            let opts = match (&target_project, &spec.worktree) {
                (Some(p), None) if p.home_git => Some(WorktreeOptions::default()),
                (_, w) => w.clone(),
            }.map(|mut o| {
                if matches!(spec.target, Target::Predraft { .. }) { o.when_done = crate::model::WhenDone::LocalPr; }
                o
            });
            let in_checkout = project_target.is_some() && opts.is_none();
            if let (Some(project), Some(opts)) = (target_project, &opts) {
                let (w, folder) = crate::worktrees::plan(app, &id, &project, opts, true, &preview.title, None).await?;
                created.push(w.id.clone());
                cwd = folder.clone();
                worktrees.push((w, folder.clone()));
                setups.push((project, folder));
                if matches!(spec.target, Target::Predraft { .. }) {
                    local_pr = Some((None, preview.title.clone()));
                }
            }
            for cp in &spec.context_projects {
                if cp.editable && !in_checkout { continue }
                if let Some(p) = crate::projects::get(app, &cp.project_id).await?.filter(|p| p.home_git || p.plain) { views.push(p); }
            }
            for cp in spec.context_projects.iter().filter(|c| c.editable && !in_checkout) {
                let Some(project) = crate::projects::get(app, &cp.project_id).await? else { continue };
                let opts = WorktreeOptions { base: cp.branch.clone(), ..Default::default() };
                let (w, folder) = crate::worktrees::plan(app, &id, &project, &opts, false, &preview.title, None).await?;
                if !created.contains(&w.id) { created.push(w.id.clone()); }
                if let Some(d) = preview.add_dirs.iter_mut().find(|d| **d == project.path) { *d = folder.clone(); }
                if !worktrees.iter().any(|(x, _)| x.id == w.id) { setups.push((project, folder.clone())); }
                worktrees.push((w, folder));
            }
            if !worktrees.is_empty() {
                let projects = crate::projects::list(app).await?;
                let when_done = if in_checkout || local_pr.as_ref().is_some_and(|(p, _)| p.is_some()) { crate::model::WhenDone::Keep } else { opts.clone().unwrap_or_default().when_done };
                preview.prompt.push_str(&format!("\n\n{}", crate::worktrees::instruction(app, &id, &worktrees, &projects, when_done)));
            }
            let (images, attachment_dir, instruction) = crate::attachments::prepare(app, &id, &spec.attachments).await?;
            if let Some(dir) = attachment_dir { preview.add_dirs.push(dir.to_string_lossy().into_owned()); }
            preview.prompt.push_str(&instruction);
            if let Some(text) = crate::ext::extensions().suggestion_instruction(app, &id) { preview.prompt.push_str(&format!("\n\n{text}")); }
            let now = now_ms();
            let session = Session {
                runner_id: None,
                generation: 1,
                id: id.clone(),
                harness,
                harness_session_id: None,
                project_id,
                cwd: cwd.clone(),
                title: preview.title.clone(),
                title_pending: true,
                title_refreshed: false,
                status: SessionStatus::Starting,
                archived_done: false,
                status_since: now,
                live: false,
                open_question: false,
                model: model.clone(),
                effort: effort.clone(),
                prompt: preview.prompt.clone(),
                usage: Usage::default(),
                started_at: now,
                ended_at: None,
                last_seq: 0,
                launch_spec: spec.clone(),
                quota_pause: None,
                capacity_retry_count: 0,
                capacity_retry_at: None,
                seen_at: 0,
                last_message: None,
                unsaved: None,
            };
            Self::persist(&app.state_db, &session).await?;
            let add_dirs = preview.add_dirs;
            let start = StartSpec {
                permission_mode: spec.permission_mode,
                session_id: id.clone(),
                harness_session_id: None,
                resume: false,
                cwd: PathBuf::from(&cwd),
                title: preview.title,
                model,
                effort,
                add_dirs,
                env,
                jail: None,
            };
            app.bus.publish("sessions", "updated", serde_json::to_value(session.light())?);
            self.inner.records.lock().unwrap().insert(id.clone(), session);
            self.publish_counts(app);
            Ok::<_, anyhow::Error>((start, images, preview.prompt))
        }.await;
        let (start, images, prompt) = match prepared {
            Ok(result) => result,
            Err(error) => {
                self.inner.records.lock().unwrap().remove(&id);
                let failed_id = id.clone();
                let _ = app.state_db.call(move |c| {
                    c.execute("DELETE FROM session WHERE id = ?1", [&failed_id])?;
                    crate::replica::queue(c, &crate::replica::Item::Forget { id: failed_id })
                }).await;
                let mut created = Vec::new();
                for (w, _) in worktrees { if w.sessions == [id.clone()] && !created.iter().any(|c: &crate::worktrees::TaskWorktree| c.id == w.id) { created.push(w); } }
                crate::worktrees::abandon(app, &created).await;
                if let Err(e) = crate::worktrees::leave(app, &id).await {
                    tracing::warn!("could not unlink failed launch worktrees: {e:#}");
                }
                return Err(error);
            }
        };
        setups.retain(|(p, _)| !p.worktree_setup.trim().is_empty());
        // The remote sync, worktree setup and harness start can take minutes (network, slow file
        // systems, setup commands): they run in the background so the launch request returns now.
        let (this, app2, id2) = (self.clone(), app.clone(), id.clone());
        let (ready, wait) = tokio::sync::oneshot::channel::<()>();
        let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
        let (stopped, stop_wait) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = wait.await;
            let sink = this.sink(&app2, &id2);
            let mut prompt = prompt;
            let mut prepared = true;
            for wid in &created {
                if *cancelled.borrow() { break; }
                let result = match crate::worktrees::get(&app2, wid).await {
                    Ok(w) if w.state == crate::worktrees::WorktreeState::Removed => crate::worktrees::restore(&app2, wid).await,
                    _ => crate::worktrees::prepare(&app2, wid).await,
                };
                if let Err(e) = result {
                    sink.emit(EventBody::Notice { text: format!("Could not prepare the worktree from Home Git: {e:#}") }).await;
                    prepared = false;
                    break;
                }
            }
            // Plain folders and asset folders come down from Home; the session takes their locks.
            if prepared && !*cancelled.borrow() && let Err(e) = crate::plain::prepare(&app2, &id2).await {
                sink.emit(EventBody::Notice { text: format!("Could not prepare the folder from Home: {e:#}") }).await;
                prepared = false;
            }
            for p in &views {
                if *cancelled.borrow() || !prepared { break; }
                let synced = if p.plain { crate::plain::reference(&app2, &p.id, std::path::Path::new(&p.path)).await } else { crate::repo_cache::sync(&app2, p).await.map(drop) };
                if let Err(e) = synced {
                    sink.emit(EventBody::Notice { text: format!("Could not refresh the context folder {}: {e:#}", p.name) }).await;
                }
            }
            if prepared && let Some((existing, title)) = &local_pr && let Some(w) = created.first() && let Ok(w) = crate::worktrees::get(&app2, w).await {
                let opened = match existing {
                    Some(p) => crate::predrafts::open_for_branch(&app2, &p.project_id, &p.branch, Some(&p.base_branch), Some(&id2), None).await,
                    None => crate::predrafts::open_for_branch(&app2, &w.project_id, &w.branch, w.base_branch.as_deref(), Some(&id2), Some(title.clone())).await,
                };
                match opened {
                    Ok(p) => { let _ = crate::worktrees::mark_predraft(&app2, &w.id, &p.id).await; }
                    Err(e) => sink.emit(EventBody::Notice { text: format!("Could not open the Local PR on Home: {e:#}") }).await,
                }
            }
            if !prepared {
                if this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) { sink.set_status(SessionStatus::Error).await; }
                let _ = stopped.send(());
                this.inner.setups.lock().unwrap().remove(&id2);
                return;
            }
            for (project, folder) in &setups {
                if *cancelled.borrow() { break; }
                if let Err(e) = crate::worktrees::run_setup(&app2, project, folder, &sink, &mut cancelled).await {
                    if *cancelled.borrow() { break; }
                    sink.emit(EventBody::Notice { text: format!("Worktree setup failed: {e:#}") }).await;
                    if this.ensure_shutdown_finished(&id2).is_err() { break; }
                    prompt.push_str(&format!("\n\nNote: the worktree setup command of {} (`{}`) failed in `{folder}`:\n{e:#}\nFix the environment yourself if the task needs it.", project.name, project.worktree_setup.trim()));
                }
            }
            // The contained start (lease from Home, tool installs, knowledge base sync) can take minutes: it runs
            // before the session's action lock, while Stop can still cancel the launch.
            let contained = if !*cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) {
                Some(crate::contain::apply(&app2, start).await)
            } else {
                None
            };
            if *cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) {
                sink.set_status(SessionStatus::Stopped).await;
            }
            // Acknowledge before taking actions: Stop/Archive may still be waiting there.
            let _ = stopped.send(());
            let _action = this.action_lock(&id2).await;
            this.inner.setups.lock().unwrap().remove(&id2);
            let start = match contained {
                Some(start) if !*cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) => start,
                // Cancelled meanwhile: the lease this start took goes (no process holds it).
                Some(Ok(_)) if this.live(&id2).is_none() => return crate::contain::release(&app2, &id2).await,
                _ => return,
            };
            if let Err(e) = this.ensure_shutdown_finished(&id2) {
                if start.is_ok() && this.live(&id2).is_none() { crate::contain::release(&app2, &id2).await; }
                sink.emit(EventBody::Notice { text: e.to_string() }).await;
                sink.set_status(SessionStatus::Stopped).await;
                return;
            }
            this.start_new(&app2, &id2, start, (prompt, images), automatic).await;
        });
        self.inner.setups.lock().unwrap().insert(id.clone(), SetupTask { cancel, stopped: stop_wait });
        let _ = ready.send(());
        Ok(self.get(&id).unwrap())
    }

    /// Starts the harness of a new session and sends its first prompt, or holds it at exhausted quota. `start`: the
    /// contained start `contain::apply` prepared before the caller took the session's action lock.
    async fn start_new(&self, app: &App, id: &str, start: Result<StartSpec>, input: (String, Vec<ImageAttachment>), automatic: bool) {
        let (prompt, images) = input;
        let Some(session) = self.get(id) else { return };
        let harness = session.harness;
        let sink = self.sink(app, id);
        let protected = crate::quota::protected(app, harness, &session.launch_spec);
        if protected && (!crate::quota::blockers(app, harness, &session.launch_spec).is_empty() || (automatic && !crate::quota::available(&crate::quota::reading_for(app, harness, &session.launch_spec), now_ms()))) {
            // No process holds the lease until the quota allows the start (`continue_quota` takes it again).
            if start.is_ok() && self.live(id).is_none() { crate::contain::release(app, id).await; }
            if let Err(e) = self.hold_input(app, id, &prompt, images, session.launch_spec.from.clone()).await {
                sink.fail(&format!("{e:#}")).await;
            }
            return;
        }
        if protected && !crate::quota::available(&crate::quota::reading_for(app, harness, &session.launch_spec), now_ms()) {
            sink.emit(EventBody::Notice { text: "Quota reading unavailable or stale. Starting with best-effort protection; additional spending cannot be ruled out until quota is known.".into() }).await;
        }
        let attached = match start {
            Ok(start) => self.attach_contained(app, harness, start, sink.clone()).await,
            Err(e) => Err(e),
        };
        match attached {
            Ok(live) => {
                let client_id = uuid::Uuid::new_v4().to_string();
                sink.emit(EventBody::UserMessage { text: prompt.clone(), client_id: client_id.clone(), queued: false, images: images.clone(), from: session.launch_spec.from.clone() }).await;
                sink.set_status(SessionStatus::Working).await;
                let sent = match crate::attachments::for_harness(app, id, &images).await {
                    Ok(images) => live.send(&prompt, &client_id, &images).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = sent {
                    sink.fail(&format!("{e:#}")).await;
                }
            }
            Err(e) => {
                sink.fail(&format!("could not start {}: {e:#}", harness.display())).await
            }
        }
        if let Some(session) = self.get(id).filter(|s| s.status != SessionStatus::Error) { crate::titles::generate(app, &session); }
    }

    async fn attach(&self, app: &App, harness: HarnessKind, start: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let start = crate::contain::apply(app, start).await?;
        self.attach_contained(app, harness, start, sink).await
    }

    /// Starts the harness with a start `contain::apply` prepared. The caller holds the session's action lock.
    async fn attach_contained(&self, app: &App, harness: HarnessKind, start: StartSpec, mut sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let id = start.session_id.clone();
        let instance = self.inner.next_instance.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sink.instance = instance;
        // A runner replacing an agent CLI holds it for writing: no process starts from a half-installed CLI.
        let cli = crate::harness::CLI_INSTALL.read().await;
        let started = app.harnesses.get(harness).start(app, start, sink.clone()).await;
        drop(cli);
        let live = match started {
            Ok(live) => live,
            Err(e) => {
                // A harness that did not start holds no lease.
                if self.live(&id).is_none() {
                    crate::contain::release(app, &id).await;
                }
                return Err(e);
            }
        };
        self.inner
            .live
            .lock()
            .unwrap()
            .insert(id.clone(), (instance, live.clone()));
        self.inner.process_restarts.lock().unwrap().remove(&id);
        self.update(app, &id, |s| s.live = true).await?;
        Ok(live)
    }

    /// Starts a process for a session that has none (resume).
    /// An idle process waiting for model, effort or permission changes is replaced.
    async fn reattach(&self, app: &App, id: &str) -> Result<Arc<dyn LiveSession>> {
        self.ensure_local(id)?;
        self.ensure_shutdown_finished(id)?;
        let s = self.get(id).context("no such session")?;
        if let Some(l) = self.live(id) {
            if !self.inner.process_restarts.lock().unwrap().contains(id) || s.status.is_running() || s.open_question || !l.ready_for_turn() {
                return Ok(l);
            }
            self.detach(app, id).await?;
            // A failed replacement (for example unsupported Auto review) must not leave a
            // session claiming to have a live attachment.
            self.update(app, id, |s| s.live = false).await?;
        }
        if s.launch_spec.harness.is_none() && crate::terminals::tui_running(app, &s) {
            bail!("This imported conversation is still running in another client. Finish it there first.");
        }
        // A temporary folder removed at Archive (or after a move) comes back with its files from Home; Home
        // deletes them 30 days after Archive, and the agent then continues in an empty folder.
        let had_folder = crate::files::is_dir(std::path::Path::new(&s.cwd)).await;
        let restored = match crate::projects::ensure_temp(app, &s).await? {
            Some(p) => {
                if p.path != s.cwd {
                    self.update(app, id, |s| s.cwd = p.path.clone()).await?;
                }
                !had_folder
            }
            None => {
                anyhow::ensure!(!matches!(s.launch_spec.target, Target::Temp) || had_folder, "Its temporary folder is gone: start a new agent");
                false
            }
        };
        // A clean worktree removed by cleanup comes back from its branch before the agent resumes.
        crate::worktrees::restore_for_session(app, id).await?;
        crate::plain::prepare(app, id).await?;
        if restored {
            let s = self.get(id).context("no such session")?;
            if crate::plain::walk(std::path::Path::new(&s.cwd)).is_empty() {
                self.sink(app, id).emit(EventBody::Notice { text: "Its temporary folder is empty: Home had no files of it (they are deleted 30 days after Archive).".into() }).await;
            }
        }
        let s = self.get(id).context("no such session")?;
        let worktrees = crate::worktrees::for_session(app, id).await?;
        let mut add_dirs = Vec::new();
        let attachment_dir = crate::attachments::dir(app, &s.id);
        if crate::files::is_dir(&attachment_dir).await { add_dirs.push(attachment_dir.to_string_lossy().into_owned()); }
        for cp in &s.launch_spec.context_projects {
            if let Ok(p) = crate::projects::resolve(app, &cp.project_id).await {
                // Editable context projects resolve to their recorded worktree, never to today's checkout.
                let in_checkout = matches!(s.launch_spec.target, Target::Project { .. }) && s.launch_spec.worktree.is_none() && !worktrees.iter().any(|w| w.primary);
                if cp.editable && !in_checkout {
                    add_dirs.push(crate::worktrees::context_folder(app, &worktrees, &p).await?);
                } else {
                    // A session that moved here has no copy of its read-only context folders yet.
                    if p.plain {
                        crate::plain::reference(app, &p.id, std::path::Path::new(&p.path)).await?;
                    } else if p.home_git && !crate::files::is_dir(&p.path).await {
                        crate::repo_cache::sync(app, &p).await?;
                    }
                    add_dirs.push(p.path);
                }
            }
        }
        for w in worktrees.iter().filter(|w| !w.primary && w.state == crate::worktrees::WorktreeState::Active) {
            let dir = crate::worktrees::folder(w);
            if !add_dirs.contains(&dir) { add_dirs.push(dir); }
        }
        if !crate::files::is_dir(&s.cwd).await {
            bail!("the folder {} no longer exists", s.cwd);
        }
        let cwd = PathBuf::from(&s.cwd);
        let env = crate::profiles::env(app, s.harness, &s.launch_spec, s.model.as_deref())?;
        let start = StartSpec {
            permission_mode: s.launch_spec.permission_mode,
            session_id: s.id.clone(),
            harness_session_id: s.harness_session_id.clone(),
            resume: s.harness_session_id.is_some(),
            cwd,
            title: s.title.clone(),
            model: s.model.clone(),
            effort: s.effort.clone(),
            add_dirs,
            env,
            jail: None,
        };
        let live = self.attach(app, s.harness, start, self.sink(app, id)).await?;
        Ok(live)
    }

    /// Hold an input without sending it to a harness. This also covers launches at exhausted quota.
    async fn hold_input(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, from: Option<crate::model::MessageFrom>) -> Result<()> {
        self.inner.quota_stops.lock().unwrap().remove(id);
        let s = self.get(id).context("no such session")?;
        let client_id = uuid::Uuid::new_v4().to_string();
        self.update(app, id, |s| s.quota_pause = Some(QuotaPause {
            phase: QuotaPausePhase::Paused, blockers: crate::quota::blockers(app, s.harness, &s.launch_spec),
            message: text.into(), images: images.clone(), client_id: Some(client_id.clone()), error: None, messages: vec![],
        })).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::UserMessage { text: text.into(), client_id, queued: true, images, from }).await;
        sink.set_status(SessionStatus::QuotaPaused).await;
        sink.emit(EventBody::Notice { text: format!("{} input held until a fresh quota reading confirms included quota availability.", s.harness.display()) }).await;
        Ok(())
    }

    /// Durable claim before sending, serialized with owner actions. An uncertain send is never replayed.
    /// A session whose action lock is taken waits for the next check; the resume (which may download, install and
    /// start a process) runs in its own task with that session's lock, so the guard never waits for it.
    pub(crate) async fn check_capacity_retries(&self, app: &App) -> Result<()> {
        // A runner's session (on Home) is retried by its runner.
        for pending in self.list().into_iter().filter(|s| s.runner_id.is_none() && s.capacity_retry_at.is_some_and(|at| at <= now_ms())) {
            let Some(action) = self.try_action_lock(&pending.id) else { continue };
            let Some(s) = self.get(&pending.id) else { continue };
            if s.capacity_retry_at.is_none_or(|at| at > now_ms()) { continue; }
            if self.is_busy(&s) { continue; }
            let eligible = s.status == SessionStatus::Error && s.capacity_retry_count < MAX_CAPACITY_RETRIES;
            self.update(app, &s.id, |s| {
                s.capacity_retry_at = None;
                if eligible { s.capacity_retry_count += 1; }
            }).await?;
            if !eligible { continue; }
            let (this, app) = (self.clone(), app.clone());
            tokio::spawn(async move {
                let _action = action;
                if let Err(e) = this.capacity_retry(&app, &s).await {
                    tracing::warn!("capacity retry {}: {e:#}", s.id);
                }
            });
        }
        Ok(())
    }

    /// One claimed capacity retry. The caller holds the session's action lock.
    async fn capacity_retry(&self, app: &App, s: &Session) -> Result<()> {
        let sink = self.sink(app, &s.id);
        sink.emit(EventBody::Notice { text: format!("Retrying automatically after model capacity failure (attempt {}/{}).", s.capacity_retry_count + 1, MAX_CAPACITY_RETRIES) }).await;
        if let Err(e) = self.resume_locked(app, &s.id, false, true).await {
            let message = format!("{e:#}");
            // Reattachment may fail before a turn starts and before the adapter can
            // report a new status. Only an explicit capacity error earns another retry.
            if is_capacity_error(&message) {
                self.update(app, &s.id, |s| {
                    if s.status == SessionStatus::Error && s.capacity_retry_at.is_none() && s.capacity_retry_count < MAX_CAPACITY_RETRIES {
                        s.capacity_retry_at = Some(now_ms() + CAPACITY_RETRY_DELAY_MS);
                    }
                }).await?;
            }
            sink.emit(EventBody::Notice { text: format!("Automatic capacity retry could not continue: {message}. If no retry is pending, inspect the trace and resume manually.") }).await;
        }
        Ok(())
    }

    /// Each session is checked under its action lock, so a user Stop and an automatic continuation never race. A
    /// session whose lock is taken (an action of its own runs) waits for the next check; an automatic continuation
    /// (which may download, install and start a process) runs in its own task with that lock, so one session never
    /// delays the checks of the others.
    pub(crate) async fn check_quotas(&self, app: &App) -> Result<()> {
        crate::spend::publish(app);
        for s in self.list() {
            // Home: a runner's session is protected by its runner, which receives Home's readings.
            if s.runner_id.is_some() { continue; }
            // Setup has not sent a task yet; start_new checks quota after it finishes.
            if self.setup_running(&s.id) { continue; }
            let Some(action) = self.try_action_lock(&s.id) else { continue };
            // The record as it is now that no action of this session runs.
            let Some(s) = self.get(&s.id) else { continue };
            if s.live && s.quota_pause.is_none() && (s.status.is_running() || s.status.is_done()) {
                crate::titles::generate(app, &s);
            }
            if let Some(live) = self.live(&s.id)
                && let Some(after) = live.idle_release()
                && idle_release_due(&s, now_ms(), after)
                && self.ensure_shutdown_finished(&s.id).is_ok()
                && live.ready_for_turn() && !live.turn_active() && !live.has_background_work() && !live.has_open_questions() {
                drop(live);
                // Its own task with the lock: stopping a process may take seconds, and the others are checked now.
                let (this, app, id) = (self.clone(), app.clone(), s.id.clone());
                tokio::spawn(async move {
                    let _action = action;
                    if let Err(e) = this.release(&app, &id).await { tracing::info!("release idle process {id}: {e:#}"); }
                });
                continue;
            }
            if (s.budget_reached() || !crate::spend::reached(app, &s.launch_spec).is_empty()) && s.status.is_running() && !s.open_question
                && let Some(live) = self.live(&s.id).filter(|l| l.turn_active())
                && self.inner.budget_interrupts.lock().unwrap().get(&s.id).is_none_or(|at| now_ms() - at >= 10_000) {
                // Ends as Stopped, with an automatic-stop notification; the trace also records the reached limit.
                self.inner.budget_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                self.inner.stop_requests.lock().unwrap().insert(s.id.clone());
                if let Err(e) = live.interrupt().await {
                    tracing::warn!("cost limit interrupt {}: {e:#}", s.id);
                }
            }
            if !crate::quota::protected(app, s.harness, &s.launch_spec) && s.quota_pause.is_none() { continue; }
            let blockers = self.quota_blockers(app, &s);
            if s.quota_pause.is_none() && s.status.is_running() && !blockers.is_empty() {
                // A question is already idle: leave it answerable after the quota resets.
                if s.open_question { continue; }
                self.update(app, &s.id, |s| s.quota_pause = Some(QuotaPause {
                    phase: QuotaPausePhase::Pausing, blockers, message: "Continue the interrupted task from where you stopped; check the current state before repeating actions.".into(),
                    images: vec![], client_id: None, error: None, messages: vec![],
                })).await?;
                // The turn's real end (the harness reports it once interrupted) makes it QuotaPaused (`transition`).
                self.sink(app, &s.id).emit(EventBody::Notice { text: "Included quota exhausted. Pausing until quota is available; interruption is best effort and commands may still finish.".into() }).await;
                self.inner.quota_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                if let Some(live) = self.live(&s.id)
                    && let Err(e) = live.interrupt().await {
                        // A turn may have ended naturally while we were deciding to interrupt.
                        if !live.ready_for_turn() {
                            self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::NeedsAttention; p.error = Some(format!("Could not interrupt: {e:#}. Stop or inspect the trace.")); } }).await?;
                        }
                }
            }
            let Some(current) = self.get(&s.id) else { continue };
            let Some(p) = current.quota_pause.clone() else { continue };
            if matches!(p.phase, QuotaPausePhase::Pausing | QuotaPausePhase::Paused) && !self.quota_blockers(app, &s).is_empty()
                && self.live(&s.id).is_some_and(|l| l.turn_active())
                && self.inner.quota_interrupts.lock().unwrap().get(&s.id).is_none_or(|at| now_ms() - at >= 10_000) {
                self.inner.quota_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Pausing; } }).await?;
                if let Some(l) = self.live(&s.id) { let _ = l.interrupt().await; }
            }
            // A provider's spending/billing limit is independent of the included-quota
            // meters. Its explanation stays visible and only an owner retry can clear it.
            if matches!(p.phase, QuotaPausePhase::NeedsAttention | QuotaPausePhase::Resuming | QuotaPausePhase::ProviderLimited) { continue; }
            if self.live(&s.id).is_some_and(|l| !l.ready_for_turn()) { continue; }
            if p.phase == QuotaPausePhase::Pausing {
                self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Paused; } }).await?;
            }
            // The turn is over (or its process is gone) without a final status from the harness: this is its end.
            if self.get(&s.id).is_some_and(|s| s.status != SessionStatus::QuotaPaused) {
                self.sink(app, &s.id).set_status(SessionStatus::QuotaPaused).await;
            }
            let reading = crate::quota::reading_for(app, s.harness, &s.launch_spec);
            let mode = if crate::quota::available(&reading, now_ms()) { Some(QuotaResume::Automatic) }
                else if crate::quota::reset_fallback(&reading, &p, now_ms()) { Some(QuotaResume::ResetFallback) }
                else { None };
            if !current.budget_reached() && crate::spend::reached(app, &current.launch_spec).is_empty() && let Some(mode) = mode {
                let (this, app, id) = (self.clone(), app.clone(), s.id.clone());
                tokio::spawn(async move {
                    let _action = action;
                    if let Err(e) = this.continue_quota(&app, &id, mode).await {
                        tracing::warn!("quota continuation {id}: {e:#}");
                    }
                });
            }
        }
        Ok(())
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
        let spec = LaunchSpec { text: text.into(), github_refs: context.github_refs, mentions: context.mentions, skills: context.skills, profile: session.launch_spec.profile.clone(), ..Default::default() };
        let mut prompt = render_prompt(&spec, &folders, &[]);
        prompt.push_str(&crate::prompt_context::render_skills(app, session.harness, &spec, Some(&session.cwd)).await?);
        if !traces.is_empty() {
            if !prompt.is_empty() { prompt.push_str("\n\n"); }
            prompt.push_str(&crate::compose::trace_references(&traces, &app.paths.cli_command()));
        }
        self.send_locked(app, id, &prompt, images, InputMode::Manual, client_id, context.from).await
    }

    /// `from`: an agent scheduled it (stamped by Home, `senders`); its text arrives already under its header.
    pub async fn send_follow_up(&self, app: &App, id: &str, text: &str, automatic: bool, from: Option<crate::model::MessageFrom>) -> Result<()> {
        self.ensure_local(id)?;
        self.send_inner(app, id, text, vec![], if automatic { InputMode::FollowUpAutomatic } else { InputMode::FollowUpManual }, None, from).await
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
        if mode == InputMode::FollowUpAutomatic && protected && !crate::quota::available(&crate::quota::reading_for(app, s.harness, &s.launch_spec), now_ms()) {
            return Err(FollowUpBusy.into());
        }
        if protected && !self.quota_blockers(app, &s).is_empty() {
            if s.status.is_running() || s.open_question { bail!("Included quota exhausted; wait for quota to reset before sending input or answering this question."); }
            self.hold_input(app, id, text, images, from).await?;
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
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        crate::plain::after_archive(&app, &id).await;
        crate::worktrees::after_archive(&app, &id).await;
        crate::projects::after_archive(&app, &id).await;
    });
    Ok(())
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


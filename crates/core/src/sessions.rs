//! Session manager (ARCHITECTURE.md, section 9): the registry of sessions, their event log, and the
//! routing of input to the harness adapters.
//!
//! Every session is kept in memory (a few hundred records) and written through to `state.db`.
//! Adapters report through a [`Sink`], which numbers events, stores them, pushes them over the
//! WebSocket and keeps the session record (status, usage, cost) up to date.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rusqlite::params;
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
    fn has_open_questions(&self) -> bool { false }
    /// Answers an `ask_user` request: one answer per question.
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()>;
    /// Stops the process; the conversation stays resumable.
    async fn stop(&self) -> Result<()>;
    /// Uses this model and effort from the next turn. Returns false when the process must restart to apply them.
    fn set_model(&self, _model: Option<&str>, _effort: Option<&str>) -> bool { false }
}

/// Everything an adapter needs to start or resume a session.
pub struct StartSpec {
    pub session_id: String,
    pub harness_session_id: Option<String>,
    pub resume: bool,
    pub cwd: PathBuf,
    pub title: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub add_dirs: Vec<String>,
    /// Variant environment overrides; adapters add them to every native process of the session.
    pub env: Vec<(String, String)>,
}

const RAW_LINES_CAP: i64 = 20_000;

#[derive(Debug)]
pub(crate) struct FollowUpBusy;
impl std::fmt::Display for FollowUpBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("The session is running, in a terminal or awaiting an answer; the follow-up stays queued.") }
}
impl std::error::Error for FollowUpBusy {}

#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Inner>,
}

type LiveSessions = HashMap<String, (u64, Arc<dyn LiveSession>)>;

struct Inner {
    records: Mutex<HashMap<String, Session>>,
    stop_requests: Mutex<HashSet<String>>,
    quota_stops: Mutex<HashSet<String>>,
    title_requests: Mutex<HashSet<String>>,
    quota_interrupts: Mutex<HashMap<String, i64>>,
    /// A manual subscription-only retry ignores the old quota snapshot until a newer reading arrives.
    manual_quota_retries: Mutex<HashMap<String, i64>>,
    /// Running turns interrupted at their cost limit, with the time of the last attempt.
    budget_interrupts: Mutex<HashMap<String, i64>>,
    /// Sessions whose process must restart to use a new model or effort, once their turn ends.
    model_restarts: Mutex<HashSet<String>>,
    /// Serializes user actions and quota decisions; adapter events never take this lock.
    actions: tokio::sync::Mutex<()>,
    /// Serializes new work with pre-draft publication and cleanup.
    work: tokio::sync::Mutex<()>,
    /// Attached processes, with an instance number: the exit of an old process never detaches a newer one.
    live: Mutex<LiveSessions>,
    next_instance: std::sync::atomic::AtomicU64,
    raw_counts: Mutex<HashMap<String, i64>>,
    /// Sessions shown in a focused window, per window connection: no desktop notification for them.
    watched: Mutex<HashMap<u64, Presence>>,
}

/// A window's focus, the sessions it shows, and when it said so.
type Presence = (bool, Vec<String>, i64);

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    pub fn new() -> Sessions {
        Sessions {
            inner: Arc::new(Inner {
                records: Mutex::new(HashMap::new()),
                stop_requests: Mutex::new(HashSet::new()),
                quota_stops: Mutex::new(HashSet::new()),
                title_requests: Mutex::new(HashSet::new()),
                quota_interrupts: Mutex::new(HashMap::new()),
                manual_quota_retries: Mutex::new(HashMap::new()),
                budget_interrupts: Mutex::new(HashMap::new()),
                model_restarts: Mutex::new(HashSet::new()),
                actions: tokio::sync::Mutex::new(()),
                work: tokio::sync::Mutex::new(()),
                live: Mutex::new(HashMap::new()),
                next_instance: std::sync::atomic::AtomicU64::new(1),
                raw_counts: Mutex::new(HashMap::new()),
                watched: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Loads sessions from `state.db`. Sessions that were mid-turn when the daemon stopped become
    /// `interrupted`; done ones stay done without a process (sending a message resumes them).
    pub async fn load(&self, app: &App) -> Result<()> {
        let rows: Vec<Session> = app
            .state_db
            .call(|c| {
                let mut st = c.prepare(
                    "SELECT id, harness, harness_session_id, project_id, cwd, title, status, status_since, open_question, model,
                            effort, prompt, usage_json, cost_usd, started_at, ended_at, last_seq, launch_spec_json, quota_pause_json, title_pending
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
                            id: r.get(0)?,
                            harness: HarnessKind::parse(&r.get::<_, String>(1)?).ok_or_else(|| rusqlite::Error::InvalidParameterName("unregistered stored harness".into()))?,
                            harness_session_id: r.get(2)?,
                            project_id: r.get(3)?,
                            cwd: r.get(4)?,
                            title,
                            title_pending,
                            status: SessionStatus::parse(&r.get::<_, String>(6)?),
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
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        let mut interrupted = vec![];
        {
            let mut recs = self.inner.records.lock().unwrap();
            for mut s in rows {
                if let Some(pause) = &mut s.quota_pause {
                    pause.phase = if pause.phase == QuotaPausePhase::Resuming {
                        pause.error = Some("Daemon stopped during continuation; inspect the trace before resuming.".into());
                        QuotaPausePhase::NeedsAttention
                    } else if pause.phase == QuotaPausePhase::Pausing { QuotaPausePhase::Paused } else { pause.phase };
                    s.status = SessionStatus::QuotaPaused;
                    interrupted.push(s.clone());
                } else if s.status.is_running() {
                    s.status = SessionStatus::Interrupted;
                    s.status_since = now_ms();
                    interrupted.push(s.clone());
                }
                recs.insert(s.id.clone(), s);
            }
        }
        for s in interrupted {
            self.persist(app, &s).await?;
        }
        self.publish_counts(app);
        Ok(())
    }

    pub(crate) async fn allow_work(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.work.lock().await
    }

    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<Session> = self.inner.records.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        v
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
        let title = title.trim();
        anyhow::ensure!(!title.is_empty() && title.chars().count() <= 100 && !title.contains(['\n', '\r']), "Use a title of 1 to 100 characters on one line.");
        self.update(app, id, |s| { s.title = title.to_string(); s.title_pending = false; }).await
    }

    pub(crate) fn claim_title_request(&self, id: &str) -> bool {
        let mut requests = self.inner.title_requests.lock().unwrap();
        self.get(id).is_some_and(|s| s.title_pending) && requests.insert(id.to_string())
    }

    pub(crate) fn finish_title_request(&self, id: &str) {
        self.inner.title_requests.lock().unwrap().remove(id);
    }

    /// Each window reports which sessions it shows, and whether it has the focus. `None` when it closes.
    pub fn set_watched(&self, window: u64, presence: Option<(bool, Vec<String>)>) {
        let mut w = self.inner.watched.lock().unwrap();
        match presence {
            Some((focused, sessions)) => w.insert(window, (focused, sessions, now_ms())),
            None => w.remove(&window),
        };
    }

    fn is_watched(&self, id: &str) -> bool {
        let w = self.inner.watched.lock().unwrap();
        w.values().any(|(focused, sessions, at)| *focused && now_ms() - at < 10 * 60_000 && sessions.iter().any(|s| s == id))
    }

    async fn persist(&self, app: &App, s: &Session) -> Result<()> {
        let s = s.clone();
        app.state_db
            .call(move |c| {
                c.execute(
                    "INSERT INTO session (id, harness, harness_session_id, project_id, cwd, title, status, status_since,
                        open_question, model, effort, launch_spec_json, prompt, cost_usd, usage_json, started_at, ended_at, last_seq, quota_pause_json, title_pending)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                     ON CONFLICT (id) DO UPDATE SET harness_session_id = ?3, project_id = ?4, cwd = ?5, title = ?6, status = ?7,
                        status_since = ?8, open_question = ?9, model = ?10, effort = ?11, cost_usd = ?14, usage_json = ?15,
                        ended_at = ?17, last_seq = ?18, launch_spec_json = ?12, quota_pause_json = ?19, title_pending = ?20",
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
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Applies a change to a session record, stores it, and pushes it to the UI.
    pub(crate) async fn update(&self, app: &App, id: &str, f: impl FnOnce(&mut Session)) -> Result<Session> {
        let s = {
            let mut recs = self.inner.records.lock().unwrap();
            let s = recs.get_mut(id).context("no such session")?;
            f(s);
            s.clone()
        };
        self.persist(app, &s).await?;
        app.bus.publish("sessions", "updated", serde_json::to_value(&s)?);
        Ok(s)
    }

    fn publish_counts(&self, app: &App) {
        let (mut running, mut done, mut quota_paused) = (0, 0, 0);
        for s in self.inner.records.lock().unwrap().values() {
            if s.status.is_running() {
                running += 1;
            } else if s.status == SessionStatus::QuotaPaused {
                quota_paused += 1;
            } else if s.status.is_done() {
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
        let source = self.get(id).context("no such session")?;
        let through_seq = through_seq.unwrap_or(source.last_seq);
        anyhow::ensure!(through_seq > 0 && through_seq <= source.last_seq, "invalid trace sequence");
        let events = self.events_through(app, &source.id, 0, through_seq).await?;
        anyhow::ensure!(events.last().is_some_and(|e| e.seq == through_seq), "trace is unavailable");
        crate::compose::render_trace(&source, through_seq, &events)
    }

    pub fn sink(&self, app: &App, id: &str) -> Sink {
        Sink { app: app.clone(), sessions: self.clone(), id: id.to_string(), instance: 0 }
    }

    // ---------------------------------------------------------------- operations

    /// Resolves a launch spec: cwd, prompt, title and context directories. No side effect.
    pub async fn preview(&self, app: &App, spec: &LaunchSpec) -> Result<Preview> {
        let config = app.config.get();
        let mut context = Vec::new();
        let mut add_dirs = Vec::new();
        for cp in &spec.context_projects {
            if let Some(p) = crate::projects::get(app, &cp.project_id).await? {
                add_dirs.push(p.path.clone());
                context.push(ContextInfo { path: p.path, description: p.description, branch: cp.branch.clone() });
            }
        }
        let mut rendered_spec = spec.clone();
        if matches!(
            spec.target,
            Target::Predraft { .. } | Target::ExistingPredraft { .. }
        ) {
            rendered_spec
                .presets
                .retain(|id| id != "draft-pr" && id != "worktree");
        }
        let mut prompt = render_prompt(&rendered_spec, &context, &config.presets);
        let mut traces = Vec::new();
        for trace in &spec.agent_traces {
            let source = self.get(&trace.session_id).context("context agent no longer exists")?;
            anyhow::ensure!(trace.through_seq > 0 && trace.through_seq <= source.last_seq, "invalid context trace sequence");
            traces.push((source, trace.through_seq));
        }
        if !traces.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&crate::compose::trace_references(&traces, &app.paths.cli_command()));
        }
        if !prompt.trim().is_empty() && matches!(
            spec.target,
            Target::Predraft { .. } | Target::ExistingPredraft { .. }
        ) {
            prompt.push_str("\n\nWork in a dedicated local pre-draft worktree. Commit your changes, update the local PR title and description with launchpad pr update. Do not push, publish, or delete the worktree.");
        }
        let (cwd, project) = match &spec.target {
            Target::Project { project_id } | Target::Predraft { project_id } => {
                let p = crate::projects::get(app, project_id).await?.context("the project no longer exists")?;
                (Some(p.path.clone()), Some(p))
            }
            Target::ExistingPredraft { predraft_id } => {
                let p = crate::predrafts::get(app, predraft_id).await?;
                anyhow::ensure!(
                    !p.archived && p.published_url.is_none() && p.pushed_head.is_none(),
                    "pre-draft is archived, published, or publication has started"
                );
                let project = crate::projects::get(app, &p.project_id)
                    .await?
                    .context("project not found")?;
                (Some(p.root), Some(project))
            }
            Target::Temp => (None, None),
            Target::None => (None, None),
        };
        Ok(Preview {
            title: title_from(&spec.text, project.as_ref().map(|p| p.name.as_str()).unwrap_or("New session")),
            prompt,
            cwd,
            add_dirs,
        })
    }

    pub async fn launch(&self, app: &App, spec: LaunchSpec) -> Result<Session> {
        self.launch_with_id(app, spec, uuid::Uuid::new_v4().to_string()).await
    }

    pub(crate) async fn launch_with_id(&self, app: &App, spec: LaunchSpec, id: String) -> Result<Session> {
        let _action = self.inner.actions.lock().await;
        let _guard = self.allow_work().await;
        let harness = spec.harness.context("pick a harness")?;
        if spec.target == Target::None {
            bail!("pick a project or a temporary folder");
        }
        let variant = crate::variants::resolve(app, harness, &spec)?;
        let env = crate::variants::env(app, harness, &spec)?;
        check_budget(harness, spec.budget_usd)?;
        let (model, effort) = (
            spec.model.clone().or_else(|| variant.as_ref().and_then(|v| v.model.clone())),
            spec.effort.clone().or_else(|| variant.as_ref().and_then(|v| v.effort.clone())),
        );
        let mut preview = self.preview(app, &spec).await?;
        if preview.prompt.trim().is_empty() {
            bail!("the prompt is empty");
        }
        let mut predraft = None;
        let (cwd, project_id) = match &spec.target {
            Target::Temp => {
                let p = crate::projects::create_temp(app, &spec.text).await?;
                (p.path, Some(p.id))
            }
            Target::Project { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
            Target::Predraft { project_id } => {
                let mut p = crate::predrafts::create(
                    app,
                    crate::predrafts::Create {
                        project_id: Some(project_id.clone()),
                        title: Some(preview.title.clone()),
                        ..Default::default()
                    },
                )
                .await?;
                p.session_id = Some(id.clone());
                crate::predrafts::save(app, &p).await?;
                preview
                    .prompt
                    .push_str(&format!("\n\n{}", crate::predrafts::instruction(app, &p)));
                let cwd = p.root.clone();
                predraft = Some(p);
                (cwd, Some(project_id.clone()))
            }
            Target::ExistingPredraft { predraft_id } => {
                let _lock = app.predraft_lock.lock().await;
                let mut p = crate::predrafts::get(app, predraft_id).await?;
                anyhow::ensure!(
                    !p.archived && p.published_url.is_none() && p.pushed_head.is_none(),
                    "pre-draft is archived, published, or publication has started"
                );
                anyhow::ensure!(
                    !self.list().iter().any(|s| s.cwd == p.root
                        && (self.is_busy(s) || s.status == SessionStatus::InTerminal)),
                    "pre-draft already has an active agent"
                );
                p.session_id = Some(id.clone());
                crate::predrafts::save(app, &p).await?;
                preview
                    .prompt
                    .push_str(&format!("\n\n{}", crate::predrafts::instruction(app, &p)));
                let cwd = p.root.clone();
                let project_id = p.project_id.clone();
                predraft = Some(p);
                (cwd, Some(project_id))
            }
            Target::None => unreachable!(),
        };
        if let (Some(ws), Some(project_id)) = (&spec.workspace_id, &project_id)
            && let Err(e) = crate::workspace::add_project(app, ws, project_id).await
        {
            tracing::warn!("launch: could not add the project to its workspace: {e:#}");
        }
        preview.prompt.push_str(&format!("\n\n{}", crate::suggestions::instruction(app, &id)));
        let now = now_ms();
        let session = Session {
            id: id.clone(),
            harness,
            harness_session_id: None,
            project_id,
            cwd: cwd.clone(),
            title: preview.title.clone(),
            title_pending: true,
            status: SessionStatus::Starting,
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
        };
        self.inner.records.lock().unwrap().insert(id.clone(), session.clone());
        self.persist(app, &session).await?;
        if let Some(p) = predraft {
            let session_id = id.clone();
            let predraft_id = p.id;
            app.state_db
                .call(move |c| {
                    c.execute(
                        "UPDATE session SET predraft_id=?1 WHERE id=?2",
                        rusqlite::params![predraft_id, session_id],
                    )?;
                    Ok(())
                })
                .await?;
        }
        app.bus.publish("sessions", "updated", serde_json::to_value(&session)?);
        self.publish_counts(app);

        let protected = crate::quota::protected(app, harness, &session.launch_spec);
        if protected && !crate::quota::blockers(app, harness).is_empty() {
            self.update(app, &id, |s| s.harness_session_id = None).await?;
            self.hold_input(app, &id, &preview.prompt, vec![]).await?;
            return Ok(self.get(&id).unwrap());
        }

        if protected && !crate::quota::available(&crate::quota::reading(app, harness), now_ms()) {
            self.sink(app, &id).emit(EventBody::Notice { text: "Quota reading unavailable or stale. Starting with best-effort protection; additional spending cannot be ruled out until quota is known.".into() }).await;
        }
        let start = StartSpec {
            session_id: id.clone(),
            harness_session_id: session.harness_session_id.clone(),
            resume: false,
            cwd: cwd.into(),
            title: preview.title,
            model,
            effort,
            add_dirs: preview.add_dirs,
            env,
        };
        let sink = self.sink(app, &id);
        match self.attach(app, harness, start, sink.clone()).await {
            Ok(live) => {
                let client_id = uuid::Uuid::new_v4().to_string();
                sink.emit(EventBody::UserMessage { text: preview.prompt.clone(), client_id: client_id.clone(), queued: false, images: vec![] }).await;
                sink.set_status(SessionStatus::Working).await;
                if let Err(e) = live.send(&preview.prompt, &client_id, &[]).await {
                    sink.fail(&format!("{e:#}")).await;
                }
            }
            Err(e) => {
                sink.fail(&format!("could not start {}: {e:#}", harness.display())).await
            }
        }
        if let Some(session) = self.get(&id).filter(|s| s.status != SessionStatus::Error) { crate::titles::generate(app, &session); }
        Ok(self.get(&id).unwrap())
    }

    async fn attach(&self, app: &App, harness: HarnessKind, start: StartSpec, mut sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let id = start.session_id.clone();
        let instance = self.inner.next_instance.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sink.instance = instance;
        let live = app
            .harnesses
            .get(harness)
            .start(app, start, sink.clone())
            .await?;
        self.inner
            .live
            .lock()
            .unwrap()
            .insert(id.clone(), (instance, live.clone()));
        self.inner.model_restarts.lock().unwrap().remove(&id);
        self.update(app, &id, |s| s.live = true).await?;
        Ok(live)
    }

    /// Starts a process for a session that has none (resume).
    /// An idle process waiting for a model or effort change is replaced.
    async fn reattach(&self, app: &App, id: &str) -> Result<Arc<dyn LiveSession>> {
        let s = self.get(id).context("no such session")?;
        if let Some(l) = self.live(id) {
            if !self.inner.model_restarts.lock().unwrap().contains(id) || s.status.is_running() || s.open_question || !l.ready_for_turn() {
                return Ok(l);
            }
            self.detach(id).await;
        }
        if s.launch_spec.harness.is_none() && crate::terminals::tui_running(app, &s) {
            bail!("This imported conversation is still running in another client. Finish it there first.");
        }
        let mut add_dirs = Vec::new();
        for cp in &s.launch_spec.context_projects {
            if let Some(p) = crate::projects::get(app, &cp.project_id).await? {
                add_dirs.push(p.path);
            }
        }
        let start = StartSpec {
            session_id: s.id.clone(),
            harness_session_id: s.harness_session_id.clone(),
            resume: s.harness_session_id.is_some(),
            cwd: s.cwd.clone().into(),
            title: s.title.clone(),
            model: s.model.clone(),
            effort: s.effort.clone(),
            add_dirs,
            env: crate::variants::env(app, s.harness, &s.launch_spec)?,
        };
        if !start.cwd.exists() {
            bail!("the folder {} no longer exists", s.cwd);
        }
        let live = self.attach(app, s.harness, start, self.sink(app, id)).await?;
        Ok(live)
    }

    /// Hold an input without sending it to a harness. This also covers launches at exhausted quota.
    async fn hold_input(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>) -> Result<()> {
        self.inner.quota_stops.lock().unwrap().remove(id);
        let s = self.get(id).context("no such session")?;
        let client_id = uuid::Uuid::new_v4().to_string();
        self.update(app, id, |s| s.quota_pause = Some(QuotaPause {
            phase: QuotaPausePhase::Paused, blockers: crate::quota::blockers(app, s.harness),
            message: text.into(), images: images.clone(), client_id: Some(client_id.clone()), error: None,
        })).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::UserMessage { text: text.into(), client_id, queued: true, images }).await;
        sink.set_status(SessionStatus::QuotaPaused).await;
        sink.emit(EventBody::Notice { text: format!("{} included quota exhausted. Input held until a fresh quota reading confirms availability.", s.harness.display()) }).await;
        Ok(())
    }

    /// Runs under the action lock, preventing user Stop and an automatic continuation from racing.
    pub(crate) async fn check_quotas(&self, app: &App) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        for s in self.list() {
            if s.live && s.quota_pause.is_none() && (s.status.is_running() || s.status.is_done()) {
                crate::titles::generate(app, &s);
            }
            if s.budget_reached() && s.status.is_running() && !s.open_question
                && let Some(live) = self.live(&s.id).filter(|l| l.turn_active())
                && self.inner.budget_interrupts.lock().unwrap().get(&s.id).is_none_or(|at| now_ms() - at >= 10_000) {
                // Ends as Stopped, like an owner Stop; the reached limit was announced when the cost was reported.
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
                    images: vec![], client_id: None, error: None,
                })).await?;
                self.sink(app, &s.id).set_status(SessionStatus::QuotaPaused).await;
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
            if matches!(p.phase, QuotaPausePhase::NeedsAttention | QuotaPausePhase::Resuming) { continue; }
            if self.live(&s.id).is_some_and(|l| !l.ready_for_turn()) { continue; }
            if p.phase == QuotaPausePhase::Pausing {
                self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Paused; } }).await?;
            }
            if !current.budget_reached() && crate::quota::available(&crate::quota::reading(app, s.harness), now_ms()) {
                match self.continue_quota(app, &s.id, false, false).await {
                    Ok(()) => {}
                    Err(e) => tracing::warn!("quota continuation {}: {e:#}", s.id),
                }
            }
        }
        Ok(())
    }

    pub async fn resume_paid(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        self.continue_quota(app, id, true, false).await
    }

    pub async fn resume_subscription(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        self.continue_quota(app, id, false, true).await
    }

    fn quota_blockers(&self, app: &App, s: &Session) -> Vec<crate::model::QuotaBlock> {
        let reading = crate::quota::reading(app, s.harness);
        let mut retries = self.inner.manual_quota_retries.lock().unwrap();
        if let Some(&baseline) = retries.get(&s.id) {
            if reading.quota_at.unwrap_or(0) <= baseline { return vec![]; }
            retries.remove(&s.id);
        }
        crate::quota::exhausted(&reading)
    }

    /// Toggles paid usage for one session. Turning it off puts the session back under quota protection at the next guard check.
    pub async fn set_paid_usage(&self, app: &App, id: &str, allow: bool) -> Result<Session> {
        let _action = self.inner.actions.lock().await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(crate::harness::registration(s.harness).descriptor.capabilities.quota,
            "This integration does not monitor subscription quotas; native provider billing settings apply.");
        anyhow::ensure!(!crate::variants::pay_as_you_go(app, &s.launch_spec),
            "This pay-as-you-go variant has no subscription quota protection; use its cost limit instead.");
        if s.launch_spec.allow_paid_usage == allow { return Ok(s); }
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        let s = self.update(app, id, |s| s.launch_spec.allow_paid_usage = allow).await?;
        self.sink(app, id).emit(EventBody::Notice { text: if allow { "Paid usage allowed for this session; additional credits may be used after included quota runs out." } else { "Paid usage disabled; this session pauses at included quota limits." }.into() }).await;
        Ok(s)
    }

    /// Sets or removes the cost limit of one session. A running turn above the new limit is interrupted by the guard.
    pub async fn set_budget(&self, app: &App, id: &str, budget_usd: Option<f64>) -> Result<Session> {
        let _action = self.inner.actions.lock().await;
        let s = self.get(id).context("no such session")?;
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

    /// Changes model and effort through the live adapter, or resumes a replacement process once idle.
    pub async fn set_model(&self, app: &App, id: &str, model: Option<String>, effort: Option<String>) -> Result<Session> {
        let _action = self.inner.actions.lock().await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.status != SessionStatus::InTerminal, "The session is open in a terminal: re-attach it first.");
        let defaults = app.models.defaults(app, s.harness);
        anyhow::ensure!(effort.is_none() || !defaults.efforts.is_empty() || defaults.models.iter().any(|m| !m.efforts.is_empty()), "This harness does not support an effort override.");
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
        self.inner.model_restarts.lock().unwrap().insert(id.to_string());
        if s.status.is_running() || s.open_question || !live.ready_for_turn() {
            sink.emit(EventBody::Notice { text: format!("Model set to {label}; the agent restarts with it after the current turn.") }).await;
            return Ok(s);
        }
        let _guard = self.allow_work().await;
        self.reattach(app, id).await?;
        sink.emit(EventBody::Notice { text: format!("Restarted with {label}.") }).await;
        self.get(id).context("no such session")
    }

    async fn continue_quota(&self, app: &App, id: &str, paid: bool, manual: bool) -> Result<()> {
        let _guard = self.allow_work().await;
        let s = self.get(id).context("no such session")?;
        let p = s.quota_pause.clone().context("The session is not quota paused")?;
        ensure_budget(&s)?;
        anyhow::ensure!(!s.open_question && self.live(id).is_none_or(|l| l.ready_for_turn()), "Wait for the interrupted turn and outstanding commands to finish.");
        if manual {
            anyhow::ensure!(p.phase == QuotaPausePhase::Paused, "Wait for the interrupted turn to finish; a claimed continuation needs inspection before retrying.");
            anyhow::ensure!(!s.launch_spec.allow_paid_usage, "Paid usage is already allowed for this session.");
        } else if !paid {
            anyhow::ensure!(p.phase == QuotaPausePhase::Paused && crate::quota::available(&crate::quota::reading(app, s.harness), now_ms()), "Waiting for a fresh quota reading");
        }
        // A published/deleted pre-draft must not be resumed by the guard.
        if let Err(e) = crate::predrafts::check_session_work(app, &s.cwd).await {
            self.quota_attention(app, id, &format!("{e:#}")).await?;
            return Err(e);
        }
        self.update(app, id, |s| {
            if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Resuming; p.error = None; }
            if paid { s.launch_spec.allow_paid_usage = true; }
        }).await?;
        if manual {
            let baseline = crate::quota::reading(app, s.harness).quota_at.unwrap_or(0);
            self.inner.manual_quota_retries.lock().unwrap().insert(id.to_string(), baseline);
        }
        // Preserve the claim until the send has returned. Recovery never retries a claimed send.
        let result = async {
            let live = self.reattach(app, id).await?;
            let sink = self.sink(app, id);
            let client_id = p.client_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if p.client_id.is_none() {
                sink.emit(EventBody::UserMessage { text: p.message.clone(), client_id: client_id.clone(), queued: false, images: p.images.clone() }).await;
            }
            sink.emit(EventBody::Notice { text: if paid { "Continuing with paid usage allowed" } else if manual { "Continuing with subscription-only protection despite an unconfirmed quota reading; paid credits may still be used if the included quota is exhausted." } else { "Quota available; continuing automatically" }.into() }).await;
            sink.set_status(SessionStatus::Working).await;
            live.send(&p.message, &client_id, &p.images).await?;
            if p.client_id.is_some() { sink.emit(EventBody::MessageDelivered { client_id }).await; }
            Ok::<(), anyhow::Error>(())
        }.await;
        match result {
            Ok(()) => {
                self.update(app, id, |s| s.quota_pause = None).await?;
                Ok(())
            }
            Err(e) => { self.inner.manual_quota_retries.lock().unwrap().remove(id); self.quota_attention(app, id, &format!("Continuation outcome needs inspection: {e:#}")).await?; Err(e) }
        }
    }

    async fn quota_attention(&self, app: &App, id: &str, message: &str) -> Result<()> {
        self.update(app, id, |s| { s.status = SessionStatus::QuotaPaused; if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::NeedsAttention; p.error = Some(message.into()); } }).await?;
        self.publish_counts(app);
        Ok(())
    }

    pub async fn send(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>) -> Result<()> {
        self.send_inner(app, id, text, images, false).await
    }

    pub async fn send_follow_up(&self, app: &App, id: &str, text: &str) -> Result<()> {
        self.send_inner(app, id, text, vec![], true).await
    }

    async fn send_inner(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, follow_up: bool) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        let _guard = self.allow_work().await;
        let s = self.get(id).context("no such session")?;
        if follow_up && (s.status.is_running() || s.open_question || s.status == SessionStatus::InTerminal || s.quota_pause.is_some()) {
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
        crate::predrafts::check_session_work(app, &s.cwd).await?;
        if s.status == SessionStatus::InTerminal {
            bail!("the session is open in a terminal: re-attach it first");
        }
        anyhow::ensure!(s.quota_pause.is_none(), "This session is quota paused. Use a Resume button or Stop first.");
        ensure_budget(&s)?;
        let protected = crate::quota::protected(app, s.harness, &s.launch_spec);
        if protected && !self.quota_blockers(app, &s).is_empty() {
            if s.status.is_running() || s.open_question { bail!("Included quota exhausted; wait for quota to reset before sending input or answering this question."); }
            self.hold_input(app, id, text, images).await?;
            return Ok(());
        }
        anyhow::ensure!(
            s.open_question
                || s.status.is_running()
                || self.live(id).is_none_or(|l| l.ready_for_turn()),
            "Wait for outstanding commands to finish before starting another turn."
        );
        if protected && !crate::quota::available(&crate::quota::reading(app, s.harness), now_ms()) {
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
        let client_id = uuid::Uuid::new_v4().to_string();
        let queued = s.status.is_running() && !resumed;
        sink.emit(EventBody::UserMessage { text: text.to_string(), client_id: client_id.clone(), queued, images: images.clone() }).await;
        if !queued {
            sink.set_status(SessionStatus::Working).await;
        }
        if let Err(e) = live.send(text, &client_id, &images).await {
            sink.fail(&format!("{e:#}")).await;
            return Err(e);
        }
        Ok(())
    }

    pub async fn interrupt(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        if self.get(id).is_some_and(|s| s.quota_pause.is_some()) {
            if let Some(live) = self.live(id).filter(|l| l.turn_active()) { live.interrupt().await?; }
            self.inner.quota_stops.lock().unwrap().insert(id.to_string());
            self.update(app, id, |s| { s.quota_pause = None; }).await?;
            self.sink(app, id).set_status(SessionStatus::Waiting).await;
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
        let _action = self.inner.actions.lock().await;
        let _guard = self.allow_work().await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.quota_pause.is_none(), "The session is quota paused.");
        anyhow::ensure!(!crate::quota::protected(app, s.harness, &s.launch_spec) || crate::quota::blockers(app, s.harness).is_empty(), "Included quota exhausted; wait for the quota pause.");
        ensure_budget(&s)?;
        let live = self.live(id).context("the session has no running process")?;
        live.answer(request_id, answers.clone()).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::AskAnswered { request_id: request_id.to_string(), answers }).await;
        sink.set_open_question(live.has_open_questions()).await;
        sink.set_status(SessionStatus::Working).await;
        Ok(())
    }

    async fn detach(&self, id: &str) {
        let live = self.inner.live.lock().unwrap().remove(id);
        if let Some((_, l)) = live
            && let Err(e) = l.stop().await {
            tracing::warn!("stop {id}: {e:#}");
        }
    }

    /// Stops the process. The conversation stays resumable.
    pub async fn close(&self, app: &App, id: &str) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        self.inner.manual_quota_retries.lock().unwrap().remove(id);
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.update(app, id, |s| s.quota_pause = None).await?;
        self.detach(id).await;
        let sink = self.sink(app, id);
        sink.set_status(SessionStatus::Closed).await;
        self.update(app, id, |s| {
            s.live = false;
            s.open_question = false;
            s.ended_at = Some(now_ms());
        })
        .await?;
        self.publish_counts(app);
        Ok(())
    }

    /// Resumes a closed or interrupted session in Launchpad, without sending anything.
    /// With `close_terminal`, an integrated terminal still running the session's TUI is closed first.
    pub async fn resume(&self, app: &App, id: &str, close_terminal: bool) -> Result<()> {
        let _action = self.inner.actions.lock().await;
        let _guard = self.allow_work().await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(s.quota_pause.is_none(), "Use Resume with paid usage to continue a quota-paused session.");
        crate::predrafts::check_session_work(app, &s.cwd).await?;
        if let Some(t) = app.terminals.tui_of(id) {
            if !close_terminal {
                bail!("the session is still open in an integrated terminal: close it first");
            }
            app.terminals.close_and_wait(&[t]).await?;
        }
        if s.status == SessionStatus::InTerminal && crate::terminals::tui_running(app, &s) {
            bail!("the session is still open in another terminal: close it there first");
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
        let _action = self.inner.actions.lock().await;
        let _guard = self.allow_work().await;
        let s = self.get(id).context("no such session")?;
        anyhow::ensure!(crate::harness::registration(s.harness).descriptor.capabilities.terminal, "This harness does not support a terminal TUI.");
        if let Some(t) = app.terminals.tui_of(id) {
            return app.terminals.list().into_iter().find(|i| i.id == t).context("no such terminal");
        }
        crate::predrafts::check_session_work(app, &s.cwd).await?;
        if s.status.is_running() || s.quota_pause.is_some() {
            bail!("stop the session before opening it in a terminal; terminal sessions are outside quota protection");
        }
        if s.harness_session_id.is_none() {
            bail!("the session has no conversation yet");
        }
        self.inner.quota_stops.lock().unwrap().remove(id);
        self.detach(id).await;
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
        app.terminals.close_in(path).await?;
        let ids: Vec<String> =
            self.list().into_iter().filter(|s| s.cwd == path || s.cwd.starts_with(&format!("{path}/"))).map(|s| s.id).collect();
        for id in ids {
            if self.get(&id).is_some_and(|s| s.live || s.status.is_running() || s.status.is_done() || s.quota_pause.is_some()) {
                self.close(app, &id).await?;
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
        s.status.is_running() || self.live(&s.id).is_some_and(|l| !l.ready_for_turn())
    }

    pub fn running_in(&self, path: &str) -> bool {
        self.list().iter().any(|s| (s.cwd == path || s.cwd.starts_with(&format!("{path}/"))) && (s.live || s.status.is_running()))
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

fn check_budget(harness: HarnessKind, budget_usd: Option<f64>) -> Result<()> {
    let Some(b) = budget_usd else { return Ok(()) };
    anyhow::ensure!(b.is_finite() && b > 0.0, "The cost limit must be a positive amount.");
    anyhow::ensure!(crate::harness::registration(harness).descriptor.capabilities.cost,
        "{} does not report costs, so a cost limit cannot apply.", harness.display());
    Ok(())
}

fn ensure_budget(s: &Session) -> Result<()> {
    anyhow::ensure!(!s.budget_reached(), "Cost limit reached: ${:.2} reported of ${:.2}. Raise or remove the limit to continue.",
        s.usage.cost_usd.unwrap_or(0.0), s.launch_spec.budget_usd.unwrap_or(0.0));
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, ts_rs::TS)]
pub struct Preview {
    pub title: String,
    pub prompt: String,
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
}

impl Sink {
    pub async fn emit(&self, body: EventBody) {
        let ts = now_ms();
        if body.is_ephemeral() {
            let ev = SessionEvent { session_id: self.id.clone(), seq: 0, ts, body };
            self.publish(&ev);
            return;
        }
        let seq = {
            let mut recs = self.sessions.inner.records.lock().unwrap();
            let Some(s) = recs.get_mut(&self.id) else { return };
            s.last_seq += 1;
            s.last_seq
        };
        let ev = SessionEvent { session_id: self.id.clone(), seq, ts, body };
        let (id, kind, json) = (self.id.clone(), ev.body.kind(), serde_json::to_string(&ev.body).unwrap_or_default());
        let res = self
            .app
            .state_db
            .call(move |c| {
                c.execute("INSERT INTO session_event (session_id, seq, ts, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, seq, ts, kind, json])?;
                c.execute("UPDATE session SET last_seq = ?1 WHERE id = ?2", params![seq, id])?;
                Ok(())
            })
            .await;
        if let Err(e) = res {
            tracing::error!("store event: {e:#}");
        }
        self.publish(&ev);
    }

    fn publish(&self, ev: &SessionEvent) {
        self.app.bus.send(Event {
            seq: ev.seq as u64,
            topic: format!("session/{}", self.id),
            kind: "event".into(),
            payload: serde_json::to_value(ev).unwrap_or(Value::Null),
        });
    }

    /// Stores a raw harness line (capped per session), so the normalizer can be improved and replayed.
    pub async fn raw(&self, dir: &str, line: &str) {
        let known = self.sessions.inner.raw_counts.lock().unwrap().contains_key(&self.id);
        if !known {
            // First line since the daemon started: continue the numbering of earlier processes.
            let id = self.id.clone();
            let next = self
                .app
                .state_db
                .call(move |c| Ok(c.query_row("SELECT coalesce(max(n) + 1, 0) FROM session_raw WHERE session_id = ?1", [id], |r| r.get::<_, i64>(0))?))
                .await
                .unwrap_or(0);
            self.sessions.inner.raw_counts.lock().unwrap().entry(self.id.clone()).or_insert(next);
        }
        let n = {
            let mut counts = self.sessions.inner.raw_counts.lock().unwrap();
            let c = counts.get_mut(&self.id).unwrap();
            *c += 1;
            *c - 1
        };
        if n >= RAW_LINES_CAP {
            return;
        }
        let (id, dir, line) = (self.id.clone(), dir.to_string(), line.chars().take(200_000).collect::<String>());
        let _ = self
            .app
            .state_db
            .call(move |c| Ok(c.execute("INSERT OR IGNORE INTO session_raw (session_id, n, ts, dir, line) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, n, now_ms(), dir, line])?))
            .await;
    }

    pub fn stop_requested(&self) -> bool {
        self.sessions.inner.stop_requests.lock().unwrap().contains(&self.id)
    }

    pub fn status(&self) -> Option<SessionStatus> {
        self.sessions.get(&self.id).map(|s| s.status)
    }

    pub async fn set_status(&self, status: SessionStatus) {
        // Detached adapters can still report a final result after Archive or reattachment.
        if self.instance != 0 && self.sessions.inner.live.lock().unwrap().get(&self.id).is_none_or(|(instance, _)| *instance != self.instance) { return; }
        if matches!(status, SessionStatus::Stopped | SessionStatus::Closed | SessionStatus::Interrupted | SessionStatus::Error) {
            self.sessions.inner.stop_requests.lock().unwrap().remove(&self.id);
        }
        let Some(before) = self.sessions.get(&self.id) else { return };
        // A quota interruption is durable and must survive late adapter state updates.
        if before.quota_pause.as_ref().is_some_and(|p| p.phase != QuotaPausePhase::Resuming) && status != SessionStatus::QuotaPaused {
            return;
        }
        if self.sessions.inner.quota_stops.lock().unwrap().contains(&self.id) && status != SessionStatus::Closed && status != SessionStatus::Waiting { return; }
        if before.status == status {
            return;
        }
        let _ = self
            .sessions
            .update(&self.app, &self.id, |s| {
                if s.quota_pause.as_ref().is_some_and(|p| p.phase != QuotaPausePhase::Resuming) && status != SessionStatus::QuotaPaused { return; }
                s.status = status;
                s.status_since = now_ms();
                if status.is_running() { s.ended_at = None; }
            })
            .await;
        if self.sessions.get(&self.id).is_none_or(|s| s.status != status) { return; }
        self.emit(EventBody::Status { status }).await;
        self.sessions.publish_counts(&self.app);
        if before.status.is_running() && status.is_done() && !self.sessions.is_watched(&self.id) {
            let s = self.sessions.get(&self.id).unwrap();
            crate::notify::session_done(&self.app, &s);
        }
    }

    pub async fn set_open_question(&self, open: bool) {
        let _ = self.sessions.update(&self.app, &self.id, |s| s.open_question = open).await;
    }

    pub async fn set_harness_id(&self, harness_id: &str) {
        let h = harness_id.to_string();
        let _ = self.sessions.update(&self.app, &self.id, |s| s.harness_session_id = Some(h)).await;
    }

    pub async fn set_model(&self, model: &str) {
        let m = model.to_string();
        let _ = self.sessions.update(&self.app, &self.id, |s| {
            if s.model.is_none() {
                s.model = Some(m)
            }
        })
        .await;
    }

    pub async fn update_usage(&self, f: impl FnOnce(&mut Usage)) {
        let mut crossed = false;
        let _ = self.sessions.update(&self.app, &self.id, |s| {
            let before = s.budget_reached();
            f(&mut s.usage);
            crossed = !before && s.budget_reached();
        }).await;
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

    pub async fn exited(&self, error: Option<String>) {
        self.sessions.process_exited(&self.app, &self.id, self.instance, error).await;
    }

    pub fn app(&self) -> &App {
        &self.app
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

pub fn shorten(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(max).collect::<String>())
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn truncate_unicode_output_without_overlapping_ends() {
        let text = "🦀".repeat(30);
        let result = super::truncate_output(&text, 100);
        assert!(result.starts_with("🦀"));
        assert!(result.ends_with("🦀"));
        assert!(result.contains("24 bytes omitted"));
        assert_eq!(super::truncate_output("small", 100), "small");
        assert!(super::truncate_output("🦀", 1).contains("4 bytes omitted"));
    }
}

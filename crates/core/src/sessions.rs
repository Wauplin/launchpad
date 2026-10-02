//! Session manager (ARCHITECTURE.md, section 9): the registry of sessions, their event log, and the
//! routing of input to the harness adapters.
//!
//! Every session is kept in memory (a few hundred records) and written through to `state.db`.
//! Adapters report through a [`Sink`], which numbers events, stores them, pushes them over the
//! WebSocket and keeps the session record (status, usage, cost) up to date.

use std::collections::HashMap;
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
    async fn send(&self, text: &str, client_id: &str) -> Result<()>;
    async fn interrupt(&self) -> Result<()>;
    /// Answers an `ask_user` request: one answer per question.
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()>;
    /// Stops the process; the conversation stays resumable.
    async fn stop(&self) -> Result<()>;
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
}

const RAW_LINES_CAP: i64 = 20_000;

#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Inner>,
}

struct Inner {
    records: Mutex<HashMap<String, Session>>,
    /// Attached processes, with an instance number: the exit of an old process never detaches a newer one.
    live: Mutex<HashMap<String, (u64, Arc<dyn LiveSession>)>>,
    next_instance: std::sync::atomic::AtomicU64,
    raw_counts: Mutex<HashMap<String, i64>>,
    /// Sessions shown in a focused window: no desktop notification for them.
    watched: Mutex<(bool, Vec<String>, i64)>,
}

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
                live: Mutex::new(HashMap::new()),
                next_instance: std::sync::atomic::AtomicU64::new(1),
                raw_counts: Mutex::new(HashMap::new()),
                watched: Mutex::new((false, vec![], 0)),
            }),
        }
    }

    /// Loads sessions from `state.db`. Sessions that were mid-turn when the daemon stopped become
    /// `interrupted`; pending ones stay pending without a process (sending a message resumes them).
    pub async fn load(&self, app: &App) -> Result<()> {
        let rows: Vec<Session> = app
            .state_db
            .call(|c| {
                let mut st = c.prepare(
                    "SELECT id, harness, harness_session_id, project_id, cwd, title, status, status_since, open_question, model,
                            effort, prompt, usage_json, cost_usd, started_at, ended_at, last_seq, launch_spec_json
                     FROM session",
                )?;
                let rows = st
                    .query_map([], |r| {
                        let usage: Option<String> = r.get(12)?;
                        let mut usage: Usage = usage.and_then(|u| serde_json::from_str(&u).ok()).unwrap_or_default();
                        usage.cost_usd = r.get(13)?;
                        let spec: String = r.get(17)?;
                        Ok(Session {
                            id: r.get(0)?,
                            harness: HarnessKind::parse(&r.get::<_, String>(1)?).unwrap_or(HarnessKind::Claude),
                            harness_session_id: r.get(2)?,
                            project_id: r.get(3)?,
                            cwd: r.get(4)?,
                            title: r.get(5)?,
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
                            launch_spec: serde_json::from_str(&spec).unwrap_or_default(),
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
                if s.status.is_running() {
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

    /// The UI reports which sessions it shows, and whether its window has the focus.
    pub fn set_watched(&self, focused: bool, sessions: Vec<String>) {
        *self.inner.watched.lock().unwrap() = (focused, sessions, now_ms());
    }

    fn is_watched(&self, id: &str) -> bool {
        let w = self.inner.watched.lock().unwrap();
        w.0 && now_ms() - w.2 < 10 * 60_000 && w.1.iter().any(|s| s == id)
    }

    async fn persist(&self, app: &App, s: &Session) -> Result<()> {
        let s = s.clone();
        app.state_db
            .call(move |c| {
                c.execute(
                    "INSERT INTO session (id, harness, harness_session_id, project_id, cwd, title, status, status_since,
                        open_question, model, effort, launch_spec_json, prompt, cost_usd, usage_json, started_at, ended_at, last_seq)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                     ON CONFLICT (id) DO UPDATE SET harness_session_id = ?3, project_id = ?4, cwd = ?5, title = ?6, status = ?7,
                        status_since = ?8, open_question = ?9, model = ?10, effort = ?11, cost_usd = ?14, usage_json = ?15,
                        ended_at = ?17, last_seq = ?18",
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
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Applies a change to a session record, stores it, and pushes it to the UI.
    async fn update(&self, app: &App, id: &str, f: impl FnOnce(&mut Session)) -> Result<Session> {
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
        let (mut running, mut pending) = (0, 0);
        for s in self.inner.records.lock().unwrap().values() {
            if s.status.is_running() {
                running += 1;
            } else if s.status.is_pending() {
                pending += 1;
            }
        }
        app.status.update(|st| {
            st.sessions.running = running;
            st.sessions.pending = pending;
        });
    }

    pub async fn events(&self, app: &App, id: &str, after: i64) -> Result<Vec<SessionEvent>> {
        let id = id.to_string();
        app.state_db
            .call(move |c| {
                let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq > ?2 ORDER BY seq")?;
                let rows = st
                    .query_map(params![id, after], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?
                    .filter_map(|r| r.ok())
                    .filter_map(|(seq, ts, json)| {
                        serde_json::from_str::<EventBody>(&json).ok().map(|body| SessionEvent { session_id: id.clone(), seq, ts, body })
                    })
                    .collect();
                Ok(rows)
            })
            .await
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
        let prompt = render_prompt(spec, &context, &config.presets);
        let (cwd, project) = match &spec.target {
            Target::Project { project_id } => {
                let p = crate::projects::get(app, project_id).await?.context("the project no longer exists")?;
                (Some(p.path.clone()), Some(p))
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
        let harness = spec.harness.context("pick Claude Code or Codex")?;
        if spec.target == Target::None {
            bail!("pick a project or a temporary folder");
        }
        let preview = self.preview(app, &spec).await?;
        if preview.prompt.trim().is_empty() {
            bail!("the prompt is empty");
        }
        let (cwd, project_id) = match &spec.target {
            Target::Temp => {
                let p = crate::projects::create_temp(app, &spec.text).await?;
                (p.path, Some(p.id))
            }
            Target::Project { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
            Target::None => unreachable!(),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let session = Session {
            id: id.clone(),
            harness,
            harness_session_id: if harness == HarnessKind::Claude { Some(id.clone()) } else { None },
            project_id,
            cwd: cwd.clone(),
            title: preview.title.clone(),
            status: SessionStatus::Starting,
            status_since: now,
            live: false,
            open_question: false,
            model: spec.model.clone(),
            effort: spec.effort.clone(),
            prompt: preview.prompt.clone(),
            usage: Usage::default(),
            started_at: now,
            ended_at: None,
            last_seq: 0,
            launch_spec: spec.clone(),
        };
        self.inner.records.lock().unwrap().insert(id.clone(), session.clone());
        self.persist(app, &session).await?;
        app.bus.publish("sessions", "updated", serde_json::to_value(&session)?);
        self.publish_counts(app);

        let start = StartSpec {
            session_id: id.clone(),
            harness_session_id: session.harness_session_id.clone(),
            resume: false,
            cwd: cwd.into(),
            title: preview.title,
            model: spec.model,
            effort: spec.effort,
            add_dirs: preview.add_dirs,
        };
        let sink = self.sink(app, &id);
        match self.attach(app, harness, start, sink.clone()).await {
            Ok(live) => {
                let client_id = uuid::Uuid::new_v4().to_string();
                sink.emit(EventBody::UserMessage { text: preview.prompt.clone(), client_id: client_id.clone(), queued: false }).await;
                sink.set_status(SessionStatus::Working).await;
                if let Err(e) = live.send(&preview.prompt, &client_id).await {
                    sink.fail(&format!("{e:#}")).await;
                }
            }
            Err(e) => sink.fail(&format!("could not start {}: {e:#}", harness.display())).await,
        }
        Ok(self.get(&id).unwrap())
    }

    async fn attach(&self, app: &App, harness: HarnessKind, start: StartSpec, mut sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let id = start.session_id.clone();
        let instance = self.inner.next_instance.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sink.instance = instance;
        let live: Arc<dyn LiveSession> = match harness {
            HarnessKind::Claude => crate::harness::claude::start(app, start, sink.clone()).await?,
            HarnessKind::Codex => crate::harness::codex::start(app, start, sink.clone()).await?,
        };
        self.inner.live.lock().unwrap().insert(id.clone(), (instance, live.clone()));
        self.update(app, &id, |s| s.live = true).await?;
        Ok(live)
    }

    /// Starts a process for a session that has none (resume).
    async fn reattach(&self, app: &App, id: &str) -> Result<Arc<dyn LiveSession>> {
        if let Some(l) = self.live(id) {
            return Ok(l);
        }
        let s = self.get(id).context("no such session")?;
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
        };
        if !start.cwd.exists() {
            bail!("the folder {} no longer exists", s.cwd);
        }
        let live = self.attach(app, s.harness, start, self.sink(app, id)).await?;
        Ok(live)
    }

    pub async fn send(&self, app: &App, id: &str, text: &str) -> Result<()> {
        let s = self.get(id).context("no such session")?;
        if s.status == SessionStatus::InTerminal {
            bail!("the session is open in a terminal: re-attach it first");
        }
        let resumed = self.live(id).is_none();
        let live = self.reattach(app, id).await?;
        let sink = self.sink(app, id);
        if resumed {
            sink.emit(EventBody::Notice { text: "Resumed".into() }).await;
        }
        let client_id = uuid::Uuid::new_v4().to_string();
        let queued = s.status.is_running() && !resumed;
        sink.emit(EventBody::UserMessage { text: text.to_string(), client_id: client_id.clone(), queued }).await;
        if !queued {
            sink.set_status(SessionStatus::Working).await;
        }
        live.send(text, &client_id).await
    }

    pub async fn interrupt(&self, _app: &App, id: &str) -> Result<()> {
        self.live(id).context("the session has no running process")?.interrupt().await
    }

    pub async fn answer(&self, app: &App, id: &str, request_id: &str, answers: Vec<String>) -> Result<()> {
        let live = self.live(id).context("the session has no running process")?;
        live.answer(request_id, answers.clone()).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::AskAnswered { request_id: request_id.to_string(), answers }).await;
        sink.set_open_question(false).await;
        sink.set_status(SessionStatus::Working).await;
        Ok(())
    }

    async fn detach(&self, id: &str) {
        let live = self.inner.live.lock().unwrap().remove(id);
        if let Some((_, l)) = live {
            if let Err(e) = l.stop().await {
                tracing::warn!("stop {id}: {e:#}");
            }
        }
    }

    /// Stops the process. The conversation stays resumable.
    pub async fn close(&self, app: &App, id: &str) -> Result<()> {
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
    pub async fn resume(&self, app: &App, id: &str) -> Result<()> {
        let s = self.get(id).context("no such session")?;
        if s.status == SessionStatus::InTerminal && crate::terminal::tui_running(app, &s) {
            bail!("the session is still open in a terminal: close it there first");
        }
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

    /// Hands the session to the terminal chosen in Settings.
    pub async fn open_terminal(&self, app: &App, id: &str) -> Result<()> {
        let s = self.get(id).context("no such session")?;
        if s.status.is_running() {
            bail!("wait for the turn to end, or stop it first");
        }
        if s.harness_session_id.is_none() {
            bail!("the session has no conversation yet");
        }
        self.detach(id).await;
        self.update(app, id, |s| {
            s.live = false;
            s.open_question = false;
        })
        .await?;
        crate::terminal::open(app, &s).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::Notice { text: "Opened in the terminal".into() }).await;
        sink.set_status(SessionStatus::InTerminal).await;
        Ok(())
    }

    /// Stops and forgets the sessions running in a folder (temporary project clean-up).
    pub async fn close_in(&self, app: &App, path: &str) -> Result<()> {
        let ids: Vec<String> =
            self.list().into_iter().filter(|s| s.cwd == path || s.cwd.starts_with(&format!("{path}/"))).map(|s| s.id).collect();
        for id in ids {
            if self.get(&id).is_some_and(|s| s.live || s.status.is_running() || s.status.is_pending()) {
                self.close(app, &id).await?;
            }
        }
        Ok(())
    }

    /// After a folder rename (promote): sessions in `old` now run in `new`.
    pub async fn move_cwd(&self, app: &App, old: &str, new: &str) -> Result<()> {
        let ids: Vec<String> = self.list().into_iter().filter(|s| s.cwd == old || s.cwd.starts_with(&format!("{old}/"))).map(|s| s.id).collect();
        for id in ids {
            let (old, new) = (old.to_string(), new.to_string());
            self.update(app, &id, move |s| s.cwd = format!("{new}{}", &s.cwd[old.len()..])).await?;
        }
        Ok(())
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
                None => sink.set_status(SessionStatus::Interrupted).await,
            }
        } else if let Some(e) = error {
            sink.emit(EventBody::Error { message: e }).await;
        }
    }
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

    pub fn status(&self) -> Option<SessionStatus> {
        self.sessions.get(&self.id).map(|s| s.status)
    }

    pub async fn set_status(&self, status: SessionStatus) {
        let Some(before) = self.sessions.get(&self.id) else { return };
        if before.status == status {
            return;
        }
        let _ = self
            .sessions
            .update(&self.app, &self.id, |s| {
                s.status = status;
                s.status_since = now_ms();
            })
            .await;
        self.emit(EventBody::Status { status }).await;
        self.sessions.publish_counts(&self.app);
        if before.status.is_running() && status.is_pending() && !self.sessions.is_watched(&self.id) {
            let s = self.sessions.get(&self.id).unwrap();
            crate::notify::session_pending(&self.app, &s);
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
        let _ = self.sessions.update(&self.app, &self.id, |s| f(&mut s.usage)).await;
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
    let head: String = s.chars().take(max * 2 / 3).collect();
    let tail: String = s.chars().rev().take(max / 3).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{head}\n… {} characters omitted …\n{tail}", s.len() - head.len() - tail.len())
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

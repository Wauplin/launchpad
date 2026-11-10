//! Launchpad's own failures, for diagnosis and the `failure` routine trigger: failed or interrupted sessions,
//! failed Burst jobs, routine runs that could not start (or were skipped after waiting an hour for a quota
//! reading), failed scheduled launches, runners that went offline with running work, quota readings that hold
//! automatic work, and Home restarts (`daemon_start`, written at every start and clean stop).
//!
//! Home checks every 30 s (`collect`) and keeps a feed of the last failures in memory. Failures with their own
//! time (sessions, Burst runs, routine runs, restarts) are found again after a restart; the others (scheduled
//! launches, runners) count from the first check after the start. Routines with a `failure` trigger read the feed
//! after their watermark (`CronAgent::failure_seen`), claimed before the run like the GitHub trigger.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use launchpad_core::model::{EventBody, SessionStatus};
use launchpad_core::{App, util::now_ms};

/// What failed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// An agent session ended Failed or Interrupted.
    Session,
    /// A Burst job (HF Job) failed.
    Burst,
    /// A routine run could not start.
    Routine,
    /// A scheduled launch failed.
    Schedule,
    /// A runner went offline while sessions ran on it.
    Runner,
    /// Home started again (an update, a restart or a crash).
    HomeRestart,
    /// Automatic work has waited for a quota reading of a login for 15 minutes (`quota::holds`).
    Quota,
    /// Every kind (routine triggers only).
    Any,
}

impl FailureKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "session failed",
            Self::Burst => "Burst job failed",
            Self::Routine => "routine run failed",
            Self::Schedule => "scheduled launch failed",
            Self::Runner => "runner offline",
            Self::HomeRestart => "Home restarted",
            Self::Quota => "quota reading missing",
            Self::Any => "failure",
        }
    }
}

/// One failure, with the ids to investigate it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Failure {
    /// Stable key, so a failure is reported once.
    pub key: String,
    pub kind: FailureKind,
    /// When it happened (Unix ms).
    pub at: i64,
    /// When Home noticed it (Unix ms): `at` when the failure carries its own time.
    pub seen_at: i64,
    /// None: it concerns every workspace (runners, Home, Burst jobs without a session).
    pub workspace_id: Option<String>,
    pub title: String,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    /// The bot whose thread the failed session belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
}

/// How far back failures with their own time are found (after a restart too).
const LOOKBACK_MS: i64 = 24 * 3600 * 1000;
const FEED_MAX: usize = 300;

#[derive(Default)]
struct Feed {
    failures: Vec<Failure>,
    keys: HashSet<String>,
    /// The first check after the start primes the failures without their own time.
    primed: bool,
    /// Runners connected at the last check, with their running sessions.
    connected: HashMap<String, Vec<String>>,
}

static FEED: Mutex<Option<Feed>> = Mutex::new(None);

fn with_feed<T>(f: impl FnOnce(&mut Feed) -> T) -> T {
    let mut g = FEED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Feed::default))
}

/// The failures noticed since `since` (Unix ms, by `seen_at`), oldest first.
pub fn recent(since: i64) -> Vec<Failure> {
    with_feed(|f| f.failures.iter().filter(|x| x.seen_at > since).cloned().collect())
}

// ------------------------------------------------------------------ starts

static STARTED_AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

/// One start of this daemon.
#[derive(Clone, Debug, Serialize)]
pub struct Start {
    pub at: i64,
    pub commit: String,
    pub role: String,
    /// When it stopped cleanly; None for the current run, or a crash, kill or power loss.
    pub stopped_at: Option<i64>,
}

/// Records this daemon's start (once per process).
pub async fn record_start(app: &App) -> Result<()> {
    let at = *STARTED_AT.get_or_init(now_ms);
    let (commit, role) = (launchpad_core::commit().to_string(), app.paths.role.name().to_string());
    app.state_db
        .call(move |c| {
            c.execute("INSERT OR IGNORE INTO daemon_start (at, commit_id, role) VALUES (?1, ?2, ?3)", rusqlite::params![at, commit, role])?;
            c.execute("DELETE FROM daemon_start WHERE at NOT IN (SELECT at FROM daemon_start ORDER BY at DESC LIMIT 100)", [])?;
            Ok(())
        })
        .await
}

/// Records a clean stop of this daemon (after the server stopped).
pub async fn record_stop(app: &App) -> Result<()> {
    let Some(at) = STARTED_AT.get().copied() else { return Ok(()) };
    app.state_db.call(move |c| Ok(c.execute("UPDATE daemon_start SET stopped_at = ?2 WHERE at = ?1", rusqlite::params![at, now_ms()])?)).await.map(|_| ())
}

/// When this process started (Unix ms).
pub fn started_at() -> Option<i64> {
    STARTED_AT.get().copied()
}

/// The last starts, newest first.
pub async fn starts(app: &App) -> Result<Vec<Start>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT at, commit_id, role, stopped_at FROM daemon_start ORDER BY at DESC LIMIT 50")?;
            let rows = st.query_map([], |r| Ok(Start { at: r.get(0)?, commit: r.get(1)?, role: r.get(2)?, stopped_at: r.get(3)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

/// Why a start happened, from the run before it.
pub fn restart_reason(previous: Option<&Start>) -> String {
    match previous {
        None => "first start".into(),
        Some(p) if p.stopped_at.is_some() => format!("the previous run (build {}) stopped cleanly", short(&p.commit)),
        Some(p) => format!("the previous run (build {}) did not stop cleanly: a crash, a kill or the machine stopped", short(&p.commit)),
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

// ------------------------------------------------------------------ collection

/// The last error a session reported (its Error or Notice events), for its failure reason.
pub async fn session_reason(app: &App, s: &launchpad_core::model::Session) -> Option<String> {
    let events = app.sessions.events(app, &s.id, (s.last_seq - 40).max(0)).await.ok()?;
    events.iter().rev().find_map(|e| match &e.body {
        EventBody::Error { message } => Some(message.clone()),
        _ => None,
    })
}

/// Finds new failures and adds them to the feed. Home only.
pub async fn collect(app: &App) -> Result<Vec<Failure>> {
    let now = now_ms();
    let floor = now - LOOKBACK_MS;
    let projects = launchpad_core::projects::list(app).await?;
    let workspace_of = |s: &launchpad_core::model::Session| {
        s.project_id.as_ref().and_then(|f| projects.iter().find(|p| &p.id == f)).and_then(|p| p.workspace_id.clone()).or_else(|| s.launch_spec.workspace_id.clone())
    };
    let known = |key: &str| with_feed(|f| f.keys.contains(key));
    let mut found: Vec<Failure> = vec![];
    let base = |key: String, kind: FailureKind, at: i64, seen_at: i64, title: String| Failure { key, kind, at, seen_at, workspace_id: None, title, reason: None, session_id: None, runner_id: None, routine_id: None, schedule_id: None, bot_id: None };

    let sessions = app.sessions.list();
    let burst = launchpad_core::burst::runs(app).await.unwrap_or_default();
    for s in sessions.iter().filter(|s| matches!(s.status, SessionStatus::Error | SessionStatus::Interrupted) && s.status_since > floor) {
        let key = format!("session:{}:{}", s.id, s.status_since);
        if known(&key) {
            continue;
        }
        let what = if s.status == SessionStatus::Error { "failed" } else { "was interrupted" };
        let mut f = base(key, FailureKind::Session, s.status_since, s.status_since, format!("“{}” {what}", s.title.chars().take(80).collect::<String>()));
        f.reason = session_reason(app, s).await;
        if let Some(run) = burst.iter().find(|r| r.session_id.as_deref() == Some(&s.id) && r.error.is_some()) {
            f.reason = Some(format!("{}{}Burst {}: {}", f.reason.as_deref().unwrap_or(""), if f.reason.is_some() { "; " } else { "" }, run.runner_id, run.error.as_deref().unwrap_or_default()));
        }
        f.workspace_id = workspace_of(s);
        f.session_id = Some(s.id.clone());
        f.runner_id = s.runner_id.clone();
        f.bot_id = s.launch_spec.bot.as_ref().map(|b| b.id.clone());
        found.push(f);
    }
    for r in burst.iter().filter(|r| r.state == launchpad_core::burst::BurstState::Failed) {
        let at = r.stopped_at.unwrap_or(r.created_at);
        let key = format!("burst:{}", r.runner_id);
        if at <= floor || known(&key) {
            continue;
        }
        let mut f = base(key, FailureKind::Burst, at, at, format!("Burst job {} failed", r.job_id.as_deref().unwrap_or(&r.runner_id)));
        f.reason = r.error.clone();
        f.runner_id = Some(r.runner_id.clone());
        if let Some(s) = r.session_id.as_ref().and_then(|id| sessions.iter().find(|s| &s.id == id)) {
            f.workspace_id = workspace_of(s);
            f.bot_id = s.launch_spec.bot.as_ref().map(|b| b.id.clone());
        }
        f.session_id = r.session_id.clone();
        found.push(f);
    }
    for r in crate::crons::list(app).await? {
        for run in r.history.iter().filter(|x| (x.outcome == crate::crons::RunOutcome::Failed || x.missed) && x.detail.is_some() && x.at > floor) {
            let key = format!("routine:{}:{}", r.id, run.at);
            if known(&key) {
                continue;
            }
            let what = if run.missed { "was skipped" } else { "could not run" };
            let mut f = base(key, FailureKind::Routine, run.at, run.at, format!("Routine “{}” {what} ({})", r.title, run.cause));
            f.reason = run.detail.clone();
            f.workspace_id = r.spec.workspace_id.clone();
            f.routine_id = Some(r.id.clone());
            f.bot_id = r.bot_id.clone();
            f.session_id = run.session_id.clone();
            found.push(f);
        }
    }
    let starts = starts(app).await.unwrap_or_default();
    for (i, s) in starts.iter().enumerate().filter(|(_, s)| s.at > floor && s.role != "runner") {
        let key = format!("home:{}", s.at);
        if known(&key) {
            continue;
        }
        let mut f = base(key, FailureKind::HomeRestart, s.at, s.at, format!("Home started (build {})", short(&s.commit)));
        f.reason = Some(restart_reason(starts.get(i + 1)));
        // The first start of a new Home is not a restart.
        if starts.get(i + 1).is_some() {
            found.push(f);
        }
    }
    // Quota readings that hold automatic work. In memory (`quota::holds`): a hold counts from Home's start.
    let quotas = app.status.get().quotas;
    for (login, hold) in launchpad_core::quota::holds(now).into_iter().filter(|(_, h)| now - h.since >= QUOTA_HOLD_MS) {
        let key = format!("quota:{login}:{}", hold.since);
        if known(&key) {
            continue;
        }
        let q = quotas.get(&login).cloned().unwrap_or_default();
        let mut f = base(key, FailureKind::Quota, hold.since, now, format!("Automatic work has waited {} min for a quota reading of {}", (now - hold.since) / 60_000, launchpad_core::quota::login_label(&login, &q)));
        f.reason = Some(launchpad_core::quota::reading_detail(&q, now));
        found.push(f);
    }
    // Failures without their own time: only the changes after the first check count.
    let primed = with_feed(|f| f.primed);
    for l in crate::schedules::list(app).await?.into_iter().filter(|l| l.status == crate::schedules::ScheduleStatus::Failed) {
        let key = format!("schedule:{}", l.id);
        if known(&key) {
            continue;
        }
        let mut f = base(key, FailureKind::Schedule, now, now, format!("Scheduled launch “{}” failed", l.title));
        f.reason = l.error.clone();
        f.schedule_id = Some(l.id.clone());
        f.workspace_id = match &l.spec.target {
            launchpad_core::model::Target::Project { project_id } | launchpad_core::model::Target::Predraft { project_id } => projects.iter().find(|p| &p.id == project_id).and_then(|p| p.workspace_id.clone()),
            _ => l.spec.workspace_id.clone(),
        };
        if primed {
            found.push(f);
        } else {
            with_feed(|feed| feed.keys.insert(f.key.clone()));
        }
    }
    let runners = launchpad_core::runners::list(app).await.unwrap_or_default();
    let mut connected: HashMap<String, Vec<String>> = HashMap::new();
    for r in runners.iter().filter(|r| r.connected) {
        let running = sessions.iter().filter(|s| s.runner_id.as_deref() == Some(&r.runner.id) && (s.status.is_running() || s.status == SessionStatus::Starting)).map(|s| s.id.clone()).collect();
        connected.insert(r.runner.id.clone(), running);
    }
    let before = with_feed(|f| std::mem::replace(&mut f.connected, connected.clone()));
    for (id, running) in before.into_iter().filter(|(id, running)| !connected.contains_key(id) && !running.is_empty()) {
        let Some(r) = runners.iter().find(|r| r.runner.id == id) else { continue };
        let mut f = base(format!("runner:{id}:{now}"), FailureKind::Runner, now, now, format!("Runner {} went offline with {} running session(s)", r.runner.name, running.len()));
        f.reason = Some(format!("sessions: {}", running.join(", ")));
        f.runner_id = Some(id);
        if r.runner.burst {
            f.kind = FailureKind::Burst;
            f.title = format!("Burst runner {} disconnected while its session ran", r.runner.id);
        }
        found.push(f);
    }
    found.sort_by_key(|f| f.seen_at);
    with_feed(|feed| {
        feed.primed = true;
        for f in &found {
            feed.keys.insert(f.key.clone());
        }
        feed.failures.extend(found.iter().cloned());
        feed.failures.sort_by_key(|f| f.seen_at);
        let extra = feed.failures.len().saturating_sub(FEED_MAX);
        feed.failures.drain(..extra);
    });
    for f in &found {
        tracing::info!("failure noticed: {}: {}{}", f.kind.label(), f.title, f.reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default());
    }
    Ok(found)
}

/// How long automatic work waits for a quota reading before it is a failure (and a health problem).
pub const QUOTA_HOLD_MS: i64 = 15 * 60_000;

/// Whether a routine with these trigger kinds, in this workspace, takes the failure. A routine never takes the
/// failures of its own runs or of its own bot's thread.
pub fn routine_takes(events: &[FailureKind], workspace: Option<&str>, routine_id: &str, bot_id: Option<&str>, runs: &[String], f: &Failure) -> bool {
    let kind = events.iter().any(|e| *e == FailureKind::Any || *e == f.kind);
    let place = f.workspace_id.is_none() || f.workspace_id.as_deref() == workspace;
    let own = f.routine_id.as_deref() == Some(routine_id)
        || f.session_id.as_ref().is_some_and(|s| runs.contains(s))
        || (bot_id.is_some() && f.bot_id.as_deref() == bot_id);
    kind && place && !own
}

/// The line a run's message carries for one failure.
pub fn describe(f: &Failure) -> String {
    let mut ids = vec![];
    if let Some(s) = &f.session_id { ids.push(format!("session {s}")); }
    if let Some(r) = &f.runner_id { ids.push(format!("runner {r}")); }
    if let Some(r) = &f.routine_id { ids.push(format!("routine {r}")); }
    if let Some(s) = &f.schedule_id { ids.push(format!("scheduled launch {s}")); }
    let ids = if ids.is_empty() { String::new() } else { format!(" [{}]", ids.join(", ")) };
    let reason = f.reason.as_deref().map(|r| format!(": {}", r.chars().take(600).collect::<String>())).unwrap_or_default();
    format!("- {} ({}){ids}{reason}", f.title.replace('"', "'"), f.kind.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(kind: FailureKind, workspace: Option<&str>) -> Failure {
        Failure { key: "k".into(), kind, at: 1, seen_at: 1, workspace_id: workspace.map(str::to_string), title: "t".into(), reason: Some("r".into()), session_id: Some("s1".into()), runner_id: None, routine_id: None, schedule_id: None, bot_id: Some("b1".into()) }
    }

    #[test]
    fn routines_take_failures_of_their_workspace_and_never_their_own() {
        let f = failure(FailureKind::Session, Some("w1"));
        assert!(routine_takes(&[FailureKind::Session], Some("w1"), "r", None, &[], &f));
        assert!(routine_takes(&[FailureKind::Any], Some("w1"), "r", Some("b2"), &[], &f));
        assert!(!routine_takes(&[FailureKind::Burst], Some("w1"), "r", None, &[], &f));
        assert!(!routine_takes(&[FailureKind::Session], Some("w2"), "r", None, &[], &f));
        // Its own bot's thread and its own runs never trigger it.
        assert!(!routine_takes(&[FailureKind::Any], Some("w1"), "r", Some("b1"), &[], &f));
        assert!(!routine_takes(&[FailureKind::Any], Some("w1"), "r", None, &["s1".into()], &f));
        // Shared failures (runners, Home) reach every workspace.
        assert!(routine_takes(&[FailureKind::HomeRestart], Some("w2"), "r", None, &[], &failure(FailureKind::HomeRestart, None)));
        assert!(describe(&f).contains("[session s1]: r"));
    }

    #[test]
    fn restart_reason_names_a_crash() {
        let p = |stopped: Option<i64>| Start { at: 1, commit: "abcdef0123456789".into(), role: "home".into(), stopped_at: stopped };
        assert!(restart_reason(Some(&p(Some(2)))).contains("stopped cleanly"));
        assert!(restart_reason(Some(&p(None))).contains("did not stop cleanly"));
        assert_eq!(restart_reason(None), "first start");
    }
}

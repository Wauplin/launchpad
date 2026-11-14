//! Durable delayed launches. Claim before launching; never retry a claimed launch after a crash.
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use launchpad_core::{
    App,
    model::{HarnessKind, LaunchSpec, Session, Target},
    util::now_ms,
};

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaKind {
    FiveHour,
    Weekly,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchDelay {
    In {
        minutes: u32,
    },
    At {
        at: i64,
    },
    Agents {
        session_ids: Vec<String>,
    },
    Quota {
        harness: HarnessKind,
        window: QuotaKind,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Queued,
    Starting,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ScheduledLaunch {
    pub id: String,
    pub spec: LaunchSpec,
    /// None starts a new session; Some sends spec.text verbatim to that existing session.
    #[serde(default)]
    pub resume_session_id: Option<String>,
    /// All conditions must be met before the launch starts.
    pub conditions: Vec<LaunchDelay>,
    pub title: String,
    pub created_at: i64,
    /// Latest time or quota condition, resolved once at scheduling time (Unix milliseconds).
    pub ready_at: Option<i64>,
    pub status: ScheduleStatus,
    pub error: Option<String>,
}

pub async fn list(app: &App) -> Result<Vec<ScheduledLaunch>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT id, json FROM scheduled_launch ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(crate::decode_rows("scheduled launch", rows, |s| {
                let mut v: serde_json::Value = serde_json::from_str(s)?;
                // Records written before multiple conditions stored a single `delay`.
                if let Some(delay) = v.as_object_mut().and_then(|o| o.remove("delay")) {
                    v["conditions"] = serde_json::json!([delay]);
                }
                Ok(serde_json::from_value(v)?)
            }))
        })
        .await
}

fn changed(app: &App) {
    app.bus
        .publish("schedules", "changed", serde_json::Value::Null);
}

pub async fn create(
    app: &App,
    spec: LaunchSpec,
    conditions: Vec<LaunchDelay>,
) -> Result<ScheduledLaunch> {
    create_target(app, spec, conditions, None).await
}

/// `from`: the agent that schedules it (stamped by Home, `senders`); its text then arrives under its header.
pub async fn follow_up(
    app: &App,
    id: &str,
    message: String,
    conditions: Vec<LaunchDelay>,
    from: Option<launchpad_core::model::MessageFrom>,
) -> Result<ScheduledLaunch> {
    let session = app.sessions.get(id).context("no such session")?;
    anyhow::ensure!(
        !session.status.is_running(),
        "wait for the session to stop first"
    );
    anyhow::ensure!(!message.trim().is_empty(), "the message is empty");
    let mut spec = session.launch_spec;
    spec.harness = Some(session.harness);
    spec.text = message;
    // Not the session's own first sender: the follow-up's.
    spec.from = from;
    // Use the actual session project for workspace filtering, without creating a worktree.
    spec.target = session
        .project_id
        .map(|project_id| Target::Project { project_id })
        .unwrap_or(Target::None);
    create_target(app, spec, conditions, Some(id.to_string())).await
}

/// What an agent asks for with `launchpad sleep`: end its turn now and get a message in the same conversation later.
#[derive(Clone, Debug, Deserialize)]
pub struct SleepRequest {
    pub session_id: String,
    /// Wake in this many minutes…
    #[serde(default)]
    pub minutes: Option<u32>,
    /// …or at this time (Unix ms).
    #[serde(default)]
    pub at: Option<i64>,
    /// What to do when it wakes up, in the agent's words (it comes back in the wake-up message).
    #[serde(default)]
    pub note: String,
}

/// `launchpad sleep --in`: "45" or "45m" minutes, "2h" hours, "1d" days.
pub fn parse_minutes(text: &str) -> Result<u32> {
    let t = text.trim().to_ascii_lowercase();
    let (number, unit) = match t.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => t.split_at(i),
        None => (t.as_str(), "m"),
    };
    let n: u32 = number.parse().map_err(|_| anyhow::anyhow!("--in takes minutes, hours or days: 45m, 2h, 1d"))?;
    let factor = match unit.trim() {
        "m" | "min" | "mins" | "minute" | "minutes" => 1,
        "h" | "hour" | "hours" => 60,
        "d" | "day" | "days" => 1440,
        _ => bail!("--in takes minutes, hours or days: 45m, 2h, 1d"),
    };
    n.checked_mul(factor).filter(|m| *m > 0).context("pick a delay from 1 minute to 1 year")
}

/// `launchpad sleep --until`: an RFC 3339 time ("2026-11-14T15:00:00Z", "2026-11-14T16:00:00+01:00"), as Unix ms.
pub fn parse_until(text: &str) -> Result<i64> {
    Ok(chrono::DateTime::parse_from_rfc3339(text.trim()).map_err(|_| anyhow::anyhow!("--until takes an RFC 3339 time with its offset, such as 2026-11-14T15:00:00Z"))?.timestamp_millis())
}

/// Whether a scheduled follow-up is a wake-up an agent asked for (`sleep`).
pub fn is_wake(record: &ScheduledLaunch) -> bool {
    record.resume_session_id.is_some() && record.spec.from.as_ref().is_some_and(|f| f.kind == launchpad_core::model::SenderKind::Wake)
}

/// The message that wakes an agent: Launchpad's own, with its note.
fn wake_text(until: i64, note: &str) -> String {
    let at = chrono::DateTime::from_timestamp_millis(until).map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default();
    let note = note.trim();
    let mut text = format!("Wake-up · You asked Launchpad to wake you at {at} (`launchpad sleep`). This is the same conversation: continue the task you paused.");
    if !note.is_empty() {
        let f = launchpad_core::senders::fence(note);
        text.push_str(&format!("\n\nYour note:\n\n{f}\n{note}\n{f}"));
    }
    text
}

/// Home: the session asks to sleep (`launchpad sleep`, its own session only: `agent_auth`). Records a wake-up, a
/// scheduled follow-up to the same session sent by the usual scheduler (it waits for the turn to end, for confirmed
/// quota and for a free session), and marks the session as sleeping until then. A new sleep replaces the session's
/// earlier wake-up; a follow-up the owner scheduled is kept and refuses it.
pub async fn sleep(app: &App, q: SleepRequest) -> Result<ScheduledLaunch> {
    let session = app.sessions.get(&q.session_id).context("no such session")?;
    anyhow::ensure!(session.status != launchpad_core::model::SessionStatus::Closed, "this session is archived");
    let condition = match (q.minutes, q.at) {
        (Some(minutes), None) => LaunchDelay::In { minutes },
        (None, Some(at)) => LaunchDelay::At { at },
        _ => bail!("give either --in or --until"),
    };
    anyhow::ensure!(q.note.len() <= 4000, "the note is too long (at most 4000 bytes)");
    for earlier in list(app).await? {
        if earlier.resume_session_id.as_deref() == Some(q.session_id.as_str()) {
            anyhow::ensure!(is_wake(&earlier), "this session already has a follow-up scheduled by the owner; it will receive that message instead");
            anyhow::ensure!(earlier.status != ScheduleStatus::Starting, "the earlier wake-up is being sent now");
            cancel_record(app, &earlier.id).await?;
        }
    }
    let mut spec = session.launch_spec.clone();
    spec.harness = Some(session.harness);
    // Its own message, not a step in a chain: the hop count of the turn that went to sleep, so sleeping never meets
    // the loop guard on its own.
    let hops = launchpad_core::senders::hops_of(app, &session.id).await;
    spec.from = Some(launchpad_core::model::MessageFrom { kind: launchpad_core::model::SenderKind::Wake, id: session.id.clone(), name: "Wake-up".into(), hops, ..Default::default() });
    spec.target = session.project_id.clone().map(|project_id| Target::Project { project_id }).unwrap_or(Target::None);
    let (_, until) = validate(app, &spec, std::slice::from_ref(&condition), Some(&session.id)).await?;
    let until = until.context("no wake-up time")?;
    spec.text = wake_text(until, &q.note);
    let record = create_target(app, spec, vec![condition], Some(session.id.clone())).await?;
    let sleep = launchpad_core::model::Sleep { until, note: q.note.trim().to_string(), schedule_id: record.id.clone() };
    if let Err(e) = app.sessions.set_sleep(app, &session.id, Some(sleep)).await {
        let _ = cancel_record(app, &record.id).await;
        return Err(e);
    }
    Ok(record)
}

/// Home: a message woke the sleeping session before its wake-up (`Sessions::apply_replica` cleared its sleep): the
/// wake-up goes, unless it is being sent now (then it is that message).
pub fn woken(app: &App, session_id: &str, schedule_id: &str) {
    let (app, session_id, schedule_id) = (app.clone(), session_id.to_string(), schedule_id.to_string());
    tokio::spawn(async move {
        let queued = list(&app).await.ok().and_then(|l| l.into_iter().find(|r| r.id == schedule_id)).is_some_and(|r| is_wake(&r) && r.status != ScheduleStatus::Starting);
        if queued && let Err(e) = cancel_record(&app, &schedule_id).await {
            tracing::debug!("wake-up of {session_id} after a message: {e:#}");
        }
    });
}

/// Clears the session's sleep when it names this wake-up (sent, cancelled or failed).
async fn wake(app: &App, record: &ScheduledLaunch) {
    let Some(id) = record.resume_session_id.as_deref().filter(|_| is_wake(record)) else { return };
    if app.sessions.get(id).is_some_and(|s| s.sleep.as_ref().is_some_and(|z| z.schedule_id == record.id))
        && let Err(e) = app.sessions.set_sleep(app, id, None).await
    {
        tracing::warn!("wake-up of {id}: {e:#}");
    }
}

/// Checks a launch and its conditions; returns its title and the latest time or quota condition.
async fn validate(
    app: &App,
    spec: &LaunchSpec,
    conditions: &[LaunchDelay],
    resume_session_id: Option<&String>,
) -> Result<(String, Option<i64>)> {
    let title = if let Some(id) = resume_session_id {
        let s = app.sessions.get(id).context("no such session")?;
        if spec.from.as_ref().is_some_and(|f| f.kind == launchpad_core::model::SenderKind::Wake) { format!("Wake-up · {}", s.title) } else { format!("Follow-up · {}", s.title) }
    } else {
        spec.harness.context("pick a harness")?;
        if spec.target == Target::None {
            bail!("pick a project or a temporary folder");
        }
        let preview = app.sessions.preview(app, spec).await?;
        if preview.prompt.trim().is_empty() {
            bail!("the prompt is empty");
        }
        preview.title
    };
    if conditions.is_empty() {
        bail!("pick at least one start condition");
    }
    let now = now_ms();
    let scheduled = list(app).await?;
    let mut ready_at = None;
    for delay in conditions {
        let at = match delay {
            LaunchDelay::In { minutes } => {
                if *minutes == 0 || *minutes > 525_600 {
                    bail!("pick a delay from 1 minute to 1 year");
                }
                Some(now + i64::from(*minutes) * 60_000)
            }
            LaunchDelay::At { at } => {
                if *at <= now || *at > now + 366 * 86_400_000 {
                    bail!("pick a future date within the next year");
                }
                Some(*at)
            }
            LaunchDelay::Agents { session_ids } => {
                if session_ids.is_empty() {
                    bail!("pick at least one agent");
                }
                // Scheduled ids become session ids at launch, so dependencies need no rewriting.
                for id in session_ids {
                    if app.sessions.get(id).is_none()
                        && !scheduled
                            .iter()
                            .any(|s| &s.id == id && s.resume_session_id.is_none())
                    {
                        bail!("a selected agent no longer exists");
                    }
                }
                None
            }
            LaunchDelay::Quota { harness, window } => {
                anyhow::ensure!(
                    launchpad_core::harness::registration(*harness)
                        .descriptor
                        .capabilities
                        .quota,
                    "This harness has no monitored subscription quota"
                );
                let h = if spec.harness == Some(*harness) { launchpad_core::quota::reading_for(app, *harness, spec) } else { launchpad_core::quota::reading(app, harness.as_str()) };
                if h.quota_error.is_some() || h.quota_at.is_none_or(|at| now - at > 15 * 60_000) {
                    bail!("quota reset is unavailable; wait for a fresh quota reading");
                }
                let q = match window {
                    QuotaKind::FiveHour => h.five_hour,
                    QuotaKind::Weekly => h.weekly,
                }
                .context("quota reset is unknown")?;
                let at = q
                    .resets_at
                    .context("this quota window has not started, so it has no reset")?
                    .checked_mul(1000)
                    .context("invalid quota reset")?;
                if at <= now {
                    bail!("wait for the next quota reading before scheduling its reset");
                }
                Some(at)
            }
        };
        ready_at = ready_at.max(at);
    }
    Ok((title, ready_at))
}

async fn create_target(
    app: &App,
    spec: LaunchSpec,
    conditions: Vec<LaunchDelay>,
    resume_session_id: Option<String>,
) -> Result<ScheduledLaunch> {
    let (title, ready_at) = validate(app, &spec, &conditions, resume_session_id.as_ref()).await?;
    let now = now_ms();
    let scheduled = list(app).await?;
    if let Some(id) = &resume_session_id {
        anyhow::ensure!(
            !scheduled
                .iter()
                .any(|s| s.resume_session_id.as_ref() == Some(id)),
            "this session already has a follow-up"
        );
        anyhow::ensure!(
            !conditions.iter().any(
                |c| matches!(c, LaunchDelay::Agents { session_ids } if session_ids.contains(id))
            ),
            "a follow-up cannot wait for its own session"
        );
    }
    let launch = ScheduledLaunch {
        id: uuid::Uuid::new_v4().to_string(),
        spec,
        conditions,
        title,
        resume_session_id,
        created_at: now,
        ready_at,
        status: ScheduleStatus::Queued,
        error: None,
    };
    let record = launch.clone();
    app.state_db
        .call(move |c| {
            if let Some(id) = &record.resume_session_id {
                let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM scheduled_launch WHERE json_extract(json, '$.resume_session_id') = ?1)", [id], |r| r.get(0))?;
                anyhow::ensure!(!exists, "this session already has a follow-up");
            }
            c.execute(
                "INSERT INTO scheduled_launch (id, json) VALUES (?1, ?2)",
                params![record.id, serde_json::to_string(&record)?],
            )?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(launch)
}

/// The DB actor serializes claims and cancellation, including simultaneous UI/scheduler requests.
async fn claim(app: &App, id: &str, manual: bool) -> Result<ScheduledLaunch> {
    let id = id.to_string();
    let record = app
        .state_db
        .call(move |c| {
            let json: String = c
                .query_row(
                    "SELECT json FROM scheduled_launch WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )
                .context("this scheduled launch no longer exists")?;
            let mut record: ScheduledLaunch = serde_json::from_str(&json)?;
            if record.status == ScheduleStatus::Starting
                || (!manual && record.status != ScheduleStatus::Queued)
            {
                bail!("this launch is already being started");
            }
            record.status = ScheduleStatus::Starting;
            record.error = None;
            c.execute(
                "UPDATE scheduled_launch SET json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&record)?],
            )?;
            Ok(record)
        })
        .await?;
    changed(app);
    Ok(record)
}

/// Replaces the launch spec and start conditions of a scheduled agent that has not started yet. The id stays,
/// so agents that wait for it keep waiting. `conditions: None` keeps the current ones.
pub async fn update(
    app: &App,
    id: &str,
    spec: LaunchSpec,
    conditions: Option<Vec<LaunchDelay>>,
) -> Result<ScheduledLaunch> {
    let scheduled = list(app).await?;
    let current = scheduled
        .iter()
        .find(|s| s.id == id)
        .context("this scheduled launch no longer exists")?;
    anyhow::ensure!(
        current.resume_session_id.is_none(),
        "a follow-up cannot be edited; cancel it and schedule it again"
    );
    anyhow::ensure!(
        current.status != ScheduleStatus::Starting && app.sessions.get(id).is_none(),
        "this agent already started"
    );
    let conditions = conditions.unwrap_or_else(|| current.conditions.clone());
    // Waiting for itself, directly or through other scheduled agents, would never start.
    let mut pending: Vec<&String> = conditions
        .iter()
        .flat_map(|c| match c {
            LaunchDelay::Agents { session_ids } => session_ids.iter().collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(dep) = pending.pop() {
        anyhow::ensure!(dep != id, "a scheduled agent cannot wait for itself");
        if !seen.insert(dep) {
            continue;
        }
        if let Some(other) = scheduled.iter().find(|s| &s.id == dep) {
            for c in &other.conditions {
                if let LaunchDelay::Agents { session_ids } = c {
                    pending.extend(session_ids);
                }
            }
        }
    }
    let (title, ready_at) = validate(app, &spec, &conditions, None).await?;
    let id = id.to_string();
    let launch = app
        .state_db
        .call(move |c| {
            let json: String = c
                .query_row(
                    "SELECT json FROM scheduled_launch WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )
                .context("this scheduled launch no longer exists")?;
            let mut record: ScheduledLaunch = serde_json::from_str(&json)?;
            if record.status == ScheduleStatus::Starting {
                bail!("this agent already started");
            }
            record.spec = spec;
            record.conditions = conditions;
            record.title = title;
            record.ready_at = ready_at;
            record.status = ScheduleStatus::Queued;
            record.error = None;
            c.execute(
                "UPDATE scheduled_launch SET json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&record)?],
            )?;
            Ok(record)
        })
        .await?;
    changed(app);
    Ok(launch)
}

pub async fn cancel(app: &App, id: &str) -> Result<()> {
    let record = cancel_record(app, id).await?;
    wake(app, &record).await;
    Ok(())
}

/// Deletes a scheduled launch that is not starting; returns it.
async fn cancel_record(app: &App, id: &str) -> Result<ScheduledLaunch> {
    let id = id.to_string();
    let record = app.state_db
        .call(move |c| {
            let json: String = c
                .query_row(
                    "SELECT json FROM scheduled_launch WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )
                .context("this scheduled launch no longer exists")?;
            let record: ScheduledLaunch = serde_json::from_str(&json)?;
            if record.status == ScheduleStatus::Starting {
                bail!("this agent is already starting; use Stop in its session");
            }
            c.execute("DELETE FROM scheduled_launch WHERE id = ?1", [id])?;
            Ok(record)
        })
        .await?;
    changed(app);
    Ok(record)
}

async fn finish(app: &App, mut record: ScheduledLaunch, error: Option<String>) -> Result<()> {
    // Sent or failed: the agent sleeps no more (a failed wake-up shows on the session and in Today).
    wake(app, &record).await;
    app.state_db
        .call(move |c| {
            if let Some(error) = error {
                record.status = ScheduleStatus::Failed;
                record.error = Some(error);
                c.execute(
                    "UPDATE scheduled_launch SET json = ?2 WHERE id = ?1",
                    params![record.id, serde_json::to_string(&record)?],
                )?;
            } else {
                c.execute("DELETE FROM scheduled_launch WHERE id = ?1", [record.id])?;
            }
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

async fn run(app: &App, record: ScheduledLaunch, automatic: bool) -> Result<Session> {
    // A stable session id makes crash recovery unambiguous, even before the harness attaches.
    let result = if let Some(id) = &record.resume_session_id {
        send_follow_up(app, id, &record, automatic)
            .await
            .and_then(|()| {
                app.sessions
                    .get(id)
                    .context("session disappeared after sending")
            })
    } else {
        match app.sessions.get(&record.id) {
            Some(s) => Ok(s),
            None => {
                launchpad_core::runners::launch_anywhere(app, record.spec.clone(), record.id.clone(), automatic).await
            }
        }
    };
    if result.as_ref().err().is_some_and(|e| {
        e.is::<launchpad_core::sessions::FollowUpBusy>()
    }) {
        // A concurrent turn is temporary: preserve this queued work.
        let mut record = record;
        record.status = ScheduleStatus::Queued;
        app.state_db
            .call(move |c| {
                c.execute(
                    "UPDATE scheduled_launch SET json = ?2 WHERE id = ?1",
                    params![record.id, serde_json::to_string(&record)?],
                )?;
                Ok(())
            })
            .await?;
        changed(app);
    } else {
        finish(app, record, result.as_ref().err().map(|e| format!("{e:#}"))).await?;
    }
    result
}

/// Sends a follow-up. Home places it like every message (`runners::continue_anywhere`: the session's runner,
/// else another one, else Burst); a busy session answers 409, which keeps the follow-up queued. No idempotency
/// key: a run is claimed once (never retried), and a queued one is sent again later.
async fn send_follow_up(app: &App, id: &str, record: &ScheduledLaunch, automatic: bool) -> Result<()> {
    if app.paths.role != launchpad_core::role::Role::Home {
        let text = record.spec.from.as_ref().map_or_else(|| record.spec.text.clone(), |f| launchpad_core::senders::wrap(f, &record.spec.text));
        return app.sessions.send_follow_up(app, id, &text, automatic, record.spec.from.clone()).await;
    }
    // An agent's follow-up names it (`context.from`): `runners::deliver` puts its text under its header.
    let context = launchpad_core::model::MessageContext { from: record.spec.from.clone(), ..Default::default() };
    let body = serde_json::json!({"text": record.spec.text, "follow_up": if automatic { "automatic" } else { "manual" }, "context": context}).to_string();
    let (status, answer) = launchpad_core::runners::continue_anywhere(app, id, "input", Some(body), None).await?;
    if status == 409 {
        return Err(launchpad_core::sessions::FollowUpBusy.into());
    }
    if !(200..300).contains(&status) {
        let error = serde_json::from_str::<serde_json::Value>(&answer).ok().and_then(|v| v["error"].as_str().map(str::to_string));
        bail!("{}", error.unwrap_or(answer));
    }
    Ok(())
}

pub async fn start_now(app: &App, id: &str) -> Result<Session> {
    let record = claim(app, id, true).await?;
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { run(&app, record, false).await }).await?
}

fn ready(record: &ScheduledLaunch, now: i64, done: impl Fn(&str) -> bool) -> bool {
    if record.status != ScheduleStatus::Queued {
        return false;
    }
    !record.conditions.is_empty()
        && record.ready_at.is_none_or(|at| at <= now)
        && record.conditions.iter().all(|c| match c {
            LaunchDelay::Agents { session_ids } => {
                !session_ids.is_empty() && session_ids.iter().all(|id| done(id))
            }
            _ => true,
        })
}

pub async fn recover(app: &App) -> Result<()> {
    for record in list(app).await? {
        if record.status == ScheduleStatus::Starting {
            let error = if record.resume_session_id.is_none()
                && app.sessions.get(&record.id).is_some()
            {
                None
            } else {
                Some("Daemon stopped during a claimed start. Inspect the session before manually retrying; the message may already have been sent.".into())
            };
            finish(app, record, error).await?;
        }
    }
    Ok(())
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let records = match list(&app).await {
                Ok(records) => records,
                Err(e) => {
                    tracing::error!("scheduled launches: {e:#}");
                    continue;
                }
            };
            for record in records {
                let target = record.resume_session_id.as_ref().and_then(|id| app.sessions.get(id));
                // An archived (or deleted) agent is not woken up: its wake-up goes.
                if is_wake(&record) && record.status == ScheduleStatus::Queued && target.as_ref().is_none_or(|s| s.status == launchpad_core::model::SessionStatus::Closed) {
                    if let Err(e) = cancel(&app, &record.id).await { tracing::debug!("wake-up of an archived agent: {e:#}"); }
                    continue;
                }
                let (harness, spec) = target.as_ref()
                    .map(|s| (Some(s.harness), &s.launch_spec))
                    .unwrap_or((record.spec.harness, &record.spec));
                if harness.is_some_and(|h| !launchpad_core::quota::automatic_ready(&app, h, spec)) { continue; }

                if record.resume_session_id.as_ref().is_some_and(|id| {
                    app.sessions.get(id).is_some_and(|s| {
                        s.status.is_running()
                            || s.quota_pause.is_some()
                            || s.open_question
                            || s.status == launchpad_core::model::SessionStatus::InTerminal
                    })
                }) {
                    continue;
                }
                if ready(&record, now_ms(), |id| {
                    app.sessions.get(id).is_some_and(|s| s.dependency_done())
                }) {
                    match claim(&app, &record.id, false).await {
                        Ok(record) => {
                            let app = app.clone();
                            tokio::spawn(async move {
                                if let Err(e) = run(&app, record, true).await {
                                    tracing::error!("delayed launch: {e:#}");
                                }
                            });
                        }
                        Err(e) => tracing::debug!("scheduled launch claim: {e:#}"),
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archived_done_dependencies_survive_archive_but_not_an_unfinished_new_turn() {
        use launchpad_core::model::SessionStatus;
        launchpad_harness::install();
        let mut session: Session = serde_json::from_value(serde_json::json!({
            "id": "a", "harness": "codex", "cwd": "/tmp", "title": "Done task",
            "status": "waiting", "status_since": 0, "live": false, "open_question": false,
            "prompt": "task", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0}, "started_at": 0, "last_seq": 0,
            "launch_spec": {}, "seen_at": 0
        })).unwrap();
        let record = ScheduledLaunch {
            id: "q".into(), spec: LaunchSpec::default(), resume_session_id: None,
            conditions: vec![LaunchDelay::Agents { session_ids: vec!["a".into()] }],
            title: String::new(), created_at: 0, ready_at: None,
            status: ScheduleStatus::Queued, error: None,
        };
        assert!(ready(&record, 0, |_| session.dependency_done()));
        session.open_question = true; // Stopped for a question: not done.
        assert!(!ready(&record, 0, |_| session.dependency_done()));
        session.open_question = false;
        session.change_status(SessionStatus::Closed);
        assert!(!session.needs_you());
        // The same saved state is used after restarting the daemon.
        session = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert!(ready(&record, 0, |_| session.dependency_done()));
        session.change_status(SessionStatus::Closed);
        assert!(session.dependency_done());
        for status in [SessionStatus::Working, SessionStatus::Stopped, SessionStatus::Error,
            SessionStatus::Interrupted, SessionStatus::QuotaPaused, SessionStatus::InTerminal] {
            session.change_status(status);
            assert!(!session.dependency_done());
            session.change_status(SessionStatus::Closed);
            assert!(!ready(&record, 0, |_| session.dependency_done()));
        }
        assert!(!ready(&record, 0, |_| false)); // Missing or cancelled dependency.
    }

    #[test]
    fn a_sleeping_agent_neither_needs_you_nor_counts_as_done() {
        launchpad_harness::install();
        let mut s: Session = serde_json::from_value(serde_json::json!({
            "id": "a", "harness": "codex", "cwd": "/tmp", "title": "Wait for CI",
            "status": "waiting", "status_since": 10, "live": false, "open_question": false,
            "prompt": "task", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0}, "started_at": 0, "last_seq": 0,
            "launch_spec": {}, "seen_at": 0
        })).unwrap();
        assert!(s.needs_you() && s.dependency_done() && !s.sleeping());
        s.sleep = Some(launchpad_core::model::Sleep { until: 100, note: "check CI".into(), schedule_id: "w".into() });
        assert!(s.sleeping() && !s.needs_you() && !s.dependency_done());
        s.open_question = true; // A question still needs you.
        assert!(!s.sleeping() && s.needs_you());
        let wake = ScheduledLaunch {
            id: "w".into(), spec: LaunchSpec { from: Some(launchpad_core::model::MessageFrom { kind: launchpad_core::model::SenderKind::Wake, ..Default::default() }), ..Default::default() },
            resume_session_id: Some("a".into()), conditions: vec![LaunchDelay::In { minutes: 5 }],
            title: String::new(), created_at: 0, ready_at: Some(100), status: ScheduleStatus::Queued, error: None,
        };
        assert!(is_wake(&wake));
        assert!(!is_wake(&ScheduledLaunch { spec: LaunchSpec::default(), ..wake }));
    }

    #[test]
    fn sleep_times_parse() {
        assert_eq!(parse_minutes("45").unwrap(), 45);
        assert_eq!(parse_minutes("45m").unwrap(), 45);
        assert_eq!(parse_minutes("2h").unwrap(), 120);
        assert_eq!(parse_minutes(" 1D ").unwrap(), 1440);
        assert!(parse_minutes("0m").is_err());
        assert!(parse_minutes("1w").is_err());
        assert!(parse_minutes("h").is_err());
        assert_eq!(parse_until("1970-01-01T00:01:00Z").unwrap(), 60_000);
        assert_eq!(parse_until("1970-01-01T01:01:00+01:00").unwrap(), 60_000);
        assert!(parse_until("15:00").is_err());
    }

    #[test]
    fn readiness_requires_every_condition_and_a_queued_record() {
        let mut r = ScheduledLaunch {
            id: "q".into(),
            spec: LaunchSpec::default(),
            resume_session_id: None,
            conditions: vec![LaunchDelay::Agents {
                session_ids: vec!["a".into(), "b".into()],
            }],
            title: String::new(),
            created_at: 0,
            ready_at: None,
            status: ScheduleStatus::Queued,
            error: None,
        };
        assert!(!ready(&r, 100, |id| id == "a"));
        assert!(ready(&r, 100, |_| true));
        r.status = ScheduleStatus::Starting;
        assert!(!ready(&r, 100, |_| true));
        r.status = ScheduleStatus::Queued;
        r.conditions = vec![LaunchDelay::Agents {
            session_ids: vec![],
        }];
        assert!(!ready(&r, 100, |_| true));
        r.conditions = vec![];
        assert!(!ready(&r, 100, |_| true));
        r.conditions = vec![LaunchDelay::Quota {
            harness: launchpad_core::model::HarnessKind::new("codex"),
            window: QuotaKind::FiveHour,
        }];
        r.ready_at = Some(101);
        assert!(!ready(&r, 100, |_| true));
        assert!(ready(&r, 101, |_| false));
        // Combined conditions: the reset time and the agent must both be satisfied.
        r.conditions.push(LaunchDelay::Agents {
            session_ids: vec!["a".into()],
        });
        assert!(!ready(&r, 101, |_| false));
        assert!(!ready(&r, 100, |_| true));
        assert!(ready(&r, 101, |id| id == "a"));
        r.status = ScheduleStatus::Failed;
        assert!(!ready(&r, 102, |_| true));
    }
}

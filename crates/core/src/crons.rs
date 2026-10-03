//! Cron agents: launch the same spec on a 5-field cron schedule (local time) when conditions hold.
//! Each due run is claimed by moving `next_run_at` forward before launching, so a crash never repeats it.
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    App,
    model::{HarnessKind, LaunchSpec, Session, Target},
    schedules::QuotaKind,
    status::HarnessStatus,
    util::now_ms,
};

/// Runs later than this (daemon stopped, update installing) are skipped instead of started.
const MISSED_GRACE_MS: i64 = 60 * 60_000;
const RUNS_KEPT: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronCondition {
    /// The cron's harness has at least this share of the quota window left.
    QuotaLeft { window: QuotaKind, min_left_percent: u8 },
    /// The previous run of this cron is no longer Running.
    PreviousRunFinished,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CronAgent {
    pub id: String,
    pub spec: LaunchSpec,
    pub title: String,
    /// Standard 5-field cron expression, in the local time zone.
    pub schedule: String,
    pub conditions: Vec<CronCondition>,
    pub paused: bool,
    pub created_at: i64,
    /// Unix milliseconds; None while paused.
    pub next_run_at: Option<i64>,
    pub last_check_at: Option<i64>,
    /// What happened at the last due time or manual run, for example "Skipped: …".
    pub last_outcome: Option<String>,
    /// Session ids of past runs, oldest first.
    pub runs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CronPatch {
    pub schedule: Option<String>,
    pub conditions: Option<Vec<CronCondition>>,
    pub paused: Option<bool>,
}

fn parse(schedule: &str) -> Result<croner::Cron> {
    let schedule = schedule.split_whitespace().collect::<Vec<_>>();
    if schedule.len() != 5 {
        bail!("use 5 fields: minute hour day-of-month month day-of-week");
    }
    croner::Cron::new(&schedule.join(" "))
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid cron schedule: {e}"))
}

/// The first local time matching `schedule` strictly after `after` (Unix milliseconds).
pub fn next_after(schedule: &str, after: i64) -> Result<i64> {
    use chrono::TimeZone;
    let cron = parse(schedule)?;
    let after = chrono::Local
        .timestamp_millis_opt(after)
        .single()
        .context("invalid time")?;
    let next = cron
        .find_next_occurrence(&after, false)
        .map_err(|e| anyhow::anyhow!("this schedule never runs: {e}"))?;
    Ok(next.timestamp_millis())
}

/// The next `count` run times, to preview a schedule in the UI.
pub fn next_runs(schedule: &str, count: usize) -> Result<Vec<i64>> {
    let mut at = now_ms();
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        at = next_after(schedule, at)?;
        out.push(at);
    }
    Ok(out)
}

fn validate_conditions(conditions: &[CronCondition]) -> Result<()> {
    for c in conditions {
        if let CronCondition::QuotaLeft { min_left_percent, .. } = c
            && !(1..=100).contains(min_left_percent)
        {
            bail!("pick a quota share from 1% to 100%");
        }
    }
    Ok(())
}

/// Why the conditions block a run, or None when it may start.
fn blocked(
    conditions: &[CronCondition],
    quota: &HarnessStatus,
    now: i64,
    previous_running: bool,
) -> Option<String> {
    for c in conditions {
        match c {
            CronCondition::PreviousRunFinished if previous_running => {
                return Some("the previous run is still running".into());
            }
            CronCondition::PreviousRunFinished => {}
            CronCondition::QuotaLeft { window, min_left_percent } => {
                let name = match window {
                    QuotaKind::FiveHour => "5-hour",
                    QuotaKind::Weekly => "weekly",
                };
                if quota.quota_error.is_some() || quota.quota_at.is_none_or(|at| now - at > 15 * 60_000) {
                    return Some(format!("no fresh {name} quota reading"));
                }
                let Some(q) = (match window {
                    QuotaKind::FiveHour => &quota.five_hour,
                    QuotaKind::Weekly => &quota.weekly,
                }) else {
                    return Some(format!("the {name} quota is unknown"));
                };
                let left = (100.0 - q.used_percent).max(0.0);
                if left < f64::from(*min_left_percent) {
                    return Some(format!("{left:.0}% of the {name} quota left, below {min_left_percent}%"));
                }
            }
        }
    }
    None
}

fn changed(app: &App) {
    app.bus.publish("crons", "changed", serde_json::Value::Null);
}

pub async fn list(app: &App) -> Result<Vec<CronAgent>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM cron_agent ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|s| Ok(serde_json::from_str(&s)?))
                .collect()
        })
        .await
}

/// Read-modify-write in the DB actor, which serializes the scheduler and UI requests.
async fn modify<T: Send + 'static>(
    app: &App,
    id: &str,
    f: impl FnOnce(&mut CronAgent) -> Result<T> + Send + 'static,
) -> Result<(CronAgent, T)> {
    let id = id.to_string();
    let out = app
        .state_db
        .call(move |c| {
            let json: String = c
                .query_row("SELECT json FROM cron_agent WHERE id = ?1", [&id], |r| r.get(0))
                .context("this cron agent no longer exists")?;
            let mut record: CronAgent = serde_json::from_str(&json)?;
            let out = f(&mut record)?;
            c.execute(
                "UPDATE cron_agent SET json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&record)?],
            )?;
            Ok((record, out))
        })
        .await?;
    changed(app);
    Ok(out)
}

pub async fn create(
    app: &App,
    spec: LaunchSpec,
    schedule: String,
    conditions: Vec<CronCondition>,
) -> Result<CronAgent> {
    spec.harness.context("pick Claude Code or Codex")?;
    if spec.target == Target::None {
        bail!("pick a project or a temporary folder");
    }
    let preview = app.sessions.preview(app, &spec).await?;
    if preview.prompt.trim().is_empty() {
        bail!("the prompt is empty");
    }
    validate_conditions(&conditions)?;
    let now = now_ms();
    let schedule = schedule.split_whitespace().collect::<Vec<_>>().join(" ");
    let next = next_after(&schedule, now)?;
    let record = CronAgent {
        id: uuid::Uuid::new_v4().to_string(),
        spec,
        title: preview.title,
        schedule,
        conditions,
        paused: false,
        created_at: now,
        next_run_at: Some(next),
        last_check_at: None,
        last_outcome: None,
        runs: vec![],
    };
    let r = record.clone();
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO cron_agent (id, json) VALUES (?1, ?2)",
                params![r.id, serde_json::to_string(&r)?],
            )?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(record)
}

pub async fn update(app: &App, id: &str, patch: CronPatch) -> Result<CronAgent> {
    if let Some(c) = &patch.conditions {
        validate_conditions(c)?;
    }
    let now = now_ms();
    let (record, ()) = modify(app, id, move |r| {
        if let Some(s) = patch.schedule {
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            next_after(&s, now)?;
            r.schedule = s;
        }
        if let Some(c) = patch.conditions {
            r.conditions = c;
        }
        if let Some(p) = patch.paused {
            r.paused = p;
        }
        r.next_run_at = if r.paused { None } else { Some(next_after(&r.schedule, now)?) };
        Ok(())
    })
    .await?;
    Ok(record)
}

pub async fn delete(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            if c.execute("DELETE FROM cron_agent WHERE id = ?1", [id])? == 0 {
                bail!("this cron agent no longer exists");
            }
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

async fn launch(app: &App, record: &CronAgent) -> Result<Session> {
    let result = app.sessions.launch(app, record.spec.clone()).await;
    let outcome = match &result {
        Ok(_) => "Started".to_string(),
        Err(e) => format!("Launch failed: {e:#}"),
    };
    let session = result.as_ref().ok().map(|s| s.id.clone());
    modify(app, &record.id, move |r| {
        r.last_outcome = Some(outcome);
        if let Some(id) = session {
            r.runs.push(id);
            let extra = r.runs.len().saturating_sub(RUNS_KEPT);
            r.runs.drain(..extra);
        }
        Ok(())
    })
    .await?;
    result
}

/// Manual run: ignores the conditions and keeps the schedule.
pub async fn run_now(app: &App, id: &str) -> Result<Session> {
    let record = list(app)
        .await?
        .into_iter()
        .find(|r| r.id == id)
        .context("this cron agent no longer exists")?;
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { launch(&app, &record).await }).await?
}

async fn tick(app: &App, record: CronAgent) -> Result<()> {
    let now = now_ms();
    let due = record.next_run_at;
    let (record, late) = modify(app, &record.id, move |r| {
        if r.paused || r.next_run_at != due {
            bail!("already handled");
        }
        r.next_run_at = Some(next_after(&r.schedule, now)?);
        r.last_check_at = Some(now);
        Ok(due.is_some_and(|d| now - d > MISSED_GRACE_MS))
    })
    .await?;
    let harness = record.spec.harness.unwrap_or(HarnessKind::Claude);
    let status = app.status.get();
    let quota = match harness {
        HarnessKind::Claude => status.claude,
        HarnessKind::Codex => status.codex,
    };
    let previous_running = record
        .runs
        .last()
        .and_then(|id| app.sessions.get(id))
        .is_some_and(|s| s.status.is_running());
    let reason = if late {
        Some("the daemon was not running at the scheduled time".into())
    } else {
        blocked(&record.conditions, &quota, now, previous_running)
    };
    if let Some(reason) = reason {
        modify(app, &record.id, move |r| {
            r.last_outcome = Some(format!("Skipped: {reason}"));
            Ok(())
        })
        .await?;
        return Ok(());
    }
    launch(app, &record).await.map(|_| ())
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            if app.sessions.installing().await {
                continue;
            }
            let records = match list(&app).await {
                Ok(records) => records,
                Err(e) => {
                    tracing::error!("cron agents: {e:#}");
                    continue;
                }
            };
            let now = now_ms();
            for record in records {
                if !record.paused && record.next_run_at.is_some_and(|at| at <= now) {
                    let app = app.clone();
                    tokio::spawn(async move {
                        if let Err(e) = tick(&app, record).await {
                            tracing::debug!("cron agent run: {e:#}");
                        }
                    });
                }
            }
        }
    });
}

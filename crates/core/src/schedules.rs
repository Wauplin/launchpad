//! Durable delayed launches. Claim before launching; never retry a claimed launch after a crash.
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    App,
    model::{HarnessKind, LaunchSpec, Session, Target},
    util::now_ms,
};

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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
    pub delay: LaunchDelay,
    pub title: String,
    pub created_at: i64,
    /// Time and quota delays resolve once, at scheduling time (Unix milliseconds).
    pub ready_at: Option<i64>,
    pub status: ScheduleStatus,
    pub error: Option<String>,
}

pub async fn list(app: &App) -> Result<Vec<ScheduledLaunch>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM scheduled_launch ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|s| Ok(serde_json::from_str(&s)?))
                .collect()
        })
        .await
}

fn changed(app: &App) {
    app.bus
        .publish("schedules", "changed", serde_json::Value::Null);
}

pub async fn create(app: &App, spec: LaunchSpec, delay: LaunchDelay) -> Result<ScheduledLaunch> {
    spec.harness.context("pick Claude Code or Codex")?;
    if spec.target == Target::None {
        bail!("pick a project or a temporary folder");
    }
    let preview = app.sessions.preview(app, &spec).await?;
    if preview.prompt.trim().is_empty() {
        bail!("the prompt is empty");
    }
    let now = now_ms();
    let ready_at = match &delay {
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
            for id in session_ids {
                app.sessions
                    .get(id)
                    .context("a selected agent no longer exists")?;
            }
            None
        }
        LaunchDelay::Quota { harness, window } => {
            let status = app.status.get();
            let h = match harness {
                HarnessKind::Claude => status.claude,
                HarnessKind::Codex => status.codex,
            };
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
                .checked_mul(1000)
                .context("invalid quota reset")?;
            if at <= now {
                bail!("wait for the next quota reading before scheduling its reset");
            }
            Some(at)
        }
    };
    let launch = ScheduledLaunch {
        id: uuid::Uuid::new_v4().to_string(),
        spec,
        delay,
        title: preview.title,
        created_at: now,
        ready_at,
        status: ScheduleStatus::Queued,
        error: None,
    };
    let record = launch.clone();
    app.state_db
        .call(move |c| {
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

pub async fn cancel(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    app.state_db
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
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

async fn finish(app: &App, mut record: ScheduledLaunch, error: Option<String>) -> Result<()> {
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

async fn run(app: &App, record: ScheduledLaunch) -> Result<Session> {
    // A stable session id makes crash recovery unambiguous, even before the harness attaches.
    let result = match app.sessions.get(&record.id) {
        Some(s) => Ok(s),
        None => {
            app.sessions
                .launch_with_id(app, record.spec.clone(), record.id.clone())
                .await
        }
    };
    if result
        .as_ref()
        .err()
        .is_some_and(|e| e.is::<crate::sessions::InstallingUpdate>())
    {
        // Installation is temporary: preserve this launch for the replacement daemon.
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

pub async fn start_now(app: &App, id: &str) -> Result<Session> {
    let record = claim(app, id, true).await?;
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { run(&app, record).await }).await?
}

fn ready(record: &ScheduledLaunch, now: i64, done: impl Fn(&str) -> bool) -> bool {
    if record.status != ScheduleStatus::Queued {
        return false;
    }
    match &record.delay {
        LaunchDelay::Agents { session_ids } => {
            !session_ids.is_empty() && session_ids.iter().all(|id| done(id))
        }
        _ => record.ready_at.is_some_and(|at| at <= now),
    }
}

pub async fn recover(app: &App) -> Result<()> {
    for record in list(app).await? {
        if record.status == ScheduleStatus::Starting {
            let error = if app.sessions.get(&record.id).is_some() {
                None
            } else {
                Some("Daemon stopped before launch completed. Click Start now to retry.".into())
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
            if app.sessions.installing().await {
                continue;
            }
            let records = match list(&app).await {
                Ok(records) => records,
                Err(e) => {
                    tracing::error!("scheduled launches: {e:#}");
                    continue;
                }
            };
            for record in records {
                if ready(&record, now_ms(), |id| {
                    app.sessions.get(id).is_some_and(|s| s.status.is_done())
                }) {
                    match claim(&app, &record.id, false).await {
                        Ok(record) => {
                            let app = app.clone();
                            tokio::spawn(async move {
                                if let Err(e) = run(&app, record).await {
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
    fn readiness_requires_all_agents_done_and_a_queued_record() {
        let mut r = ScheduledLaunch {
            id: "q".into(),
            spec: LaunchSpec::default(),
            delay: LaunchDelay::Agents {
                session_ids: vec!["a".into(), "b".into()],
            },
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
        r.delay = LaunchDelay::Agents {
            session_ids: vec![],
        };
        assert!(!ready(&r, 100, |_| true));
        r.delay = LaunchDelay::Quota {
            harness: HarnessKind::Codex,
            window: QuotaKind::FiveHour,
        };
        assert!(!ready(&r, 100, |_| true));
        r.ready_at = Some(101);
        assert!(!ready(&r, 100, |_| true));
        assert!(ready(&r, 101, |_| false));
        r.status = ScheduleStatus::Failed;
        assert!(!ready(&r, 102, |_| true));
    }
}

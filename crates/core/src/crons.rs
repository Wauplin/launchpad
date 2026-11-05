//! Routines (code: crons): runs that start on their own. A trigger starts a run: a 5-field cron schedule (local
//! time), new GitHub activity in the Inbox, or another agent's turn ending Done. A routine of a bot sends its
//! instructions to the bot's thread; any other routine launches the same spec as a new session.
//!
//! Every run is claimed before it starts (`next_run_at` moves forward, the GitHub watermark advances, a history
//! entry is written), so a crash never repeats it, and a run more than one hour late is skipped. A run ends with
//! a report, or with the final message `no_update`, which stays silent (no notification, not Needs you).
use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    App,
    model::{LaunchSpec, RoutineLaunch, Session, SessionStatus, Target},
    schedules::QuotaKind,
    util::now_ms,
};

/// Runs later than this (daemon stopped, old GitHub activity) are skipped instead of started.
const MISSED_GRACE_MS: i64 = 60 * 60_000;
const RUNS_KEPT: usize = 50;
/// The JSON forms of a trigger, for agents that propose routines.
pub const TRIGGER_FORMS: &str = r#"{"kind": "schedule"} | {"kind": "github", "repo": "owner/name" or null for the workspace's repositories, "events": ["review_requested" | "mentioned" | "assigned" | "ci_activity" | "release" | "any"]} | {"kind": "session_done", "folder": "<folder id>" or null for any folder of the workspace}"#;
/// The final message of a run with nothing to report.
pub const NO_UPDATE: &str = "no_update";

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronCondition {
    /// The cron's harness has at least this share of the quota window left.
    QuotaLeft { window: QuotaKind, min_left_percent: u8 },
    /// The previous run of this cron is no longer Running.
    PreviousRunFinished,
}

/// What starts a run.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// The routine's `schedule`.
    #[default]
    Schedule,
    /// New activity in the Inbox (GitHub notifications) of a repository: one of the workspace's when None.
    Github { repo: Option<String>, events: Vec<GithubEvent> },
    /// A turn of another agent ended Done, in this folder (any folder of the workspace when None).
    SessionDone { folder: Option<String> },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubEvent {
    ReviewRequested,
    Mentioned,
    Assigned,
    /// Workflow runs GitHub notifies you about (by default, failed runs you triggered).
    CiActivity,
    Release,
    /// Any new notification.
    Any,
}

impl GithubEvent {
    fn matches(self, kind: &str, reason: &str) -> bool {
        match self {
            Self::ReviewRequested => reason == "review_requested",
            Self::Mentioned => reason == "mention" || reason == "team_mention",
            Self::Assigned => reason == "assign",
            Self::CiActivity => reason == "ci_activity" || kind == "CheckSuite",
            Self::Release => kind == "Release",
            Self::Any => true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ReviewRequested => "review requested",
            Self::Mentioned => "mentioned",
            Self::Assigned => "assigned",
            Self::CiActivity => "CI activity",
            Self::Release => "new release",
            Self::Any => "activity",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Running,
    /// The run ended with something to report.
    Report,
    /// The run ended with `no_update`: silent.
    NoUpdate,
    Skipped,
    Failed,
    Interrupted,
    Stopped,
}

/// One due time, event or manual run, with what came of it.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RoutineRun {
    #[ts(type = "number")]
    pub at: i64,
    /// What started it, for example "Scheduled" or "Review requested: owner/repo#12".
    pub cause: String,
    pub session_id: Option<String>,
    pub outcome: RunOutcome,
    /// Why it was skipped or failed.
    pub detail: Option<String>,
    /// Nothing in its thread waited for you when it started: a quiet run (`no_update`) may mark it seen.
    #[serde(default)]
    #[ts(skip)]
    pub quiet_ok: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CronAgent {
    pub id: String,
    pub spec: LaunchSpec,
    pub title: String,
    /// Standard 5-field cron expression, in `time_zone` (Schedule trigger).
    pub schedule: String,
    /// IANA time zone of the schedule, from the browser that saved it. None: the host's time zone.
    #[serde(default)]
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub trigger: Trigger,
    /// A routine of this bot: its runs are messages to the bot's thread (`spec.text` is the instructions).
    #[serde(default)]
    pub bot_id: Option<String>,
    /// The bot is paused: no run starts, whatever the routine's own state.
    #[serde(default)]
    pub bot_paused: bool,
    pub conditions: Vec<CronCondition>,
    pub paused: bool,
    pub created_at: i64,
    /// Unix milliseconds; None while paused or for event triggers.
    pub next_run_at: Option<i64>,
    pub last_check_at: Option<i64>,
    /// What happened at the last due time or manual run, for example "Skipped: …".
    pub last_outcome: Option<String>,
    /// Session ids of past runs, oldest first.
    pub runs: Vec<String>,
    /// Past runs with their outcome, oldest first.
    #[serde(default)]
    pub history: Vec<RoutineRun>,
    /// GitHub trigger: the newest Inbox activity (`updated_at`) already handled.
    #[serde(default)]
    pub github_seen: Option<String>,
}

impl CronAgent {
    fn active(&self) -> bool {
        !self.paused && !self.bot_paused
    }
}

#[derive(Debug, Deserialize)]
pub struct CronPatch {
    pub schedule: Option<String>,
    pub conditions: Option<Vec<CronCondition>>,
    pub paused: Option<bool>,
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default)]
    pub title: Option<String>,
    /// A bot routine's instructions.
    #[serde(default)]
    pub instructions: Option<String>,
    /// The editing browser's time zone.
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// A new routine of a bot.
#[derive(Debug, Deserialize)]
pub struct BotRoutine {
    pub title: String,
    pub instructions: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub conditions: Vec<CronCondition>,
    /// The browser's time zone.
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// Whether a final message means "nothing to report" (`no_update`, alone, maybe quoted or with a period).
pub fn is_no_update(message: Option<&str>) -> bool {
    message.is_some_and(|m| m.trim().trim_matches(|c: char| c == '`' || c == '*' || c == '"' || c == '.' || c.is_whitespace()).eq_ignore_ascii_case(NO_UPDATE))
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

fn parse_zone(zone: &str) -> Result<chrono_tz::Tz> {
    zone.parse().map_err(|_| anyhow::anyhow!("unknown time zone {zone}"))
}

/// The zone a new or edited schedule keeps: the browser's, else `[home] time_zone`, else None (the host's).
pub fn zone_for(app: &App, given: Option<String>) -> Result<Option<String>> {
    if let Some(z) = given.map(|z| z.trim().to_string()).filter(|z| !z.is_empty()) {
        parse_zone(&z)?;
        return Ok(Some(z));
    }
    let home = app.config.get().home.time_zone.trim().to_string();
    Ok((!home.is_empty() && parse_zone(&home).is_ok()).then_some(home))
}

fn next_in<Z: chrono::TimeZone>(cron: &croner::Cron, zone: &Z, after: i64) -> Result<i64> {
    let after = zone.timestamp_millis_opt(after).single().context("invalid time")?;
    let next = cron.find_next_occurrence(&after, false).map_err(|e| anyhow::anyhow!("this schedule never runs: {e}"))?;
    Ok(next.timestamp_millis())
}

/// The first time matching `schedule` in `zone` (None: the host's time zone) strictly after `after` (Unix
/// milliseconds).
pub fn next_after(schedule: &str, zone: Option<&str>, after: i64) -> Result<i64> {
    let cron = parse(schedule)?;
    match zone {
        Some(z) => next_in(&cron, &parse_zone(z)?, after),
        None => next_in(&cron, &chrono::Local, after),
    }
}

/// The next `count` run times, to preview a schedule in the UI.
pub fn next_runs(schedule: &str, zone: Option<&str>, count: usize) -> Result<Vec<i64>> {
    let mut at = now_ms();
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        at = next_after(schedule, zone, at)?;
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
    quota: &crate::status::QuotaStatus,
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

pub async fn get(app: &App, id: &str) -> Result<CronAgent> {
    list(app).await?.into_iter().find(|r| r.id == id).context("this routine no longer exists")
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
                .context("this routine no longer exists")?;
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

fn validate_trigger(trigger: &Trigger, schedule: &str, zone: Option<&str>) -> Result<()> {
    match trigger {
        Trigger::Schedule => next_after(schedule, zone, now_ms()).map(|_| ()),
        Trigger::Github { repo, events } => {
            ensure!(!events.is_empty(), "pick at least one kind of GitHub activity");
            if let Some(r) = repo {
                ensure!(r.split('/').filter(|p| !p.is_empty()).count() == 2, "write the repository as owner/name");
            }
            Ok(())
        }
        Trigger::SessionDone { .. } => Ok(()),
    }
}

/// The next due time of a schedule trigger, None for event triggers and paused routines.
fn next_due(r: &CronAgent, now: i64) -> Result<Option<i64>> {
    Ok(if r.active() && r.trigger == Trigger::Schedule { Some(next_after(&r.schedule, r.time_zone.as_deref(), now)?) } else { None })
}

async fn insert(app: &App, record: &CronAgent) -> Result<()> {
    let r = record.clone();
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO cron_agent (id, json) VALUES (?1, ?2)", params![r.id, serde_json::to_string(&r)?])?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

fn normalize(schedule: &str) -> String {
    schedule.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub async fn create(
    app: &App,
    spec: LaunchSpec,
    schedule: String,
    conditions: Vec<CronCondition>,
    trigger: Trigger,
    title: Option<String>,
    time_zone: Option<String>,
) -> Result<CronAgent> {
    // Ask Launchpad's routines (the morning brief) follow the assistant's own profile and model.
    let spec = match &spec.target {
        Target::Assistant { workspace_id } => crate::proposals::ask_spec(app, workspace_id.clone(), spec.text.clone(), vec![])?,
        _ => spec,
    };
    let harness = spec.harness.context("pick a harness")?;
    if spec.target == Target::None {
        bail!("pick a project or a temporary folder");
    }
    let preview = app.sessions.preview(app, &spec).await?;
    if preview.prompt.trim().is_empty() {
        bail!("the prompt is empty");
    }
    validate_conditions(&conditions)?;
    validate_harness_conditions(harness, &conditions)?;
    let schedule = normalize(&schedule);
    let time_zone = zone_for(app, time_zone)?;
    validate_trigger(&trigger, &schedule, time_zone.as_deref())?;
    let now = now_ms();
    let mut record = CronAgent {
        id: uuid::Uuid::new_v4().to_string(),
        spec,
        title: title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or(preview.title),
        schedule,
        time_zone,
        trigger,
        bot_id: None,
        bot_paused: false,
        conditions,
        paused: false,
        created_at: now,
        next_run_at: None,
        last_check_at: None,
        last_outcome: None,
        runs: vec![],
        history: vec![],
        github_seen: None,
    };
    record.next_run_at = next_due(&record, now)?;
    insert(app, &record).await?;
    Ok(record)
}

/// The spec a bot routine stands for: the bot's profile and folder (for quotas and workspace lists).
async fn bot_spec(app: &App, bot: &crate::bots::Bot, text: String) -> Result<LaunchSpec> {
    let profile = bot.settings.profile.as_ref().and_then(|p| app.config.get().profiles.iter().find(|x| &x.id == p).cloned()).context("choose a profile for this bot in its Settings tab")?;
    Ok(LaunchSpec {
        text,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        target: Target::Project { project_id: bot.settings.folder.clone() },
        workspace_id: Some(bot.workspace_id.clone()),
        ..Default::default()
    })
}

/// The spec a run uses now: a bot's or Ask Launchpad's current profile, else the routine's own spec.
async fn effective_spec(app: &App, r: &CronAgent) -> Result<LaunchSpec> {
    match (&r.bot_id, &r.spec.target) {
        (Some(b), _) => bot_spec(app, &crate::bots::get(app, b).await?, r.spec.text.clone()).await,
        (None, Target::Assistant { workspace_id }) => crate::proposals::ask_spec(app, workspace_id.clone(), r.spec.text.clone(), vec![]),
        _ => Ok(r.spec.clone()),
    }
}

/// Whether a routine's runs are messages to a thread (a bot's, or Ask Launchpad's) rather than new sessions.
fn in_thread(r: &CronAgent) -> bool {
    r.bot_id.is_some() || matches!(r.spec.target, Target::Assistant { .. })
}

/// A new routine of a bot. Its runs are messages to the bot's thread.
pub async fn create_for_bot(app: &App, bot_id: &str, r: BotRoutine) -> Result<CronAgent> {
    let bot = crate::bots::get(app, bot_id).await?;
    let title = r.title.trim().to_string();
    ensure!(!title.is_empty(), "give the routine a name");
    ensure!(!r.instructions.trim().is_empty(), "write what the bot does at each run");
    let spec = bot_spec(app, &bot, r.instructions.trim().to_string()).await?;
    validate_conditions(&r.conditions)?;
    validate_harness_conditions(spec.harness.context("pick a harness")?, &r.conditions)?;
    let schedule = normalize(&r.schedule);
    let time_zone = zone_for(app, r.time_zone)?;
    validate_trigger(&r.trigger, &schedule, time_zone.as_deref())?;
    let now = now_ms();
    let mut record = CronAgent {
        id: uuid::Uuid::new_v4().to_string(),
        spec,
        title,
        schedule,
        time_zone,
        trigger: r.trigger,
        bot_id: Some(bot.id.clone()),
        bot_paused: bot.paused,
        conditions: r.conditions,
        paused: false,
        created_at: now,
        next_run_at: None,
        last_check_at: None,
        last_outcome: None,
        runs: vec![],
        history: vec![],
        github_seen: None,
    };
    record.next_run_at = next_due(&record, now)?;
    insert(app, &record).await?;
    Ok(record)
}

pub async fn update(app: &App, id: &str, patch: CronPatch) -> Result<CronAgent> {
    if let Some(c) = &patch.conditions {
        validate_conditions(c)?;
    }
    if let Some(t) = &patch.title {
        ensure!(!t.trim().is_empty(), "give the routine a name");
    }
    let now = now_ms();
    let zone = match patch.time_zone {
        Some(z) => Some(zone_for(app, Some(z))?),
        None => None,
    };
    let (record, ()) = modify(app, id, move |r| {
        if let Some(s) = patch.schedule {
            r.schedule = normalize(&s);
        }
        if let Some(z) = zone {
            r.time_zone = z;
        }
        if let Some(t) = patch.trigger {
            if t != r.trigger {
                r.github_seen = None;
            }
            r.trigger = t;
        }
        validate_trigger(&r.trigger, &r.schedule, r.time_zone.as_deref())?;
        if let Some(c) = patch.conditions {
            validate_harness_conditions(r.spec.harness.context("pick a harness")?, &c)?;
            r.conditions = c;
        }
        if let Some(p) = patch.paused {
            r.paused = p;
        }
        if let Some(t) = patch.title {
            r.title = t.trim().to_string();
        }
        if let Some(i) = patch.instructions {
            ensure!(r.bot_id.is_some(), "edit this routine's prompt by creating it again from the composer");
            ensure!(!i.trim().is_empty(), "write what the bot does at each run");
            r.spec.text = i.trim().to_string();
        }
        r.next_run_at = next_due(r, now)?;
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
                bail!("this routine no longer exists");
            }
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

/// Home: Pause or Resume a bot's routines (`bots::set_paused`).
pub async fn set_bot_paused(app: &App, bot_id: &str, paused: bool) -> Result<()> {
    let now = now_ms();
    for r in list(app).await?.into_iter().filter(|r| r.bot_id.as_deref() == Some(bot_id)) {
        modify(app, &r.id, move |r| {
            r.bot_paused = paused;
            r.next_run_at = next_due(r, now)?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Home: a deleted bot takes its routines with it.
pub async fn delete_for_bot(app: &App, bot_id: &str) -> Result<()> {
    for r in list(app).await?.into_iter().filter(|r| r.bot_id.as_deref() == Some(bot_id)) {
        delete(app, &r.id).await?;
    }
    Ok(())
}

fn summary(run: &RoutineRun) -> String {
    let what = match run.outcome {
        RunOutcome::Running => "Started",
        RunOutcome::Report => "Report",
        RunOutcome::NoUpdate => "No update",
        RunOutcome::Skipped => "Skipped",
        RunOutcome::Failed => "Failed",
        RunOutcome::Interrupted => "Interrupted",
        RunOutcome::Stopped => "Stopped",
    };
    match &run.detail {
        Some(d) => format!("{what}: {d}"),
        None => what.to_string(),
    }
}

fn push_run(r: &mut CronAgent, run: RoutineRun) {
    r.last_outcome = Some(summary(&run));
    if let Some(id) = &run.session_id
        && r.runs.last() != Some(id)
    {
        r.runs.push(id.clone());
        let extra = r.runs.len().saturating_sub(RUNS_KEPT);
        r.runs.drain(..extra);
    }
    r.history.push(run);
    let extra = r.history.len().saturating_sub(RUNS_KEPT);
    r.history.drain(..extra);
}

async fn skip(app: &App, id: &str, cause: String, reason: String) -> Result<()> {
    modify(app, id, move |r| {
        push_run(r, RoutineRun { at: now_ms(), cause, session_id: None, outcome: RunOutcome::Skipped, detail: Some(reason), quiet_ok: false });
        Ok(())
    })
    .await
    .map(|_| ())
}

/// The instructions a run adds to the routine's prompt.
pub fn instruction(r: &RoutineLaunch) -> String {
    let mut out = format!("This is a background run of the routine “{}” ({}). Nobody is watching it.", r.title, r.cause);
    if !r.context.trim().is_empty() {
        out.push_str(&format!(" What started it:\n\n{}\n", r.context.trim_end()));
    }
    out.push_str("\nPrepare work for the owner's review (branches, Local PRs, suggestions, drafts in the Outbox) unless these instructions explicitly ask you to publish. When you finish, write a short report for the owner. If there is nothing worth reporting, reply with exactly `no_update` and nothing else: Launchpad then stays silent.");
    out
}

/// The message a bot routine sends to the bot's thread. The UI recognizes its first line.
fn bot_message(r: &CronAgent, launch: &RoutineLaunch) -> String {
    format!("Routine “{}” · {}\n\n{}\n\n{}", r.title, launch.cause, r.spec.text.trim(), instruction(launch))
}

/// Why a bot cannot take a routine's message now.
fn bot_busy(app: &App, bot_id: &str) -> Option<String> {
    let thread = crate::bots::thread(app, bot_id);
    let current = thread.iter().rev().find(|s| s.status != SessionStatus::Closed)?;
    (current.status.is_running() || current.quota_pause.is_some()).then(|| format!("{} is working", current.launch_spec.bot.as_ref().map(|b| b.name.as_str()).unwrap_or("the bot")))
}

/// The open session of the thread a routine posts to (a bot's, or Ask Launchpad's), None for other routines.
fn thread_current(app: &App, r: &CronAgent) -> Option<Session> {
    match (&r.bot_id, &r.spec.target) {
        (Some(b), _) => crate::bots::thread(app, b).into_iter().rev().find(|s| s.status != SessionStatus::Closed),
        (None, Target::Assistant { workspace_id }) => crate::proposals::thread(app, workspace_id.as_deref()),
        _ => None,
    }
}

/// Why the thread a routine posts to (a bot's, or Ask Launchpad's) cannot take a message now.
fn thread_busy(app: &App, r: &CronAgent) -> Option<String> {
    match (&r.bot_id, &r.spec.target) {
        (Some(b), _) => bot_busy(app, b),
        (None, Target::Assistant { workspace_id }) => crate::proposals::thread(app, workspace_id.as_deref())
            .filter(|s| s.status.is_running() || s.quota_pause.is_some())
            .map(|_| "Ask Launchpad is working".to_string()),
        _ => None,
    }
}

/// Why the routine's conditions block a run now, or None when it may start.
async fn why_blocked(app: &App, r: &CronAgent) -> Result<Option<String>> {
    if let Some(why) = thread_busy(app, r) {
        return Ok(Some(why));
    }
    let spec = effective_spec(app, r).await?;
    let harness = spec.harness.context("this routine has no registered harness")?;
    let quota = crate::quota::reading_for(app, harness, &spec);
    let previous_running = !in_thread(r)
        && r.runs.last().and_then(|id| app.sessions.get(id)).is_some_and(|s| app.sessions.is_busy(&s) || s.quota_pause.is_some());
    Ok(blocked(&r.conditions, &quota, now_ms(), previous_running))
}

/// Starts one run: claims a history entry, then sends the bot's message or launches the spec (sent once, never
/// retried). `automatic` runs need confirmed quota.
async fn start(app: &App, id: &str, cause: String, context: String, automatic: bool) -> Result<Session> {
    let at = now_ms();
    let current = get(app, id).await?;
    let quiet_ok = thread_current(app, &current).is_none_or(|s| !s.needs_you());
    let (record, ()) = modify(app, id, {
        let cause = cause.clone();
        move |r| {
            if automatic {
                ensure!(r.active(), "paused");
            }
            push_run(r, RoutineRun { at, cause, session_id: None, outcome: RunOutcome::Running, detail: None, quiet_ok });
            Ok(())
        }
    })
    .await?;
    let launch = RoutineLaunch { id: record.id.clone(), title: record.title.clone(), cause, context };
    let result = match (&record.bot_id, &record.spec.target) {
        (Some(bot), _) => crate::bots::message_with(app, bot, bot_message(&record, &launch), vec![], false, automatic).await,
        (None, Target::Assistant { workspace_id }) => crate::proposals::ask_with(app, workspace_id.clone(), bot_message(&record, &launch), vec![], automatic).await,
        _ => {
            let spec = LaunchSpec { routine: Some(launch), ..record.spec.clone() };
            crate::runners::launch_anywhere(app, spec, uuid::Uuid::new_v4().to_string(), automatic).await
        }
    };
    let (session, error) = match &result {
        Ok(s) => (Some(s.id.clone()), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    modify(app, id, move |r| {
        if let Some(run) = r.history.iter_mut().rev().find(|x| x.at == at && x.outcome == RunOutcome::Running && x.session_id.is_none()) {
            run.session_id = session.clone();
            if let Some(e) = error {
                run.outcome = RunOutcome::Failed;
                run.detail = Some(e);
            }
            let run = run.clone();
            r.last_outcome = Some(summary(&run));
            if let Some(s) = session
                && r.runs.last() != Some(&s)
            {
                r.runs.push(s);
                let extra = r.runs.len().saturating_sub(RUNS_KEPT);
                r.runs.drain(..extra);
            }
        }
        Ok(())
    })
    .await?;
    result
}

/// Run now: ignores the conditions and the trigger.
pub async fn run_now(app: &App, id: &str) -> Result<Session> {
    let record = get(app, id).await?;
    if let Some(why) = thread_busy(app, &record) {
        bail!("{why}: wait for its turn to end, or Stop it");
    }
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { start(&app, &record.id, "Run now".into(), String::new(), false).await }).await?
}

/// Starts an event run when the routine allows it; otherwise records why it was skipped.
async fn on_event(app: &App, r: &CronAgent, cause: String, context: String) -> Result<()> {
    match why_blocked(app, r).await? {
        Some(why) => skip(app, &r.id, cause, why).await,
        None => start(app, &r.id, cause, context, true).await.map(|_| ()),
    }
}

async fn tick(app: &App, record: CronAgent) -> Result<()> {
    let now = now_ms();
    // Keep this occurrence due until the protected harness has a fresh reading.
    let spec = effective_spec(app, &record).await?;
    if spec.harness.is_some_and(|h| !crate::quota::automatic_ready(app, h, &spec)) && record.next_run_at.is_some_and(|d| now - d <= MISSED_GRACE_MS) {
        return Ok(());
    }
    let due = record.next_run_at;
    let (record, late) = modify(app, &record.id, move |r| {
        if !r.active() || r.next_run_at != due {
            bail!("already handled");
        }
        r.next_run_at = Some(next_after(&r.schedule, r.time_zone.as_deref(), now)?);
        r.last_check_at = Some(now);
        Ok(due.is_some_and(|d| now - d > MISSED_GRACE_MS))
    })
    .await?;
    if late {
        return skip(app, &record.id, "Scheduled".into(), "Launchpad was not running at the scheduled time".into()).await;
    }
    on_event(app, &record, "Scheduled".into(), String::new()).await
}

/// Home: a turn ended (the session left Running). Settles the routine run it belongs to, then starts the
/// routines triggered by another agent's turn ending Done. A routine's own runs never trigger routines.
/// Boxed: it can launch sessions, whose status changes call it again.
pub fn turn_ended(app: &App, s: &Session) {
    let (app, s) = (app.clone(), s.clone());
    let task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async move {
        if let Err(e) = turn_ended_inner(&app, &s).await {
            tracing::warn!("routines after the turn of {}: {e:#}", s.id);
        }
    });
    tokio::spawn(task);
}

async fn turn_ended_inner(app: &App, s: &Session) -> Result<()> {
    if s.status == SessionStatus::QuotaPaused {
        return Ok(());
    }
    let outcome = match s.status {
        SessionStatus::Waiting if is_no_update(s.last_message.as_deref()) => RunOutcome::NoUpdate,
        SessionStatus::Waiting => RunOutcome::Report,
        SessionStatus::Error => RunOutcome::Failed,
        SessionStatus::Interrupted => RunOutcome::Interrupted,
        _ => RunOutcome::Stopped,
    };
    let records = list(app).await?;
    let mut was_run = false;
    for r in records.iter().filter(|r| r.history.iter().any(|x| x.outcome == RunOutcome::Running && x.session_id.as_deref() == Some(&s.id))) {
        was_run = true;
        let sid = s.id.clone();
        let (_, quiet_ok) = modify(app, &r.id, move |r| {
            let mut quiet_ok = false;
            if let Some(run) = r.history.iter_mut().rev().find(|x| x.outcome == RunOutcome::Running && x.session_id.as_deref() == Some(&sid)) {
                run.outcome = outcome;
                quiet_ok = run.quiet_ok;
                let run = run.clone();
                r.last_outcome = Some(summary(&run));
            }
            Ok(quiet_ok)
        })
        .await?;
        // A quiet run is not Needs you, unless an earlier result in its thread still waits for you.
        if outcome == RunOutcome::NoUpdate && quiet_ok {
            app.sessions.mark_seen(app, &s.id).await?;
            if let Some(runner) = s.runner_id.as_deref().filter(|r| app.runners.connected(r)) {
                let _ = app.runners.request(runner, "POST", &format!("/api/sessions/{}/seen", s.id), None).await;
            }
        }
    }
    if was_run || s.status != SessionStatus::Waiting || s.open_question || matches!(s.launch_spec.target, Target::Assistant { .. }) {
        return Ok(());
    }
    let folder = s.project_id.clone();
    let workspace = match &folder {
        Some(f) => crate::projects::get(app, f).await?.and_then(|p| p.workspace_id),
        None => None,
    }
    .or_else(|| s.launch_spec.workspace_id.clone());
    for r in records.iter().filter(|r| r.active()) {
        let Trigger::SessionDone { folder: want } = &r.trigger else { continue };
        let matches = match want {
            Some(f) => folder.as_deref() == Some(f.as_str()),
            None => workspace.is_some() && workspace == r.spec.workspace_id,
        };
        let own = r.runs.contains(&s.id) || (r.bot_id.is_some() && s.launch_spec.bot.as_ref().map(|b| &b.id) == r.bot_id.as_ref());
        if !matches || own {
            continue;
        }
        let cause = format!("“{}” is done", s.title.chars().take(80).collect::<String>());
        let context = format!(
            "- The agent session “{}” ({}) finished a turn. Read what it did with the Launchpad CLI (the command given in your first instructions), redirecting the output to a file: `<launchpad> trace {} --through {}`",
            s.title.replace('"', "'"),
            s.id,
            s.id,
            s.last_seq
        );
        on_event(app, r, cause, context).await?;
    }
    Ok(())
}

/// The repositories of a workspace's folders, lowercase.
async fn workspace_repos(app: &App, workspace: Option<&str>) -> Result<Vec<String>> {
    let Some(ws) = workspace else { return Ok(vec![]) };
    Ok(crate::projects::list(app)
        .await?
        .into_iter()
        .filter(|p| p.workspace_id.as_deref() == Some(ws))
        .flat_map(|p| [p.gh_repo, p.gh_upstream])
        .flatten()
        .map(|r| r.to_lowercase())
        .collect())
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default()
}

/// After each Inbox sync: starts GitHub routines with the new activity that matches them, one run for all of it.
pub async fn inbox_synced(app: &App) {
    if let Err(e) = inbox_synced_inner(app).await {
        tracing::warn!("GitHub routines: {e:#}");
    }
}

async fn inbox_synced_inner(app: &App) -> Result<()> {
    let records: Vec<CronAgent> = list(app).await?.into_iter().filter(|r| r.active() && matches!(r.trigger, Trigger::Github { .. })).collect();
    if records.is_empty() {
        return Ok(());
    }
    let threads = app.inbox.list(app).await?.threads;
    let now = now_ms();
    for r in records {
        let Trigger::Github { repo, events } = &r.trigger else { continue };
        let repos = match repo {
            Some(x) => vec![x.to_lowercase()],
            None => workspace_repos(app, r.spec.workspace_id.as_deref()).await?,
        };
        let seen = r.github_seen.clone().unwrap_or_else(|| rfc3339(r.created_at));
        let new: Vec<_> = threads
            .iter()
            .filter(|t| t.unread && t.updated_at > seen && repos.contains(&t.repo.to_lowercase()))
            .filter_map(|t| events.iter().find(|e| e.matches(&t.kind, &t.reason)).map(|e| (t, *e)))
            .collect();
        let Some(newest) = new.iter().map(|(t, _)| t.updated_at.clone()).max() else { continue };
        let spec = effective_spec(app, &r).await?;
        let stale = |t: &crate::inbox::Thread| chrono::DateTime::parse_from_rfc3339(&t.updated_at).is_ok_and(|d| now - d.timestamp_millis() > MISSED_GRACE_MS);
        // Wait for a fresh quota reading, but never longer than an hour.
        if spec.harness.is_some_and(|h| !crate::quota::automatic_ready(app, h, &spec)) && new.iter().any(|(t, _)| !stale(t)) {
            continue;
        }
        let claimed = newest.clone();
        let (record, ()) = modify(app, &r.id, move |r| {
            ensure!(r.github_seen.as_deref().is_none_or(|s| s < claimed.as_str()), "already handled");
            r.github_seen = Some(claimed);
            r.last_check_at = Some(now);
            Ok(())
        })
        .await?;
        let fresh: Vec<_> = new.iter().filter(|(t, _)| !stale(t)).collect();
        let cause = match fresh.as_slice() {
            [(t, e)] => format!("GitHub, {}: {}{}", e.label(), t.repo, t.number.map(|n| format!("#{n}")).unwrap_or_default()),
            [] => format!("GitHub activity ({})", new.len()),
            many => format!("GitHub activity ({})", many.len()),
        };
        if fresh.is_empty() {
            skip(app, &record.id, cause, "the activity is more than 1 hour old".into()).await?;
            continue;
        }
        let context = fresh
            .iter()
            .map(|(t, e)| format!("- {}: {}{} “{}” ({}, {}) {}", e.label(), t.repo, t.number.map(|n| format!("#{n}")).unwrap_or_default(), t.title.replace('"', "'"), t.kind, t.reason, t.url))
            .collect::<Vec<_>>()
            .join("\n");
        on_event(app, &record, cause, context).await?;
    }
    Ok(())
}

/// At start: a run claimed but never started (Launchpad stopped in between) is uncertain, never retried.
async fn recover(app: &App) -> Result<()> {
    for r in list(app).await? {
        if r.history.iter().any(|x| x.outcome == RunOutcome::Running && x.session_id.is_none()) {
            modify(app, &r.id, |r| {
                for x in r.history.iter_mut().filter(|x| x.outcome == RunOutcome::Running && x.session_id.is_none()) {
                    x.outcome = RunOutcome::Failed;
                    x.detail = Some("Launchpad stopped while starting this run; check Agents before running it again".into());
                }
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = recover(&app).await {
            tracing::warn!("routines: {e:#}");
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let records = match list(&app).await {
                Ok(records) => records,
                Err(e) => {
                    tracing::error!("routines: {e:#}");
                    continue;
                }
            };
            let now = now_ms();
            for record in records {
                if record.active() && record.trigger == Trigger::Schedule && record.next_run_at.is_some_and(|at| at <= now) {
                    let app = app.clone();
                    tokio::spawn(async move {
                        if let Err(e) = tick(&app, record).await {
                            tracing::debug!("routine run: {e:#}");
                        }
                    });
                }
            }
        }
    });
}

fn validate_harness_conditions(
    harness: crate::model::HarnessKind,
    conditions: &[CronCondition],
) -> Result<()> {
    anyhow::ensure!(
        crate::harness::registration(harness)
            .descriptor
            .capabilities
            .quota
            || !conditions
                .iter()
                .any(|c| matches!(c, CronCondition::QuotaLeft { .. })),
        "This harness does not expose subscription quota readings"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_update_is_the_whole_final_message() {
        for m in ["no_update", " `no_update` ", "**no_update**", "NO_UPDATE.", "\"no_update\""] {
            assert!(is_no_update(Some(m)), "{m}");
        }
        for m in ["", "no_update: the build is red", "Nothing new, no_update", "noupdate"] {
            assert!(!is_no_update(Some(m)), "{m}");
        }
        assert!(!is_no_update(None));
    }

    #[test]
    fn github_events_match_notification_reasons() {
        assert!(GithubEvent::ReviewRequested.matches("PullRequest", "review_requested"));
        assert!(!GithubEvent::ReviewRequested.matches("PullRequest", "mention"));
        assert!(GithubEvent::Mentioned.matches("Issue", "team_mention"));
        assert!(GithubEvent::CiActivity.matches("CheckSuite", "subscribed"));
        assert!(GithubEvent::Release.matches("Release", "subscribed"));
        assert!(GithubEvent::Any.matches("Discussion", "comment"));
    }

    #[test]
    fn schedules_run_in_their_time_zone() {
        // 2026-01-05 00:00 UTC, a Monday: 8:30 in Paris (UTC+1 in winter) is 7:30 UTC.
        let monday = 1_767_571_200_000;
        assert_eq!(next_after("30 8 * * 1-5", Some("Europe/Paris"), monday).unwrap(), monday + (7 * 60 + 30) * 60_000);
        assert_eq!(next_after("30 8 * * 1-5", Some("UTC"), monday).unwrap(), monday + (8 * 60 + 30) * 60_000);
        assert!(next_after("30 8 * * *", Some("Mars/Base"), monday).is_err());
    }
}

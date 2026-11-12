//! Routines (code: crons): runs that start on their own. A trigger starts a run: a 5-field cron schedule (local
//! time), new GitHub activity in the Inbox, another agent's turn ending Done, a Launchpad failure, or new messages
//! in a bot channel (`channel_trigger`). A routine of a bot sends its
//! instructions to the bot's thread; any other routine launches the same spec as a new session.
//!
//! A bot's routines are defined by files of its store, `routines/<name>.toml` (`RoutineFile`): saving one here
//! writes its file, and every write of the bot's files syncs the records (`sync_bot_files`). The records keep what
//! files do not: runs, the next due time, the GitHub and failure watermarks. Other routines are records only.
//!
//! Every run is claimed before it starts (`next_run_at` moves forward, the GitHub watermark advances, a history
//! entry is written), so a crash never repeats it, and a run more than one hour late is skipped. A run ends with
//! a report, or with the final message `no_update`, which stays silent (no notification, not Needs you).
use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::schedules::QuotaKind;
use launchpad_core::{App, model::{LaunchSpec, RoutineLaunch, Session, SessionStatus, Target}, util::now_ms};

/// Runs later than this (daemon stopped, old GitHub activity) are skipped instead of started.
pub(crate) const MISSED_GRACE_MS: i64 = 60 * 60_000;
const RUNS_KEPT: usize = 50;
/// Runs a routine started by events (GitHub, another agent's turn, failures) may start in an hour; a later event is
/// skipped. Schedules are not limited: their cron sets their pace.
pub const EVENT_RUNS_PER_HOUR: usize = 6;
pub use launchpad_core::routines::{NO_UPDATE, TRIGGER_FORMS, instruction, is_no_update};

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
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
    /// A turn of another agent ended Done, in this folder (any folder of the workspace when None). Turns of bot
    /// threads count only with `include_bots`: by default a bot's answer never starts a routine.
    SessionDone {
        folder: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(optional, as = "Option<bool>")]
        include_bots: bool,
    },
    /// Launchpad failures of these kinds (`failures::FailureKind`) in the workspace, plus failures shared by
    /// every workspace (runners, Home, Burst jobs without a session). One run for all new failures.
    Failure { events: Vec<crate::failures::FailureKind> },
    /// New messages in this bot channel or DM (its ID), never the routine's own bot's (bot routines only). With
    /// `mentions_only`, only messages that mention the bot. One run for all new messages of a check.
    Channel {
        channel: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(optional, as = "Option<bool>")]
        mentions_only: bool,
    },
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
    /// Skipped because Launchpad could not confirm what it needed (a quota reading) for an hour: a failure for
    /// `failures::collect`.
    #[serde(default)]
    #[ts(skip)]
    pub missed: bool,
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
    /// Failure trigger: the newest failure (`Failure::seen_at`, Unix ms) already handled.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub failure_seen: Option<i64>,
    /// Channel trigger: the newest channel message (its ID) already handled.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub channel_seen: Option<i64>,
    /// A bot routine: its file in the bot's store (`routines/<name>.toml`), the definition this record follows.
    /// None for other routines.
    #[serde(default)]
    pub file: Option<String>,
    /// Why its file cannot be used (a hand edit with an error): the routine keeps its last good definition.
    #[serde(default)]
    pub file_error: Option<String>,
    /// Why its due run has not started yet (for example no confirmed quota reading). Never stored: `list` fills it.
    #[serde(default, skip_deserializing)]
    pub waiting: Option<String>,
}

impl CronAgent {
    pub(crate) fn active(&self) -> bool {
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
    quota: &launchpad_core::status::QuotaStatus,
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
            let mut st = c.prepare("SELECT id, json FROM cron_agent ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(crate::decode_rows("routine", rows, |s| Ok(serde_json::from_str(s)?)))
        })
        .await
        .map(|mut records: Vec<CronAgent>| {
            let held = HELD.lock().unwrap();
            for r in &mut records {
                r.waiting = held.get(&r.id).filter(|(due, _)| r.next_run_at == Some(*due)).map(|(_, why)| why.clone());
            }
            records
        })
}

pub async fn get(app: &App, id: &str) -> Result<CronAgent> {
    list(app).await?.into_iter().find(|r| r.id == id).context("this routine no longer exists")
}

/// Read-modify-write in the DB actor, which serializes the scheduler and UI requests.
pub(crate) async fn modify<T: Send + 'static>(
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

pub(crate) fn validate_trigger(trigger: &Trigger, schedule: &str, zone: Option<&str>) -> Result<()> {
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
        Trigger::Failure { events } => {
            ensure!(!events.is_empty(), "pick at least one kind of failure");
            Ok(())
        }
        Trigger::Channel { channel, .. } => {
            ensure!(!channel.trim().is_empty(), "pick a channel");
            Ok(())
        }
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
    ensure!(!matches!(trigger, Trigger::Channel { .. }), "a channel trigger is for a bot's routines: add it on the bot's page");
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
        failure_seen: None,
        channel_seen: None,
        file: None,
        file_error: None,
        waiting: None,
    };
    record.next_run_at = next_due(&record, now)?;
    insert(app, &record).await?;
    Ok(record)
}

/// The spec a bot routine stands for: the bot's profile and folder (for quotas and workspace lists).
async fn bot_spec(app: &App, bot: &launchpad_core::bots::Bot, text: String) -> Result<LaunchSpec> {
    let config = app.config.get();
    let profile = launchpad_core::bots::profile_of(&config, &bot.settings).cloned().context("choose a profile for this bot on its page (About → Access)")?;
    Ok(LaunchSpec {
        text,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        target: bot.settings.folder.clone().map_or(Target::Temp, |project_id| Target::Project { project_id }),
        workspace_id: Some(bot.workspace_id.clone()),
        ..Default::default()
    })
}

/// The spec a run uses now: a bot's current profile, else the routine's own spec.
async fn effective_spec(app: &App, r: &CronAgent) -> Result<LaunchSpec> {
    match &r.bot_id {
        Some(b) => bot_spec(app, &launchpad_core::bots::get(app, b).await?, r.spec.text.clone()).await,
        None => Ok(r.spec.clone()),
    }
}

/// Whether a routine's runs are messages to a bot's thread rather than new sessions.
fn in_thread(r: &CronAgent) -> bool {
    r.bot_id.is_some()
}

/// A new routine of a bot: writes its file `routines/<name>.toml` (the name from its title), which creates the
/// record. Its runs are messages to the bot's thread.
pub async fn create_for_bot(app: &App, bot_id: &str, r: BotRoutine) -> Result<CronAgent> {
    let _edit = EDIT.lock().await;
    let bot = launchpad_core::bots::get(app, bot_id).await?;
    let mut r = r;
    crate::channel_trigger::resolve_trigger(app, &bot, &mut r.trigger).await?;
    let file = RoutineFile {
        title: r.title.trim().to_string(),
        paused: false,
        schedule: normalize(&r.schedule),
        time_zone: zone_for(app, r.time_zone)?,
        instructions: r.instructions.trim().to_string(),
        trigger: r.trigger,
        conditions: r.conditions,
    };
    let text = render_file(&file)?;
    check_file(app, &bot, &text).await?;
    let taken: Vec<String> = launchpad_core::bots::routine_files(app, bot_id).await?.into_iter().map(|(p, _)| p).collect();
    let path = file_path_for(&file.title, &taken);
    launchpad_core::bots::write_files(app, bot_id, vec![(path.clone(), Some(text))]).await?;
    by_file(app, bot_id, &path).await
}

/// Serializes the saves that build a routine file from its record (create, edit, delete), so two of them never
/// pick the same new file name or write over each other's change.
static EDIT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The record of a bot's routine file.
async fn by_file(app: &App, bot_id: &str, path: &str) -> Result<CronAgent> {
    list(app).await?.into_iter().find(|r| r.bot_id.as_deref() == Some(bot_id) && r.file.as_deref() == Some(path)).context("the routine's file was saved, but its routine is missing: open the bot's routines")
}

pub async fn update(app: &App, id: &str, patch: CronPatch) -> Result<CronAgent> {
    let _edit = EDIT.lock().await;
    let current = get(app, id).await?;
    if let (Some(bot_id), Some(path)) = (&current.bot_id, &current.file) {
        return update_file(app, &current, bot_id, path, patch).await;
    }
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
            ensure!(!matches!(t, Trigger::Channel { .. }), "a channel trigger is for a bot's routines: add it on the bot's page");
            if t != r.trigger {
                // A new trigger starts from now: never runs for activity or failures from before the change.
                r.github_seen = Some(rfc3339(now));
                r.failure_seen = Some(now);
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

/// A bot routine's change: its file gets the record's definition with the patch, then the record follows it.
async fn update_file(app: &App, current: &CronAgent, bot_id: &str, path: &str, patch: CronPatch) -> Result<CronAgent> {
    let mut f = file_of(current);
    if let Some(s) = patch.schedule {
        f.schedule = normalize(&s);
    }
    if let Some(z) = patch.time_zone {
        f.time_zone = zone_for(app, Some(z))?;
    }
    if let Some(t) = patch.trigger {
        f.trigger = t;
    }
    if let Some(c) = patch.conditions {
        f.conditions = c;
    }
    if let Some(p) = patch.paused {
        f.paused = p;
    }
    if let Some(t) = patch.title {
        f.title = t.trim().to_string();
    }
    if let Some(i) = patch.instructions {
        f.instructions = i.trim().to_string();
    }
    let bot = launchpad_core::bots::get(app, bot_id).await?;
    crate::channel_trigger::resolve_trigger(app, &bot, &mut f.trigger).await?;
    let text = render_file(&f)?;
    check_file(app, &bot, &text).await?;
    launchpad_core::bots::write_files(app, bot_id, vec![(path.to_string(), Some(text))]).await?;
    get(app, &current.id).await
}

pub async fn delete(app: &App, id: &str) -> Result<()> {
    let _edit = EDIT.lock().await;
    if let Ok(r) = get(app, id).await
        && let (Some(bot_id), Some(path)) = (&r.bot_id, &r.file)
        && launchpad_core::bots::routine_files(app, bot_id).await?.iter().any(|(p, _)| p == path)
    {
        // Deleting its file deletes the record (`sync_bot_files`).
        return launchpad_core::bots::write_files(app, bot_id, vec![(path.clone(), None)]).await;
    }
    delete_record(app, id).await
}

async fn delete_record(app: &App, id: &str) -> Result<()> {
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

/// Home: a deleted bot takes its routines with it (its files go with its store).
pub async fn delete_for_bot(app: &App, bot_id: &str) -> Result<()> {
    for r in list(app).await?.into_iter().filter(|r| r.bot_id.as_deref() == Some(bot_id)) {
        delete_record(app, &r.id).await?;
    }
    Ok(())
}

/// `routines/<name>.toml` in a bot's store: the definition of one of its routines.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RoutineFile {
    pub title: String,
    /// No run starts while it is paused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
    /// 5-field cron expression (schedule trigger).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schedule: String,
    /// IANA time zone of the schedule. None: `[home] time_zone`, else the host's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// What the bot does at each run.
    pub instructions: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<CronCondition>,
}

const FILE_HEADER: &str = "# A routine of this bot: its runs are messages to the bot's thread. Saved from the bot's page; a hand edit\n# applies at once. trigger.kind: schedule (with schedule, a 5-field cron), github, session_done, failure or channel.\n";

pub fn render_file(f: &RoutineFile) -> Result<String> {
    Ok(format!("{FILE_HEADER}{}", toml::to_string(f)?))
}

/// Reads a routine file and checks what needs no bot: a name, instructions, a valid trigger and conditions.
pub fn parse_file(text: &str) -> Result<RoutineFile> {
    let mut f: RoutineFile = toml::from_str(text).context("routine file")?;
    f.title = f.title.trim().to_string();
    f.instructions = f.instructions.trim().to_string();
    f.schedule = normalize(&f.schedule);
    f.time_zone = f.time_zone.map(|z| z.trim().to_string()).filter(|z| !z.is_empty());
    ensure!(!f.title.is_empty(), "give the routine a title");
    ensure!(!f.instructions.is_empty(), "write what the bot does at each run (instructions)");
    if let Some(z) = &f.time_zone {
        parse_zone(z)?;
    }
    validate_conditions(&f.conditions)?;
    validate_trigger(&f.trigger, &f.schedule, f.time_zone.as_deref())?;
    Ok(f)
}

/// `parse_file`, plus the checks that need the bot (its harness reads quota for quota conditions).
pub async fn check_file(app: &App, bot: &launchpad_core::bots::Bot, text: &str) -> Result<RoutineFile> {
    let f = parse_file(text)?;
    if let Ok(spec) = bot_spec(app, bot, String::new()).await
        && let Some(h) = spec.harness
    {
        validate_harness_conditions(h, &f.conditions)?;
    }
    if let Trigger::Channel { channel, .. } = &f.trigger {
        crate::channel_trigger::check(app, bot, channel).await?;
    }
    Ok(f)
}

/// The definition a record holds, as a file.
fn file_of(r: &CronAgent) -> RoutineFile {
    RoutineFile { title: r.title.clone(), paused: r.paused, schedule: r.schedule.clone(), time_zone: r.time_zone.clone(), instructions: r.spec.text.clone(), trigger: r.trigger.clone(), conditions: r.conditions.clone() }
}

/// `routines/<slug of the title>.toml`, not one of `taken`.
pub fn file_path_for(title: &str, taken: &[String]) -> String {
    let names: Vec<String> = taken.iter().filter_map(|p| p.strip_prefix(launchpad_core::bots::ROUTINES)?.strip_suffix(".toml")).map(str::to_string).collect();
    let mut slug = launchpad_core::bots::slug_for(title, &names);
    if slug == "bot" && !title.to_ascii_lowercase().contains("bot") {
        slug = launchpad_core::bots::slug_for("routine", &names);
    }
    format!("{}{slug}.toml", launchpad_core::bots::ROUTINES)
}

/// Serializes syncs, so two writes never create two records for one file.
static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Home, after every write of a bot's files: its routine records follow `routines/*.toml`. A new file makes a
/// record, a changed one updates its definition (a new schedule, time zone, trigger or pause recomputes the next
/// run from now; a new trigger starts its watermarks from now), a removed one deletes its record and history. A file with
/// an error keeps the record's last good definition and shows the error (`file_error`); a new file with an error
/// gets a paused record that shows it.
pub async fn sync_bot_files(app: &App, bot_id: &str) -> Result<()> {
    let _sync = SYNC.lock().await;
    let Ok(bot) = launchpad_core::bots::get(app, bot_id).await else { return Ok(()) };
    let files = launchpad_core::bots::routine_files(app, bot_id).await?;
    let records: Vec<CronAgent> = list(app).await?.into_iter().filter(|r| r.bot_id.as_deref() == Some(bot_id)).collect();
    let now = now_ms();
    let head = crate::channel_trigger::head(app).await?;
    for (path, text) in &files {
        let parsed = check_file(app, &bot, text).await.map_err(|e| format!("{e:#}"));
        match (records.iter().find(|r| r.file.as_deref() == Some(path.as_str())), parsed) {
            (Some(r), Ok(f)) => {
                // An active schedule always has its next due time.
                let undue = |r: &CronAgent| r.active() && r.trigger == Trigger::Schedule && r.next_run_at.is_none();
                if file_of(r) == f && r.file_error.is_none() && !undue(r) {
                    continue;
                }
                modify(app, &r.id, move |r| {
                    let timing = r.schedule != f.schedule || r.time_zone != f.time_zone || r.trigger != f.trigger || r.paused != f.paused || undue(r);
                    if r.trigger != f.trigger {
                        r.github_seen = Some(rfc3339(now));
                        r.failure_seen = Some(now);
                        r.channel_seen = Some(head);
                    }
                    r.title = f.title;
                    r.paused = f.paused;
                    r.schedule = f.schedule;
                    r.time_zone = f.time_zone;
                    r.spec.text = f.instructions;
                    r.trigger = f.trigger;
                    r.conditions = f.conditions;
                    r.file_error = None;
                    if timing {
                        r.next_run_at = next_due(r, now)?;
                    }
                    Ok(())
                })
                .await?;
            }
            (Some(r), Err(e)) => {
                if r.file_error.as_deref() != Some(e.as_str()) {
                    modify(app, &r.id, move |r| {
                        r.file_error = Some(e);
                        Ok(())
                    })
                    .await?;
                }
            }
            (None, parsed) => {
                let (f, error) = match parsed {
                    Ok(f) => (f, None),
                    // Nothing good to run yet: a paused record that shows the error until the file is fixed.
                    Err(e) => (RoutineFile { title: path.trim_start_matches(launchpad_core::bots::ROUTINES).to_string(), paused: true, schedule: String::new(), time_zone: None, instructions: String::new(), trigger: Trigger::Schedule, conditions: vec![] }, Some(e)),
                };
                let spec = match bot_spec(app, &bot, f.instructions.clone()).await {
                    Ok(s) => s,
                    Err(_) => LaunchSpec { text: f.instructions.clone(), target: Target::Temp, workspace_id: Some(bot.workspace_id.clone()), ..Default::default() },
                };
                let mut record = CronAgent {
                    id: uuid::Uuid::new_v4().to_string(),
                    spec,
                    title: f.title,
                    schedule: f.schedule,
                    time_zone: f.time_zone,
                    trigger: f.trigger,
                    bot_id: Some(bot.id.clone()),
                    bot_paused: bot.paused,
                    conditions: f.conditions,
                    paused: f.paused,
                    created_at: now,
                    next_run_at: None,
                    last_check_at: None,
                    last_outcome: None,
                    runs: vec![],
                    history: vec![],
                    github_seen: None,
                    failure_seen: None,
                    channel_seen: Some(head),
                    file: Some(path.clone()),
                    file_error: error,
                    waiting: None,
                };
                record.next_run_at = next_due(&record, now).unwrap_or(None);
                insert(app, &record).await?;
            }
        }
    }
    for r in records.iter().filter(|r| r.file.as_ref().is_some_and(|p| !files.iter().any(|(f, _)| f == p))) {
        delete_record(app, &r.id).await?;
    }
    Ok(())
}

/// Home, at start, once: every bot routine made before routines were files gets its file, and its record is keyed
/// to it. Then every bot's records follow its files. Idempotent until claimed: a file already written with the
/// same content is kept.
pub async fn files_from_records(app: &App) -> Result<()> {
    if !launchpad_core::home_git::here(app) {
        return Ok(());
    }
    const KEY: &str = "bot-routine-files";
    let done = app.state_db.call(|c| Ok(c.query_row("SELECT 1 FROM seeded WHERE key = ?1", [KEY], |_| Ok(())).is_ok())).await?;
    let bots = launchpad_core::bots::list(app).await?;
    if !done {
        let mut moved = 0;
        for r in list(app).await?.into_iter().filter(|r| r.file.is_none()) {
            let Some(bot) = r.bot_id.as_deref().and_then(|b| bots.iter().find(|x| x.id == b)) else { continue };
            let text = render_file(&file_of(&r))?;
            let files = launchpad_core::bots::routine_files(app, &bot.id).await?;
            let path = match files.iter().find(|(_, t)| *t == text) {
                // Written by an attempt that stopped before keying the record.
                Some((p, _)) if !list(app).await?.iter().any(|x| x.file.as_deref() == Some(p.as_str()) && x.bot_id == r.bot_id) => p.clone(),
                _ => file_path_for(&r.title, &files.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()),
            };
            launchpad_core::bots::write_store(app, &launchpad_core::bots::store(&bot.id), vec![(path.clone(), Some(text))]).await?;
            modify(app, &r.id, move |r| {
                r.file = Some(path);
                Ok(())
            })
            .await?;
            moved += 1;
        }
        if moved > 0 {
            tracing::info!("wrote {moved} bot routines as files of their bots");
        }
        app.state_db.call(|c| Ok(c.execute("INSERT OR IGNORE INTO seeded (key, at) VALUES (?1, ?2)", params![KEY, now_ms()]).map(|_| ())?)).await?;
    }
    for b in &bots {
        if let Err(e) = sync_bot_files(app, &b.id).await {
            tracing::warn!("routines of {}: {e:#}", b.settings.name);
        }
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

pub(crate) async fn skip(app: &App, id: &str, cause: String, reason: String) -> Result<()> {
    skip_run(app, id, cause, reason, false).await
}

/// `skip`, marked as a failure when `missed`.
pub(crate) async fn skip_run(app: &App, id: &str, cause: String, reason: String, missed: bool) -> Result<()> {
    modify(app, id, move |r| {
        push_run(r, RoutineRun { at: now_ms(), cause, session_id: None, outcome: RunOutcome::Skipped, detail: Some(reason), quiet_ok: false, missed });
        Ok(())
    })
    .await
    .map(|_| ())
}

/// Why an event may not start a run of the routine now: the run's message would pass the loop guard (`hops`, the
/// automatic messages in its chain, itself included: `senders::guard`), or the routine already started
/// `EVENT_RUNS_PER_HOUR` runs in the last hour (event triggers only).
pub(crate) fn limit_reason(r: &CronAgent, hops: u32, now: i64) -> Option<String> {
    if let Err(e) = launchpad_core::senders::guard(hops) {
        return Some(format!("{e:#}"));
    }
    if r.trigger == Trigger::Schedule {
        return None;
    }
    let started = r.history.iter().filter(|x| x.outcome != RunOutcome::Skipped && now - x.at < 3_600_000).count();
    (started >= EVENT_RUNS_PER_HOUR).then(|| format!("rate limit: {started} runs in the last hour (at most {EVENT_RUNS_PER_HOUR} for a routine started by events)"))
}

/// The sender of a routine run's message: the routine, `hops` into its chain.
fn routine_sender(r: &CronAgent, hops: u32) -> launchpad_core::model::MessageFrom {
    launchpad_core::model::MessageFrom { kind: launchpad_core::model::SenderKind::Routine, id: r.id.clone(), name: r.title.clone(), hops, ..Default::default() }
}

/// The message a bot routine sends to the bot's thread. The UI recognizes its first line.
fn bot_message(r: &CronAgent, launch: &RoutineLaunch) -> String {
    format!("Routine “{}” · {}\n\n{}\n\n{}", r.title, launch.cause, r.spec.text.trim(), instruction(launch))
}

/// Why a bot cannot take a routine's message now.
fn bot_busy(app: &App, bot_id: &str) -> Option<String> {
    let thread = launchpad_core::bots::thread(app, bot_id);
    let current = thread.iter().rev().find(|s| s.status != SessionStatus::Closed)?;
    (current.status.is_running() || current.quota_pause.is_some()).then(|| format!("{} is working", current.launch_spec.bot.as_ref().map(|b| b.name.as_str()).unwrap_or("the bot")))
}

/// The open session of the bot thread a routine posts to, None for other routines.
fn thread_current(app: &App, r: &CronAgent) -> Option<Session> {
    let b = r.bot_id.as_ref()?;
    launchpad_core::bots::thread(app, b).into_iter().rev().find(|s| s.status != SessionStatus::Closed)
}

/// Why the bot thread a routine posts to cannot take a message now.
fn thread_busy(app: &App, r: &CronAgent) -> Option<String> {
    bot_busy(app, r.bot_id.as_ref()?)
}

/// Why the routine's conditions block a run now, or None when it may start.
pub(crate) async fn why_blocked(app: &App, r: &CronAgent) -> Result<Option<String>> {
    if let Some(why) = thread_busy(app, r) {
        return Ok(Some(why));
    }
    let spec = effective_spec(app, r).await?;
    let harness = spec.harness.context("this routine has no registered harness")?;
    let quota = launchpad_core::quota::reading_for(app, harness, &spec);
    let previous_running = !in_thread(r)
        && r.runs.last().and_then(|id| app.sessions.get(id)).is_some_and(|s| app.sessions.is_busy(&s) || s.quota_pause.is_some());
    Ok(blocked(&r.conditions, &quota, now_ms(), previous_running))
}

/// Starts one run: claims a history entry, then sends the bot's message or launches the spec (sent once, never
/// retried). `automatic` runs need confirmed quota.
async fn start(app: &App, id: &str, cause: String, context: String, automatic: bool, hops: u32) -> Result<Session> {
    let at = now_ms();
    let current = get(app, id).await?;
    let quiet_ok = thread_current(app, &current).is_none_or(|s| !s.needs_you());
    let (record, ()) = modify(app, id, {
        let cause = cause.clone();
        move |r| {
            if automatic {
                ensure!(r.active(), "paused");
            }
            push_run(r, RoutineRun { at, cause, session_id: None, outcome: RunOutcome::Running, detail: None, quiet_ok, missed: false });
            Ok(())
        }
    })
    .await?;
    let launch = RoutineLaunch { id: record.id.clone(), title: record.title.clone(), cause, context };
    let result = match &record.bot_id {
        Some(bot) => {
            let context = launchpad_core::model::MessageContext { from: Some(routine_sender(&record, hops)), ..Default::default() };
            launchpad_core::bots::message_with(app, bot, bot_message(&record, &launch), vec![], context, None, automatic, None).await
        }
        None => {
            let spec = LaunchSpec { routine: Some(launch), from: Some(routine_sender(&record, hops)), ..record.spec.clone() };
            launchpad_core::runners::launch_anywhere(app, spec, uuid::Uuid::new_v4().to_string(), automatic).await
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
    // A routine whose file never worked has nothing to run yet.
    ensure!(!record.spec.text.trim().is_empty(), "{} has no instructions: fix its file {}", record.title, record.file.as_deref().unwrap_or_default());
    if let Some(why) = thread_busy(app, &record) {
        bail!("{why}: wait for its turn to end, or Stop it");
    }
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { start(&app, &record.id, "Run now".into(), String::new(), false, 0).await }).await?
}

/// Starts an event run when the routine allows it; otherwise records why it was skipped. `hops`: the automatic
/// messages in the chain of its message, itself included (1 for a schedule, GitHub or a failure).
pub(crate) async fn on_event(app: &App, r: &CronAgent, cause: String, context: String, hops: u32) -> Result<()> {
    if let Some(why) = limit_reason(r, hops, now_ms()) {
        return skip(app, &r.id, cause, why).await;
    }
    if let Some(why) = why_blocked(app, r).await? {
        return skip(app, &r.id, cause, why).await;
    }
    // Automatic work needs confirmed quota; an event never waits for it here (schedules, GitHub and failures wait
    // before they claim), so the run is missed.
    if let Some(why) = quota_hold(app, r).await? {
        return skip_run(app, &r.id, cause, format!("automatic work needs confirmed quota: {why}"), true).await;
    }
    start(app, &r.id, cause, context, true, hops).await.map(|_| ())
}

/// Why the routine's automatic run waits for its quota now (`quota::hold_reason`): of the bot's thread
/// (`bots::quota_hold`), else of the session it would start. Every trigger checks it before it starts a run.
pub(crate) async fn quota_hold(app: &App, r: &CronAgent) -> Result<Option<String>> {
    if let Some(b) = &r.bot_id {
        return Ok(launchpad_core::bots::quota_hold(app, &launchpad_core::bots::get(app, b).await?));
    }
    let spec = effective_spec(app, r).await?;
    Ok(spec.harness.and_then(|h| launchpad_core::quota::hold_reason(app, h, &spec)))
}

/// Why a routine's due occurrence waited, with that due time (Unix ms): the reason its late skip gives. In memory.
static HELD: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (i64, String)>>> = std::sync::LazyLock::new(Default::default);

async fn tick(app: &App, record: CronAgent) -> Result<()> {
    let now = now_ms();
    // Keep this occurrence due until the protected harness has a fresh reading.
    let held = quota_hold(app, &record).await?;
    let due = record.next_run_at;
    if let (Some(why), Some(d)) = (&held, due)
        && now - d <= MISSED_GRACE_MS
    {
        HELD.lock().unwrap().insert(record.id.clone(), (d, why.clone()));
        return Ok(());
    }
    let (record, late) = modify(app, &record.id, move |r| {
        if !r.active() || r.next_run_at != due {
            bail!("already handled");
        }
        r.next_run_at = Some(next_after(&r.schedule, r.time_zone.as_deref(), now)?);
        r.last_check_at = Some(now);
        Ok(due.is_some_and(|d| now - d > MISSED_GRACE_MS))
    })
    .await?;
    let waited = HELD.lock().unwrap().remove(&record.id).filter(|(d, _)| Some(*d) == due).map(|(_, why)| why);
    if late {
        let (reason, missed) = late_reason(due.unwrap_or(now), held.or(waited), crate::failures::started_at());
        return skip_run(app, &record.id, "Scheduled".into(), reason, missed).await;
    }
    on_event(app, &record, "Scheduled".into(), String::new(), 1).await
}

/// Why a scheduled run more than an hour late is skipped, and whether that is a failure: it waited for its quota
/// (`quota::hold_reason`), or Home started after the scheduled time (`home_started`, Unix ms).
fn late_reason(due: i64, held: Option<String>, home_started: Option<i64>) -> (String, bool) {
    if let Some(why) = held {
        return (format!("waited an hour after the scheduled time: {why}"), true);
    }
    match home_started.filter(|s| *s > due) {
        Some(s) => (format!("Home was not running at the scheduled time (it started {} min later)", (s - due) / 60_000), false),
        None => ("Home checked this routine more than an hour after the scheduled time".into(), false),
    }
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
    if was_run || s.status != SessionStatus::Waiting || s.open_question {
        return Ok(());
    }
    // Its runner still saves the turn's branches and files on Home: the routines start once they are there
    // (`turn_saved`), so a routine that reviews the work sees all of it.
    if s.unsaved.is_some() {
        UNSAVED_DONE.lock().unwrap().insert(s.id.clone(), now_ms());
        return Ok(());
    }
    UNSAVED_DONE.lock().unwrap().remove(&s.id);
    session_done(app, s, &records).await
}

/// Sessions whose turn ended Done while its work was not on Home yet, with when, for `turn_saved`. In memory:
/// after a restart they trigger nothing (routines never catch up).
static UNSAVED_DONE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> = std::sync::LazyLock::new(Default::default);

/// Home: the work of a session's last turn reached Home. Starts the `session_done` routines its turn end held
/// back, unless the session moved on (a new turn, a question, Archive) or it came over an hour late.
pub fn turn_saved(app: &App, s: &Session) {
    let Some(at) = UNSAVED_DONE.lock().unwrap().remove(&s.id) else { return };
    if s.status != SessionStatus::Waiting || s.open_question || now_ms() - at > 3_600_000 {
        return;
    }
    let (app, s) = (app.clone(), s.clone());
    let task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async move {
        let result = async { session_done(&app, &s, &list(&app).await?).await }.await;
        if let Err(e) = result {
            tracing::warn!("routines after the saved turn of {}: {e:#}", s.id);
        }
    });
    tokio::spawn(task);
}

/// Starts the routines triggered by another agent's turn ending Done.
async fn session_done(app: &App, s: &Session, records: &[CronAgent]) -> Result<()> {
    let folder = s.project_id.clone();
    let workspace = match &folder {
        Some(f) => launchpad_core::projects::get(app, f).await?.and_then(|p| p.workspace_id),
        None => None,
    }
    .or_else(|| s.launch_spec.workspace_id.clone());
    // The chain of the message its turn answered: the run's message is one more hop.
    let hops = launchpad_core::senders::hops_of(app, &s.id).await;
    for r in records.iter().filter(|r| r.active()) {
        let Trigger::SessionDone { folder: want, include_bots } = &r.trigger else { continue };
        if s.launch_spec.bot.is_some() && !include_bots {
            continue;
        }
        // Only agents of the routine's own workspace, even when it names a folder (workspaces are a boundary for
        // bots and agents); an older routine without a workspace keeps its folder.
        let matches = match want {
            Some(f) => folder.as_deref() == Some(f.as_str()) && (r.spec.workspace_id.is_none() || workspace == r.spec.workspace_id),
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
        on_event(app, r, cause, context, hops + 1).await?;
    }
    Ok(())
}

/// The repositories of a workspace's folders, lowercase.
async fn workspace_repos(app: &App, workspace: Option<&str>) -> Result<Vec<String>> {
    let Some(ws) = workspace else { return Ok(vec![]) };
    Ok(launchpad_core::projects::list(app)
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
    let threads = launchpad_core::ext::extensions().inbox_threads(app).await?;
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
        let stale = |t: &launchpad_core::github::Thread| chrono::DateTime::parse_from_rfc3339(&t.updated_at).is_ok_and(|d| now - d.timestamp_millis() > MISSED_GRACE_MS);
        // Wait for a fresh quota reading, but never longer than an hour.
        if new.iter().any(|(t, _)| !stale(t)) && quota_hold(app, &r).await?.is_some() {
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
            let (why, missed) = match quota_hold(app, &record).await? {
                Some(held) => (format!("the activity is more than 1 hour old; it waited for the quota: {held}"), true),
                None => ("the activity is more than 1 hour old".into(), false),
            };
            skip_run(app, &record.id, cause, why, missed).await?;
            continue;
        }
        let context = fresh
            .iter()
            .map(|(t, e)| format!("- {}: {}{} “{}” ({}, {}) {}", e.label(), t.repo, t.number.map(|n| format!("#{n}")).unwrap_or_default(), t.title.replace('"', "'"), t.kind, t.reason, t.url))
            .collect::<Vec<_>>()
            .join("\n");
        on_event(app, &record, cause, context, 1).await?;
    }
    Ok(())
}

/// Every 30 s on Home: notices new failures (`failures::collect`), then starts the failure routines that take
/// some of them, one run for all of it. A routine whose bot is working waits (its failures stay unclaimed) until
/// they are more than one hour old.
async fn failures_noticed(app: &App) -> Result<()> {
    crate::failures::collect(app).await?;
    let records: Vec<CronAgent> = list(app).await?.into_iter().filter(|r| r.active() && matches!(r.trigger, Trigger::Failure { .. })).collect();
    let now = now_ms();
    for r in records {
        // One routine's error (its bot has no profile) never holds up the others.
        if let Err(e) = failure_one(app, &r, now).await {
            tracing::debug!("failure routine {}: {e:#}", r.id);
        }
    }
    Ok(())
}

async fn failure_one(app: &App, r: &CronAgent, now: i64) -> Result<()> {
    let Trigger::Failure { events } = &r.trigger else { return Ok(()) };
    let seen = r.failure_seen.unwrap_or(r.created_at);
    let new: Vec<crate::failures::Failure> = crate::failures::recent(seen)
        .into_iter()
        .filter(|f| crate::failures::routine_takes(events, r.spec.workspace_id.as_deref(), &r.id, r.bot_id.as_deref(), &r.runs, f))
        .collect();
    let Some(newest) = new.iter().map(|f| f.seen_at).max() else { return Ok(()) };
    let fresh: Vec<&crate::failures::Failure> = new.iter().filter(|f| now - f.seen_at <= MISSED_GRACE_MS).collect();
    if !fresh.is_empty() && (why_blocked(app, r).await?.is_some() || quota_hold(app, r).await?.is_some()) {
        return Ok(());
    }
    let (record, ()) = modify(app, &r.id, move |r| {
        ensure!(r.failure_seen.is_none_or(|s| s < newest), "already handled");
        r.failure_seen = Some(newest);
        r.last_check_at = Some(now);
        Ok(())
    })
    .await?;
    let cause = match fresh.as_slice() {
        [f] => format!("Failure: {}", f.title.chars().take(100).collect::<String>()),
        many => format!("Failures ({})", many.len().max(new.len())),
    };
    if fresh.is_empty() {
        match quota_hold(app, &record).await? {
            Some(held) => skip_run(app, &record.id, cause, format!("the failures are more than 1 hour old; they waited for the quota: {held}"), true).await?,
            None => skip(app, &record.id, cause, "the failures are more than 1 hour old".into()).await?,
        }
        return Ok(());
    }
    let lines = fresh.iter().map(|f| crate::failures::describe(f)).collect::<Vec<_>>().join("\n");
    let context = format!("{lines}\n\nInvestigate with the Launchpad CLI (the command given in your first instructions): `health`, `sessions show <id>`, `logs --session <id>` (or `--runner <id>`), `burst runs`, `burst job <runner id>`, `runners list`, `home`, `failures`.");
    on_event(app, &record, cause, context, 1).await
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
        let mut ticks: u64 = 0;
        loop {
            interval.tick().await;
            ticks += 1;
            if ticks % 6 == 1 && app.paths.role != launchpad_core::role::Role::Runner {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = failures_noticed(&app).await {
                        tracing::warn!("failure routines: {e:#}");
                    }
                    if let Err(e) = crate::channel_trigger::check_all(&app).await {
                        tracing::warn!("channel routines: {e:#}");
                    }
                });
            }
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
    harness: launchpad_core::model::HarnessKind,
    conditions: &[CronCondition],
) -> Result<()> {
    anyhow::ensure!(
        launchpad_core::harness::registration(harness)
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
    fn librarian_routine_is_a_valid_paused_schedule() {
        assert!(launchpad_core::bots::routine_path_ok(launchpad_core::librarian::ROUTINE_PATH));
        let f = parse_file(launchpad_core::librarian::ROUTINE).unwrap();
        assert!(f.paused);
        assert_eq!((&f.trigger, f.schedule.as_str(), &f.time_zone), (&Trigger::Schedule, "0 3 * * 0", &None));
        assert_eq!(render_file(&f).unwrap(), launchpad_core::librarian::ROUTINE);
    }

    #[test]
    fn late_runs_say_why_they_were_skipped() {
        let due = 10 * 3_600_000;
        // Held by quota while Home ran: a failure, with the quota's reason.
        let (why, missed) = late_reason(due, Some("no confirmed Claude quota reading for vault:a: last reading 70 min ago; 429".into()), Some(due - 3_600_000));
        assert!(missed);
        assert_eq!(why, "waited an hour after the scheduled time: no confirmed Claude quota reading for vault:a: last reading 70 min ago; 429");
        // Home started late and held nothing: Home was down, not a failure of its own.
        let (why, missed) = late_reason(due, None, Some(due + 75 * 60_000));
        assert!(!missed);
        assert_eq!(why, "Home was not running at the scheduled time (it started 75 min later)");
        // Home started late, then waited for the quota for the rest of the hour: the quota is the reason.
        assert!(late_reason(due, Some("q".into()), Some(due + 10 * 60_000)).1);
        // Home ran the whole time and nothing held the run.
        let (why, missed) = late_reason(due, None, Some(due - 1));
        assert!(!missed);
        assert!(why.starts_with("Home checked this routine more than an hour"));
    }

    #[test]
    fn routine_files_round_trip_and_name_their_errors() {
        let f = RoutineFile {
            title: "Daily check".into(),
            paused: false,
            schedule: "30 8 * * 1-5".into(),
            time_zone: Some("Europe/Paris".into()),
            instructions: "Look at what changed.\nReport in a few lines.".into(),
            trigger: Trigger::Schedule,
            conditions: vec![CronCondition::QuotaLeft { window: QuotaKind::FiveHour, min_left_percent: 20 }],
        };
        let text = render_file(&f).unwrap();
        assert_eq!(parse_file(&text).unwrap(), f);
        let g = RoutineFile { trigger: Trigger::Github { repo: None, events: vec![GithubEvent::ReviewRequested] }, schedule: String::new(), conditions: vec![], paused: true, ..f.clone() };
        assert_eq!(parse_file(&render_file(&g).unwrap()).unwrap(), g);
        // A hand-written file with the trigger inline and defaults left out.
        let hand = "title = \"CI\"\ninstructions = \"Fix it\"\ntrigger = { kind = \"failure\", events = [\"any\"] }\n";
        assert!(matches!(parse_file(hand).unwrap().trigger, Trigger::Failure { .. }));
        assert!(parse_file("title = \"x\"\ninstructions = \"y\"\nschedule = \"every day\"\n").unwrap_err().to_string().contains("5 fields"));
        assert!(parse_file("title = \"x\"\ninstructions = \"y\"\nschedule = \"0 9 * * *\"\ncolor = 1\n").is_err());
        assert!(parse_file("title = \" \"\ninstructions = \"y\"\nschedule = \"0 9 * * *\"\n").is_err());
        assert_eq!(file_path_for("Daily check!", &["routines/daily-check.toml".into()]), "routines/daily-check-2.toml");
        assert_eq!(file_path_for("✦", &[]), "routines/routine.toml");
    }

    #[test]
    fn event_routines_stop_at_the_loop_guard_and_the_hourly_limit() {
        let now = 100 * 3_600_000;
        let record = |trigger: Trigger, runs: Vec<(i64, RunOutcome)>| -> CronAgent {
            let mut r: CronAgent = serde_json::from_value(serde_json::json!({"id": "r1", "spec": {}, "title": "Review", "schedule": "", "conditions": [], "paused": false, "created_at": 0, "next_run_at": null, "last_check_at": null, "last_outcome": null, "runs": []})).unwrap();
            r.trigger = trigger;
            r.history = runs.into_iter().map(|(at, outcome)| RoutineRun { at, cause: "x".into(), session_id: None, outcome, detail: None, quiet_ok: false, missed: false }).collect();
            r
        };
        let done = Trigger::SessionDone { folder: None, include_bots: false };
        // Within the hop limit and with few runs: it starts.
        assert_eq!(limit_reason(&record(done.clone(), vec![]), 1, now), None);
        assert_eq!(limit_reason(&record(done.clone(), vec![]), launchpad_core::senders::MAX_HOPS, now), None);
        // A message caused by too many automatic messages before it.
        let why = limit_reason(&record(done.clone(), vec![]), launchpad_core::senders::MAX_HOPS + 1, now).unwrap();
        assert!(why.starts_with("loop guard: 4 hops"), "{why}");
        // Six started runs in the last hour (skipped ones and older ones do not count).
        let mut runs: Vec<(i64, RunOutcome)> = (0..EVENT_RUNS_PER_HOUR as i64).map(|i| (now - i * 60_000, RunOutcome::Report)).collect();
        assert!(limit_reason(&record(done.clone(), runs.clone()), 1, now).unwrap().starts_with("rate limit: 6 runs in the last hour"));
        runs[0].1 = RunOutcome::Skipped;
        runs.push((now - 3_600_000, RunOutcome::Report));
        assert_eq!(limit_reason(&record(done, runs.clone()), 1, now), None);
        // Schedules keep the pace of their cron.
        let many: Vec<(i64, RunOutcome)> = (0..20).map(|i| (now - i * 60_000, RunOutcome::NoUpdate)).collect();
        assert_eq!(limit_reason(&record(Trigger::Schedule, many), 1, now), None);
    }

    #[test]
    fn session_done_triggers_leave_bots_out_unless_asked() {
        let hand = "title = \"x\"\ninstructions = \"y\"\ntrigger = { kind = \"session_done\", folder = \"f1\" }\n";
        assert_eq!(parse_file(hand).unwrap().trigger, Trigger::SessionDone { folder: Some("f1".into()), include_bots: false });
        let with_bots = RoutineFile { title: "x".into(), paused: false, schedule: String::new(), time_zone: None, instructions: "y".into(), trigger: Trigger::SessionDone { folder: None, include_bots: true }, conditions: vec![] };
        let text = render_file(&with_bots).unwrap();
        assert!(text.contains("include_bots = true"), "{text}");
        assert_eq!(parse_file(&text).unwrap(), with_bots);
        assert!(!render_file(&RoutineFile { trigger: Trigger::SessionDone { folder: None, include_bots: false }, ..with_bots }).unwrap().contains("include_bots"));
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

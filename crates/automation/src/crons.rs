//! Routines (code: crons): runs that start on their own. A trigger starts a run: a 5-field cron schedule (local
//! time), another agent's turn ending Done, a Launchpad failure, or new messages in a bot channel
//! (`channel_trigger`). A routine of a bot sends its
//! instructions to the bot's thread; any other routine launches the same spec as a new session.
//!
//! A bot's routines are defined by files of its store, `routines/<name>.toml` (`RoutineFile`): saving one here
//! writes its file, and every write of the bot's files syncs the records (`sync_bot_files`). The records keep what
//! files do not: runs, the next due time, the failure and channel watermarks. Other routines are records only.
//!
//! Every run is claimed before it starts (`next_run_at` moves forward, a watermark advances, a history
//! entry is written), so a crash never repeats it, and a run more than one hour late is skipped. A run ends with
//! a report, or with the final message `no_update`, which stays silent (no notification, not Needs you).
mod calendar;
mod repository;
mod files;

pub use calendar::{next_after, next_runs, zone_for};
pub use repository::{get, list};
pub(crate) use repository::modify;
pub use files::{RoutineFile, check_file, file_path_for, files_from_records, parse_file, render_file, sync_bot_files};
use calendar::{next_due, normalize, validate_conditions};
pub(crate) use calendar::validate_trigger;
use repository::insert;
use files::file_of;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::schedules::QuotaKind;
use launchpad_core::{App, model::{LaunchSpec, RoutineLaunch, Session, SessionStatus, Target}, util::now_ms};

/// Runs later than this (daemon stopped, old failures) are skipped instead of started.
pub(crate) const MISSED_GRACE_MS: i64 = 60 * 60_000;
const RUNS_KEPT: usize = 50;
/// Runs a routine started by events (another agent's turn, failures, channel messages) may start in an hour; a later event is
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
    /// A gate check that started no run (`gates`): not a run, never counted as one.
    Checked,
}

/// One due time, event or manual run, with what came of it.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RoutineRun {
    #[ts(type = "number")]
    pub at: i64,
    /// What started it, for example "Scheduled" or "Run now".
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
    /// The session's `status_since` when the run ended Done (report or no update): an auto archive applies only while
    /// the session keeps that state (`auto_archive_ready`), so a session the owner continued is never archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub done_since: Option<i64>,
    /// The routine archived the run's session (or its bot thread) on its own (`CronAgent.auto_archive`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub archived: bool,
    /// A gated routine: the oracle check behind this entry (a run's evaluation, or the quiet checks it collapses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub check: Option<crate::gates::Check>,
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
    /// Archive each run's session once it ended Done with its work saved and nothing waiting (`auto_archive_ready`,
    /// `unfinished_work`); a bot routine archives the run's own thread. Off by default.
    #[serde(default)]
    pub auto_archive: bool,
    /// Only when an oracle says so (`gates`): schedule triggers only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gate: Option<crate::gates::Gate>,
    /// What the gate remembers between ticks (fire policy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gate_state: Option<crate::gates::GateState>,
    /// Why its due run has not started yet (for example no confirmed quota reading). Never stored: `list` fills it.
    #[serde(default, skip_deserializing)]
    pub waiting: Option<String>,
    /// Who created it when not the owner: the agent session or bot whose CLI created it (`routine.create`), or the
    /// author of a card accepted without the owner's click. Stamped by Home at creation, never from a request body;
    /// None for the owner's and older routines. Its runs count as that bot's launches (`task_archive`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub created_by: Option<launchpad_core::model::MessageFrom>,
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
    #[serde(default)]
    pub auto_archive: Option<bool>,
    /// A new gate, or null to remove it.
    #[serde(default, deserialize_with = "some")]
    pub gate: Option<Option<crate::gates::Gate>>,
}

/// A field that may be null: missing is None, null is Some(None).
fn some<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
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
    #[serde(default)]
    pub auto_archive: bool,
    #[serde(default)]
    pub gate: Option<crate::gates::Gate>,
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

/// A new routine that launches a spec (`routine.create`, cron cards).
#[derive(Debug, Deserialize)]
pub struct NewCron {
    pub spec: LaunchSpec,
    #[serde(default)]
    pub schedule: String,
    pub conditions: Vec<CronCondition>,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub title: Option<String>,
    /// The browser's time zone.
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub auto_archive: bool,
    #[serde(default)]
    pub gate: Option<crate::gates::Gate>,
    /// Who creates it when not the owner (`CronAgent.created_by`), set by Home: never read from a request body.
    #[serde(skip)]
    pub created_by: Option<launchpad_core::model::MessageFrom>,
}

pub async fn create(app: &App, new: NewCron) -> Result<CronAgent> {
    let NewCron { spec, schedule, conditions, trigger, title, time_zone, auto_archive, gate, created_by } = new;
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
    if let Some(g) = &gate {
        crate::gates::check(g, &trigger)?;
        crate::gates::check_oracle(app, launchpad_core::scope::of_spec(app, &spec).await.as_deref(), g).await?;
    }
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
        failure_seen: None,
        channel_seen: None,
        file: None,
        file_error: None,
        auto_archive,
        gate,
        gate_state: None,
        waiting: None,
        created_by,
    };
    record.next_run_at = next_due(&record, now)?;
    insert(app, &record).await?;
    Ok(record)
}

/// The spec a bot routine stands for: the bot's profile and folder (for quotas and workspace lists).
async fn bot_spec(app: &App, bot: &launchpad_core::bots::Bot, text: String) -> Result<LaunchSpec> {
    let config = app.config.get();
    let profile = launchpad_core::bots::launch_profile(app, &config, &bot.settings).cloned().context("choose a profile for this bot on its page (About → Access)")?;
    Ok(LaunchSpec {
        text,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        target: launchpad_core::bots::effective_folder(app, &bot.settings).await.ok().flatten().map_or(Target::Temp, |p| Target::Project { project_id: p.id }),
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
        auto_archive: r.auto_archive,
        gate: r.gate,
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
    if let Some(Some(g)) = &patch.gate {
        crate::gates::check_oracle(app, launchpad_core::scope::of_spec(app, &current.spec).await.as_deref(), g).await?;
    }
    let (record, reset) = modify(app, id, move |r| {
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
        if let Some(a) = patch.auto_archive {
            r.auto_archive = a;
        }
        if let Some(t) = patch.title {
            r.title = t.trim().to_string();
        }
        if let Some(i) = patch.instructions {
            ensure!(r.bot_id.is_some(), "edit this routine's prompt by creating it again from the composer");
            ensure!(!i.trim().is_empty(), "write what the bot does at each run");
            r.spec.text = i.trim().to_string();
        }
        let mut reset = false;
        if let Some(g) = patch.gate {
            reset = gate_moved(&r.gate, &g);
            r.gate = g;
            if reset {
                r.gate_state = None;
            }
        }
        if let Some(g) = &r.gate {
            crate::gates::check(g, &r.trigger)?;
        }
        r.next_run_at = next_due(r, now)?;
        Ok(reset)
    })
    .await?;
    if reset {
        crate::oracle_eval::forget_cursor(app, &crate::gates::consumer(id)).await?;
    }
    Ok(record)
}

/// A gate on another oracle or with other parameters starts from scratch: no cursor, no fire state.
fn gate_moved(old: &Option<crate::gates::Gate>, new: &Option<crate::gates::Gate>) -> bool {
    old.as_ref().map(|g| (&g.oracle, &g.params)) != new.as_ref().map(|g| (&g.oracle, &g.params))
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
    if let Some(a) = patch.auto_archive {
        f.auto_archive = a;
    }
    if let Some(t) = patch.title {
        f.title = t.trim().to_string();
    }
    if let Some(i) = patch.instructions {
        f.instructions = i.trim().to_string();
    }
    if let Some(g) = patch.gate {
        f.gate = g;
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
    let key = id.to_string();
    app.state_db
        .call(move |c| {
            if c.execute("DELETE FROM cron_agent WHERE id = ?1", [&key])? == 0 {
                bail!("this routine no longer exists");
            }
            Ok(())
        })
        .await?;
    crate::oracle_eval::forget_cursor(app, &crate::gates::consumer(id)).await?;
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

fn summary(run: &RoutineRun) -> String {
    let what = match run.outcome {
        RunOutcome::Running => "Started",
        RunOutcome::Report => "Report",
        RunOutcome::NoUpdate => "No update",
        RunOutcome::Skipped => "Skipped",
        RunOutcome::Failed => "Failed",
        RunOutcome::Interrupted => "Interrupted",
        RunOutcome::Stopped => "Stopped",
        RunOutcome::Checked => "Checked",
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
    trim_history(&mut r.history);
}

/// Pure: keeps `RUNS_KEPT` entries, dropping the oldest gate check before a run (never the newest entry), so quiet
/// checks never push out a run that has not settled yet.
pub(crate) fn trim_history(history: &mut Vec<RoutineRun>) {
    while history.len() > RUNS_KEPT {
        let i = history[..history.len() - 1].iter().position(|x| x.outcome == RunOutcome::Checked).unwrap_or(0);
        history.remove(i);
    }
}

pub(crate) async fn skip(app: &App, id: &str, cause: String, reason: String) -> Result<()> {
    skip_run(app, id, cause, reason, false).await
}

/// `skip`, marked as a failure when `missed`.
pub(crate) async fn skip_run(app: &App, id: &str, cause: String, reason: String, missed: bool) -> Result<()> {
    modify(app, id, move |r| {
        push_run(r, RoutineRun { at: now_ms(), cause, session_id: None, outcome: RunOutcome::Skipped, detail: Some(reason), quiet_ok: false, missed, done_since: None, archived: false, check: None });
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
    let started = r.history.iter().filter(|x| !matches!(x.outcome, RunOutcome::Skipped | RunOutcome::Checked) && now - x.at < 3_600_000).count();
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

/// Why a bot routine cannot start a run now: each run starts a thread of its own, so only the bot's limit of
/// threads at work (`bot_threads::at_cap`) holds it.
async fn thread_busy(app: &App, r: &CronAgent) -> Option<String> {
    let bot = launchpad_core::bots::get(app, r.bot_id.as_ref()?).await.ok()?;
    launchpad_core::bot_threads::at_cap(app, &bot, None)
}

/// Why the routine's conditions block a run now, or None when it may start.
pub(crate) async fn why_blocked(app: &App, r: &CronAgent) -> Result<Option<String>> {
    if let Some(why) = thread_busy(app, r).await {
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
/// retried). `automatic` runs need confirmed quota. A gated run (`gate`) commits its oracle cursor and its fire state
/// in the same transaction as the history entry, and may carry other instructions (`[gate.failed]`).
async fn start(app: &App, id: &str, cause: String, context: String, automatic: bool, hops: u32, gate: Option<crate::gates::GateRun>) -> Result<Session> {
    let at = now_ms();
    // A run starts its own session (a bot routine: in a thread of its own), so nothing of it is unseen yet.
    let quiet_ok = true;
    let (check, state, cursor, instructions, evaluated) = match gate {
        Some(g) => (Some(g.check), Some(g.state), g.cursor, g.instructions, Some(g.gate)),
        None => (None, None, None, None, None),
    };
    let key = crate::gates::consumer(id);
    let (record, ()) = modify_tx(app, id, {
        let cause = cause.clone();
        move |c, r| {
            if automatic {
                ensure!(r.active(), "paused");
            }
            push_run(r, RoutineRun { at, cause, session_id: None, outcome: RunOutcome::Running, detail: None, quiet_ok, missed: false, done_since: None, archived: false, check });
            // A gate changed since the evaluation (its file was saved meanwhile) starts from scratch: nothing of the
            // old one is kept.
            if evaluated.as_ref().is_some_and(|g| crate::gates::same_gate(r, g)) {
                if let Some(s) = state {
                    r.gate_state = Some(s);
                }
                if let Some((ws, oracle, value, eval)) = &cursor {
                    crate::oracle_eval::commit_in(c, &key, ws, oracle, value, Some(eval), at)?;
                }
            }
            Ok(())
        }
    })
    .await?;
    let record = match instructions {
        Some(text) => CronAgent { spec: LaunchSpec { text, ..record.spec.clone() }, ..record },
        None => record,
    };
    let launch = RoutineLaunch { id: record.id.clone(), title: record.title.clone(), cause, context };
    let result = match &record.bot_id {
        Some(bot) => {
            let context = launchpad_core::model::MessageContext { from: Some(routine_sender(&record, hops)), ..Default::default() };
            let file = record.file.clone().unwrap_or_else(|| record.id.clone());
            let to = launchpad_core::bot_threads::route(launchpad_core::bot_threads::Source::Routine { file: &file, title: &record.title });
            launchpad_core::bots::message_with(app, bot, launchpad_core::bots::Message { text: bot_message(&record, &launch), attachments: vec![], context, automatic, key: None, to }).await
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

/// `modify` with the transaction, for changes of other tables that go with the record's (a gate's cursor).
pub(crate) async fn modify_tx<T: Send + 'static>(
    app: &App,
    id: &str,
    f: impl FnOnce(&rusqlite::Connection, &mut CronAgent) -> Result<T> + Send + 'static,
) -> Result<(CronAgent, T)> {
    let id = id.to_string();
    let out = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let json: String = tx.query_row("SELECT json FROM cron_agent WHERE id = ?1", [&id], |r| r.get(0)).context("this routine no longer exists")?;
            let mut record: CronAgent = serde_json::from_str(&json)?;
            let out = f(&tx, &mut record)?;
            tx.execute("UPDATE cron_agent SET json = ?2 WHERE id = ?1", params![id, serde_json::to_string(&record)?])?;
            tx.commit()?;
            Ok((record, out))
        })
        .await?;
    changed(app);
    Ok(out)
}

/// Claims a due occurrence of a schedule (its next due time moves on) and applies `f` in the same write.
pub(crate) async fn claim_due(app: &App, id: &str, due: Option<i64>, now: i64, f: impl FnOnce(&mut CronAgent) + Send + 'static) -> Result<CronAgent> {
    claim_due_tx(app, id, due, now, move |_, r| {
        f(r);
        Ok(())
    })
    .await
}

/// `claim_due` with the transaction.
pub(crate) async fn claim_due_tx(app: &App, id: &str, due: Option<i64>, now: i64, f: impl FnOnce(&rusqlite::Connection, &mut CronAgent) -> Result<()> + Send + 'static) -> Result<CronAgent> {
    Ok(modify_tx(app, id, move |c, r| {
        if !r.active() || r.next_run_at != due {
            bail!("already handled");
        }
        r.next_run_at = Some(next_after(&r.schedule, r.time_zone.as_deref(), now)?);
        r.last_check_at = Some(now);
        f(c, r)
    })
    .await?
    .0)
}

/// A failure that starts no run (a gate that cannot be checked): one entry, its time updated while it repeats.
pub(crate) fn push_or_merge_failure(r: &mut CronAgent, at: i64, detail: String) {
    if let Some(last) = r.history.last_mut()
        && last.outcome == RunOutcome::Failed
        && last.session_id.is_none()
        && last.detail.as_deref() == Some(detail.as_str())
    {
        last.at = at;
        return;
    }
    push_run(r, RoutineRun { at, cause: "Scheduled".into(), session_id: None, outcome: RunOutcome::Failed, detail: Some(detail), quiet_ok: false, missed: false, done_since: None, archived: false, check: None });
}

/// Run now: ignores the conditions and the trigger.
pub async fn run_now(app: &App, id: &str) -> Result<Session> {
    let record = get(app, id).await?;
    // A routine whose file never worked has nothing to run yet.
    ensure!(!record.spec.text.trim().is_empty(), "{} has no instructions: fix its file {}", record.title, record.file.as_deref().unwrap_or_default());
    if let Some(why) = thread_busy(app, &record).await {
        bail!("{why}: wait for one of its threads to end, or Stop one");
    }
    let app = app.clone();
    // Keep starting even if the browser disconnects while it waits for the response.
    tokio::spawn(async move { start(&app, &record.id, "Run now".into(), String::new(), false, 0, None).await }).await?
}

/// Starts an event run when the routine allows it; otherwise records why it was skipped. `hops`: the automatic
/// messages in the chain of its message, itself included (1 for a schedule or a failure).
pub(crate) async fn on_event(app: &App, r: &CronAgent, cause: String, context: String, hops: u32) -> Result<()> {
    on_event_with(app, r, cause, context, hops, None).await.map(|_| ())
}

/// What came of an event: a run started, or why not.
pub(crate) enum Started {
    Run(String),
    Skipped(String),
    Failed(String),
}

/// `on_event` for a gated run (`gate`: its check, fire state and cursor, committed only when the run is claimed).
pub(crate) async fn on_event_with(app: &App, r: &CronAgent, cause: String, context: String, hops: u32, gate: Option<crate::gates::GateRun>) -> Result<Started> {
    if let Some(why) = limit_reason(r, hops, now_ms()) {
        skip(app, &r.id, cause, why.clone()).await?;
        return Ok(Started::Skipped(why));
    }
    if let Some(why) = why_blocked(app, r).await? {
        skip(app, &r.id, cause, why.clone()).await?;
        return Ok(Started::Skipped(why));
    }
    // Automatic work needs confirmed quota; an event never waits for it here (schedules and failures wait before
    // they claim), so the run is missed.
    if let Some(why) = quota_hold(app, r).await? {
        let why = format!("automatic work needs confirmed quota: {why}");
        skip_run(app, &r.id, cause, why.clone(), true).await?;
        return Ok(Started::Skipped(why));
    }
    Ok(match start(app, &r.id, cause, context, true, hops, gate).await {
        Ok(s) => Started::Run(s.id),
        Err(e) => Started::Failed(format!("{e:#}")),
    })
}

/// Why the routine's automatic run waits for its quota now (`quota::hold_reason`): of the bot's thread
/// (`bots::quota_hold`), else of the session it would start. Every trigger checks it before it starts a run.
pub(crate) async fn quota_hold(app: &App, r: &CronAgent) -> Result<Option<String>> {
    if let Some(b) = &r.bot_id {
        return Ok(launchpad_core::bots::quota_hold(app, &launchpad_core::bots::get(app, b).await?, None));
    }
    let spec = effective_spec(app, r).await?;
    Ok(spec.harness.and_then(|h| launchpad_core::quota::hold_reason(app, h, &spec)))
}

/// Why a routine's due occurrence waited, with that due time (Unix ms): the reason its late skip gives. In memory.
static HELD: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (i64, String)>>> = std::sync::LazyLock::new(Default::default);

/// Keeps a due occurrence waiting for its quota (`list` shows why).
pub(crate) fn hold(id: &str, due: i64, why: String) {
    HELD.lock().unwrap().insert(id.to_string(), (due, why));
}

async fn tick(app: &App, record: CronAgent) -> Result<()> {
    if let Some(gate) = record.gate.clone() {
        return crate::gates::tick(app, record, gate).await;
    }
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
pub(crate) fn late_reason(due: i64, held: Option<String>, home_started: Option<i64>) -> (String, bool) {
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
    // A sleeping agent (`launchpad sleep`) has not finished: its run settles and its turn triggers routines when a
    // later turn ends awake.
    if s.status == SessionStatus::QuotaPaused || s.sleeping() {
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
    let since = s.status_since;
    for r in records.iter().filter(|r| r.history.iter().any(|x| x.outcome == RunOutcome::Running && x.session_id.as_deref() == Some(&s.id))) {
        was_run = true;
        let sid = s.id.clone();
        let (_, quiet_ok) = modify(app, &r.id, move |r| {
            let mut quiet_ok = false;
            if let Some(run) = r.history.iter_mut().rev().find(|x| x.outcome == RunOutcome::Running && x.session_id.as_deref() == Some(&sid)) {
                run.outcome = outcome;
                quiet_ok = run.quiet_ok;
                if matches!(outcome, RunOutcome::Report | RunOutcome::NoUpdate) {
                    run.done_since = Some(since);
                }
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
            // A bot routine's run had nothing to say: its thread leaves the bot's thread bar (archived, with Bring back),
            // only while the thread is still just that run (a routine's own thread, one session, nothing asked or
            // running), so a thread the owner continued is never archived.
            if let Some(b) = &s.launch_spec.bot
                && launchpad_core::bot_threads::get(app, &b.id, b.thread_id()).await.ok().flatten().is_some_and(|t| t.archived_at.is_none() && t.key.as_deref().is_some_and(|k| k.starts_with("routine:")))
                && let [only] = launchpad_core::bot_threads::sessions(app, &b.id, b.thread_id()).as_slice()
                && only.id == s.id
                && only.status == SessionStatus::Waiting
                && !only.open_question
                && let Err(e) = launchpad_core::bot_threads::archive(app, &b.id, b.thread_id(), true).await
            {
                tracing::warn!("archiving the thread of a quiet run of {}: {e:#}", r.title);
            }
        }
    }
    if was_run {
        // A run whose work is already on Home may go at once; else at `turn_saved` or the 30-second pass.
        auto_archive_soon(app);
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
    auto_archive_soon(app);
    crate::task_archive::soon(app);
    let Some(at) = UNSAVED_DONE.lock().unwrap().remove(&s.id) else { return };
    if s.status != SessionStatus::Waiting || s.open_question || s.sleeping() || now_ms() - at > 3_600_000 {
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

/// How long after its run ended Done a session may still be archived on its own: a merge that takes longer, or work
/// left waiting, keeps it visible for good.
const AUTO_ARCHIVE_WINDOW_MS: i64 = 24 * 3_600_000;

/// Whether a routine run's session ended Done and still is exactly as the run left it, with nothing waiting in it:
/// a report or no update, the same Done state since (`done_since`, so a session the owner continued, resumed,
/// stopped or brought back never goes), no question, not asleep, no quota pause or capacity retry, its work saved on
/// Home (no `unsaved` mark), within `AUTO_ARCHIVE_WINDOW_MS`. Failed, interrupted, stopped, sleeping and waiting
/// sessions stay. Worktrees are checked after this (`away::unfinished_work`).
fn auto_archive_ready(run: &RoutineRun, s: &Session, now: i64) -> bool {
    matches!(run.outcome, RunOutcome::Report | RunOutcome::NoUpdate)
        && !run.archived
        && run.session_id.as_deref() == Some(s.id.as_str())
        && run.done_since.is_some_and(|d| d == s.status_since && now - d <= AUTO_ARCHIVE_WINDOW_MS)
        && s.status == SessionStatus::Waiting
        && !s.open_question
        && s.sleep.is_none()
        && s.quota_pause.is_none()
        && s.capacity_retry_at.is_none()
        && s.unsaved.is_none()
}

/// Whether a bot thread holds only this routine's run: the thread the run started (`routine:<file>`),
/// open, and with no session but the run's (older threads may hold several). A thread the owner continued, or any other thread of the bot, stays.
fn own_run_thread(thread: &launchpad_core::bot_threads::BotThread, sessions: &[Session], file: &str, session_id: &str) -> bool {
    thread.archived_at.is_none()
        && thread.key.as_deref() == Some(format!("routine:{file}").as_str())
        && matches!(sessions, [only] if only.id == session_id)
}

/// One auto-archive pass at a time.
static ARCHIVING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn auto_archive_soon(app: &App) {
    if launchpad_core::role::Role::Runner == app.paths.role {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = auto_archive(&app).await {
            tracing::warn!("routine auto archive: {e:#}");
        }
    });
}

/// Home: archives the sessions of finished runs of routines with `auto_archive` (`auto_archive_ready`, then no
/// waiting work in their worktrees). A bot routine archives the run's own thread only (`own_run_thread`), another
/// routine only a session it launched. Each run is claimed (`archived`) before its archive, never tried again.
async fn auto_archive(app: &App) -> Result<()> {
    let _pass = ARCHIVING.lock().await;
    let now = now_ms();
    for r in list(app).await?.into_iter().filter(|r| r.auto_archive) {
        for run in r.history.iter().filter(|x| x.done_since.is_some_and(|d| now - d <= AUTO_ARCHIVE_WINDOW_MS) && !x.archived) {
            let Some(s) = run.session_id.as_deref().and_then(|id| app.sessions.get(id)) else { continue };
            // Work that waits, or a check that failed, keeps it.
            if !auto_archive_ready(run, &s, now) || !matches!(launchpad_core::away::unfinished_work(app, &s.id).await, Ok(None)) {
                continue;
            }
            // Only the routine's own session: a bot routine's run in its own thread, another routine's launch.
            let thread = match (&r.bot_id, &s.launch_spec.bot) {
                (Some(bot), Some(b)) if b.id == *bot => {
                    let file = r.file.clone().unwrap_or_else(|| r.id.clone());
                    let Some(t) = launchpad_core::bot_threads::get(app, bot, b.thread_id()).await?.filter(|t| own_run_thread(t, &launchpad_core::bot_threads::sessions(app, bot, b.thread_id()), &file, &s.id)) else { continue };
                    Some((bot.clone(), t.id))
                }
                (None, None) if s.launch_spec.routine.as_ref().is_some_and(|l| l.id == r.id) => None,
                _ => continue,
            };
            let (at, sid) = (run.at, s.id.clone());
            let claimed = modify(app, &r.id, move |r| {
                let Some(x) = r.history.iter_mut().find(|x| x.at == at && x.session_id.as_deref() == Some(sid.as_str()) && !x.archived) else { return Ok(false) };
                x.archived = true;
                Ok(true)
            })
            .await?
            .1;
            if !claimed {
                continue;
            }
            let done = match &thread {
                Some((bot, id)) => launchpad_core::bot_threads::archive(app, bot, id, true).await.map(|_| ()),
                None => launchpad_core::runners::session_action(app, &s.id, "close").await,
            };
            if let Err(e) = done {
                tracing::warn!("auto archive of {} ({}): {e:#}", s.id, r.title);
                let (at, sid, why) = (run.at, s.id.clone(), format!("{e:#}"));
                modify(app, &r.id, move |r| {
                    if let Some(x) = r.history.iter_mut().find(|x| x.at == at && x.session_id.as_deref() == Some(sid.as_str())) {
                        x.detail = Some(format!("not archived: {why}"));
                    }
                    Ok(())
                })
                .await?;
            }
        }
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
            // A planned update (`maintenance`): no run starts; due runs are judged by the Home that runs next
            // (more than an hour late, they are skipped like any late run).
            if launchpad_core::maintenance::active() {
                continue;
            }
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
                    // Runs whose merge or save finished after their turn ended.
                    if let Err(e) = auto_archive(&app).await {
                        tracing::warn!("routine auto archive: {e:#}");
                    }
                    // Bots' tasks that finished cleanly, also once their merge or publication completed.
                    if let Err(e) = crate::task_archive::pass(&app).await {
                        tracing::warn!("archive of bots' finished tasks: {e:#}");
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
            auto_archive: false,
            gate: None,
        };
        let text = render_file(&f).unwrap();
        assert_eq!(parse_file(&text).unwrap(), f);
        let g = RoutineFile { trigger: Trigger::Failure { events: vec![crate::failures::FailureKind::Any] }, schedule: String::new(), conditions: vec![], paused: true, ..f.clone() };
        assert_eq!(parse_file(&render_file(&g).unwrap()).unwrap(), g);
        // The removed GitHub trigger is an error of the file.
        assert!(parse_file("title = \"x\"\ninstructions = \"y\"\ntrigger = { kind = \"github\", events = [\"any\"] }\n").is_err());
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
            r.history = runs.into_iter().map(|(at, outcome)| RoutineRun { at, cause: "x".into(), session_id: None, outcome, detail: None, quiet_ok: false, missed: false, done_since: None, archived: false, check: None }).collect();
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
        let with_bots = RoutineFile { title: "x".into(), paused: false, schedule: String::new(), time_zone: None, instructions: "y".into(), trigger: Trigger::SessionDone { folder: None, include_bots: true }, conditions: vec![], auto_archive: false, gate: None };
        let text = render_file(&with_bots).unwrap();
        assert!(text.contains("include_bots = true"), "{text}");
        assert_eq!(parse_file(&text).unwrap(), with_bots);
        assert!(!render_file(&RoutineFile { trigger: Trigger::SessionDone { folder: None, include_bots: false }, ..with_bots }).unwrap().contains("include_bots"));
    }

    #[test]
    fn auto_archive_is_off_by_default_and_round_trips_in_files() {
        // Older records and files have no field: off.
        let r: CronAgent = serde_json::from_value(serde_json::json!({"id": "r1", "spec": {}, "title": "Review", "schedule": "", "conditions": [], "paused": false, "created_at": 0, "next_run_at": null, "last_check_at": null, "last_outcome": null, "runs": []})).unwrap();
        assert!(!r.auto_archive);
        let f = parse_file("title = \"x\"\ninstructions = \"y\"\nschedule = \"0 9 * * *\"\n").unwrap();
        assert!(!f.auto_archive);
        // Off stays out of the file (existing files keep their text); on round-trips.
        assert!(!render_file(&f).unwrap().contains("auto_archive"));
        let on = RoutineFile { auto_archive: true, ..f };
        let text = render_file(&on).unwrap();
        assert!(text.contains("auto_archive = true"), "{text}");
        assert_eq!(parse_file(&text).unwrap(), on);
        assert_ne!(file_of(&CronAgent { auto_archive: true, ..r.clone() }), file_of(&r));
        let old_run: RoutineRun = serde_json::from_value(serde_json::json!({"at": 1, "cause": "x", "session_id": "s", "outcome": "report", "detail": null})).unwrap();
        assert_eq!((old_run.done_since, old_run.archived), (None, false));
    }

    fn done_session(id: &str, since: i64) -> Session {
        launchpad_harness::install();
        serde_json::from_value(serde_json::json!({
            "id": id, "harness": "codex", "cwd": "/tmp/x", "title": "run", "status": "waiting", "status_since": since,
            "live": false, "open_question": false, "prompt": "p", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0},
            "started_at": 0, "last_seq": 1, "launch_spec": {}, "seen_at": 0
        }))
        .unwrap()
    }

    #[test]
    fn auto_archive_takes_only_runs_that_ended_done_and_stayed_so() {
        let now = 50 * 3_600_000;
        let since = now - 60_000;
        let run = |outcome: RunOutcome| RoutineRun { at: since - 1, cause: "Scheduled".into(), session_id: Some("s1".into()), outcome, detail: None, quiet_ok: true, missed: false, done_since: Some(since), archived: false, check: None };
        let s = done_session("s1", since);
        assert!(auto_archive_ready(&run(RunOutcome::Report), &s, now));
        assert!(auto_archive_ready(&run(RunOutcome::NoUpdate), &s, now));
        // Failed, interrupted, stopped and still running runs stay.
        for o in [RunOutcome::Failed, RunOutcome::Interrupted, RunOutcome::Stopped, RunOutcome::Running, RunOutcome::Skipped] {
            assert!(!auto_archive_ready(&run(o), &s, now), "{o:?}");
        }
        // Already archived once, another session, or a run that never ended Done.
        assert!(!auto_archive_ready(&RoutineRun { archived: true, ..run(RunOutcome::Report) }, &s, now));
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &done_session("s2", since), now));
        assert!(!auto_archive_ready(&RoutineRun { done_since: None, ..run(RunOutcome::Report) }, &s, now));
        // The session moved on since (the owner continued it, or brought it back): it stays.
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &done_session("s1", since + 5), now));
        // Waiting for input, asleep, quota paused, retrying, or its work not saved on Home yet.
        let mut asking = s.clone();
        asking.open_question = true;
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &asking, now));
        let mut asleep = s.clone();
        asleep.sleep = Some(launchpad_core::model::Sleep { until: Some(now + 1), waits: String::new(), deadline: None, note: String::new(), schedule_id: "z".into() });
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &asleep, now));
        let mut unsaved = s.clone();
        unsaved.unsaved = Some(launchpad_core::model::Unsaved { since, attempts: 1, note: Some("push refused".into()) });
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &unsaved, now));
        let mut retry = s.clone();
        retry.capacity_retry_at = Some(now + 1);
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &retry, now));
        for status in [SessionStatus::Error, SessionStatus::Interrupted, SessionStatus::Stopped, SessionStatus::Working, SessionStatus::Closed, SessionStatus::QuotaPaused] {
            let mut other = s.clone();
            other.status = status;
            assert!(!auto_archive_ready(&run(RunOutcome::Report), &other, now), "{status:?}");
        }
        // Too old: a merge that took longer keeps it visible.
        assert!(!auto_archive_ready(&run(RunOutcome::Report), &s, since + AUTO_ARCHIVE_WINDOW_MS + 1));
    }

    #[test]
    fn auto_archive_takes_only_the_run_s_own_bot_thread() {
        let thread = |id: &str, key: Option<&str>, archived: bool| launchpad_core::bot_threads::BotThread { id: id.into(), bot_id: "b".into(), key: key.map(str::to_string), title: "t".into(), created_at: 0, archived_at: archived.then_some(1), conversation: format!("b:{id}") };
        let one = [done_session("s1", 1)];
        let own = thread("t1", Some("routine:routines/daily.toml"), false);
        assert!(own_run_thread(&own, &one, "routines/daily.toml", "s1"));
        // Another routine's thread, a channel's, the owner's, an archived one.
        assert!(!own_run_thread(&own, &one, "routines/other.toml", "s1"));
        assert!(!own_run_thread(&thread("t1", Some("channel:c"), false), &one, "routines/daily.toml", "s1"));
        assert!(!own_run_thread(&thread("t1", None, false), &one, "routines/daily.toml", "s1"));
        assert!(!own_run_thread(&thread("t1", Some("routine:routines/daily.toml"), true), &one, "routines/daily.toml", "s1"));
        // An older thread with another session, or not the run's session.
        assert!(!own_run_thread(&own, &[done_session("s0", 0), done_session("s1", 1)], "routines/daily.toml", "s1"));
        assert!(!own_run_thread(&own, &one, "routines/daily.toml", "s9"));
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

//! Bot routine files and their synchronization with records.
use super::*;
use super::calendar::parse_zone;

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
    /// Archive each run's thread once the run ended Done with nothing waiting. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_archive: bool,
    /// Only when an oracle says so: `[gate]` (schedule triggers only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<crate::gates::Gate>,
}

const FILE_HEADER: &str = "# A routine of this bot: its runs are messages to the bot's thread. Saved from the bot's page; a hand edit\n# applies at once. trigger.kind: schedule (with schedule, a 5-field cron), session_done, failure or channel.\n";

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
    if let Some(g) = &f.gate {
        crate::gates::check(g, &f.trigger)?;
    }
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
    if let Some(g) = &f.gate {
        crate::gates::check_oracle(app, Some(&bot.workspace_id), g).await?;
    }
    Ok(f)
}

/// The definition a record holds, as a file.
pub(super) fn file_of(r: &CronAgent) -> RoutineFile {
    RoutineFile { title: r.title.clone(), paused: r.paused, schedule: r.schedule.clone(), time_zone: r.time_zone.clone(), instructions: r.spec.text.clone(), trigger: r.trigger.clone(), conditions: r.conditions.clone(), auto_archive: r.auto_archive, gate: r.gate.clone() }
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
        if let (Some(r), Ok(f)) = (records.iter().find(|r| r.file.as_deref() == Some(path.as_str())), &parsed)
            && gate_moved(&r.gate, &f.gate)
        {
            crate::oracle_eval::forget_cursor(app, &crate::gates::consumer(&r.id)).await?;
        }
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
                    r.auto_archive = f.auto_archive;
                    if gate_moved(&r.gate, &f.gate) {
                        r.gate_state = None;
                    }
                    r.gate = f.gate;
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
                    Err(e) => (RoutineFile { title: path.trim_start_matches(launchpad_core::bots::ROUTINES).to_string(), paused: true, schedule: String::new(), time_zone: None, instructions: String::new(), trigger: Trigger::Schedule, conditions: vec![], auto_archive: false, gate: None }, Some(e)),
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
                    failure_seen: None,
                    channel_seen: Some(head),
                    file: Some(path.clone()),
                    file_error: error,
                    auto_archive: f.auto_archive,
                    gate: f.gate,
                    gate_state: None,
                    waiting: None,
                    created_by: None,
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

//! Calendar parsing and definition validation.
use super::*;

pub(super) fn parse(schedule: &str) -> Result<croner::Cron> {
    let schedule = schedule.split_whitespace().collect::<Vec<_>>();
    if schedule.len() != 5 {
        bail!("use 5 fields: minute hour day-of-month month day-of-week");
    }
    croner::Cron::new(&schedule.join(" "))
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid cron schedule: {e}"))
}

pub(super) fn parse_zone(zone: &str) -> Result<chrono_tz::Tz> {
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

pub(super) fn next_in<Z: chrono::TimeZone>(cron: &croner::Cron, zone: &Z, after: i64) -> Result<i64> {
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

pub(super) fn validate_conditions(conditions: &[CronCondition]) -> Result<()> {
    for c in conditions {
        if let CronCondition::QuotaLeft { min_left_percent, .. } = c
            && !(1..=100).contains(min_left_percent)
        {
            bail!("pick a quota share from 1% to 100%");
        }
    }
    Ok(())
}

pub(crate) fn validate_trigger(trigger: &Trigger, schedule: &str, zone: Option<&str>) -> Result<()> {
    match trigger {
        Trigger::Schedule => next_after(schedule, zone, now_ms()).map(|_| ()),
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
pub(super) fn next_due(r: &CronAgent, now: i64) -> Result<Option<i64>> {
    Ok(if r.active() && r.trigger == Trigger::Schedule { Some(next_after(&r.schedule, r.time_zone.as_deref(), now)?) } else { None })
}

pub(super) fn normalize(schedule: &str) -> String {
    schedule.split_whitespace().collect::<Vec<_>>().join(" ")
}

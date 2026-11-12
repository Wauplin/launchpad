//! Automation: routines (code: crons), one-time scheduled launches, agent proposals and the activity list of
//! admin actions.

pub mod activity;
pub mod channel_trigger;
pub mod crons;
pub mod failures;
pub mod proposals;
pub mod schedules;
pub mod today;

use anyhow::Result;
use launchpad_core::App;

/// Settles claims left by a previous run, then starts the background services. Called once after
/// `App::start`.
pub async fn start(app: &App) -> Result<()> {
    schedules::recover(app).await?;
    proposals::recover(app).await?;
    if let Err(e) = failures::record_start(app).await {
        tracing::warn!("daemon start record: {e:#}");
    }
    if let Err(e) = crons::files_from_records(app).await {
        tracing::warn!("bot routine files: {e:#}");
    }
    schedules::spawn(app);
    crons::spawn(app);
    Ok(())
}

/// Decodes stored JSON records, skipping the ones this build cannot read (for example a record that names a
/// removed variant). Each unreadable record is logged once per process, not at every scheduler tick.
pub(crate) fn decode_rows<T>(kind: &str, rows: Vec<(String, String)>, decode: impl Fn(&str) -> Result<T>) -> Vec<T> {
    use std::{collections::HashSet, sync::Mutex};
    static WARNED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    rows.into_iter()
        .filter_map(|(id, json)| match decode(&json) {
            Ok(v) => Some(v),
            Err(e) => {
                let key = format!("{kind}:{id}");
                let mut warned = WARNED.lock().unwrap_or_else(|p| p.into_inner());
                if warned.get_or_insert_with(HashSet::new).insert(key) {
                    tracing::warn!("ignoring unreadable {kind} {id}: {e:#}");
                }
                None
            }
        })
        .collect()
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    schedules::ScheduledLaunch::export_all(cfg)?;
    crons::CronAgent::export_all(cfg)?;
    proposals::Proposal::export_all(cfg)?;
    activity::Activity::export_all(cfg)?;
    Ok(())
}

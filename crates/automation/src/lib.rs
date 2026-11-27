//! Automation: routines (code: crons), one-time scheduled launches, agent proposals and the activity list of
//! admin actions.

pub mod activity;
pub mod bulk_archive;
pub mod channel_trigger;
pub mod crons;
pub mod failures;
pub mod folio_catalog;
pub mod folio_check;
pub mod folio_data;
pub mod folio_share;
pub mod folio_split;
pub mod folios;
pub mod gates;
pub mod oracle_check;
pub mod oracle_eval;
pub mod oracle_monty;
pub mod oracle_tools;
pub mod oracle_views;
pub mod oracles;
pub mod proposals;
pub mod run_groups;
pub mod schedules;
pub mod task_archive;
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
    oracle_eval::spawn(app);
    crons::spawn(app);
    folios::spawn(app);
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
    bulk_archive::ArchivePreview::export_all(cfg)?;
    bulk_archive::ArchiveOutcome::export_all(cfg)?;
    oracles::OracleFile::export_all(cfg)?;
    oracle_eval::Evaluation::export_all(cfg)?;
    oracle_eval::RunResult::export_all(cfg)?;
    oracle_eval::Streak::export_all(cfg)?;
    oracle_views::ListItem::export_all(cfg)?;
    oracle_views::Detail::export_all(cfg)?;
    oracle_views::TestAnswer::export_all(cfg)?;
    oracles::Saved::export_all(cfg)?;
    oracle_check::Refused::export_all(cfg)?;
    oracle_views::ToolInfo::export_all(cfg)?;
    folios::FolioDetail::export_all(cfg)?;
    folios::Rev::export_all(cfg)?;
    folios::Activity::export_all(cfg)?;
    folios::Written::export_all(cfg)?;
    folios::MarkInput::export_all(cfg)?;
    folios::Target::export_all(cfg)?;
    folios::SendTo::export_all(cfg)?;
    folios::Ask::export_all(cfg)?;
    folios::RenderReport::export_all(cfg)?;
    folios::BlockRequest::export_all(cfg)?;
    folios::Version::export_all(cfg)?;
    folio_share::ShareInput::export_all(cfg)?;
    folio_data::Entry::export_all(cfg)?;
    folio_catalog::export(cfg)?;
    Ok(())
}

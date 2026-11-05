//! Automation: routines (code: crons), one-time scheduled launches and agent proposals.

pub mod crons;
pub mod proposals;
pub mod schedules;

use anyhow::Result;
use launchpad_core::App;

/// Settles claims left by a previous run, then starts the background services. Called once after
/// `App::start`.
pub async fn start(app: &App) -> Result<()> {
    schedules::recover(app).await?;
    proposals::recover(app).await?;
    schedules::spawn(app);
    crons::spawn(app);
    Ok(())
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    schedules::ScheduledLaunch::export_all(cfg)?;
    crons::CronAgent::export_all(cfg)?;
    proposals::Proposal::export_all(cfg)?;
    Ok(())
}

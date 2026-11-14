//! GitHub features: issue and pull request views (`items`), the Inbox and agent Suggestions. The API client
//! is `launchpad_core::github`.

pub mod ext;
pub mod inbox;
pub mod items;
pub mod ledger;
pub mod notes;
pub mod suggestions;
pub mod today;

use anyhow::Result;
use launchpad_core::App;

/// Starts the background services. Called once after `App::start`.
pub fn spawn(app: &App) {
    let dir = app.paths.data_dir.join("images");
    tokio::spawn(async move {
        loop {
            if let Err(e) = items::collect_images(&dir).await { tracing::warn!("image cleanup: {e:#}"); }
            tokio::time::sleep(std::time::Duration::from_secs(86400)).await;
        }
    });
    inbox::spawn(app);
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    suggestions::Suggestion::export_all(cfg)?;
    ledger::LedgerRecord::export_all(cfg)?;
    notes::ItemNote::export_all(cfg)?;
    notes::Recipient::export_all(cfg)?;
    inbox::InboxSnapshot::export_all(cfg)?;
    inbox::ThreadPreview::export_all(cfg)?;
    ext::ExtContext::export_all(cfg)?;
    ext::ExtRows::export_all(cfg)?;
    ext::ExtNotifications::export_all(cfg)?;
    inbox::ObservedResult::export_all(cfg)?;
    Ok(())
}

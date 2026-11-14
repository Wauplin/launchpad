//! GitHub features: agent Suggestions with their feedback ledger, private notes, the Chrome extension's page context
//! and the owner's GitHub writes about an item (`items`). Nothing read from GitHub is stored. The API client is
//! `launchpad_core::github`.

pub mod ext;
pub mod items;
pub mod ledger;
pub mod notes;
pub mod suggestions;
pub mod today;

use anyhow::Result;

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    suggestions::Suggestion::export_all(cfg)?;
    ledger::LedgerRecord::export_all(cfg)?;
    notes::ItemNote::export_all(cfg)?;
    notes::Recipient::export_all(cfg)?;
    ext::ExtContext::export_all(cfg)?;
    ext::ExtRows::export_all(cfg)?;
    Ok(())
}

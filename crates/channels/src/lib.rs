//! Connections: messages to other people through webhooks (held in the Outbox for an owner click), the owner's
//! own Telegram chat, and incoming hooks (`POST /hooks/<id>`); and bot channels and DMs (`channel`, `delivery`), where
//! bots talk to each other. Home owns them; runners forward the API.

pub mod channel;
pub mod connections;
pub mod delivery;
pub mod hooks;
pub mod telegram;
pub mod today;

use anyhow::Result;
use launchpad_core::App;

/// Starts the background services. Called once after `App::start`.
pub fn spawn(app: &App) {
    connections::spawn(app);
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    connections::Connection::export_all(cfg)?;
    connections::OutboxMessage::export_all(cfg)?;
    telegram::TelegramStatus::export_all(cfg)?;
    hooks::Hook::export_all(cfg)?;
    hooks::CreatedHook::export_all(cfg)?;
    hooks::QueuedMessage::export_all(cfg)?;
    channel::Channel::export_all(cfg)?;
    channel::ChannelPage::export_all(cfg)?;
    channel::ChannelPosted::export_all(cfg)?;
    Ok(())
}

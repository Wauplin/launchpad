//! Channels: messages to other people through webhooks (held in the Outbox for an owner click), the owner's
//! own Telegram chat, and incoming hooks (`POST /hooks/<id>`). Home owns them; runners forward the API.

pub mod channels;
pub mod hooks;
pub mod telegram;

use anyhow::Result;
use launchpad_core::App;

/// Starts the background services. Called once after `App::start`.
pub fn spawn(app: &App) {
    channels::spawn(app);
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    channels::Channel::export_all(cfg)?;
    channels::OutboxMessage::export_all(cfg)?;
    telegram::TelegramStatus::export_all(cfg)?;
    hooks::Hook::export_all(cfg)?;
    hooks::CreatedHook::export_all(cfg)?;
    Ok(())
}

//! Connections: messages to other people through webhooks (held in the Outbox for an owner click), the owner's
//! own Telegram chat, and incoming hooks (`POST /hooks/<id>`); and bot channels and DMs (`channel`, `delivery`), where
//! bots talk to each other. Home owns them; runners forward the API.

pub mod agent;
pub mod cards;
pub mod channel;
pub mod connections;
pub mod conversation;
pub mod delivery;
pub mod digest;
pub mod hooks;
pub mod scheduled;
pub mod sends;
pub mod push;
pub mod telegram;
pub mod today;

use anyhow::Result;
use launchpad_core::App;

/// Starts the background services. Called once after `App::start`.
pub fn spawn(app: &App) {
    connections::spawn(app);
    // Daily digests of bot channels: Home only, never on runners.
    if launchpad_core::vault::here(app) {
        digest::spawn(app);
        // The owner's bot messages Home accepted: sent in the background, in order per thread.
        sends::spawn(app);
        // Scheduled posts in bot channels and DMs, posted once due.
        scheduled::spawn(app);
        // Wake-ups for the Android app when what waits for the owner changes.
        push::spawn(app);
    }
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    connections::Connection::export_all(cfg)?;
    connections::OutboxMessage::export_all(cfg)?;
    telegram::TelegramStatus::export_all(cfg)?;
    push::PhoneNotifications::export_all(cfg)?;
    push::PushStatus::export_all(cfg)?;
    hooks::Hook::export_all(cfg)?;
    hooks::CreatedHook::export_all(cfg)?;
    hooks::QueuedMessage::export_all(cfg)?;
    hooks::ScheduledMessage::export_all(cfg)?;
    sends::BotSend::export_all(cfg)?;
    conversation::Conversation::export_all(cfg)?;
    channel::Channel::export_all(cfg)?;
    channel::ChannelPage::export_all(cfg)?;
    channel::ChannelPosted::export_all(cfg)?;
    scheduled::ScheduledPost::export_all(cfg)?;
    Ok(())
}

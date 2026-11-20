//! The `launchpad_core::ext::Extensions` of this binary: connects the core to the crates above it.
use anyhow::Result;
use launchpad_automation::crons;
use launchpad_core::{App, model::Session};

pub struct Extensions;

#[async_trait::async_trait]
impl launchpad_core::ext::Extensions for Extensions {
    fn notify_connections(&self, app: &App, session: &Session, event: &'static str) {
        launchpad_channels::telegram::notify(app, session, event);
    }
    fn turn_ended(&self, app: &App, session: &Session) {
        crons::turn_ended(app, session);
        launchpad_channels::conversation::turn_ended(app, session);
    }
    fn turn_saved(&self, app: &App, session: &Session) {
        crons::turn_saved(app, session);
    }
    fn woken(&self, app: &App, session_id: &str, schedule_id: &str) {
        launchpad_automation::schedules::woken(app, session_id, schedule_id);
    }
    async fn replay_sleep(&self, app: &App, session_id: &str, schedule: serde_json::Value) -> Result<()> {
        launchpad_automation::schedules::replay_sleep(app, session_id, schedule).await
    }
    async fn update_wake(&self, app: &App, session_id: &str, text: String) -> Result<String> {
        launchpad_automation::schedules::update_wake(app, session_id, &text).await
    }
    fn suggestion_instruction(&self, app: &App, session_id: &str) -> Option<String> {
        let text = format!("{}\n\n{}", launchpad_github::suggestions::instruction(app, session_id), launchpad_github::notes::instruction(app, session_id));
        Some(match launchpad_channels::connections::instruction(app, session_id) {
            Some(connections) => format!("{text}\n\n{connections}"),
            None => text,
        })
    }
    async fn bot_paused(&self, app: &App, bot_id: &str, paused: bool) -> Result<()> {
        crons::set_bot_paused(app, bot_id, paused).await
    }
    async fn bot_files_changed(&self, app: &App, bot_id: &str) -> Result<()> {
        crons::sync_bot_files(app, bot_id).await
    }
    async fn bot_deleted(&self, app: &App, bot_id: &str) -> Result<()> {
        crons::delete_for_bot(app, bot_id).await?;
        launchpad_channels::channel::bot_deleted(app, bot_id).await
    }
    async fn channel_card(&self, app: &App, workspace: &str, card: serde_json::Value, bot: Option<&str>, apply: bool, by: &str) -> Result<serde_json::Value> {
        launchpad_channels::cards::card(app, workspace, card, bot, apply, by).await
    }
    async fn selection_lines(&self, app: &App, selection: &launchpad_core::model::DiffSelection) -> Option<String> {
        launchpad_github::ext::selection_lines(app, selection).await
    }
    async fn today(&self, app: &App) -> Vec<launchpad_core::today::Item> {
        let mut out = launchpad_automation::today::items(app).await;
        out.extend(launchpad_channels::today::items(app).await);
        out.extend(launchpad_github::today::items(app).await);
        out.extend(launchpad_setup::home_update::today_items(app));
        out.extend(launchpad_setup::cli_update::today_items(app).await);
        out
    }
    async fn waiting_on_owner(&self, app: &App, session_id: &str) -> Result<Option<String>> {
        if let Some(why) = launchpad_github::suggestions::waiting_for(app, session_id).await? {
            return Ok(Some(why));
        }
        launchpad_channels::connections::outbox_waiting_for(app, session_id).await
    }
    async fn oracle_read(&self, app: &App, ws: &str, path: &str) -> Result<(u16, String)> {
        launchpad_server::oracle_read(app, ws, path).await
    }
}

/// The React app (`web/dist`), embedded in the release binary and read from disk in debug builds. The derive sits in
/// the binary, which CI's compiler cache never caches, so added and removed files are always seen.
#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Ui;

/// Starts the services of the crates above the core, after `App::start`.
pub async fn start(app: &App) -> Result<()> {
    // First: a Home restored from an update's checkpoint completes it, or keeps writes paused while its replacement
    // may still come (`home_update::resume`), before any service writes.
    launchpad_setup::home_update::resume(app).await?;
    launchpad_server::set_assets(launchpad_server::Assets { get: Ui::get, paths: || Ui::iter().map(|p| p.into_owned()).collect() });
    // Shared folio snapshots carry the folio-viewer build embedded with the UI.
    launchpad_automation::folio_share::set_viewer(launchpad_server::folio_viewer_files);
    launchpad_automation::start(app).await?;
    launchpad_channels::spawn(app);
    launchpad_github::ledger::resume_checks(app);
    launchpad_setup::home_update::spawn(app);
    launchpad_setup::cli_update::spawn(app);
    Ok(())
}

/// Writes the TypeScript versions of the types shared with the UI (ts-rs), for every crate.
pub fn gen_types(out: &std::path::Path) -> Result<()> {
    let cfg = launchpad_core::types_config(out)?;
    launchpad_core::gen_types(&cfg)?;
    launchpad_channels::gen_types(&cfg)?;
    launchpad_github::gen_types(&cfg)?;
    launchpad_automation::gen_types(&cfg)?;
    launchpad_setup::gen_types(&cfg)?;
    Ok(())
}

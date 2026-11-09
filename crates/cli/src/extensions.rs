//! The `launchpad_core::ext::Extensions` of this binary: connects the core to the crates above it.
use anyhow::Result;
use launchpad_automation::crons;
use launchpad_core::{App, model::Session};

pub struct Extensions;

#[async_trait::async_trait]
impl launchpad_core::ext::Extensions for Extensions {
    fn notify_channels(&self, app: &App, session: &Session, event: &'static str) {
        launchpad_channels::telegram::notify(app, session, event);
    }
    fn turn_ended(&self, app: &App, session: &Session) {
        crons::turn_ended(app, session);
    }
    fn suggestion_instruction(&self, app: &App, session_id: &str) -> Option<String> {
        let text = launchpad_github::suggestions::instruction(app, session_id);
        Some(match launchpad_channels::channels::instruction(app, session_id) {
            Some(channels) => format!("{text}\n\n{channels}"),
            None => text,
        })
    }
    async fn bot_paused(&self, app: &App, bot_id: &str, paused: bool) -> Result<()> {
        crons::set_bot_paused(app, bot_id, paused).await
    }
    async fn bot_deleted(&self, app: &App, bot_id: &str) -> Result<()> {
        crons::delete_for_bot(app, bot_id).await
    }
    async fn inbox_synced(&self, app: &App) {
        crons::inbox_synced(app).await;
    }
    async fn inbox_threads(&self, app: &App) -> Result<Vec<launchpad_core::github::Thread>> {
        Ok(launchpad_github::inbox::of(app).list(app).await?.threads)
    }
    async fn today(&self, app: &App) -> Vec<launchpad_core::today::Item> {
        let mut out = launchpad_automation::today::items(app).await;
        out.extend(launchpad_channels::today::items(app).await);
        out.extend(launchpad_github::today::items(app).await);
        out.extend(launchpad_setup::home_update::today_items(app));
        out
    }
}

/// Starts the services of the crates above the core, after `App::start`.
pub async fn start(app: &App) -> Result<()> {
    launchpad_automation::start(app).await?;
    launchpad_github::spawn(app);
    launchpad_channels::spawn(app);
    launchpad_setup::home_update::spawn(app);
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

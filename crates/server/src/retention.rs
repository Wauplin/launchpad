//! Daily pruning of `state.db` tables that only grow (at start, then every day, on Home and runners): completed
//! browser operation claims (`operations::prune`, the claims of this crate's `operations` guard), raw harness lines
//! (`Sink::raw`), proxy access entries (`contain::prune`) and GitHub item uses older than 180 days (`ref_use::prune`).
use std::time::Duration;

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::util::now_ms;

async fn run(app: &App) -> Result<()> {
    let now = now_ms();
    launchpad_core::operations::prune(&app.state_db, now).await?;
    let before = now - launchpad_core::sessions::RAW_KEEP_MS;
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM session_raw WHERE ts < ?1", [before])?)).await?;
    launchpad_core::ref_use::prune(app, now - 180 * 86_400_000).await?;
    launchpad_core::contain::prune(app).await
}

pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        loop {
            tick.tick().await;
            if let Err(e) = run(&app).await {
                tracing::warn!("state.db retention: {e:#}");
            }
        }
    });
}

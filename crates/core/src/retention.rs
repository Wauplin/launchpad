//! Daily pruning of `state.db` tables that only grow (at start, then every day, on Home and runners): completed
//! browser operation claims (`operations::prune`), raw harness lines (`Sink::raw`) and proxy access entries
//! (`contain::prune`).
use std::time::Duration;

use anyhow::Result;

use crate::App;
use crate::util::now_ms;

async fn run(app: &App) -> Result<()> {
    let now = now_ms();
    crate::operations::prune(&app.state_db, now).await?;
    let before = now - crate::sessions::RAW_KEEP_MS;
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM session_raw WHERE ts < ?1", [before])?)).await?;
    crate::contain::prune(app).await
}

pub fn spawn(app: &App) {
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

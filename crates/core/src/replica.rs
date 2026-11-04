//! Session records and events flow from a runner to Home, durably.
//!
//! On a runner, every stored session record and event also goes into `home_outbox`, in the same database
//! transaction. The tunnel sends the outbox in order; Home stores each item and its outbox number in one
//! transaction (`runner_ack`), then acknowledges; only then does the runner drop the items. After a
//! reconnection, the runner resends everything after Home's acknowledged number, so nothing is lost or applied
//! twice. Home ignores items of an older session generation (the session moved to another runner).

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::App;
use crate::model::{Session, SessionEvent};

static ENABLED: AtomicBool = AtomicBool::new(false);
static WAKE: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

/// Turns on the outbox (runner role).
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Session { session: Box<Session> },
    Event { generation: u32, event: SessionEvent },
    /// A launch that failed before it started: the record is gone.
    Forget { id: String },
}

/// Queues an item in the caller's transaction (runner only; replicas of other instances never go back).
pub(crate) fn queue(c: &Connection, item: &Item) -> Result<()> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Ok(());
    }
    if let Item::Session { session } = item
        && (session.imported_from.is_some() || session.runner_id.is_some())
    {
        return Ok(());
    }
    c.execute("INSERT INTO home_outbox (json) VALUES (?1)", [serde_json::to_string(item)?])?;
    WAKE.notify_one();
    Ok(())
}

/// Waits until new items are queued (or a second passes).
pub async fn wait() {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), WAKE.notified()).await;
}

/// Outbox items after `after`, oldest first.
pub async fn pending(app: &App, after: u64, limit: usize) -> Result<Vec<(u64, serde_json::Value)>> {
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT n, json FROM home_outbox WHERE n > ?1 ORDER BY n LIMIT ?2")?;
            let rows = st.query_map(params![after as i64, limit as i64], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|(n, j)| serde_json::from_str(&j).ok().map(|v| (n, v))).collect())
        })
        .await
}

/// Home saved everything up to `n`: the runner drops it.
pub async fn acked(app: &App, n: u64) -> Result<()> {
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM home_outbox WHERE n <= ?1", [n as i64])?)).await?;
    Ok(())
}

/// Items still waiting for Home on this runner.
pub async fn backlog(app: &App) -> Result<u64> {
    app.state_db.call(|c| Ok(c.query_row("SELECT count(*) FROM home_outbox", [], |r| r.get::<_, i64>(0))? as u64)).await
}

/// The last outbox number Home saved for a runner.
pub async fn acked_for(app: &App, runner_id: &str) -> Result<u64> {
    let id = runner_id.to_string();
    app.state_db
        .call(move |c| Ok(c.query_row("SELECT coalesce(max(n), 0) FROM runner_ack WHERE runner_id = ?1", [id], |r| r.get::<_, i64>(0))? as u64))
        .await
}

pub(crate) fn set_acked(c: &Connection, runner_id: &str, n: u64) -> Result<()> {
    c.execute(
        "INSERT INTO runner_ack (runner_id, n) VALUES (?1, ?2) ON CONFLICT (runner_id) DO UPDATE SET n = max(n, excluded.n)",
        params![runner_id, n as i64],
    )?;
    Ok(())
}

/// Stores one item from a runner on Home. Items at or below the acknowledged number are replays and skipped.
pub async fn ingest(app: &App, runner_id: &str, n: u64, item: serde_json::Value) -> Result<()> {
    if n <= acked_for(app, runner_id).await? {
        return Ok(());
    }
    match serde_json::from_value::<Item>(item) {
        Ok(item) => app.sessions.apply_replica(app, runner_id, n, item).await,
        Err(e) => {
            // An item this Home cannot read (another build) is skipped, never retried forever.
            tracing::warn!("runner {runner_id}: unreadable item {n}: {e}");
            let id = runner_id.to_string();
            app.state_db.call(move |c| set_acked(c, &id, n)).await
        }
    }
}

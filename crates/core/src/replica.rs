//! Session records and events flow from a runner to Home, durably.
//!
//! On a runner, every stored session record and event also goes into `home_outbox`, in the same database
//! transaction. The tunnel sends the outbox in order, in batches of at most `BATCH_BYTES` (a larger item goes
//! alone), one batch at a time; Home stores each item and its outbox number in one transaction (`runner_ack`),
//! then acknowledges the last item it stored, even when it stored none; only then does the runner drop the items,
//! and it sends the next batch from that acknowledgement (`after_ack`), so an item Home failed to store is sent
//! again, never skipped. After a reconnection, the runner resends everything after Home's acknowledged number, so
//! nothing is lost or applied twice. Home ignores items of an older session generation (the session moved).
//! Items carry no attachment bytes (`Session::without_bytes`, `attachments`); an item over `MAX_ITEM_BYTES`
//! still in the outbox loses its inline data URLs, else is replaced by a marker Home skips, so the outbox always
//! progresses.

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
    /// A folder registered or changed on the runner (its disk has it).
    Project { project: crate::model::Project },
    ForgetProject { id: String },
    /// A request of a contained session that received a token (`proxy.rs`), for Home's access log.
    Access { access: crate::proxy::Access },
    /// A worktree record, with the branch tip in its folder (`worktrees::save`), for Home's away views.
    Worktree { worktree: Box<crate::worktrees::TaskWorktree> },
}

/// Queues an item in the caller's transaction (runner only; replicas of other instances never go back).
pub(crate) fn queue(c: &Connection, item: &Item) -> Result<()> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Ok(());
    }
    if let Item::Session { session } = item
        && session.runner_id.is_some()
    {
        return Ok(());
    }
    if let Item::Project { project } = item
        && project.runner_id.is_some()
    {
        return Ok(());
    }
    c.execute("INSERT INTO home_outbox (json) VALUES (?1)", [serde_json::to_string(item)?])?;
    WAKE.notify_one();
    Ok(())
}

/// Whether this daemon queues items for Home.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Wakes the sender (Home acknowledged a batch).
pub fn wake() {
    WAKE.notify_one();
}

/// Waits until new items are queued (or a second passes).
pub async fn wait() {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), WAKE.notified()).await;
}

/// The most bytes of items in one batch (one tunnel message); a larger item goes alone.
pub const BATCH_BYTES: usize = 4 << 20;
/// The largest item sent as is: tunnel messages stop at 64 MiB (daemon and Space gateway), with room for the envelope.
pub const MAX_ITEM_BYTES: usize = 48 << 20;
/// Items in one batch at most.
const BATCH_ITEMS: usize = 200;

/// Whether an item of `len` bytes joins a batch that holds `count` items of `total` bytes: the first item always
/// goes (alone when it is larger than the cap), the others while the batch stays within `BATCH_BYTES`.
pub fn fits(count: usize, total: usize, len: usize) -> bool {
    count == 0 || (count < BATCH_ITEMS && total + len <= BATCH_BYTES)
}

/// After Home acknowledged `ack` while the batch through `sent` was in flight: Home's mark (the runner sends from
/// there) and whether Home stored the whole batch (if not, the runner waits a moment before sending it again).
pub fn after_ack(acked: u64, sent: Option<u64>, ack: u64) -> (u64, bool) {
    (acked.max(ack), sent.is_none_or(|sent| ack >= sent))
}

/// The next batch: outbox items after `after`, oldest first, cut by `fits`.
pub async fn batch(app: &App, after: u64) -> Result<Vec<(u64, serde_json::Value)>> {
    let rows = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT n, json FROM home_outbox WHERE n > ?1 ORDER BY n LIMIT ?2")?;
            let mut rows = st.query(params![after as i64, BATCH_ITEMS as i64])?;
            let (mut out, mut total) = (Vec::new(), 0);
            while let Some(r) = rows.next()? {
                let json: String = r.get(1)?;
                if !fits(out.len(), total, json.len()) {
                    break;
                }
                total += json.len();
                out.push((r.get::<_, i64>(0)? as u64, json));
            }
            Ok(out)
        })
        .await?;
    Ok(rows.into_iter().map(|(n, json)| (n, sendable(n, &json))).collect())
}

/// An outbox item as it can travel: unreadable or too large items become a marker Home acknowledges and skips.
fn sendable(n: u64, json: &str) -> serde_json::Value {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        tracing::warn!("outbox item {n} is unreadable: skipped");
        return serde_json::json!({"kind": "unreadable"});
    };
    if json.len() <= MAX_ITEM_BYTES {
        return value;
    }
    strip_data_urls(&mut value);
    let len = value.to_string().len();
    if len <= MAX_ITEM_BYTES {
        tracing::warn!("outbox item {n} ({} bytes): sent without its inline attachments", json.len());
        value
    } else {
        tracing::warn!("outbox item {n} ({len} bytes) is too large for the tunnel: skipped");
        serde_json::json!({"kind": "too_large", "bytes": len})
    }
}

fn strip_data_urls(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) if s.starts_with("data:") => s.clear(),
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_data_urls),
        serde_json::Value::Object(o) => o.values_mut().for_each(strip_data_urls),
        _ => {}
    }
}

/// How long Home retries an item it fails to store (the runner sends it again every few seconds) before it skips
/// it, so one item that never stores cannot stop a runner's replication for good.
pub const GIVE_UP_MS: i64 = 10 * 60 * 1000;

/// Home, after item `n` failed to store: whether to skip it now. `failing` holds the failing item and since when.
pub fn give_up(failing: &mut Option<(u64, i64)>, n: u64, now: i64) -> bool {
    match *failing {
        Some((f, since)) if f == n => now - since >= GIVE_UP_MS,
        _ => {
            *failing = Some((n, now));
            false
        }
    }
}

/// Home: moves a runner's mark past item `n` without storing it (`give_up`).
pub async fn skip(app: &App, runner_id: &str, n: u64) -> Result<()> {
    let id = runner_id.to_string();
    app.state_db.call(move |c| set_acked(c, &id, n)).await
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

/// The newest item queued for Home (0 when none waits).
pub async fn newest(app: &App) -> Result<u64> {
    app.state_db.call(|c| Ok(c.query_row("SELECT coalesce(max(n), 0) FROM home_outbox", [], |r| r.get::<_, i64>(0))? as u64)).await
}

/// Items up to `n` (`newest`) still waiting for Home.
pub async fn waiting_through(app: &App, n: u64) -> Result<u64> {
    app.state_db.call(move |c| Ok(c.query_row("SELECT count(*) FROM home_outbox WHERE n <= ?1", [n as i64], |r| r.get::<_, i64>(0))? as u64)).await
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

/// Home: one access log entry of a runner's session.
pub(crate) async fn apply_access(app: &App, runner_id: &str, n: u64, access: crate::proxy::Access) -> Result<()> {
    let id = runner_id.to_string();
    let session = access.session_id.clone();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            crate::contain::insert_access(&tx, &access)?;
            set_acked(&tx, &id, n)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("vault", "access", serde_json::json!({"session_id": session}));
    Ok(())
}

/// Stores one item from a runner on Home. Items at or below the acknowledged number are replays and skipped.
pub async fn ingest(app: &App, runner_id: &str, n: u64, item: serde_json::Value) -> Result<()> {
    if n <= acked_for(app, runner_id).await? {
        return Ok(());
    }
    match serde_json::from_value::<Item>(item) {
        Ok(item @ (Item::Project { .. } | Item::ForgetProject { .. })) => crate::projects::apply_replica(app, runner_id, n, item).await,
        Ok(Item::Access { access }) => apply_access(app, runner_id, n, access).await,
        Ok(Item::Worktree { worktree }) => crate::away::apply_replica(app, runner_id, n, *worktree).await,
        Ok(item) => app.sessions.apply_replica(app, runner_id, n, item).await,
        Err(e) => {
            // An item this Home cannot read (another build) is skipped, never retried forever.
            tracing::warn!("runner {runner_id}: unreadable item {n}: {e}");
            let id = runner_id.to_string();
            app.state_db.call(move |c| set_acked(c, &id, n)).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_are_cut_by_bytes_and_a_large_item_goes_alone() {
        assert!(fits(0, 0, BATCH_BYTES * 10));
        assert!(!fits(1, 10, BATCH_BYTES));
        assert!(fits(1, 10, BATCH_BYTES - 10));
        assert!(!fits(BATCH_ITEMS, 0, 1));
    }

    #[test]
    fn an_acknowledgement_short_of_the_batch_resends_from_it() {
        // Home stored everything: the next batch starts after it.
        assert_eq!(after_ack(10, Some(20), 20), (20, true));
        // Home failed at item 16: the runner sends again after 15, never after 20.
        assert_eq!(after_ack(10, Some(20), 15), (15, false));
        // Home stored nothing (it acknowledges the item before the batch).
        assert_eq!(after_ack(10, Some(20), 10), (10, false));
        // A late acknowledgement never moves the mark back.
        assert_eq!(after_ack(20, None, 15), (20, true));
    }

    #[test]
    fn an_item_that_keeps_failing_is_skipped_after_ten_minutes() {
        let mut failing = None;
        assert!(!give_up(&mut failing, 5, 0));
        assert!(!give_up(&mut failing, 5, GIVE_UP_MS - 1));
        assert!(give_up(&mut failing, 5, GIVE_UP_MS));
        // Another item starts its own clock.
        assert!(!give_up(&mut failing, 6, GIVE_UP_MS * 2));
        assert_eq!(failing, Some((6, GIVE_UP_MS * 2)));
    }

    #[test]
    fn oversized_items_lose_their_data_urls() {
        let mut v = serde_json::json!({"kind": "event", "images": [{"name": "a.png", "data_url": "data:image/png;base64,AAAA"}, {"name": "b.png", "data_url": "upload:image/png;00"}]});
        strip_data_urls(&mut v);
        assert_eq!(v["images"][0]["data_url"], "");
        assert_eq!(v["images"][1]["data_url"], "upload:image/png;00");
    }
}

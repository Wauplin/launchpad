//! Session records and events flow from a runner to Home, durably.
//!
//! On a runner, every stored session record and event also goes into `home_outbox`, in the same database
//! transaction. The tunnel sends the outbox in order, in batches of at most `BATCH_BYTES` (a larger item goes
//! alone), one batch at a time; Home stores each item and its outbox number in one transaction (`runner_ack`),
//! then acknowledges the last item it stored, even when it stored none; only then does the runner drop the items,
//! and it sends the next batch from that acknowledgement (`after_ack`), so an item Home failed to store is sent
//! again, never skipped. After a reconnection, the runner resends everything after Home's acknowledged number, so
//! nothing is lost or applied twice. Home ignores items of an older session generation (the session moved).
//!
//! Durable acknowledgement: Home's `state.db` survives a restart only through its backups (a Home on a Space
//! restores its newest complete backup at each start), so an acknowledgement means "stored", not "safe". Home tells
//! each runner a second number, `Durable`, once a backup whose copy of `state.db` holds the runner's items is
//! readable in the destination itself (`home_backup::verify_base`, `set_durable`); only then does the runner drop
//! them. Until then it keeps them (`Ack` only moves where its next batch starts), so the items between a restored
//! Home's mark and the runner's newest are sent again after a rollback. A runner detects a rollback when Home's
//! acknowledged number on reconnect is lower than the highest one Home sent it (`reconnect`); when items it already
//! dropped are missing too (a Home restored past a durable mark), it queues its current session records again
//! (`requeue_sessions`), so Home's records converge even though those events cannot come back. A Home without a
//! backup destination keeps `state.db` on its own disk: its `Welcome` carries no durable number, and the runner
//! drops items at `Ack` like older builds. An older Home does the same; an older runner ignores `Durable` (Home
//! sends it only to runners that report `durable_ack`).
//!
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
    /// A cost increase of a session here (`spend.rs`): Home holds the ledger of every machine.
    Spend { entry: crate::spend::Entry },
    /// A wake-up Home accepted for a session of this runner (`launchpad sleep`, forwarded by the runner, with the
    /// scheduled follow-up Home answered): kept here until Home's backup holds it, so a Home restored from an older
    /// backup records it again (`ext::Extensions::replay_sleep`). An older Home skips it (unreadable item).
    Sleep { session_id: String, schedule: serde_json::Value, accepted_at: i64 },
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

/// Runner: queues an item for Home outside a record write (`Item::Sleep`).
pub fn queue_item(c: &Connection, item: &Item) -> Result<()> {
    queue(c, item)
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

/// Runner: Home's backup holds everything up to `n` (or Home keeps no backup and saved it): the runner drops it.
pub async fn acked(app: &App, n: u64) -> Result<()> {
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM home_outbox WHERE n <= ?1", [n as i64])?)).await?;
    Ok(())
}

/// Runner: whether the connected Home sends durable numbers (`Welcome.durable`): items stay until `Durable`.
static HOME_DURABLE: AtomicBool = AtomicBool::new(false);

/// Runner: Home acknowledged `n` (stored, maybe not yet in a backup). Records the highest acknowledgement seen (for
/// `reconnect`) and, with a Home that sends no durable numbers, drops the items.
pub async fn on_ack(app: &App, n: u64) -> Result<()> {
    let durable = HOME_DURABLE.load(Ordering::Relaxed);
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            if !durable {
                tx.execute("DELETE FROM home_outbox WHERE n <= ?1", [n as i64])?;
            }
            tx.execute(
                "INSERT INTO home_meta (key, value) VALUES ('home_ack_seen', ?1) ON CONFLICT (key) DO UPDATE SET value = max(CAST(value AS INTEGER), CAST(excluded.value AS INTEGER))",
                [n.to_string()],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
}

/// What a runner does on reconnecting, from Home's acknowledged number and its own records (pure).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconnect {
    /// Home kept everything it acknowledged: send from its mark.
    Resume,
    /// Home went back (restored an older backup): every lost item is still here and goes again.
    Replay { lost: u64 },
    /// Home went back past items this runner already dropped: send what is left and queue the current records.
    Gap { lost: u64, missing: u64 },
}

/// `home`: Home's acknowledged number now; `seen`: the highest Home ever acknowledged to this runner; `oldest`: the
/// oldest item still in the outbox (None: empty); `newest`: the last outbox number this runner used.
pub fn reconnect(home: u64, seen: u64, oldest: Option<u64>, newest: u64) -> Reconnect {
    if home >= seen {
        return Reconnect::Resume;
    }
    let lost = seen - home;
    // Items after Home's mark that are no longer here: dropped after an earlier durable number (or by an older
    // build at acknowledgement).
    let first_kept = oldest.unwrap_or(newest + 1);
    let missing = first_kept.saturating_sub(home + 1).min(newest.saturating_sub(home));
    if missing == 0 { Reconnect::Replay { lost } } else { Reconnect::Gap { lost, missing } }
}

/// Runner: after `Welcome`: decides `reconnect`, drops what Home's backup holds (`durable`, or everything Home
/// acknowledged when it sends no durable number) and resets the highest acknowledgement to Home's.
pub async fn welcome(app: &App, acked: u64, durable: Option<u64>) -> Result<Reconnect> {
    HOME_DURABLE.store(durable.is_some(), Ordering::Relaxed);
    let plan = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let seen: u64 = tx.query_row("SELECT CAST(value AS INTEGER) FROM home_meta WHERE key = 'home_ack_seen'", [], |r| r.get::<_, i64>(0)).map(|v| v as u64).unwrap_or(0);
            let oldest: Option<i64> = tx.query_row("SELECT min(n) FROM home_outbox", [], |r| r.get(0))?;
            let newest: i64 = tx.query_row("SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'home_outbox'), 0)", [], |r| r.get(0))?;
            let plan = reconnect(acked, seen, oldest.map(|n| n as u64), newest as u64);
            tx.execute("DELETE FROM home_outbox WHERE n <= ?1", [durable.unwrap_or(acked).min(acked) as i64])?;
            tx.execute("INSERT INTO home_meta (key, value) VALUES ('home_ack_seen', ?1) ON CONFLICT (key) DO UPDATE SET value = excluded.value", [acked.to_string()])?;
            tx.commit()?;
            Ok(plan)
        })
        .await?;
    match plan {
        Reconnect::Resume => {}
        Reconnect::Replay { lost } => tracing::warn!("Home went back {lost} items (it restored an older backup): sending them again"),
        Reconnect::Gap { lost, missing } => {
            tracing::error!("Home went back {lost} items and {missing} of them are no longer on this runner (dropped after an earlier durable mark): queuing this runner's current session records again");
            requeue_sessions(app).await?;
        }
    }
    Ok(plan)
}

/// Runner: queues the current record of each session here again (not archived, or archived in the last week), so
/// a Home that lost some of their items converges to their present state.
pub async fn requeue_sessions(app: &App) -> Result<usize> {
    let week = crate::util::now_ms() - 7 * 24 * 3600 * 1000;
    let sessions: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.runner_id.is_none() && (s.status != crate::model::SessionStatus::Closed || s.status_since > week)).collect();
    let n = sessions.len();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for s in sessions {
                queue(&tx, &Item::Session { session: Box::new(s.without_bytes()) })?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(n)
}

/// Home: each runner's acknowledged number that a verified backup holds (`home_backup::verify_base`), or that a
/// restored `state.db` holds (`restored`). None: Home knows no durable number yet (runners keep everything).
static DURABLE: std::sync::Mutex<Option<std::collections::BTreeMap<String, u64>>> = std::sync::Mutex::new(None);
/// Home: when the checkpoint of the `state.db` this Home restored at start was taken (`home_meta.checkpoint_at`).
static RESTORED_AT: std::sync::Mutex<Option<i64>> = std::sync::Mutex::new(None);

/// Home: what `Welcome` says about durability for this runner: None when Home keeps no backup (its own disk is the
/// durable copy: runners drop items at `Ack`), else the runner's durable number (0 while none is known).
pub fn durable_for(app: &App, runner_id: &str) -> Option<u64> {
    crate::home_backup::Store::parse(&app.config.get().home.backup)?;
    Some(DURABLE.lock().unwrap().as_ref().and_then(|m| m.get(runner_id).copied()).unwrap_or(0))
}

/// Home: a verified backup holds these marks: tell each connected runner that reports `durable_ack`.
pub fn set_durable(app: &App, marks: &[(String, u64)]) {
    let changed: Vec<(String, u64)> = {
        let mut d = DURABLE.lock().unwrap();
        let map = d.get_or_insert_with(Default::default);
        let mut changed = vec![];
        for (id, n) in marks {
            if map.get(id).is_none_or(|old| n > old) {
                map.insert(id.clone(), *n);
                changed.push((id.clone(), *n));
            }
        }
        changed
    };
    for (id, n) in changed {
        if app.runners.durable_capable(&id) {
            app.runners.send_to(&id, crate::runners::proto::ToRunner::Durable { n });
        }
    }
}

/// Home, at start: `state.db` came from a backup whose checkpoint was taken at `checkpoint_at`; every runner mark
/// in it is durable (it is the backup).
pub async fn restored(app: &App, checkpoint_at: Option<i64>) -> Result<()> {
    *RESTORED_AT.lock().unwrap() = Some(checkpoint_at.unwrap_or(0));
    let marks = app.state_db.call(|c| Ok(c.prepare("SELECT runner_id, n FROM runner_ack")?.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    set_durable(app, &marks);
    Ok(())
}

/// Home: whether this Home restored a backup at start whose checkpoint is older than `at`: work a runner did at
/// `at` may be missing here (a placement, a wake-up), and the runner's replay may bring it back.
pub fn restored_before(at: i64) -> bool {
    recoverable(*RESTORED_AT.lock().unwrap(), at)
}

/// Pure: whether work done at `at` may be missing from a Home whose `state.db` was restored from a checkpoint taken at
/// `restored` (None: not restored at start). A margin for the clocks of runners (a session's start time is theirs).
pub fn recoverable(restored: Option<i64>, at: i64) -> bool {
    restored.is_some_and(|t| t < at + CLOCK_MARGIN_MS)
}

/// How far a runner's clock may be behind Home's for `restored_before`.
const CLOCK_MARGIN_MS: i64 = 10 * 60 * 1000;

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

/// An item whose failure must never be skipped (`give_up`): skipping would acknowledge it, and the runner would then
/// drop the only copy (a replayed wake-up). It is retried until it is handled or discarded on purpose.
#[derive(Debug)]
pub struct NeverSkip;

impl std::fmt::Display for NeverSkip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retried until recorded")
    }
}

impl std::error::Error for NeverSkip {}

/// Whether a failed item may be skipped after `GIVE_UP_MS` (not a `NeverSkip` one).
pub fn skippable(e: &anyhow::Error) -> bool {
    !e.chain().any(|c| c.is::<NeverSkip>())
}

/// Home: acknowledges item `n` only once `work` succeeded. A failure is returned (retryable): the mark stays, so the
/// runner, which keeps the item, sends it again (`after_ack`), and nothing is lost.
pub(crate) async fn ack_after(db: &crate::db::Db, runner_id: &str, n: u64, work: impl std::future::Future<Output = Result<()>>) -> Result<()> {
    work.await?;
    let id = runner_id.to_string();
    db.call(move |c| set_acked(c, &id, n)).await
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
        Ok(Item::Spend { entry }) => crate::spend::apply_replica(app, runner_id, n, entry).await,
        Ok(Item::Sleep { session_id, schedule, accepted_at }) => {
            // Only a Home that restored an older backup can miss a wake-up it accepted; another one, or a session
            // that is not this runner's, is discarded on purpose (acknowledged without a replay).
            let replay = restored_before(accepted_at) && app.sessions.get(&session_id).is_some_and(|s| s.runner_id.as_deref() == Some(runner_id));
            let work = async {
                if replay {
                    crate::ext::extensions().replay_sleep(app, &session_id, schedule).await.map_err(|e| e.context(format!("replay the wake-up of {session_id}")).context(NeverSkip))?;
                }
                Ok(())
            };
            ack_after(&app.state_db, runner_id, n, work).await
        }
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
    fn a_rolled_back_home_gets_the_items_a_runner_kept() {
        // Home kept what it acknowledged.
        assert_eq!(reconnect(50, 50, Some(41), 60), Reconnect::Resume);
        assert_eq!(reconnect(55, 50, None, 55), Reconnect::Resume);
        // Home restored a backup holding item 30 while it had acknowledged 50: items 31..=60 are all still here
        // (the runner dropped only what a backup held, through 30).
        assert_eq!(reconnect(30, 50, Some(31), 60), Reconnect::Replay { lost: 20 });
        // An empty outbox after everything was durable through 50, then Home restored 50: nothing lost.
        assert_eq!(reconnect(50, 50, None, 50), Reconnect::Resume);
        // A Home restored past a durable mark (an older backup than the one it reported): 31..=40 are gone here.
        assert_eq!(reconnect(30, 50, Some(41), 60), Reconnect::Gap { lost: 20, missing: 10 });
        // An older build dropped items at acknowledgement and the outbox is empty.
        assert_eq!(reconnect(30, 50, None, 50), Reconnect::Gap { lost: 20, missing: 20 });
    }

    /// After a restore, only work newer than the checkpoint can be missing: a Home that was not restored, or work
    /// clearly older than its checkpoint, never takes a runner's word for a session or wake-up it does not know.
    #[test]
    fn only_work_after_the_restored_checkpoint_comes_back() {
        const MIN: i64 = 60_000;
        assert!(!recoverable(None, 100 * MIN));
        assert!(recoverable(Some(100 * MIN), 101 * MIN));
        // A runner clock a few minutes behind still counts.
        assert!(recoverable(Some(100 * MIN), 95 * MIN));
        assert!(!recoverable(Some(100 * MIN), 80 * MIN));
    }

    #[tokio::test]
    async fn acknowledged_items_stay_until_durable_and_replay_after_a_rollback() {
        let path = std::env::temp_dir().join(format!("launchpad-replica-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open_state(&path).unwrap();
        db.call(|c| {
            for i in 0..10 {
                c.execute("INSERT INTO home_outbox (json) VALUES (?1)", [format!("{{\"kind\":\"forget\",\"id\":\"{i}\"}}")])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        // Home acknowledged 8, but its backup holds only 3: a durable Home's runner keeps 4..=10.
        let left = |db: crate::db::Db| async move { db.call(|c| Ok(c.query_row("SELECT count(*), min(n) FROM home_outbox", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))?)).await.unwrap() };
        db.call(|c| {
            c.execute("DELETE FROM home_outbox WHERE n <= 3", [])?;
            c.execute("INSERT INTO home_meta (key, value) VALUES ('home_ack_seen', '8')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(left(db.clone()).await, (7, Some(4)));
        // Home restarts from that backup: its acknowledged number is 3, the runner saw 8 — a replay of 4..=10.
        let (seen, oldest, newest) = db
            .call(|c| {
                Ok((
                    c.query_row("SELECT CAST(value AS INTEGER) FROM home_meta WHERE key = 'home_ack_seen'", [], |r| r.get::<_, i64>(0))? as u64,
                    c.query_row("SELECT min(n) FROM home_outbox", [], |r| r.get::<_, Option<i64>>(0))?.map(|n| n as u64),
                    c.query_row("SELECT seq FROM sqlite_sequence WHERE name = 'home_outbox'", [], |r| r.get::<_, i64>(0))? as u64,
                ))
            })
            .await
            .unwrap();
        assert_eq!(reconnect(3, seen, oldest, newest), Reconnect::Replay { lost: 5 });
        let _ = std::fs::remove_file(&path);
    }

    /// A wake-up replay that fails (the store failed) is not acknowledged, so the runner sends it again; the next
    /// try succeeds and only then moves the mark: the wake-up is never acknowledged without being recorded.
    #[tokio::test]
    async fn a_failed_replay_is_retried_never_acknowledged() {
        let path = std::env::temp_dir().join(format!("launchpad-replay-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open_state(&path).unwrap();
        let mark = |db: crate::db::Db| async move { db.call(|c| Ok(c.query_row("SELECT coalesce(max(n), 0) FROM runner_ack WHERE runner_id = 'r'", [], |r| r.get::<_, i64>(0))?)).await.unwrap() };
        let recorded = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let replay = |fail: bool| {
            let recorded = recorded.clone();
            async move {
                anyhow::ensure!(!fail, "database is locked");
                recorded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        };
        assert!(ack_after(&db, "r", 7, replay(true)).await.is_err());
        assert_eq!(mark(db.clone()).await, 0);
        // The runner kept item 7 and sends it again.
        ack_after(&db, "r", 7, replay(false)).await.unwrap();
        assert_eq!(mark(db.clone()).await, 7);
        assert_eq!(recorded.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn oversized_items_lose_their_data_urls() {
        let mut v = serde_json::json!({"kind": "event", "images": [{"name": "a.png", "data_url": "data:image/png;base64,AAAA"}, {"name": "b.png", "data_url": "upload:image/png;00"}]});
        strip_data_urls(&mut v);
        assert_eq!(v["images"][0]["data_url"], "");
        assert_eq!(v["images"][1]["data_url"], "upload:image/png;00");
    }
}

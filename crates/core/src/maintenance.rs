//! Home's maintenance mode: the write barrier of a planned update (`launchpad_setup::home_update`).
//!
//! A planned update must never report work as accepted that the replacement Home cannot restore. Home on a Space
//! restores its newest complete backup at every start, so the update runs in phases:
//!
//! 1. **Draining** (`enter`): new mutations are refused (`gated`: every HTTP request that is not a read, from
//!    windows, the CLI, agents, runners' forwarded calls, hooks; HTTP 503 with `Retry-After` and the `maintenance`
//!    object, never a false success); automation pauses (`active`: schedules, routines, queued bot messages); the
//!    mutating requests already running finish (`drained`, counted by the server's gate).
//! 2. **Checkpointing** (`freeze`): `state.db`'s only writer connection becomes read-only (`PRAGMA query_only`), so
//!    no background writer can change it after the final checkpoint; a write fails with `Paused` (HTTP 503). Then
//!    the backup is taken and verified on the storage side (`home_backup::checkpoint`).
//! 3. **Replacing**: the Space commit is made; Home stays frozen, still serving reads, until the platform replaces
//!    it. If the build fails, the checkpoint fails or the commit fails, Home thaws and leaves maintenance
//!    (`thaw`, `leave`), and says why.
//!
//! The first version keeps writes paused for the whole build (several minutes). Runners keep their replicated
//! records (`replica`) and retry; agents get a clear retryable error from every write.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;

/// Where a planned update is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "MaintenancePhase")]
pub enum Phase {
    /// New mutations are refused; running ones finish.
    Draining,
    /// `state.db` is read-only; the final backup is taken and verified.
    Checkpointing,
    /// The new build is committed: this Home serves reads until the platform replaces it.
    Replacing,
}

/// Shown in every window (the status's `maintenance`) and returned with every refused write.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Maintenance {
    pub phase: Phase,
    /// What happens, in words ("Home updates to v2026.11.19.1").
    pub reason: String,
    /// The version Home updates to.
    pub target: Option<String>,
    #[ts(type = "number")]
    pub since: i64,
    /// When the caller may try again, in seconds (also the `Retry-After` header).
    pub retry_after: u32,
}

static STATE: Mutex<Option<Maintenance>> = Mutex::new(None);
static FROZEN: AtomicBool = AtomicBool::new(false);
static INFLIGHT: AtomicUsize = AtomicUsize::new(0);
static DRAINED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// The maintenance now, if any.
pub fn current() -> Option<Maintenance> {
    STATE.lock().unwrap().clone()
}

/// Whether Home is in maintenance: writes are refused and automation waits.
pub fn active() -> bool {
    STATE.lock().unwrap().is_some()
}

/// Whether `state.db` is read-only (checkpoint taken or being taken).
pub fn frozen() -> bool {
    FROZEN.load(Ordering::SeqCst)
}

/// Enters (or changes) maintenance and tells every window.
pub fn enter(app: &App, m: Maintenance) {
    *STATE.lock().unwrap() = Some(m.clone());
    app.status.update(|s| s.maintenance = Some(m.clone()));
    app.bus.publish("maintenance", "changed", serde_json::to_value(&m).unwrap_or_default());
}

/// Leaves maintenance (after `thaw`): writes and automation resume.
pub fn leave(app: &App) {
    *STATE.lock().unwrap() = None;
    app.status.update(|s| s.maintenance = None);
    app.bus.publish("maintenance", "changed", serde_json::Value::Null);
}

/// Makes `state.db` read-only for every later write (`PRAGMA query_only` on its only writer connection). The call
/// runs on the database thread after every write queued before it.
pub async fn freeze(app: &App) -> anyhow::Result<()> {
    FROZEN.store(true, Ordering::SeqCst);
    let frozen = app.state_db.call(|c| {
        c.pragma_update(None, "query_only", true)?;
        Ok(())
    }).await;
    if frozen.is_err() {
        FROZEN.store(false, Ordering::SeqCst);
    }
    frozen
}

/// Makes `state.db` writable again (an update that stopped before the replacement).
pub async fn thaw(app: &App) -> anyhow::Result<()> {
    app.state_db.call(|c| {
        c.pragma_update(None, "query_only", false)?;
        Ok(())
    }).await?;
    FROZEN.store(false, Ordering::SeqCst);
    Ok(())
}

/// A mutating request in progress (counted by the server's gate while it runs).
pub struct Inflight(());

impl Inflight {
    pub fn begin() -> Inflight {
        INFLIGHT.fetch_add(1, Ordering::SeqCst);
        Inflight(())
    }
}

impl Drop for Inflight {
    fn drop(&mut self) {
        if INFLIGHT.fetch_sub(1, Ordering::SeqCst) == 1 {
            DRAINED.notify_waiters();
        }
    }
}

/// Mutating requests in progress.
pub fn inflight() -> usize {
    INFLIGHT.load(Ordering::SeqCst)
}

/// Waits until no mutating request runs, at most `timeout`; an error names how many still run.
pub async fn drained(timeout: Duration) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let wait = DRAINED.notified();
        let n = inflight();
        if n == 0 {
            return Ok(());
        }
        if tokio::time::timeout_at(deadline, wait).await.is_err() {
            anyhow::bail!("{n} request(s) still running after {} s", timeout.as_secs());
        }
    }
}

/// Whether a request is refused during maintenance (pure). Reads pass; so do Git fetches (`git-upload-pack` is a
/// POST that reads), the update's own routes (a retry after a lost answer gets the update's state, `home_update`;
/// Check for updates writes nothing) and Cancel update (`home.update-cancel`, the owner's way out).
pub fn gated(method: &str, path: &str) -> bool {
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return false;
    }
    let allowed = matches!(path, "/api/home/update" | "/api/home/update/cancel" | "/api/home/update/check") || path.ends_with("/git-upload-pack");
    !allowed
}

/// The error of a write refused because of maintenance: a 503 the caller retries later.
#[derive(Debug)]
pub struct Paused(pub Maintenance);

impl std::fmt::Display for Paused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&message(&self.0))
    }
}

impl std::error::Error for Paused {}

/// What a refused write says.
pub fn message(m: &Maintenance) -> String {
    let phase = match m.phase {
        Phase::Draining => "is finishing the requests in progress before it updates",
        Phase::Checkpointing => "is saving its final backup before it updates",
        Phase::Replacing => "is restarting with the new version",
    };
    format!("{}: Home {phase}, so it accepts no changes now. Nothing was changed; try again in about {} minutes (`Retry-After`).", m.reason, m.retry_after.div_ceil(60).max(1))
}

/// The maintenance a failed database write ran into (the database is frozen), as an error to return.
pub fn explain(e: anyhow::Error) -> anyhow::Error {
    let readonly = e.chain().any(|c| c.to_string().contains("readonly database"));
    match (frozen() && readonly, current()) {
        (true, Some(m)) => e.context(Paused(m)),
        _ => e,
    }
}

/// The maintenance an error carries (`Paused`), for HTTP answers.
pub fn of_error(e: &anyhow::Error) -> Option<Maintenance> {
    e.chain().find_map(|c| c.downcast_ref::<Paused>().map(|p| p.0.clone()))
}

/// Tests of other crates: set the maintenance without an `App`.
#[doc(hidden)]
pub fn state_for_tests() -> std::sync::MutexGuard<'static, Option<Maintenance>> {
    STATE.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reads_and_the_updates_own_routes_pass() {
        assert!(!gated("GET", "/api/sessions"));
        assert!(!gated("HEAD", "/"));
        assert!(gated("POST", "/api/sessions"));
        assert!(gated("POST", "/api/sleep"));
        assert!(gated("PUT", "/api/config"));
        assert!(gated("DELETE", "/api/schedules/x"));
        assert!(gated("POST", "/hooks/abc"));
        assert!(gated("POST", "/api/git/p/repo.git/git-receive-pack"));
        assert!(!gated("POST", "/api/git/p/repo.git/git-upload-pack"));
        assert!(!gated("POST", "/api/home/update"));
        assert!(!gated("POST", "/api/home/update/cancel"));
    }

    #[tokio::test]
    async fn draining_waits_for_running_writes() {
        let a = Inflight::begin();
        let b = Inflight::begin();
        assert!(drained(Duration::from_millis(20)).await.is_err());
        let wait = tokio::spawn(drained(Duration::from_secs(2)));
        drop(a);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!wait.is_finished());
        drop(b);
        assert!(wait.await.unwrap().is_ok());
    }
}

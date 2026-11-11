//! Saving a finished turn on Home: its branches (`worktrees::after_turn`), plain folders (`plain::after_turn`)
//! and conversation (`handoff::after_turn`).
//!
//! The turn's end stores the session's new status together with an `unsaved` mark (`Session::unsaved`), in the
//! same record Home receives, so Home never shows a turn as saved before it is: windows show it as saving.
//! The runner then saves the turn and clears the mark. A failure keeps the mark with its reason, and the runner
//! retries every few minutes and when its tunnel to Home comes back (`kick`). During a turn, its files go to
//! Home every five minutes (`periodic`): worktree snapshots and plain and temporary folders.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Result, ensure};

use crate::App;
use crate::model::Unsaved;
use crate::util::now_ms;

const RETRY_EVERY: Duration = Duration::from_secs(3 * 60);
/// While a turn runs, its files go to Home this often (`periodic`).
const PERIODIC_EVERY: Duration = Duration::from_secs(5 * 60);

/// One save per session at a time; a move holds it too (`hold`), so it never pushes beside a save.
static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);
static WAKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// The mark a turn's end stores with the new status: on a runner, every session has its conversation (and its
/// worktrees and plain folders) to save on Home. Home itself runs no turns.
pub(crate) fn mark(app: &App) -> Option<Unsaved> {
    (app.paths.role == crate::role::Role::Runner).then(|| Unsaved { since: now_ms(), attempts: 0, note: None })
}

/// Saves the session's last turn on Home, then clears its mark; a failure keeps the mark with the reason. A turn
/// that ended while this ran is saved again at once.
pub async fn save(app: &App, id: &str) {
    // A save of this session runs already: it sees a newer mark and saves again.
    let Ok(_saving) = lock_of(id).try_lock_owned() else { return };
    while let Some(since) = app.sessions.get(id).and_then(|s| s.unsaved).map(|u| u.since) {
        let result = attempt(app, id).await;
        if let Err(e) = &result {
            tracing::debug!("save of {id} on Home: {e:#}");
        }
        let updated = app
            .sessions
            .update(app, id, |s| {
                if s.unsaved.as_ref().is_none_or(|u| u.since != since) {
                    return;
                }
                match &result {
                    Ok(()) => s.unsaved = None,
                    Err(e) => {
                        if let Some(u) = s.unsaved.as_mut() {
                            u.attempts += 1;
                            u.note = Some(format!("{e:#}"));
                        }
                    }
                }
            })
            .await;
        let newer = updated.as_ref().ok().and_then(|s| s.unsaved.as_ref()).is_some_and(|u| u.since != since);
        // Stop unless a newer turn ended meanwhile; Home starts routines that wait for the work when it
        // receives the cleared mark (`turn_saved`).
        if !newer {
            break;
        }
    }
}

fn lock_of(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS.lock().unwrap().entry(id.to_string()).or_default().clone()
}

/// Waits for a save of the session that runs now, and keeps the next one from starting while the guard lives
/// (a move's final handoff pushes and syncs itself).
pub(crate) async fn hold(id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    lock_of(id).lock_owned().await
}

async fn attempt(app: &App, id: &str) -> Result<()> {
    let mut failed = vec![];
    if let Err(e) = crate::worktrees::after_turn(app, id).await {
        failed.push(format!("{e:#}"));
    }
    if let Err(e) = crate::plain::after_turn(app, id).await {
        failed.push(format!("{e:#}"));
    }
    if let Err(e) = crate::handoff::after_turn(app, id).await {
        failed.push(format!("{e:#}"));
    }
    ensure!(failed.is_empty(), "{}", failed.join("; "));
    Ok(())
}

/// While a turn runs: the uncommitted files of its worktrees (`worktrees::snapshots_of`) and its plain,
/// temporary and asset folders (`plain::after_turn`) go to Home, so a machine that disappears mid-turn loses at
/// most a few minutes of work. Skipped while a save or a move of the session runs.
async fn periodic(app: &App, id: &str) {
    let Ok(_saving) = lock_of(id).try_lock_owned() else { return };
    if let Err(e) = crate::worktrees::snapshots_of(app, id).await {
        tracing::debug!("snapshot of {id} on Home: {e:#}");
    }
    if let Err(e) = crate::plain::after_turn(app, id).await {
        tracing::debug!("files of {id} on Home: {e:#}");
    }
}

/// Turns that a restart of this runner cut short, with their notification: sent once the tunnel to Home is up.
static RESTART_NOTICES: LazyLock<Mutex<Vec<(String, crate::notify::SessionNotification)>>> = LazyLock::new(Default::default);

/// A turn the runner's restart interrupted (`Sessions::load`): its notification goes when Home is reachable.
pub(crate) fn notify_when_connected(id: &str, event: crate::notify::SessionNotification) {
    RESTART_NOTICES.lock().unwrap().push((id.to_string(), event));
}

/// The tunnel to Home is back: retry the turns not saved yet.
pub fn kick() {
    WAKE.notify_one();
}

/// Retries every few minutes, and after `kick`, the turns not saved on Home yet (sessions between turns only:
/// a running one saves again at its turn end; an archived one is left to Archive). Saves the files of running
/// turns every five minutes (`periodic`).
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Runner {
        return;
    }
    let periodic_app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(PERIODIC_EVERY).await;
            let running: Vec<String> = periodic_app.sessions.list().into_iter().filter(|s| periodic_app.sessions.is_busy(s)).map(|s| s.id).collect();
            for id in running {
                periodic(&periodic_app, &id).await;
            }
        }
    });
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let connected = tokio::select! {
                _ = tokio::time::sleep(RETRY_EVERY) => false,
                _ = WAKE.notified() => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    true
                }
            };
            let notices = if connected { std::mem::take(&mut *RESTART_NOTICES.lock().unwrap()) } else { vec![] };
            for (id, event) in notices {
                if let Some(s) = app.sessions.get(&id).filter(|s| !app.sessions.is_watched(&s.id)) {
                    crate::notify::session(&app, &s, event);
                }
            }
            // Archive does its own last sync (`plain::after_archive`, worktree cleanup, `handoff::clean_archived`).
            let pending = app.sessions.list().into_iter().filter(|s| s.unsaved.is_some() && s.status != crate::model::SessionStatus::Closed && !app.sessions.is_busy(s));
            for s in pending {
                save(&app, &s.id).await;
            }
        }
    });
}

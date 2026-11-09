//! Saving a finished turn on Home: its branches (`worktrees::after_turn`), plain folders (`plain::after_turn`)
//! and conversation (`handoff::after_turn`).
//!
//! The turn's end stores the session's new status together with an `unsaved` mark (`Session::unsaved`), in the
//! same record Home receives, so Home never shows a turn as saved before it is: windows show it as saving.
//! The runner then saves the turn and clears the mark. A failure keeps the mark with its reason, and the runner
//! retries every few minutes and when its tunnel to Home comes back (`kick`).

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Result, ensure};

use crate::App;
use crate::model::Unsaved;
use crate::util::now_ms;

const RETRY_EVERY: Duration = Duration::from_secs(3 * 60);

/// Sessions whose save runs now: one at a time per session.
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
static WAKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// The mark a turn's end stores with the new status, when the session has something to save on Home:
/// worktrees, plain folders, or (on a runner) its conversation.
pub(crate) async fn mark(app: &App, id: &str) -> Option<Unsaved> {
    let worktrees = crate::worktrees::for_session(app, id).await.is_ok_and(|l| l.iter().any(|w| w.state == crate::worktrees::WorktreeState::Active));
    let save = worktrees || app.paths.role == crate::role::Role::Runner || !crate::plain::session_stores(app, id).await.is_empty();
    save.then(|| Unsaved { since: now_ms(), attempts: 0, note: None })
}

/// Saves the session's last turn on Home, then clears its mark; a failure keeps the mark with the reason. A turn
/// that ended while this ran is saved again at once.
pub async fn save(app: &App, id: &str) {
    if !RUNNING.lock().unwrap().insert(id.to_string()) {
        return;
    }
    loop {
        let Some(since) = app.sessions.get(id).and_then(|s| s.unsaved).map(|u| u.since) else { break };
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
        // Stop unless a newer turn ended meanwhile (its mark has another time).
        let newer = updated.ok().and_then(|s| s.unsaved).is_some_and(|u| u.since != since);
        if result.is_err() || !newer {
            break;
        }
    }
    RUNNING.lock().unwrap().remove(id);
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

/// The tunnel to Home is back: retry the turns not saved yet.
pub fn kick() {
    WAKE.notify_one();
}

/// Retries every few minutes, and after `kick`, the turns not saved on Home yet (sessions between turns only:
/// a running one saves again at its turn end; an archived one is left to Archive).
pub fn spawn(app: &App) {
    if !app.paths.role.runs_agents() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(RETRY_EVERY) => {}
                _ = WAKE.notified() => tokio::time::sleep(Duration::from_secs(2)).await,
            }
            // Archive does its own last sync (`plain::after_archive`, worktree cleanup, `handoff::clean_archived`).
            let pending = app.sessions.list().into_iter().filter(|s| s.unsaved.is_some() && s.status != crate::model::SessionStatus::Closed && !app.sessions.is_busy(s));
            for s in pending {
                save(&app, &s.id).await;
            }
        }
    });
}

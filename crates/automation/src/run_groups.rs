//! Home: a routine run's group (`bot_threads::run_threads`): the run's own thread plus every thread its handoffs
//! started (another bot's thread of its conversation, a channel or DM thread a post of the run woke, and so on,
//! transitively, `MessageFrom.run`). A routine with `auto_archive` archives the whole group once every thread of it is
//! Done and settled with nothing waiting (`group_waits`); anything failed, asking, sleeping, working, queued or
//! waiting for the owner (a card, a Suggestion, an Outbox message, a Local PR, a merge) keeps the whole group.
//! Threads that join after the run was archived go on their own once they settle (`crons::auto_archive`). Each
//! thread's session is claimed once (`task_archive::claim`), so Bring back is never undone.

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::bot_threads::{self, BotThread};
use launchpad_core::model::{Session, SessionStatus};

/// Pure: why one thread's session keeps its run's group visible, None when it lets it go: Done for at least
/// `task_archive::SETTLE_MS` with nothing open in it (`task_archive::settled`). `None` session: the thread's first
/// message has not started it yet.
pub fn member_blocks(s: Option<&Session>, now: i64) -> Option<String> {
    let Some(s) = s else { return Some("a handoff has not started yet".into()) };
    let what = match s.status {
        SessionStatus::Closed => return None,
        SessionStatus::Error | SessionStatus::Interrupted => "failed",
        _ if s.open_question => "asks you",
        _ if s.sleeping() => "sleeps",
        _ if s.quota_pause.is_some() => "waits for its quota",
        SessionStatus::Waiting if crate::task_archive::settled(s, now) => return None,
        SessionStatus::Waiting => "just finished",
        SessionStatus::Stopped => "was stopped",
        _ if s.status.is_running() => "works",
        _ => "is not Done",
    };
    Some(format!("“{}” {what}", s.title))
}

/// The open threads of the run's group other than `except` (the run's own thread when it is still open).
pub async fn open_members(app: &App, run: &str, except: Option<(&str, &str)>) -> Result<Vec<BotThread>> {
    Ok(bot_threads::run_threads(app, run).await?.into_iter().filter(|t| t.archived_at.is_none() && except != Some((t.bot_id.as_str(), t.id.as_str()))).collect())
}

/// Messages to a thread that wait in Home's queue, or any waiting message of the run (a delivery not yet in a thread).
async fn queued(app: &App, run: &str, threads: &[BotThread]) -> Result<bool> {
    let keys: Vec<String> = threads.iter().map(|t| format!("bot:{}:{}", t.bot_id, t.id)).collect();
    let (keys, run) = (serde_json::to_string(&keys)?, run.to_string());
    app.state_db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM queued_message WHERE target IN (SELECT value FROM json_each(?1)) OR (json_valid(json) AND json_extract(json, '$.from.run') = ?2))",
                rusqlite::params![keys, run],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await
}

/// Why the run's group must stay (its open threads `members`, the run's own thread excluded), None when every thread
/// of it settled with nothing waiting (`member_blocks`, `task_archive::waits`). A check that cannot be read keeps it.
pub async fn group_waits(app: &App, run: &str, members: &[BotThread], now: i64) -> Result<Option<String>> {
    if queued(app, run, members).await? {
        return Ok(Some("a message of the run waits to be delivered".into()));
    }
    for t in members {
        let s = bot_threads::session(app, &t.bot_id, &t.id);
        if let Some(why) = member_blocks(s.as_ref(), now) {
            return Ok(Some(why));
        }
        if let Some(s) = s.filter(|s| s.status != SessionStatus::Closed)
            && let Some(why) = crate::task_archive::waits(app, &s).await?
        {
            return Ok(Some(format!("“{}”: {why}", s.title)));
        }
    }
    Ok(None)
}

/// Archives the open threads of the group (each with its conversation), each session claimed once before. Threads of
/// the run's own conversation go with the run's own thread (`bot_threads::archive`), never here, so a run the owner
/// brought back keeps them. Returns how many could not be archived.
pub async fn archive_members(app: &App, run: &str, members: &[BotThread]) -> usize {
    let mut failed = 0;
    for t in members.iter().filter(|t| t.conversation != run) {
        // Archived with an earlier thread of its conversation.
        if bot_threads::open_session(app, &t.bot_id, &t.id).is_none() {
            continue;
        }
        if let Some(s) = bot_threads::session(app, &t.bot_id, &t.id) {
            match crate::task_archive::claim(app, &s.id).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    tracing::warn!("run group archive of {}: {e:#}", s.id);
                    failed += 1;
                    continue;
                }
            }
            let outcome = match bot_threads::archive(app, &t.bot_id, &t.id, true).await {
                Ok(_) => "archived with its run".to_string(),
                Err(e) => {
                    tracing::warn!("run group archive of thread {} of bot {}: {e:#}", t.id, t.bot_id);
                    failed += 1;
                    format!("failed: {e:#}")
                }
            };
            let _ = crate::task_archive::record(app, &s.id, outcome).await;
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(status: &str, since: i64) -> Session {
        launchpad_harness::install();
        serde_json::from_value(serde_json::json!({
            "id": "s1", "harness": "codex", "cwd": "/tmp/x", "title": "Brief", "status": status, "status_since": since,
            "live": false, "open_question": false, "prompt": "p", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0},
            "started_at": 0, "last_seq": 1, "launch_spec": {}, "seen_at": 0
        }))
        .unwrap()
    }

    #[test]
    fn only_settled_done_threads_let_their_run_group_go() {
        let now = 10_000_000;
        // Done for over a minute, or already archived: it may go.
        assert_eq!(member_blocks(Some(&session("waiting", now - 120_000)), now), None);
        assert_eq!(member_blocks(Some(&session("closed", now)), now), None);
        // Failed, interrupted, working, stopped, just finished or not started: the group stays, naming the thread.
        for (status, since, why) in [("error", 0, "failed"), ("interrupted", 0, "failed"), ("working", 0, "works"), ("stopped", 0, "was stopped"), ("waiting", now - 1_000, "just finished")] {
            assert_eq!(member_blocks(Some(&session(status, since)), now).as_deref(), Some(format!("“Brief” {why}").as_str()), "{status}");
        }
        assert!(member_blocks(None, now).is_some());
        // A question or a sleep keeps it too.
        let asking = Session { open_question: true, ..session("waiting", 0) };
        assert_eq!(member_blocks(Some(&asking), now).as_deref(), Some("“Brief” asks you"));
    }
}

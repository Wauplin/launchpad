//! Home: bots' task sessions archive themselves once they finished cleanly. A session a bot started on its own (a
//! direct launch, a card started without the owner's click, a scheduled launch or a run of a routine the bot created)
//! goes to Archived once it is Done with its work saved on Home and nothing left for the owner: no question, no
//! failure, no merge, Local PR, Suggestion, Outbox message, card or follow-up that waits. The owner's own launches,
//! bots' own threads and sessions the owner took part in (a message, an answer) are never archived here; routines
//! with their own `auto_archive` keep it (`crons::auto_archive`).
//!
//! Safety: every check that cannot be read keeps the session; each session is claimed once (`task_archive` table)
//! before its archive, so Bring back is never undone and an uncertain archive is never retried; the archive itself
//! runs only while the session is still Done since the same moment with nothing running, checked by its runner under
//! the session's action lock (`runners::archive_if_done`).
use std::collections::HashSet;

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::model::{EventBody, SenderKind, Session, SessionStatus};
use launchpad_core::util::now_ms;

use crate::crons::CronAgent;

/// How long after it became Done a session may still go on its own: work that waits longer (a Local PR the owner
/// publishes a week later) keeps it visible for good.
pub const WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// How long a session stays Done before it may go: the worktree records and Local PRs of its last turn reach Home
/// with or just after its status.
pub const SETTLE_MS: i64 = 60_000;

/// The bot that started the session on its own, from Home's records; None for the owner's launches (the owner's
/// click on a card or a schedule clears the sender), another agent's, a hook's, a bot's own thread, and routines the
/// owner created or that keep their own `auto_archive`.
pub fn bot_origin(s: &Session, crons: &[CronAgent]) -> Option<String> {
    if s.launch_spec.bot.is_some() {
        return None;
    }
    let from = s.launch_spec.from.as_ref().filter(|f| !f.accepted)?;
    match from.kind {
        SenderKind::Bot => Some(from.id.clone()),
        SenderKind::Routine => {
            // Home sets both when it starts a run (`crons::start`).
            s.launch_spec.routine.as_ref().filter(|r| r.id == from.id)?;
            let r = crons.iter().find(|r| r.id == from.id && r.bot_id.is_none() && !r.auto_archive)?;
            let by = r.created_by.as_ref().filter(|b| b.kind == SenderKind::Bot && !b.accepted)?;
            Some(by.id.clone())
        }
        _ => None,
    }
}

/// Pure: the session is Done and settled, with nothing open in it (`sessions::still_done`), its work saved on Home
/// (no `unsaved` mark), Done for at least `SETTLE_MS` and at most `WINDOW_MS`.
pub fn settled(s: &Session, now: i64) -> bool {
    launchpad_core::sessions::still_done(s, s.status_since) && s.unsaved.is_none() && (SETTLE_MS..=WINDOW_MS).contains(&(now - s.status_since))
}

/// Pure: the owner took part in the session (a message of theirs, or an answer to a question): it is theirs to
/// archive. Events of its own messages and answers only.
pub fn owner_took_part(events: &[EventBody]) -> bool {
    events.iter().any(|e| matches!(e, EventBody::UserMessage { from: None, .. } | EventBody::AskAnswered { .. }))
}

/// Pure: why a Local PR of the session still waits for the owner, None when it is done: merged, discarded, or
/// published with GitHub holding its branch's commit on Home (`follow`: GitHub's follow record of its branch).
pub fn local_pr_waits(p: &launchpad_core::predrafts::Predraft, follow: Option<&launchpad_core::home_git::FollowStatus>) -> Option<String> {
    if p.archived || p.merged.is_some() {
        return None;
    }
    if p.published_url.is_none() {
        return Some(format!("Local PR “{}” is not published or merged", p.title));
    }
    match follow {
        Some(f) if !f.behind && f.error.is_none() && f.head.is_some() => None,
        _ => Some(format!("Local PR “{}” has commits that are not on GitHub yet", p.title)),
    }
}

/// Why the session still waits for the owner or for work in progress, None when nothing does. Any record that cannot
/// be read is an error: the caller keeps the session.
pub async fn waits(app: &App, s: &Session) -> Result<Option<String>> {
    let id = s.id.clone();
    let rows: Vec<String> = app
        .state_db
        .call(move |c| {
            let mut q = c.prepare("SELECT json FROM session_event WHERE session_id = ?1 AND kind IN ('user_message', 'ask_answered')")?;
            let rows = q.query_map([id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(rows)
        })
        .await?;
    let events = rows.iter().map(|j| serde_json::from_str::<EventBody>(j)).collect::<Result<Vec<_>, _>>()?;
    if owner_took_part(&events) {
        return Ok(Some("you took part in it".into()));
    }
    if crate::schedules::list(app).await?.iter().any(|x| x.resume_session_id.as_deref() == Some(s.id.as_str())) {
        return Ok(Some("a follow-up or wake-up is scheduled".into()));
    }
    use crate::proposals::ProposalState as P;
    if let Some(p) = crate::proposals::list(app).await?.into_iter().find(|p| p.session_id.as_deref() == Some(s.id.as_str()) && matches!(p.state, P::Proposed | P::Starting | P::Failed | P::Uncertain)) {
        return Ok(Some(format!("its card “{}” waits", p.title)));
    }
    if let Some(why) = launchpad_core::ext::extensions().waiting_on_owner(app, &s.id).await? {
        return Ok(Some(why));
    }
    for p in launchpad_core::predrafts::list(app).await?.into_iter().filter(|p| p.session_id.as_deref() == Some(s.id.as_str())) {
        let follow = match &p.published_url {
            Some(_) if !p.archived && p.merged.is_none() => launchpad_core::home_git::follow_status(app, &p.project_id, &p.branch).await?,
            _ => None,
        };
        if let Some(why) = local_pr_waits(&p, follow.as_ref()) {
            return Ok(Some(why));
        }
    }
    launchpad_core::away::unfinished_work(app, &s.id).await
}

/// The sessions already claimed (archived here, or tried once).
async fn claimed(app: &App) -> Result<HashSet<String>> {
    app.state_db
        .call(|c| {
            let mut q = c.prepare("SELECT session_id FROM task_archive")?;
            let rows = q.query_map([], |r| r.get(0))?.collect::<Result<HashSet<String>, _>>()?;
            Ok(rows)
        })
        .await
}

/// Claims the session's archive once (`outcome` "claimed"); false when it was claimed before.
async fn claim(app: &App, session_id: &str) -> Result<bool> {
    let id = session_id.to_string();
    app.state_db
        .call(move |c| Ok(c.execute("INSERT OR IGNORE INTO task_archive (session_id, at, outcome) VALUES (?1, ?2, 'claimed')", rusqlite::params![id, now_ms()])? == 1))
        .await
}

/// Records how the claimed archive ended; `None` removes the claim (the session changed: nothing happened, so a
/// later pass may look at it again).
async fn settle(app: &App, session_id: &str, outcome: Option<String>) -> Result<()> {
    let id = session_id.to_string();
    app.state_db
        .call(move |c| {
            match outcome {
                Some(o) => c.execute("UPDATE task_archive SET outcome = ?2 WHERE session_id = ?1", rusqlite::params![id, o])?,
                None => c.execute("DELETE FROM task_archive WHERE session_id = ?1", [id])?,
            };
            Ok(())
        })
        .await
}

/// One pass at a time.
static PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Home: a pass now, in the background (a turn ended or its work reached Home).
pub fn soon(app: &App) {
    if app.paths.role != launchpad_core::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = pass(&app).await {
            tracing::warn!("archive of bots' finished tasks: {e:#}");
        }
    });
}

/// Home: archives each bot's task session that finished cleanly (`bot_origin`, `settled`, `waits`). Also run every
/// 30 s, so a merge or a publication that completes later lets the session go.
pub async fn pass(app: &App) -> Result<()> {
    if app.paths.role != launchpad_core::role::Role::Home {
        return Ok(());
    }
    let _one = PASS.lock().await;
    let now = now_ms();
    let candidates: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.status == SessionStatus::Waiting && settled(s, now)).collect();
    if candidates.is_empty() {
        return Ok(());
    }
    let crons = crate::crons::list(app).await?;
    let done = claimed(app).await?;
    for s in candidates {
        if done.contains(&s.id) {
            continue;
        }
        let Some(bot) = bot_origin(&s, &crons) else { continue };
        match waits(app, &s).await {
            Ok(None) => {}
            Ok(Some(_)) => continue,
            Err(e) => {
                tracing::debug!("archive of {}: kept, a check failed: {e:#}", s.id);
                continue;
            }
        }
        // Still the same Done state as the checks saw, then claimed before anything happens.
        if !app.sessions.get(&s.id).is_some_and(|now| now.status_since == s.status_since && settled(&now, now_ms())) || !claim(app, &s.id).await? {
            continue;
        }
        let outcome = match launchpad_core::runners::archive_if_done(app, &s.id, s.status_since).await {
            Ok(true) => {
                tracing::info!("archived {} ({}): a task of bot {bot} that finished cleanly", s.id, s.title);
                Some("archived".to_string())
            }
            Ok(false) => None,
            // Uncertain: never tried again.
            Err(e) => {
                tracing::warn!("archive of {} ({}): {e:#}", s.id, s.title);
                Some(format!("failed: {e:#}"))
            }
        };
        settle(app, &s.id, outcome).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchpad_core::model::{MessageFrom, RoutineLaunch};

    fn from(kind: SenderKind, id: &str) -> MessageFrom {
        MessageFrom { kind, id: id.into(), name: "x".into(), hops: 1, ..Default::default() }
    }

    fn session(from: Option<MessageFrom>) -> Session {
        launchpad_harness::install();
        let mut s: Session = serde_json::from_value(serde_json::json!({
            "id": "s1", "harness": "codex", "cwd": "/tmp/x", "title": "Fix", "status": "waiting", "status_since": 1_000,
            "live": false, "open_question": false, "prompt": "p", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0},
            "started_at": 0, "last_seq": 1, "launch_spec": {}, "seen_at": 0
        }))
        .unwrap();
        s.launch_spec.from = from;
        s
    }

    fn cron(id: &str, created_by: Option<MessageFrom>) -> CronAgent {
        let mut r: CronAgent = serde_json::from_value(serde_json::json!({"id": id, "spec": {}, "title": "t", "schedule": "0 9 * * *", "conditions": [], "paused": false, "created_at": 0, "next_run_at": null, "last_check_at": null, "last_outcome": null, "runs": []})).unwrap();
        r.created_by = created_by;
        r
    }

    #[test]
    fn only_tasks_a_bot_started_on_its_own() {
        // A bot's direct launch, auto-mode card or scheduled launch carries the bot as its sender.
        assert_eq!(bot_origin(&session(Some(from(SenderKind::Bot, "b1"))), &[]).as_deref(), Some("b1"));
        // The owner's launch (or the owner's click on a card or schedule) has none.
        assert_eq!(bot_origin(&session(None), &[]), None);
        // Another agent's launch, a hook, a wake-up: not a bot's task.
        for kind in [SenderKind::Session, SenderKind::Hook, SenderKind::Channel, SenderKind::Wake] {
            assert_eq!(bot_origin(&session(Some(from(kind, "x"))), &[]), None, "{kind:?}");
        }
        // A message the owner accepted on a card is the owner's.
        assert_eq!(bot_origin(&session(Some(MessageFrom { accepted: true, ..from(SenderKind::Bot, "b1") })), &[]), None);
        // A bot's own thread never goes here.
        let mut thread = session(Some(from(SenderKind::Bot, "b1")));
        thread.launch_spec.bot = Some(Default::default());
        assert_eq!(bot_origin(&thread, &[]), None);
    }

    #[test]
    fn routine_runs_count_only_for_routines_a_bot_created() {
        let mut run = session(Some(from(SenderKind::Routine, "r1")));
        run.launch_spec.routine = Some(RoutineLaunch { id: "r1".into(), ..Default::default() });
        assert_eq!(bot_origin(&run, &[cron("r1", Some(from(SenderKind::Bot, "b1")))]).as_deref(), Some("b1"));
        // The owner's routine, an older one without a creator, another agent's, a missing record.
        assert_eq!(bot_origin(&run, &[cron("r1", None)]), None);
        assert_eq!(bot_origin(&run, &[cron("r1", Some(from(SenderKind::Session, "s9")))]), None);
        assert_eq!(bot_origin(&run, &[cron("r2", Some(from(SenderKind::Bot, "b1")))]), None);
        // A routine with its own auto archive keeps it.
        let own = CronAgent { auto_archive: true, ..cron("r1", Some(from(SenderKind::Bot, "b1"))) };
        assert_eq!(bot_origin(&run, &[own]), None);
        // The run's routine must be the sender's.
        run.launch_spec.routine = Some(RoutineLaunch { id: "r2".into(), ..Default::default() });
        assert_eq!(bot_origin(&run, &[cron("r1", Some(from(SenderKind::Bot, "b1")))]), None);
    }

    #[test]
    fn a_request_body_never_names_a_routine_creator() {
        let body = serde_json::json!({"spec": {}, "conditions": [], "created_by": {"kind": "bot", "id": "b1", "name": "x", "hops": 1}});
        let new: crate::crons::NewCron = serde_json::from_value(body).unwrap();
        assert!(new.created_by.is_none());
        // The stored record keeps it.
        let r = cron("r1", Some(from(SenderKind::Bot, "b1")));
        let back: CronAgent = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back.created_by.map(|b| b.id).as_deref(), Some("b1"));
    }

    #[test]
    fn settled_needs_a_clean_saved_done() {
        let s = session(Some(from(SenderKind::Bot, "b1")));
        let now = s.status_since + SETTLE_MS;
        assert!(settled(&s, now));
        // Too fresh, too old.
        assert!(!settled(&s, s.status_since + 1));
        assert!(!settled(&s, s.status_since + WINDOW_MS + 1));
        for status in [SessionStatus::Error, SessionStatus::Interrupted, SessionStatus::Stopped, SessionStatus::Working, SessionStatus::Background, SessionStatus::InTerminal, SessionStatus::Closed, SessionStatus::QuotaPaused] {
            let mut x = s.clone();
            x.status = status;
            assert!(!settled(&x, now), "{status:?}");
        }
        // Continued, resumed or brought back since the checks: a new Done state is not the one they saw.
        assert!(launchpad_core::sessions::still_done(&s, s.status_since));
        assert!(!launchpad_core::sessions::still_done(&s, s.status_since - 1));
        let mut asking = s.clone();
        asking.open_question = true;
        assert!(!settled(&asking, now));
        let mut retry = s.clone();
        retry.capacity_retry_at = Some(now + 1);
        assert!(!settled(&retry, now));
        let mut unsaved = serde_json::to_value(&s).unwrap();
        unsaved["unsaved"] = serde_json::json!({"since": 1, "attempts": 0, "note": null});
        let unsaved: Session = serde_json::from_value(unsaved).unwrap();
        assert!(unsaved.unsaved.is_some());
        assert!(!settled(&unsaved, now));
    }

    #[test]
    fn the_owner_taking_part_keeps_it() {
        let msg = |from: Option<MessageFrom>| EventBody::UserMessage { text: String::new(), client_id: "c".into(), queued: false, images: vec![], from };
        assert!(!owner_took_part(&[msg(Some(from(SenderKind::Bot, "b1"))), msg(Some(from(SenderKind::Wake, "s1")))]));
        assert!(owner_took_part(&[msg(Some(from(SenderKind::Bot, "b1"))), msg(None)]));
        assert!(owner_took_part(&[EventBody::AskAnswered { request_id: "r".into(), answers: vec![] }]));
    }

    #[test]
    fn local_prs_count_once_merged_discarded_or_on_github() {
        let p: launchpad_core::predrafts::Predraft = serde_json::from_value(serde_json::json!({"id": "p", "project_id": "f", "branch": "lp/x", "base_branch": "main", "initial_base": "a", "title": "T", "body": "", "session_id": "s1", "notes": [], "archived": false, "published_url": null, "published_number": null, "default_remote_branch": "x", "remote_branch": null, "pushed_head": null, "created_at": 0})).unwrap();
        assert!(local_pr_waits(&p, None).is_some());
        // Publication claimed but not done (`pushed_head` only) still waits.
        assert!(local_pr_waits(&launchpad_core::predrafts::Predraft { pushed_head: Some("h".into()), ..p.clone() }, None).is_some());
        assert!(local_pr_waits(&launchpad_core::predrafts::Predraft { archived: true, ..p.clone() }, None).is_none());
        assert!(local_pr_waits(&launchpad_core::predrafts::Predraft { merged: Some(launchpad_core::predrafts::LocalMerge { head: "h".into(), at: 1 }), ..p.clone() }, None).is_none());
        let published = launchpad_core::predrafts::Predraft { published_url: Some("https://github.com/o/r/pull/1".into()), ..p.clone() };
        let follow = |behind: bool, error: Option<&str>| launchpad_core::home_git::FollowStatus { url: String::new(), head: Some("h".into()), forwarded: Some("h".into()), behind, error: error.map(str::to_string) };
        assert!(local_pr_waits(&published, Some(&follow(false, None))).is_none());
        assert!(local_pr_waits(&published, Some(&follow(true, None))).is_some());
        assert!(local_pr_waits(&published, Some(&follow(false, Some("refused")))).is_some());
        assert!(local_pr_waits(&published, None).is_some());
    }
}

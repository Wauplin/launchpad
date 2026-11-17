//! The owner's messages to a bot (`POST /api/bots/{id}/message` without an agent): Home stores each one in
//! `bot_send` before it answers, with the thread it goes to, and answers at once with that receipt; a background task
//! per thread (`dispatch`) then sends the thread's messages in order through `bots::message_with`, which starts or
//! continues its session. So the chat shows the message and its thread at once, whatever the start takes, and keeps
//! it (with its error) through a failed start or a reload.
//!
//! States: `waiting` (stored, not claimed: sent after a Home restart too), `sending` (claimed before the send, never
//! sent again on its own), `sent` (with its session), `failed` (refused, with why; Retry sends it again with the same
//! ID) and `uncertain` (Home stopped during its send: Retry or Remove after checking the thread). The receipt's ID is the
//! window's Idempotency-Key: a new session's ID, else the message's `client_id` in the session's events, so the chat
//! matches the receipt to its message and draws one bubble; a retry with the same ID never starts a second session
//! (a session with that ID counts as sent) and the runner's own retry protection never delivers a message twice.
//! The owner's messages bypass the bot's thread cap (`max_threads`), as before; a message waits while its thread's
//! session starts, or works on a harness that takes no message during a turn, instead of failing.

use std::collections::HashSet;
use std::sync::Mutex;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::bot_threads::{self, To};
use launchpad_core::db::Db;
use launchpad_core::model::{FileAttachment, MessageContext, Session, SessionStatus};
use launchpad_core::util::now_ms;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "BotSendState")]
pub enum State {
    Waiting,
    Sending,
    Sent,
    Failed,
    Uncertain,
}

impl State {
    fn parse(s: &str) -> State {
        match s {
            "waiting" => State::Waiting,
            "sending" => State::Sending,
            "sent" => State::Sent,
            "failed" => State::Failed,
            _ => State::Uncertain,
        }
    }
}

/// An owner's message Home accepted for a bot (its receipt), as the chat shows it.
#[derive(Clone, Debug, Serialize, TS)]
pub struct BotSend {
    /// The window's Idempotency-Key: the new session's ID, or the message's `client_id` in its session.
    pub id: String,
    pub bot_id: String,
    /// The thread it goes to (a thread that could not go on: the new thread that continues it, once sent).
    pub thread: String,
    /// The thread the owner sent it to.
    pub asked_thread: String,
    pub text: String,
    pub attachments: Vec<FileAttachment>,
    pub state: State,
    pub error: Option<String>,
    /// The session it went to, once sent.
    pub session_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// What `bot_send.json` holds: the message as the owner sent it.
#[derive(Serialize, Deserialize, Default)]
struct Body {
    text: String,
    #[serde(default)]
    attachments: Vec<FileAttachment>,
    #[serde(default)]
    context: MessageContext,
}

/// How long the chat lists a sent message: until its session's events show it (they carry its ID).
const SENT_LISTED_MS: i64 = 30 * 60 * 1000;
/// Sent messages are deleted after a week, failed and uncertain ones the owner left after 30 days.
const SENT_KEPT_MS: i64 = 7 * 24 * 3600 * 1000;
const FAILED_KEPT_MS: i64 = 30 * 24 * 3600 * 1000;
/// A message waits at most this long for its thread's session to finish starting, then is sent anyway.
const START_WAIT_MS: i64 = 30 * 60 * 1000;

type Row = (String, String, String, String, String, Option<String>, Option<String>, String, i64, i64);
const COLUMNS: &str = "id, bot_id, thread, asked_thread, state, error, session_id, json, created_at, updated_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?))
}

fn receipt((id, bot_id, thread, asked_thread, state, error, session_id, json, created_at, updated_at): Row) -> BotSend {
    let body: Body = serde_json::from_str(&json).unwrap_or_default();
    BotSend { id, bot_id, thread, asked_thread, text: body.text, attachments: body.attachments, state: State::parse(&state), error, session_id, created_at, updated_at }
}

fn publish(app: &App, bot: &str) {
    app.bus.publish("bots", "sends", serde_json::json!({"id": bot}));
    CHANGED.notify_waiters();
}

/// Wakes the windows that wait for a send's outcome (`wait`).
static CHANGED: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(Default::default);

// ------------------------------------------------------------------ records (state.db only, tested)

/// Stores a new message, or returns the one stored with this ID (a retry of the same request).
async fn insert(db: &Db, id: &str, bot: &str, thread: &str, body: &Body) -> Result<BotSend> {
    let (id, bot, thread, json, now) = (id.to_string(), bot.to_string(), thread.to_string(), serde_json::to_string(body)?, now_ms());
    db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO bot_send (id, bot_id, thread, asked_thread, state, json, created_at, updated_at) VALUES (?1, ?2, ?3, ?3, 'waiting', ?4, ?5, ?5)",
            rusqlite::params![id, bot, thread, json, now],
        )?;
        let r = tx.query_row(&format!("SELECT {COLUMNS} FROM bot_send WHERE id = ?1"), [&id], row)?;
        tx.commit()?;
        Ok(r)
    })
    .await
    .map(receipt)
}

async fn get(db: &Db, id: &str) -> Result<Option<BotSend>> {
    let id = id.to_string();
    Ok(db.call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM bot_send WHERE id = ?1"), [id], row).optional()?)).await?.map(receipt))
}

/// The oldest message of a thread that waits.
async fn next(db: &Db, bot: &str, thread: &str) -> Result<Option<(BotSend, MessageContext)>> {
    let (bot, thread) = (bot.to_string(), thread.to_string());
    let r = db
        .call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM bot_send WHERE bot_id = ?1 AND thread = ?2 AND state = 'waiting' ORDER BY created_at, rowid LIMIT 1"), [bot, thread], row).optional()?))
        .await?;
    Ok(r.map(|r| {
        let context = serde_json::from_str::<Body>(&r.7).map(|b| b.context).unwrap_or_default();
        (receipt(r), context)
    }))
}

/// Claims a waiting message before its send: true for the one caller that may send it.
async fn claim(db: &Db, id: &str) -> Result<bool> {
    let id = id.to_string();
    db.call(move |c| Ok(c.execute("UPDATE bot_send SET state = 'sending', error = NULL, updated_at = ?2 WHERE id = ?1 AND state = 'waiting'", rusqlite::params![id, now_ms()])? == 1)).await
}

/// The outcome of a claimed send. A message that continued in a new thread (`thread`) takes the thread's later waiting
/// messages with it, so they never bring the old thread back.
async fn finish(db: &Db, id: &str, outcome: std::result::Result<(String, String), String>) -> Result<()> {
    let id = id.to_string();
    db.call(move |c| {
        let tx = c.transaction()?;
        let now = now_ms();
        match outcome {
            Ok((session, thread)) => {
                let (bot, old): (String, String) = tx.query_row("SELECT bot_id, thread FROM bot_send WHERE id = ?1", [&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                tx.execute("UPDATE bot_send SET state = 'sent', error = NULL, session_id = ?2, thread = ?3, updated_at = ?4 WHERE id = ?1", rusqlite::params![id, session, thread, now])?;
                if old != thread {
                    tx.execute("UPDATE bot_send SET thread = ?3, updated_at = ?4 WHERE bot_id = ?1 AND thread = ?2 AND state = 'waiting'", rusqlite::params![bot, old, thread, now])?;
                }
            }
            Err(error) => {
                tx.execute("UPDATE bot_send SET state = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1", rusqlite::params![id, error, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

/// Home start: a send claimed when Home stopped is sent when its session exists (a launch keeps the message's ID),
/// else uncertain, never sent again on its own. `exists` names the session of a send ID and its thread.
async fn after_restart(db: &Db, exists: impl Fn(&str) -> Option<(String, String)>) -> Result<Vec<String>> {
    let claimed: Vec<String> = db
        .call(|c| {
            let mut st = c.prepare("SELECT id FROM bot_send WHERE state = 'sending'")?;
            let ids = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(ids)
        })
        .await?;
    let mut bots = vec![];
    for id in claimed {
        match exists(&id) {
            Some(found) => finish(db, &id, Ok(found)).await?,
            None => {
                let i = id.clone();
                db.call(move |c| Ok(c.execute("UPDATE bot_send SET state = 'uncertain', error = ?2, updated_at = ?3 WHERE id = ?1 AND state = 'sending'", rusqlite::params![i, UNCERTAIN, now_ms()])?)).await?;
            }
        }
        if let Some(s) = get(db, &id).await? {
            bots.push(s.bot_id);
        }
    }
    Ok(bots)
}

const UNCERTAIN: &str = "Launchpad stopped while it sent this message: check the thread. Retry sends it again (a message the bot already received is not delivered twice); Remove drops it.";

/// The threads with waiting messages.
async fn waiting_threads(db: &Db) -> Result<Vec<(String, String)>> {
    db.call(|c| {
        let mut st = c.prepare("SELECT DISTINCT bot_id, thread FROM bot_send WHERE state = 'waiting'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

/// The messages a bot's chat shows: those not sent yet (waiting, sending, failed, uncertain) and those sent in the
/// last half hour (until their session's events show them), oldest first.
async fn listed(db: &Db, bot: &str, now: i64) -> Result<Vec<BotSend>> {
    let bot = bot.to_string();
    let rows = db
        .call(move |c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM bot_send WHERE bot_id = ?1 AND (state != 'sent' OR updated_at > ?2) ORDER BY created_at, rowid"))?;
            let rows = st.query_map(rusqlite::params![bot, now - SENT_LISTED_MS], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(rows.into_iter().map(receipt).collect())
}

/// Retry: a failed or uncertain message waits again (same ID).
async fn requeue(db: &Db, bot: &str, id: &str) -> Result<BotSend> {
    let (b, i) = (bot.to_string(), id.to_string());
    let changed = db.call(move |c| Ok(c.execute("UPDATE bot_send SET state = 'waiting', error = NULL, updated_at = ?3 WHERE id = ?1 AND bot_id = ?2 AND state IN ('failed', 'uncertain')", rusqlite::params![i, b, now_ms()])?)).await?;
    ensure!(changed == 1, "this message is not waiting for a retry: it was sent or removed");
    get(db, id).await?.context("this message was removed")
}

/// Remove: a message not sent (waiting, failed or uncertain) is deleted and never sent.
async fn delete(db: &Db, bot: &str, id: &str) -> Result<()> {
    let (b, i) = (bot.to_string(), id.to_string());
    let deleted = db.call(move |c| Ok(c.execute("DELETE FROM bot_send WHERE id = ?1 AND bot_id = ?2 AND state IN ('waiting', 'failed', 'uncertain')", rusqlite::params![i, b])?)).await?;
    ensure!(deleted == 1, "this message is being sent or was sent: it cannot be removed");
    Ok(())
}

async fn prune(db: &Db, now: i64) -> Result<usize> {
    db.call(move |c| Ok(c.execute("DELETE FROM bot_send WHERE (state = 'sent' AND updated_at < ?1) OR (state IN ('failed', 'uncertain') AND updated_at < ?2)", rusqlite::params![now - SENT_KEPT_MS, now - FAILED_KEPT_MS])?)).await
}

// ------------------------------------------------------------------ Home

/// Accepts the owner's message to a bot: checks what can fail at once (an empty message, the bot, its bot.toml and
/// profile, the thread), stores the message with its thread and returns its receipt; the send runs in the background.
/// `key`: the window's Idempotency-Key (its retry gets the same receipt).
pub async fn accept(app: &App, bot_id: &str, to: To, text: String, mut attachments: Vec<FileAttachment>, context: MessageContext, key: Option<String>) -> Result<BotSend> {
    ensure!(app.paths.role == launchpad_core::role::Role::Home, "only Home accepts bot messages");
    ensure!(!text.trim().is_empty() || !attachments.is_empty() || !context.is_empty(), "the message is empty");
    let id = key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Some(found) = get(&app.state_db, &id).await? {
        ensure!(found.bot_id == bot_id, "this message ID was already used for another bot");
        return Ok(found);
    }
    let bot = launchpad_core::bots::get(app, bot_id).await?;
    if let Some(e) = &bot.settings_error {
        bail!("bot.toml cannot be used ({e}); fix it on the bot's page (⋯ → Edit as file)");
    }
    launchpad_core::bots::launch_profile(app, &app.config.get(), &bot.settings).context("choose a profile for this bot on its page (About → Access)")?;
    // Uploads are stored on Home now: the record holds references, never bytes.
    launchpad_core::attachments::to_refs(&id, &mut attachments).await?;
    let thread = bot_threads::resolve(app, bot_id, &to, text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default()).await?;
    let sent = insert(&app.state_db, &id, bot_id, &thread, &Body { text, attachments, context }).await?;
    publish(app, bot_id);
    kick(app, bot_id, &sent.thread);
    Ok(sent)
}

/// Waits for the outcome of an accepted message (callers that want the session, like the extension): its session,
/// or its error.
pub async fn wait(app: &App, id: &str) -> Result<Session> {
    loop {
        let changed = CHANGED.notified();
        let s = get(&app.state_db, id).await?.context("this message was removed")?;
        match s.state {
            State::Sent => return app.sessions.get(s.session_id.as_deref().unwrap_or_default()).context("the bot's session disappeared"),
            State::Failed | State::Uncertain => bail!("{}", s.error.unwrap_or_else(|| "the message was not sent".into())),
            State::Waiting | State::Sending => {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), changed).await;
            }
        }
    }
}

pub async fn list(app: &App, bot_id: &str) -> Result<Vec<BotSend>> {
    listed(&app.state_db, bot_id, now_ms()).await
}

pub async fn retry(app: &App, bot_id: &str, id: &str) -> Result<BotSend> {
    let s = requeue(&app.state_db, bot_id, id).await?;
    publish(app, bot_id);
    kick(app, bot_id, &s.thread);
    Ok(s)
}

pub async fn remove(app: &App, bot_id: &str, id: &str) -> Result<()> {
    delete(&app.state_db, bot_id, id).await?;
    publish(app, bot_id);
    Ok(())
}

/// The threads whose task sends now (one task per thread, so its messages go in order).
static ACTIVE: std::sync::LazyLock<Mutex<HashSet<(String, String)>>> = std::sync::LazyLock::new(Default::default);

/// Starts the task that sends a thread's waiting messages, unless it runs.
fn kick(app: &App, bot: &str, thread: &str) {
    let key = (bot.to_string(), thread.to_string());
    if !ACTIVE.lock().unwrap().insert(key.clone()) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = dispatch(&app, &key.0, &key.1).await {
                tracing::warn!("bot messages of {}/{}: {e:#}", key.0, key.1);
            }
            ACTIVE.lock().unwrap().remove(&key);
            // A message stored after the last read, while the task was ending, goes too.
            let more = matches!(next(&app.state_db, &key.0, &key.1).await, Ok(Some(_)));
            if !more || !ACTIVE.lock().unwrap().insert(key.clone()) {
                break;
            }
        }
    });
}

/// Pure: why a message waits before it goes to its thread's open session (None: send now). `takes_messages`: the
/// harness takes a message during a turn.
pub fn waits_for(status: SessionStatus, takes_messages: bool) -> Option<&'static str> {
    match status {
        SessionStatus::Starting => Some("the bot starts"),
        s if s.is_running() && !takes_messages => Some("the bot's turn ends"),
        _ => None,
    }
}

/// Sends a thread's waiting messages, oldest first.
async fn dispatch(app: &App, bot: &str, thread: &str) -> Result<()> {
    let mut waited_since: Option<i64> = None;
    while let Some((s, context)) = next(&app.state_db, bot, thread).await? {
        // A session that starts refuses messages until its harness is up; a harness without messages during a turn
        // takes the next one at the turn's end.
        if let Some(cur) = bot_threads::open_session(app, bot, thread) {
            let takes = launchpad_core::harness::registration(cur.harness).descriptor.capabilities.messages_while_running;
            let since = *waited_since.get_or_insert_with(now_ms);
            if waits_for(cur.status, takes).is_some() && (cur.status != SessionStatus::Starting || now_ms() - since < START_WAIT_MS) {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        }
        waited_since = None;
        if !claim(&app.state_db, &s.id).await? {
            continue;
        }
        publish(app, bot);
        // A retry of a launch that happened: its session has the message's ID.
        let sent = match app.sessions.get(&s.id) {
            Some(found) => Ok(found),
            None => {
                let m = launchpad_core::bots::Message { text: s.text.clone(), attachments: s.attachments.clone(), context, automatic: false, key: Some(s.id.clone()), to: To::Thread(thread.to_string()) };
                launchpad_core::bots::message_with(app, bot, m).await
            }
        };
        let outcome = match &sent {
            Ok(session) => Ok((session.id.clone(), bot_threads::of(session).unwrap_or(thread).to_string())),
            Err(e) => Err(format!("{e:#}")),
        };
        if let Err(e) = &outcome {
            tracing::warn!("bot message {} to {bot}/{thread}: {e}", s.id);
        }
        let moved = outcome.as_ref().ok().map(|(_, t)| t.clone()).filter(|t| t != thread);
        finish(&app.state_db, &s.id, outcome).await?;
        publish(app, bot);
        // It continued in a new thread: the later messages went with it.
        if let Some(t) = moved {
            kick(app, bot, &t);
            return Ok(());
        }
    }
    Ok(())
}

/// Home: after a restart, claimed sends are settled and waiting ones sent; then every 30 s a sweep starts the send of
/// threads with waiting messages (in case a task ended early) and prunes old records.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        match after_restart(&app.state_db, |id| app.sessions.get(id).map(|s| (s.id.clone(), bot_threads::of(&s).unwrap_or(bot_threads::LEGACY).to_string()))).await {
            Ok(bots) => bots.iter().for_each(|b| publish(&app, b)),
            Err(e) => tracing::warn!("bot messages after a restart: {e:#}"),
        }
        loop {
            match waiting_threads(&app.state_db).await {
                Ok(threads) => threads.iter().for_each(|(b, t)| kick(&app, b, t)),
                Err(e) => tracing::warn!("bot messages: {e:#}"),
            }
            if let Err(e) = prune(&app.state_db, now_ms()).await {
                tracing::warn!("bot messages: {e:#}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        launchpad_core::db::open_state(&std::env::temp_dir().join(format!("launchpad-sends-{}.db", uuid::Uuid::new_v4()))).unwrap()
    }

    fn body(text: &str) -> Body {
        Body { text: text.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn a_message_is_stored_once_and_sent_in_order_once() {
        let db = db();
        let a = insert(&db, "a", "b1", "t1", &body("first")).await.unwrap();
        assert_eq!((a.state, a.thread.as_str(), a.asked_thread.as_str(), a.text.as_str()), (State::Waiting, "t1", "t1", "first"));
        // The same request again (a retry after a lost answer) gets the same receipt, never a second message.
        let again = insert(&db, "a", "b1", "t1", &body("changed")).await.unwrap();
        assert_eq!((again.text.as_str(), again.created_at), ("first", a.created_at));
        insert(&db, "b", "b1", "t1", &body("second")).await.unwrap();
        insert(&db, "c", "b1", "t2", &body("other thread")).await.unwrap();
        assert_eq!(next(&db, "b1", "t1").await.unwrap().unwrap().0.id, "a");
        assert!(claim(&db, "a").await.unwrap());
        assert!(!claim(&db, "a").await.unwrap(), "claimed once");
        assert_eq!(next(&db, "b1", "t1").await.unwrap().unwrap().0.id, "b");
        // A claimed message is neither removed nor retried.
        assert!(delete(&db, "b1", "a").await.is_err());
        assert!(requeue(&db, "b1", "a").await.is_err());
        let mut threads = waiting_threads(&db).await.unwrap();
        threads.sort();
        assert_eq!(threads, vec![("b1".into(), "t1".into()), ("b1".into(), "t2".into())]);
    }

    #[tokio::test]
    async fn a_message_that_continues_in_a_new_thread_takes_the_later_ones() {
        let db = db();
        for id in ["a", "b"] {
            insert(&db, id, "b1", "old", &body(id)).await.unwrap();
        }
        claim(&db, "a").await.unwrap();
        finish(&db, "a", Ok(("s1".into(), "new".into()))).await.unwrap();
        let a = get(&db, "a").await.unwrap().unwrap();
        assert_eq!((a.state, a.thread.as_str(), a.asked_thread.as_str(), a.session_id.as_deref()), (State::Sent, "new", "old", Some("s1")));
        assert!(next(&db, "b1", "old").await.unwrap().is_none());
        assert_eq!(next(&db, "b1", "new").await.unwrap().unwrap().0.id, "b");
    }

    #[tokio::test]
    async fn failures_are_kept_until_retried_or_removed() {
        let db = db();
        insert(&db, "a", "b1", "t", &body("x")).await.unwrap();
        claim(&db, "a").await.unwrap();
        finish(&db, "a", Err("no runner".into())).await.unwrap();
        let listed_now = listed(&db, "b1", now_ms()).await.unwrap();
        assert_eq!((listed_now[0].state, listed_now[0].error.as_deref()), (State::Failed, Some("no runner")));
        // Still listed long after (a reload shows it with its error).
        assert_eq!(listed(&db, "b1", now_ms() + 2 * SENT_LISTED_MS).await.unwrap().len(), 1);
        assert!(requeue(&db, "b2", "a").await.is_err(), "another bot's message");
        assert_eq!(requeue(&db, "b1", "a").await.unwrap().state, State::Waiting);
        claim(&db, "a").await.unwrap();
        finish(&db, "a", Ok(("s".into(), "t".into()))).await.unwrap();
        // A sent message is listed until its session shows it, then pruned.
        assert_eq!(listed(&db, "b1", now_ms()).await.unwrap().len(), 1);
        assert!(listed(&db, "b1", now_ms() + 2 * SENT_LISTED_MS).await.unwrap().is_empty());
        assert_eq!(prune(&db, now_ms() + SENT_KEPT_MS + 1).await.unwrap(), 1);
        insert(&db, "w", "b1", "t", &body("y")).await.unwrap();
        delete(&db, "b1", "w").await.unwrap();
        assert!(get(&db, "w").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_restart_settles_claimed_sends_and_keeps_waiting_ones() {
        let db = db();
        for id in ["launched", "lost", "waiting"] {
            insert(&db, id, "b1", "t", &body(id)).await.unwrap();
        }
        claim(&db, "launched").await.unwrap();
        claim(&db, "lost").await.unwrap();
        let bots = after_restart(&db, |id| (id == "launched").then(|| (id.to_string(), "t2".to_string()))).await.unwrap();
        assert_eq!(bots.len(), 2);
        let launched = get(&db, "launched").await.unwrap().unwrap();
        assert_eq!((launched.state, launched.session_id.as_deref(), launched.thread.as_str()), (State::Sent, Some("launched"), "t2"));
        assert_eq!(get(&db, "lost").await.unwrap().unwrap().state, State::Uncertain);
        // The waiting message moved with the launched one's new thread and is sent from there.
        assert_eq!(next(&db, "b1", "t2").await.unwrap().unwrap().0.id, "waiting");
    }

    #[test]
    fn a_message_waits_for_the_start_and_for_harnesses_without_messages_during_a_turn() {
        assert!(waits_for(SessionStatus::Starting, true).is_some());
        assert!(waits_for(SessionStatus::Working, false).is_some());
        assert!(waits_for(SessionStatus::Working, true).is_none());
        assert!(waits_for(SessionStatus::Waiting, false).is_none());
        assert!(waits_for(SessionStatus::Error, false).is_none());
    }
}

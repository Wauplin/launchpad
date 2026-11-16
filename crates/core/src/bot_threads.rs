//! A bot's threads: its parallel conversations. The bot (BOT.md, bot.toml, memory, routines, access) is shared;
//! a thread is one session (`BotLaunch.thread`). Home keeps a row per thread (`bot_thread`: its routing key and a
//! readable title). There is no default thread: the owner's message without a thread starts a new one. A thread
//! whose session is archived counts as archived. When a thread's session cannot go on (archived, or the bot's
//! profile, folder, model, effort or permission mode changed), its next message continues in a new thread with the
//! same key and title that references it (`bots::message_with`).
//!
//! `route` (pure) decides where a message goes from its source: the owner's selected thread, a GitHub item's thread,
//! a channel's, a hook's, another agent's (one per sender session), Telegram's, or a new thread (each routine run,
//! the owner's New thread or message without a thread, the extension's New conversation). `resolve` turns that into a
//! thread ID, creating its row.

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use ts_rs::TS;

use crate::model::{GitHubRef, Session, SessionStatus};
use crate::{App, util::now_ms};

/// The thread of sessions from before bots had several threads (`BotLaunch::thread_id`).
pub const LEGACY: &str = "main";
/// Its title.
const LEGACY_TITLE: &str = "Main";
/// `max_threads` of bot.toml when it names none: threads of one bot that may work at the same time.
pub const DEFAULT_MAX_THREADS: u32 = 16;

#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct BotThread {
    pub id: String,
    pub bot_id: String,
    /// What routes messages here (`github:owner/repo#12`, `channel:<id>`, `hook:<id>`, `agent:<session>`,
    /// `routine:<file>`), None for the owner's own threads.
    pub key: Option<String>,
    pub title: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub archived_at: Option<i64>,
}

/// Where a message to a bot comes from.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    /// The owner, in the bot page or the Ask panel: the thread shown (None: a new thread).
    Owner { thread: Option<&'a str> },
    /// The owner's Telegram chat: one thread per bot.
    Telegram,
    /// The owner's New thread, the extension's New conversation.
    NewThread,
    /// The extension, about GitHub items: one item has its own thread; several start a new one.
    Items { refs: &'a [GitHubRef] },
    /// A bot channel or DM (its delivery or digest).
    Channel { id: &'a str, title: &'a str },
    /// A run of a bot routine: a fresh thread per run.
    Routine { file: &'a str, title: &'a str },
    /// An incoming hook.
    Hook { id: &'a str, name: &'a str },
    /// Another agent's message: one thread per sender session.
    Agent { session: &'a str, name: &'a str },
}

/// The thread a message goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum To {
    /// An existing thread.
    Thread(String),
    /// The open thread with this key, else a new one with this title.
    Key { key: String, title: String },
    /// Always a new thread. An empty title: from the message's first line.
    New { key: Option<String>, title: String },
}

/// Pure: the thread a message from `source` goes to.
pub fn route(source: Source) -> To {
    match source {
        Source::Owner { thread } => match thread.filter(|t| !t.is_empty()) {
            Some(t) => To::Thread(t.to_string()),
            None => To::New { key: None, title: String::new() },
        },
        Source::Telegram => To::Key { key: "telegram".into(), title: "Telegram".into() },
        Source::NewThread => To::New { key: None, title: String::new() },
        Source::Items { refs } => {
            let items: Vec<&GitHubRef> = refs.iter().filter(|r| r.number.is_some() && !r.repo.is_empty()).collect();
            let mut keys: Vec<String> = items.iter().map(|r| item_key(&r.repo, r.number.unwrap_or_default())).collect();
            keys.dedup();
            match (keys.as_slice(), items.first()) {
                ([key], Some(r)) => To::Key { key: key.clone(), title: item_title(r) },
                _ => To::New { key: None, title: String::new() },
            }
        }
        Source::Channel { id, title } => To::Key { key: format!("channel:{id}"), title: title.to_string() },
        Source::Routine { file, title } => To::New { key: Some(format!("routine:{file}")), title: title.to_string() },
        Source::Hook { id, name } => To::Key { key: format!("hook:{id}"), title: format!("Hook “{name}”") },
        Source::Agent { session, name } => To::Key { key: format!("agent:{session}"), title: if name.is_empty() { "An agent".into() } else { name.to_string() } },
    }
}

/// The key of a GitHub item's thread.
pub fn item_key(repo: &str, number: u64) -> String {
    format!("github:{}#{number}", repo.to_lowercase())
}

fn item_title(r: &GitHubRef) -> String {
    let n = r.number.unwrap_or_default();
    let what = match r.kind.as_str() {
        "pr" | "PullRequest" => "PR ",
        "issue" | "Issue" => "Issue ",
        _ => "",
    };
    match r.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => short(&format!("{what}#{n} · {t}")),
        None => format!("{what}{}#{n}", r.repo),
    }
}

/// A title of at most 80 characters, on one line.
pub fn short(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 80 { line } else { format!("{}…", line.chars().take(79).collect::<String>().trim_end()) }
}

/// One lock per bot: two messages never both create a thread for the same key.
static THREADS: std::sync::LazyLock<crate::locks::Locks> = std::sync::LazyLock::new(Default::default);

fn row(r: &rusqlite::Row) -> rusqlite::Result<BotThread> {
    Ok(BotThread { bot_id: r.get(0)?, id: r.get(1)?, key: r.get(2)?, title: r.get(3)?, created_at: r.get(4)?, archived_at: r.get(5)? })
}

const COLUMNS: &str = "bot_id, id, key, title, created_at, archived_at";

/// The row of the legacy thread exists when sessions from before threads name it (older Homes created it on use).
async fn ensure_legacy(app: &App, bot: &str) -> Result<()> {
    if !crate::bots::all_sessions(app, bot).iter().any(|s| of(s) == Some(LEGACY)) {
        return Ok(());
    }
    let bot = bot.to_string();
    app.state_db.call(move |c| Ok(c.execute("INSERT OR IGNORE INTO bot_thread (bot_id, id, key, title, created_at) VALUES (?1, ?2, NULL, ?3, ?4)", params![bot, LEGACY, LEGACY_TITLE, now_ms()])?)).await?;
    Ok(())
}

/// A bot's threads, newest first; archived ones (the row, or its session) only with `archived`. A thread whose
/// session is archived shows with that time as `archived_at`.
pub async fn list(app: &App, bot: &str, archived: bool) -> Result<Vec<BotThread>> {
    ensure_legacy(app, bot).await?;
    let b = bot.to_string();
    let rows = app.state_db.call(move |c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM bot_thread WHERE bot_id = ?1 AND (?2 OR archived_at IS NULL) ORDER BY created_at DESC"))?;
        let rows = st.query_map(params![b, archived], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    Ok(rows.into_iter().filter_map(|mut t| {
        if t.archived_at.is_none() && let Some(s) = session(app, bot, &t.id).filter(|s| s.status == SessionStatus::Closed) {
            if !archived {
                return None;
            }
            t.archived_at = Some(s.status_since);
        }
        Some(t)
    }).collect())
}

pub async fn get(app: &App, bot: &str, id: &str) -> Result<Option<BotThread>> {
    if id == LEGACY {
        ensure_legacy(app, bot).await?;
    }
    let (bot, id) = (bot.to_string(), id.to_string());
    app.state_db.call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM bot_thread WHERE bot_id = ?1 AND id = ?2"), params![bot, id], row).optional()?)).await
}

/// The titles of every thread, by bot and thread ID (Today's titles).
pub async fn titles(app: &App) -> Result<std::collections::HashMap<(String, String), String>> {
    app.state_db.call(|c| {
        let mut st = c.prepare("SELECT bot_id, id, title FROM bot_thread")?;
        let rows = st.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
        Ok(rows)
    }).await
}

/// The thread ID a message goes to: an existing thread (an archived one comes back), the open thread with the key,
/// or a new thread. `fallback` names a new thread without a title (the message's first line). Whether the thread's
/// session can take the message is `bots::message_with`'s decision (else it continues in a new thread, `follow_up`).
pub async fn resolve(app: &App, bot: &str, to: &To, fallback: &str) -> Result<String> {
    let lock = THREADS.get(bot);
    let _held = lock.lock().await;
    match to {
        To::Thread(id) => {
            let t = get(app, bot, id).await?.context("this thread no longer exists")?;
            if t.archived_at.is_some() {
                set_archived(app, bot, id, false).await?;
            }
            Ok(t.id)
        }
        To::Key { key, title } => {
            let (b, k) = (bot.to_string(), key.clone());
            let found: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT id FROM bot_thread WHERE bot_id = ?1 AND key = ?2 AND archived_at IS NULL ORDER BY created_at DESC LIMIT 1", params![b, k], |r| r.get(0)).optional()?)).await?;
            match found {
                Some(id) => Ok(id),
                None => insert(app, bot, Some(key.clone()), if title.is_empty() { fallback } else { title }).await,
            }
        }
        To::New { key, title } => insert(app, bot, key.clone(), if title.is_empty() { fallback } else { title }).await,
    }
}

/// The existing thread a message to `to` would go to, without creating one (None: a new thread).
pub async fn peek(app: &App, bot: &str, to: &To) -> Result<Option<String>> {
    match to {
        To::Thread(id) => Ok(Some(id.clone())),
        To::Key { key, .. } => {
            let (b, k) = (bot.to_string(), key.clone());
            app.state_db.call(move |c| Ok(c.query_row("SELECT id FROM bot_thread WHERE bot_id = ?1 AND key = ?2 AND archived_at IS NULL ORDER BY created_at DESC LIMIT 1", params![b, k], |r| r.get(0)).optional()?)).await
        }
        To::New { .. } => Ok(None),
    }
}

/// A new thread that continues `thread` (its session cannot go on): the same key and title, and `thread` leaves the
/// thread bar (archived). Returns the new thread's ID.
pub async fn follow_up(app: &App, bot: &str, thread: &str) -> Result<String> {
    let lock = THREADS.get(bot);
    let _held = lock.lock().await;
    let t = get(app, bot, thread).await?.context("this thread no longer exists")?;
    let id = insert(app, bot, t.key.clone(), &t.title).await?;
    // Its key now routes to the new thread.
    set_archived(app, bot, thread, true).await?;
    Ok(id)
}

async fn insert(app: &App, bot: &str, key: Option<String>, title: &str) -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let title = match short(title) {
        t if t.is_empty() => "New thread".to_string(),
        t => t,
    };
    let (b, i) = (bot.to_string(), id.clone());
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO bot_thread (bot_id, id, key, title, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![b, i, key, title, now_ms()])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    Ok(id)
}

/// Rename: a new title (one line, at most 80 characters).
pub async fn rename(app: &App, bot: &str, id: &str, title: &str) -> Result<BotThread> {
    let title = short(title);
    ensure!(!title.is_empty(), "give the thread a title");
    get(app, bot, id).await?.context("this thread no longer exists")?;
    let (b, i, t) = (bot.to_string(), id.to_string(), title.clone());
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET title = ?3 WHERE bot_id = ?1 AND id = ?2", params![b, i, t])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    get(app, bot, id).await?.context("this thread no longer exists")
}

async fn set_archived(app: &App, bot: &str, id: &str, archived: bool) -> Result<()> {
    let (b, i) = (bot.to_string(), id.to_string());
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET archived_at = CASE WHEN ?3 THEN ?4 ELSE NULL END WHERE bot_id = ?1 AND id = ?2", params![b, i, archived, now_ms()])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    Ok(())
}

/// Archive (or bring back) a thread: it leaves the thread bar, and its open session is archived (brought back with
/// it). A thread at work is stopped first.
pub async fn archive(app: &App, bot: &str, id: &str, archived: bool) -> Result<BotThread> {
    get(app, bot, id).await?.context("this thread no longer exists")?;
    if archived {
        let open = sessions(app, bot, id).into_iter().filter(|s| s.status != SessionStatus::Closed).collect::<Vec<_>>();
        if open.iter().any(|s| s.status.is_running()) {
            bail!("this thread is working: stop it first");
        }
        for s in open {
            crate::runners::session_action(app, &s.id, "close").await?;
        }
    } else if let Some(s) = session(app, bot, id).filter(|s| s.status == SessionStatus::Closed) {
        crate::runners::session_action(app, &s.id, "unarchive").await?;
    }
    set_archived(app, bot, id, archived).await?;
    get(app, bot, id).await?.context("this thread no longer exists")
}

/// Deletes a bot's thread rows (the bot is deleted).
pub(crate) fn delete_all(tx: &rusqlite::Transaction, bot: &str) -> rusqlite::Result<usize> {
    tx.execute("DELETE FROM bot_thread WHERE bot_id = ?1", [bot])
}

/// The thread of a bot session.
pub fn of(s: &Session) -> Option<&str> {
    s.launch_spec.bot.as_ref().map(|b| b.thread_id())
}

/// The sessions of one thread of a bot, oldest first: one, except in threads from before a thread was one session
/// (`session` takes the newest).
pub fn sessions(app: &App, bot: &str, thread: &str) -> Vec<Session> {
    let mut list: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.launch_spec.bot.as_ref().is_some_and(|b| b.id == bot && b.thread_id() == thread)).collect();
    list.sort_by_key(|s| s.started_at);
    list
}

/// The session of a thread: its newest (older threads may hold several; only the newest counts).
pub fn session(app: &App, bot: &str, thread: &str) -> Option<Session> {
    sessions(app, bot, thread).pop()
}

/// The session of a thread while it is not archived.
pub fn open_session(app: &App, bot: &str, thread: &str) -> Option<Session> {
    session(app, bot, thread).filter(|s| s.status != SessionStatus::Closed)
}

/// The open session of each thread of a bot (each thread's newest session, when not archived).
pub fn open_sessions(app: &App, bot: &str) -> Vec<Session> {
    let mut newest: std::collections::HashMap<String, Session> = Default::default();
    for s in crate::bots::all_sessions(app, bot) {
        let t = of(&s).unwrap_or(LEGACY).to_string();
        // Oldest first: the newest wins.
        newest.insert(t, s);
    }
    newest.into_values().filter(|s| s.status != SessionStatus::Closed).collect()
}

/// Pure: why a message that would wake `thread` waits for a free slot, or None. `working`: the threads whose open
/// session works now. A thread that already works is not a new slot (its message waits for its turn instead).
pub fn cap_reason(working: &[String], thread: Option<&str>, max: u32, name: &str) -> Option<String> {
    if thread.is_some_and(|t| working.iter().any(|w| w == t)) {
        return None;
    }
    (working.len() as u32 >= max.max(1)).then(|| format!("{name} works in {} threads, its limit (max_threads in bot.toml): it waits for a free slot", working.len()))
}

/// Why an automatic message to `thread` (None: a new thread) waits for a free slot now, or None.
pub fn at_cap(app: &App, bot: &crate::bots::Bot, thread: Option<&str>) -> Option<String> {
    let working: Vec<String> = open_sessions(app, &bot.id).iter().filter(|s| s.status.is_running()).map(|s| of(s).unwrap_or(LEGACY).to_string()).collect();
    cap_reason(&working, thread, bot.max_threads(), &bot.settings.name)
}

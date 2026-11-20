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
//!
//! Conversations: several bots may share one conversation the owner follows as one chat. A conversation is named
//! after its first thread (`<bot>:<thread>`, the main bot's): each thread belongs to its own conversation unless its
//! row names another (`conversation`). When a bot's thread sends `launchpad bot message` to another bot, the message
//! goes to that bot's thread of the sender's conversation (`To::Conversation`, created on first use, key
//! `conversation:<id>`), so each bot keeps its own session and sees only what is addressed to it. A thread that
//! continues in a new one (`follow_up`) stays in its conversation; archiving one thread archives the whole
//! conversation.

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
    /// `routine:<file>`, `folio:<id>`), None for the owner's own threads.
    pub key: Option<String>,
    pub title: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub archived_at: Option<i64>,
    /// The conversation it belongs to (`<main bot>:<its first thread>`): its own (`<bot>:<id>`) unless another bot's
    /// thread messaged this bot from a conversation (`To::Conversation`).
    pub conversation: String,
    /// The owner (or an agent) renamed it, by hand or with Auto: its title then names the chat (`chatTitle`).
    pub renamed: bool,
}

impl BotThread {
    /// The main bot of its conversation: the one the conversation started with.
    pub fn main_bot(&self) -> &str {
        main_bot(&self.conversation)
    }
}

/// The ID of a thread's own conversation.
pub fn conversation_id(bot: &str, thread: &str) -> String {
    format!("{bot}:{thread}")
}

/// The main bot of a conversation (the part before the first `:`).
pub fn main_bot(conversation: &str) -> &str {
    conversation.split_once(':').map_or(conversation, |(b, _)| b)
}

/// The key of a bot's thread that joined a conversation of another bot.
pub fn conversation_key(conversation: &str) -> String {
    format!("conversation:{conversation}")
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
    /// Another agent's message: one thread per sender session; from a bot's thread of a conversation (`conversation`,
    /// of another bot than the receiver), the receiver's thread of that conversation.
    Agent { session: &'a str, name: &'a str, conversation: Option<&'a str> },
    /// The owner's message to a bot of a conversation: that bot's thread of it (created when the bot joins).
    Conversation { id: &'a str, title: &'a str },
    /// The owner's feedback on a folio, or a new thread about it: one thread per folio.
    Folio { id: &'a str, title: &'a str, new: bool },
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
    /// The bot's open thread of this conversation, else a new one that joins it (key `conversation:<id>`).
    Conversation { id: String, title: String },
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
        Source::Agent { conversation: Some(id), name, .. } => To::Conversation { id: id.to_string(), title: name.to_string() },
        Source::Conversation { id, title } => To::Conversation { id: id.to_string(), title: title.to_string() },
        Source::Agent { session, name, conversation: None } => To::Key { key: format!("agent:{session}"), title: if name.is_empty() { "An agent".into() } else { name.to_string() } },
        Source::Folio { id, title, new: false } => To::Key { key: folio_key(id), title: title.to_string() },
        Source::Folio { id, title, new: true } => To::New { key: Some(folio_key(id)), title: title.to_string() },
    }
}

/// The key of a folio's thread.
pub fn folio_key(folio_id: &str) -> String {
    format!("folio:{folio_id}")
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
    let (bot_id, id): (String, String) = (r.get(0)?, r.get(1)?);
    let conversation = r.get::<_, Option<String>>(6)?.unwrap_or_else(|| conversation_id(&bot_id, &id));
    Ok(BotThread { bot_id, id, key: r.get(2)?, title: r.get(3)?, created_at: r.get(4)?, archived_at: r.get(5)?, conversation, renamed: r.get::<_, i64>(7)? != 0 })
}

const COLUMNS: &str = "bot_id, id, key, title, created_at, archived_at, conversation, renamed";
/// The conversation of a row, in SQL.
const CONVERSATION: &str = "COALESCE(conversation, bot_id || ':' || id)";

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

/// Every thread, by bot and thread ID (Today's titles and conversations).
pub async fn all(app: &App) -> Result<std::collections::HashMap<(String, String), BotThread>> {
    app.state_db.call(|c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM bot_thread"))?;
        let rows = st.query_map([], row)?.map(|t| t.map(|t| ((t.bot_id.clone(), t.id.clone()), t))).collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
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
        To::Conversation { id, title } => match in_conversation(app, bot, id).await? {
            Some(t) => Ok(t),
            None => {
                let title = conversation_title(app, id).await.unwrap_or_else(|| if title.is_empty() { fallback.to_string() } else { title.clone() });
                insert_in(app, bot, Some(conversation_key(id)), &title, Some(id.clone())).await
            }
        },
    }
}

/// The bot's open thread of a conversation (its main bot's: the thread it started with, or the thread that
/// continues it).
async fn in_conversation(app: &App, bot: &str, conversation: &str) -> Result<Option<String>> {
    let (b, c) = (bot.to_string(), conversation.to_string());
    app.state_db.call(move |db| Ok(db.query_row(&format!("SELECT id FROM bot_thread WHERE bot_id = ?1 AND {CONVERSATION} = ?2 AND archived_at IS NULL ORDER BY created_at DESC LIMIT 1"), params![b, c], |r| r.get(0)).optional()?)).await
}

/// The title of a conversation: its main bot's newest thread of it.
async fn conversation_title(app: &App, conversation: &str) -> Option<String> {
    let c = conversation.to_string();
    app.state_db.call(move |db| Ok(db.query_row(&format!("SELECT title FROM bot_thread WHERE bot_id = ?2 AND {CONVERSATION} = ?1 ORDER BY created_at DESC LIMIT 1"), params![c, main_bot(&c)], |r| r.get(0)).optional()?)).await.ok().flatten()
}

/// Every thread of a conversation (archived ones too), oldest first.
pub async fn conversation_threads(app: &App, conversation: &str) -> Result<Vec<BotThread>> {
    let c = conversation.to_string();
    app.state_db.call(move |db| {
        let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM bot_thread WHERE {CONVERSATION} = ?1 ORDER BY created_at"))?;
        let rows = st.query_map([c], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

/// The thread of a bot session, with its row.
pub async fn of_session(app: &App, s: &Session) -> Option<BotThread> {
    let b = s.launch_spec.bot.as_ref()?;
    get(app, &b.id, b.thread_id()).await.ok().flatten()
}

/// Pure: the conversation an agent's message to `target` joins, from the sender's thread (None: the sender is no bot
/// thread, or it messages its own bot, which keeps a thread per sender session).
pub fn joins(sender: Option<&BotThread>, target: &str) -> Option<String> {
    sender.filter(|t| t.bot_id != target).map(|t| t.conversation.clone())
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
        To::Conversation { id, .. } => in_conversation(app, bot, id).await,
    }
}

/// A new thread that continues `thread` (its session cannot go on): the same key and title, and `thread` leaves the
/// thread bar (archived). Returns the new thread's ID.
pub async fn follow_up(app: &App, bot: &str, thread: &str) -> Result<String> {
    let lock = THREADS.get(bot);
    let _held = lock.lock().await;
    let t = get(app, bot, thread).await?.context("this thread no longer exists")?;
    // It stays in its conversation.
    let id = insert_in(app, bot, t.key.clone(), &t.title, Some(t.conversation.clone())).await?;
    if t.renamed {
        let (b, i) = (bot.to_string(), id.clone());
        app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET renamed = 1 WHERE bot_id = ?1 AND id = ?2", params![b, i])?)).await?;
    }
    // Its key now routes to the new thread.
    set_archived(app, bot, thread, true).await?;
    Ok(id)
}

async fn insert(app: &App, bot: &str, key: Option<String>, title: &str) -> Result<String> {
    insert_in(app, bot, key, title, None).await
}

/// A new thread; `conversation`: the conversation it joins (None: its own).
async fn insert_in(app: &App, bot: &str, key: Option<String>, title: &str, conversation: Option<String>) -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let title = match short(title) {
        t if t.is_empty() => "New thread".to_string(),
        t => t,
    };
    let (b, i) = (bot.to_string(), id.clone());
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO bot_thread (bot_id, id, key, title, created_at, conversation) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![b, i, key, title, now_ms(), conversation])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    Ok(id)
}

/// Rename: a new title (one line, at most 80 characters).
pub async fn rename(app: &App, bot: &str, id: &str, title: &str) -> Result<BotThread> {
    let title = short(title);
    ensure!(!title.is_empty(), "give the thread a title");
    get(app, bot, id).await?.context("this thread no longer exists")?;
    let (b, i, t) = (bot.to_string(), id.to_string(), title.clone());
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET title = ?3, renamed = 1 WHERE bot_id = ?1 AND id = ?2", params![b, i, t])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    get(app, bot, id).await?.context("this thread no longer exists")
}

/// The owner's Auto in a chat's rename (`bot.thread-auto-title`, Home): the session of the thread names itself
/// from its conversation as it is now on its runner (`titles::regenerate`), then the thread takes that title unless it
/// was renamed meanwhile (a later rename wins). On demand only.
pub async fn auto_title(app: &App, bot: &str, id: &str) -> Result<BotThread> {
    let t = get(app, bot, id).await?.context("this thread no longer exists")?;
    let s = session(app, bot, id).context("this chat has no conversation to name yet")?;
    let runner = s.runner_id.clone().context("this chat's agent has no runner: try Auto again later")?;
    let (status, body) = app.runners.request(&runner, "POST", &format!("/api/sessions/{}/auto-title", s.id), None).await?;
    if !(200..300).contains(&status) {
        let why = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or(body);
        anyhow::bail!("{why}");
    }
    let named: crate::titles::AutoTitle = serde_json::from_str(&body).context("read the generated title")?;
    let title = short(&named.title);
    ensure!(!title.is_empty(), "no title generated");
    let (b, i, old) = (bot.to_string(), id.to_string(), t.title.clone());
    let changed = app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET title = ?3, renamed = 1 WHERE bot_id = ?1 AND id = ?2 AND title = ?4", params![b, i, title, old])?)).await?;
    if changed > 0 { app.bus.publish("bots", "threads", serde_json::json!({"id": bot})); }
    get(app, bot, id).await?.context("this thread no longer exists")
}

async fn set_archived(app: &App, bot: &str, id: &str, archived: bool) -> Result<()> {
    let (b, i) = (bot.to_string(), id.to_string());
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot_thread SET archived_at = CASE WHEN ?3 THEN ?4 ELSE NULL END WHERE bot_id = ?1 AND id = ?2", params![b, i, archived, now_ms()])?)).await?;
    app.bus.publish("bots", "threads", serde_json::json!({"id": bot}));
    Ok(())
}

/// Archive (or bring back) a thread and the other threads of its conversation (the threads of other bots it shares,
/// open ones; Bring back: the ones archived with it). Refused while one of them works.
pub async fn archive(app: &App, bot: &str, id: &str, archived: bool) -> Result<BotThread> {
    let t = get(app, bot, id).await?.context("this thread no longer exists")?;
    let others: Vec<BotThread> = conversation_threads(app, &t.conversation).await?.into_iter().filter(|o| !(o.bot_id == bot && o.id == id)).collect();
    let others: Vec<BotThread> = if archived {
        others.into_iter().filter(|o| o.archived_at.is_none()).collect()
    } else {
        // Archived with it: within a minute of its own archive.
        let at = t.archived_at.or_else(|| session(app, bot, id).filter(|s| s.status == SessionStatus::Closed).map(|s| s.status_since)).unwrap_or(0);
        others.into_iter().filter(|o| o.archived_at.is_some_and(|a| (a - at).abs() < 60_000)).collect()
    };
    if archived && others.iter().any(|o| open_session(app, &o.bot_id, &o.id).is_some_and(|s| s.status.is_running())) {
        bail!("a bot of this conversation is working: stop it first");
    }
    let done = archive_one(app, bot, id, archived).await?;
    for o in others {
        if let Err(e) = archive_one(app, &o.bot_id, &o.id, archived).await {
            tracing::warn!("archive of thread {} of bot {} with its conversation: {e:#}", o.id, o.bot_id);
        }
    }
    Ok(done)
}

async fn archive_one(app: &App, bot: &str, id: &str, archived: bool) -> Result<BotThread> {
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

/// The threads of a workspace's bots the Chats list needs: every open thread, every archived thread of a conversation
/// that still has an open one (its participants and their order), and the threads of the `archived` newest archived
/// conversations (newest archive first). Returns them with the number of archived conversations in all, so the list
/// pages them instead of loading years of routine runs. A thread whose session is archived counts as archived, with
/// that time as `archived_at`, like `list`.
pub async fn for_chats(app: &App, bots: &[String], archived: usize) -> Result<(Vec<BotThread>, usize)> {
    for b in bots {
        ensure_legacy(app, b).await?;
    }
    let ids = bots.to_vec();
    let rows = app.state_db.call(move |c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM bot_thread WHERE bot_id IN (SELECT value FROM json_each(?1))"))?;
        let rows = st.query_map(params![serde_json::to_string(&ids).unwrap_or_default()], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    // Each thread's newest session, read once (not one scan of the sessions per thread).
    let mut newest: std::collections::HashMap<(String, String), Session> = std::collections::HashMap::new();
    for s in app.sessions.list() {
        let Some(b) = s.launch_spec.bot.as_ref() else { continue };
        let key = (b.id.clone(), b.thread_id().to_string());
        if newest.get(&key).is_none_or(|o| o.started_at <= s.started_at) {
            newest.insert(key, s);
        }
    }
    let rows: Vec<BotThread> = rows.into_iter().map(|mut t| {
        if t.archived_at.is_none() && let Some(s) = newest.get(&(t.bot_id.clone(), t.id.clone())).filter(|s| s.status == SessionStatus::Closed) {
            t.archived_at = Some(s.status_since);
        }
        t
    }).collect();
    Ok(chat_page(rows, archived))
}

/// Pure: `for_chats`'s choice of rows (see there).
pub fn chat_page(rows: Vec<BotThread>, archived: usize) -> (Vec<BotThread>, usize) {
    let open: std::collections::HashSet<String> = rows.iter().filter(|t| t.archived_at.is_none()).map(|t| t.conversation.clone()).collect();
    // Archived conversations, newest archive first.
    let mut last: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for t in rows.iter().filter(|t| !open.contains(&t.conversation)) {
        let at = t.archived_at.unwrap_or(t.created_at);
        let e = last.entry(t.conversation.as_str()).or_insert(at);
        *e = (*e).max(at);
    }
    let mut order: Vec<(&str, i64)> = last.into_iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let total = order.len();
    let shown: std::collections::HashSet<String> = order.into_iter().take(archived).map(|(c, _)| c.to_string()).collect();
    let mut out: Vec<BotThread> = rows.into_iter().filter(|t| open.contains(&t.conversation) || shown.contains(&t.conversation)).collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    (out, total)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_page_keeps_open_conversations_whole_and_pages_archived_ones() {
        let t = |bot: &str, id: &str, conv: &str, created: i64, archived: Option<i64>| BotThread { bot_id: bot.into(), id: id.into(), key: None, title: id.into(), created_at: created, archived_at: archived, renamed: false, conversation: conv.into() };
        let rows = vec![
            // A conversation that continued in a new thread: its archived first thread stays.
            t("a", "first", "a:first", 1, Some(5)), t("a", "next", "a:first", 6, None), t("b", "joined", "a:first", 3, None),
            t("a", "old", "a:old", 2, Some(10)), t("a", "older", "a:older", 1, Some(4)), t("b", "newest", "b:newest", 7, Some(20)),
        ];
        let (out, total) = chat_page(rows, 2);
        let ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(total, 3);
        assert_eq!(ids, ["newest", "next", "joined", "old", "first"]);
    }

    fn thread(bot: &str, id: &str, conversation: Option<&str>) -> BotThread {
        BotThread { id: id.into(), bot_id: bot.into(), key: None, title: "t".into(), created_at: 0, archived_at: None, renamed: false, conversation: conversation.map_or_else(|| conversation_id(bot, id), str::to_string) }
    }

    #[test]
    fn a_bot_s_message_from_a_conversation_joins_it() {
        let a = thread("a", "t1", None);
        // A's thread messages B: B's thread of A's conversation.
        assert_eq!(joins(Some(&a), "b").as_deref(), Some("a:t1"));
        let to = route(Source::Agent { session: "s1", name: "A", conversation: joins(Some(&a), "b").as_deref() });
        assert_eq!(to, To::Conversation { id: "a:t1".into(), title: "A".into() });
        // B, in A's conversation, messages C: C joins the same conversation, and an answer to A finds A's thread.
        let b = thread("b", "t9", Some("a:t1"));
        assert_eq!(joins(Some(&b), "c").as_deref(), Some("a:t1"));
        assert_eq!(b.main_bot(), "a");
        // A plain agent, or a bot messaging itself, keeps a thread per sender session.
        assert_eq!(joins(None, "b"), None);
        assert_eq!(joins(Some(&a), "a"), None);
        assert_eq!(route(Source::Agent { session: "s1", name: "", conversation: None }), To::Key { key: "agent:s1".into(), title: "An agent".into() });
        // The owner addresses a bot of a conversation.
        assert_eq!(route(Source::Conversation { id: "a:t1", title: "" }), To::Conversation { id: "a:t1".into(), title: String::new() });
        assert_eq!(conversation_key("a:t1"), "conversation:a:t1");
    }
}

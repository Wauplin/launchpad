//! Each workspace's own Telegram bot, on Home. In Settings → Workspace, the owner pastes a bot token from
//! @BotFather (vault entry `channel-telegram-<workspace id>`, write-only), then binds their own chat with a
//! single-use code (`/start <code>`). Only that chat is ever answered; other chats are ignored. Home long-polls
//! `getUpdates` once per token (a private Space has no inbound endpoint for Telegram); one bot token serves one
//! workspace only, since Telegram allows a single `getUpdates` reader.
//!
//! Messages to the owner's chat go out directly (owner decision): notifications of the workspace's sessions (the
//! session notification events, never code), the final message of turns started from that chat, and
//! `launchpad send telegram` from the workspace's agents. Incoming text goes to a bot of that workspace
//! (`/<bot> text`, `/bot <name> text`), plain text to its Ask target (`ask_target`); launches from Telegram never
//! allow paid usage. An update is marked handled before it is acted on, so a crash never runs a message twice.
//! A workspace without its own Telegram bot gets no Telegram.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::bots::Bot;
use launchpad_core::model::{Session, SessionStatus};
use launchpad_core::util::now_ms;
use launchpad_core::vault::LaunchpadAccess;

/// The vault entry of a workspace's bot token.
fn token_id(workspace: &str) -> String {
    format!("channel-telegram-{workspace}")
}
/// The single token of the time when one Telegram bot served every workspace: dropped once at start.
const OLD_TOKEN_ID: &str = "channel-telegram";
const CODE_TTL_MS: i64 = 30 * 60 * 1000;
/// Telegram's limit is 4096 characters.
const MAX_TEXT: usize = 3500;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    /// Telegram's ID of the bot (the digits before the token's colon): one workspace per bot.
    bot_id: Option<i64>,
    bot_username: Option<String>,
    chat_id: Option<i64>,
    chat_name: Option<String>,
    code: Option<String>,
    code_expires_at: Option<i64>,
    #[serde(default = "yes")]
    notify: bool,
    #[serde(default)]
    titles: bool,
    #[serde(default)]
    offset: i64,
    error: Option<String>,
}

fn yes() -> bool {
    true
}

/// Settings → Workspace → Telegram.
#[derive(Clone, Debug, Serialize, TS)]
pub struct TelegramStatus {
    pub workspace_id: String,
    /// A bot token is saved for this workspace.
    pub configured: bool,
    pub bot_username: Option<String>,
    pub bound: bool,
    pub chat_name: Option<String>,
    /// The binding code while no chat is bound (or a new one was asked for), with its `t.me` link.
    pub code: Option<String>,
    pub link: Option<String>,
    #[ts(type = "number | null")]
    pub code_expires_at: Option<i64>,
    /// Notifications of this workspace's sessions to the chat (completed turns, failures, interruptions and automatic stops).
    pub notify: bool,
    /// Include session titles in notifications.
    pub titles: bool,
    /// The last polling error, if Telegram is not reachable or refused the token.
    pub error: Option<String>,
}

async fn load(app: &App, workspace: &str) -> State {
    let ws = workspace.to_string();
    app.state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT json FROM telegram_chat WHERE workspace_id = ?1", [ws], |r| r.get::<_, String>(0)).optional()?)
        })
        .await
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

/// Every workspace's Telegram state.
async fn all(app: &App) -> Vec<(String, State)> {
    states(app).await.unwrap_or_default()
}

/// `all`, with the database error: the supervisor keeps its loops when it cannot read the states.
async fn states(app: &App) -> Result<Vec<(String, State)>> {
    let rows = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT workspace_id, json FROM telegram_chat ORDER BY workspace_id")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(rows.into_iter().filter_map(|(ws, j)| Some((ws, serde_json::from_str(&j).ok()?))).collect())
}

/// One writer of the Telegram states at a time: every change loads, changes and stores under it, so a poll loop's
/// offset never overwrites the owner's switch, and a loop's write after Delete never brings the row back.
static WRITE: std::sync::LazyLock<tokio::sync::Mutex<()>> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Changes the workspace's saved state (`set_token` alone creates it). None: it has none (deleted meanwhile).
async fn modify(app: &App, workspace: &str, f: impl FnOnce(&mut State)) -> Result<Option<State>> {
    let _w = WRITE.lock().await;
    let ws = workspace.to_string();
    let json: Option<String> = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT json FROM telegram_chat WHERE workspace_id = ?1", [ws], |r| r.get::<_, String>(0)).optional()?)
        })
        .await?;
    let Some(json) = json else { return Ok(None) };
    let mut st: State = serde_json::from_str(&json).unwrap_or_default();
    f(&mut st);
    let (ws, json) = (workspace.to_string(), serde_json::to_string(&st)?);
    app.state_db.call(move |c| Ok(c.execute("UPDATE telegram_chat SET json = ?2 WHERE workspace_id = ?1", [ws, json])?)).await?;
    Ok(Some(st))
}

async fn token(app: &App, workspace: &str) -> Option<String> {
    launchpad_core::vault::secret(app, &token_id(workspace)).await.ok().flatten().map(|(_, t)| t)
}

fn code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    rand::random::<[u8; 8]>().iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

pub async fn status(app: &App, workspace: &str) -> TelegramStatus {
    let st = load(app, workspace).await;
    let configured = token(app, workspace).await.is_some();
    let code = st.code.clone().filter(|_| st.code_expires_at.is_some_and(|e| e > now_ms()));
    TelegramStatus {
        workspace_id: workspace.to_string(),
        configured,
        link: match (&st.bot_username, &code) {
            (Some(b), Some(c)) => Some(format!("https://t.me/{b}?start={c}")),
            _ => None,
        },
        code_expires_at: code.as_ref().and(st.code_expires_at),
        code,
        bound: configured && st.chat_id.is_some(),
        bot_username: st.bot_username,
        chat_name: st.chat_name,
        notify: st.notify,
        titles: st.titles,
        error: if configured { st.error } else { None },
    }
}

/// Every workspace with a Telegram bot (diagnostics).
pub async fn statuses(app: &App) -> Vec<TelegramStatus> {
    let mut out = vec![];
    for (ws, _) in all(app).await {
        out.push(status(app, &ws).await);
    }
    out
}

/// Some workspace has a bound chat: `telegram` is a channel name agents may use.
pub async fn any_bound(app: &App) -> bool {
    for (ws, st) in all(app).await {
        if st.chat_id.is_some() && token(app, &ws).await.is_some() {
            return true;
        }
    }
    false
}

async fn ensure_workspace(app: &App, workspace: &str) -> Result<()> {
    crate::channels::ensure_home(app)?;
    launchpad_core::workspace::get(app, workspace).await.map(|_| ())
}

// ---------------------------------------------------------------- polling loops

/// The poll loop of each workspace with a Telegram state, by workspace: its wake signal. A loop runs while its
/// entry is this one (`supervise` replaces or removes it).
static LOOPS: std::sync::LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// Wakes the supervisor: a token was added or removed, a workspace changed.
static SUPERVISE: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

fn changed(app: &App, workspace: &str) {
    if let Some(n) = LOOPS.lock().unwrap().get(workspace) {
        n.notify_one();
    }
    SUPERVISE.notify_one();
    crate::channels::changed(app);
}

/// One Bot API call. Errors never contain the URL (it holds the token).
async fn call(token: &str, method: &str, body: Value, timeout_s: u64) -> Result<Value> {
    let res = reqwest::Client::new()
        .post(format!("https://api.telegram.org/bot{token}/{method}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(timeout_s))
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach Telegram: {}", e.without_url()))?;
    let v: Value = res.json().await.map_err(|e| anyhow!("Telegram answered without JSON: {}", e.without_url()))?;
    ensure!(v["ok"] == true, "Telegram: {}", v["description"].as_str().unwrap_or("the request failed"));
    Ok(v["result"].clone())
}

/// The bot's ID in a token (`<digits>:<secret>`), when it has the shape @BotFather gives.
fn token_bot_id(token: &str) -> Option<i64> {
    let (id, rest) = token.split_once(':')?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && rest.len() >= 20 && !rest.contains(char::is_whitespace)).then(|| id.parse().ok())?
}

/// The other workspace that already uses this Telegram bot, if any.
fn used_elsewhere<'a>(states: &'a [(String, State)], workspace: &str, bot_id: i64) -> Option<&'a str> {
    states.iter().find(|(ws, st)| ws != workspace && st.bot_id == Some(bot_id)).map(|(ws, _)| ws.as_str())
}

/// Owner click: saves the workspace's token after Telegram accepted it, and shows a binding code. A bot already
/// used by another workspace is refused: Telegram gives its messages to one reader only.
pub async fn set_token(app: &App, workspace: &str, token: &str) -> Result<TelegramStatus> {
    ensure_workspace(app, workspace).await?;
    let token = token.trim();
    let bot_id = token_bot_id(token).context("paste the token @BotFather gave you (digits, a colon, then letters)")?;
    if let Some(other) = used_elsewhere(&all(app).await, workspace, bot_id) {
        let name = launchpad_core::workspace::get(app, other).await.map(|w| w.name).unwrap_or_else(|_| "another workspace".into());
        bail!("this Telegram bot already serves {name}: Telegram lets one reader get a bot's messages, so create another bot with @BotFather for this workspace");
    }
    let me = call(token, "getMe", json!({}), 20).await?;
    let username = me["username"].as_str().context("Telegram did not return the bot's name")?.to_string();
    {
        let _w = WRITE.lock().await;
        // Again under the lock: two workspaces saving the same bot at once.
        ensure!(used_elsewhere(&all(app).await, workspace, bot_id).is_none(), "this Telegram bot already serves another workspace: create another bot with @BotFather for this workspace");
        launchpad_core::vault::put(app, &token_id(workspace), launchpad_core::vault::Kind::Token, "Telegram bot", token).await?;
        let st = State { bot_id: Some(bot_id), bot_username: Some(username), code: Some(code()), code_expires_at: Some(now_ms() + CODE_TTL_MS), notify: true, ..Default::default() };
        let (ws, json) = (workspace.to_string(), serde_json::to_string(&st)?);
        app.state_db.call(move |c| Ok(c.execute("INSERT INTO telegram_chat (workspace_id, json) VALUES (?1, ?2) ON CONFLICT (workspace_id) DO UPDATE SET json = ?2", [ws, json])?)).await?;
    }
    changed(app, workspace);
    Ok(status(app, workspace).await)
}

/// A new binding code: the next `/start <code>` binds that chat instead of the current one.
pub async fn new_code(app: &App, workspace: &str) -> Result<TelegramStatus> {
    ensure_workspace(app, workspace).await?;
    ensure!(token(app, workspace).await.is_some(), "add the bot token first");
    modify(app, workspace, |st| {
        st.code = Some(code());
        st.code_expires_at = Some(now_ms() + CODE_TTL_MS);
    })
    .await?
    .context("add the bot token first")?;
    changed(app, workspace);
    Ok(status(app, workspace).await)
}

#[derive(Deserialize)]
pub struct Patch {
    pub notify: Option<bool>,
    pub titles: Option<bool>,
}

pub async fn patch(app: &App, workspace: &str, p: Patch) -> Result<TelegramStatus> {
    ensure_workspace(app, workspace).await?;
    ensure!(token(app, workspace).await.is_some(), "add the bot token first");
    modify(app, workspace, |st| {
        if let Some(v) = p.notify { st.notify = v; }
        if let Some(v) = p.titles { st.titles = v; }
    })
    .await?
    .context("add the bot token first")?;
    changed(app, workspace);
    Ok(status(app, workspace).await)
}

/// Removes the workspace's token and bound chat (also when its workspace was deleted).
pub async fn remove(app: &App, workspace: &str) -> Result<()> {
    crate::channels::ensure_home(app)?;
    let _w = WRITE.lock().await;
    launchpad_core::vault::delete(app, &token_id(workspace)).await?;
    let ws = workspace.to_string();
    let gone: Vec<String> = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let ids = {
                let mut st = tx.prepare("SELECT session_id FROM telegram_answer WHERE workspace_id = ?1")?;
                st.query_map([&ws], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?
            };
            tx.execute("DELETE FROM telegram_answer WHERE workspace_id = ?1", [&ws])?;
            tx.execute("DELETE FROM telegram_chat WHERE workspace_id = ?1", [&ws])?;
            tx.commit()?;
            Ok(ids)
        })
        .await?;
    drop(_w);
    {
        let mut pending = PENDING.lock().unwrap();
        for id in gone {
            pending.remove(&id);
        }
    }
    changed(app, workspace);
    Ok(())
}

/// Sends text to the workspace's chat (plain text, no link previews).
pub(crate) async fn say(app: &App, workspace: &str, text: &str) -> Result<()> {
    let st = load(app, workspace).await;
    let chat = st.chat_id.context("this workspace has no Telegram chat: bind one in Settings → Workspace")?;
    let token = token(app, workspace).await.context("this workspace has no Telegram bot: add it in Settings → Workspace")?;
    call(&token, "sendMessage", json!({"chat_id": chat, "text": truncate(text, MAX_TEXT), "disable_web_page_preview": true}), 20).await.map(|_| ())
}

pub async fn test(app: &App, workspace: &str) -> Result<()> {
    ensure_workspace(app, workspace).await?;
    let name = launchpad_core::workspace::get(app, workspace).await?.name;
    say(app, workspace, &format!("Test message from Launchpad: this chat receives the notifications and answers of the workspace {name}.")).await
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}… (truncated; the full text is in Launchpad)", text.chars().take(max).collect::<String>().trim_end())
}

/// Where a session's messages show: its bot's chat, else the session in Agents, with `ws=` of its workspace.
async fn session_link(app: &App, id: &str, workspace: &str) -> String {
    let origin = crate::channels::home_origin(app);
    match app.sessions.get(id).and_then(|s| s.launch_spec.bot.map(|b| b.id)) {
        Some(bot) => format!("{origin}/bots/{bot}?ws={workspace}"),
        None => format!("{origin}/agents?session={id}&ws={workspace}"),
    }
}

/// The session's card in Today (`/?card=session:<id>`), which holds its main action; a bot's session opens the
/// bot's chat instead (bots talk there). No buttons in Telegram: every owner click happens in Launchpad.
async fn card_link(app: &App, id: &str, workspace: &str) -> String {
    if app.sessions.get(id).is_some_and(|s| s.launch_spec.bot.is_some()) {
        return session_link(app, id, workspace).await;
    }
    format!("{}/?card=session:{id}&ws={workspace}", crate::channels::home_origin(app))
}

fn session_name(s: &Session) -> String {
    match &s.launch_spec.bot {
        Some(b) => b.name.clone(),
        None => s.harness.display().to_string(),
    }
}

/// `launchpad send telegram`: an agent's message to the chat of its session's workspace, sent directly.
pub(crate) async fn send_from_agent(app: &App, session_id: Option<&str>, title: Option<&str>, body: &str) -> Result<()> {
    ensure!(!body.trim().is_empty(), "the message is empty");
    let id = session_id.context("telegram needs --session-id: the message goes to the Telegram chat of the session's workspace")?;
    let s = app.sessions.get(id).context("session not found")?;
    let workspace = launchpad_core::kb::workspace_of_session(app, id).await.context("this session belongs to no workspace, so it has no Telegram chat")?;
    ensure!(load(app, &workspace).await.chat_id.is_some() && token(app, &workspace).await.is_some(), "this session's workspace has no Telegram chat: the owner adds one in Settings → Workspace");
    let head = match title {
        Some(t) => format!("{}: {t}\n\n", session_name(&s)),
        None => format!("{}:\n", session_name(&s)),
    };
    let link = session_link(app, id, &workspace).await;
    say(app, &workspace, &format!("{head}{}\n\n{link}", truncate(body, MAX_TEXT - 300))).await
}

// ---------------------------------------------------------------- notifications

fn label(event: &str) -> &'static str {
    match event {
        "done" => "Done",
        "failed" => "Failed",
        "interrupted" => "Interrupted",
        "quota_paused" => "Quota paused",
        "cost_limit" => "Cost limit reached",
        "quota_resumed" => "Quota available; agent resumed",
        _ => "Update",
    }
}

/// From `notify::session`: an eligible notification. A runner forwards it to Home, which owns the chats.
pub fn notify(app: &App, s: &Session, event: &'static str) {
    let (app, id) = (app.clone(), s.id.clone());
    tokio::spawn(async move {
        let result = if launchpad_core::vault::here(&app) {
            notification(&app, &id, event).await
        } else {
            launchpad_core::home_link::home_request("POST", "/api/channels/telegram/notify", Some(json!({"session_id": id, "event": event}))).await.map(|_| ())
        };
        if let Err(e) = result {
            tracing::debug!("telegram notification: {e:#}");
        }
    });
}

/// The chat a notification of a session in `workspace` goes to: only that workspace's own, bound, with
/// notifications on. A session outside every workspace gets none.
fn notification_chat(st: Option<&State>, configured: bool) -> Option<i64> {
    st.filter(|st| configured && st.notify).and_then(|st| st.chat_id)
}

/// Home: sends a notification to the chat of the session's workspace when its notifications are on. Never
/// contains code or messages.
pub async fn notification(app: &App, session_id: &str, event: &str) -> Result<()> {
    crate::channels::ensure_home(app)?;
    // The session's workspace, through its folder, like Today.
    let Some(workspace) = launchpad_core::kb::workspace_of_session(app, session_id).await else { return Ok(()) };
    let st = load(app, &workspace).await;
    if notification_chat(Some(&st), token(app, &workspace).await.is_some()).is_none() || PENDING.lock().unwrap().contains(session_id) {
        // A turn started from Telegram answers with its final message instead.
        return Ok(());
    }
    let s = app.sessions.get(session_id).context("session not found")?;
    let pid = s.project_id.clone();
    let project = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT name FROM project WHERE id = ?1", [pid], |r| r.get::<_, String>(0)).optional()?)
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| std::path::Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let title = if st.titles && !s.title.is_empty() { format!("\n{}", s.title.chars().take(160).collect::<String>()) } else { String::new() };
    // The link opens its card in Today (finished, failed, interrupted; a bot's chat for its sessions) or the session.
    let link = match event {
        "done" | "failed" | "interrupted" => card_link(app, session_id, &workspace).await,
        _ => session_link(app, session_id, &workspace).await,
    };
    say(app, &workspace, &format!("{} in {project} · {}{title}\n{link}", label(event), session_name(&s))).await
}

// ---------------------------------------------------------------- incoming messages

/// A message from the bound chat.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Start(Option<String>),
    Help,
    /// `/<name> text` or `/bot <name> text`.
    Bot { name: String, text: String },
    /// `/<name>` or `/bot <name>` without text.
    Empty(String),
    Ask(String),
}

/// Parses a chat message. Commands may carry the bot's name (`/scout@my_bot`).
pub fn parse(text: &str) -> Incoming {
    let text = text.trim();
    let Some(rest) = text.strip_prefix('/') else { return Incoming::Ask(text.to_string()) };
    let (cmd, args) = rest.split_once(char::is_whitespace).map(|(c, a)| (c, a.trim())).unwrap_or((rest, ""));
    let cmd = cmd.split('@').next().unwrap_or("").to_ascii_lowercase();
    match cmd.as_str() {
        "start" => Incoming::Start(args.split_whitespace().next().map(str::to_string)),
        "help" | "" => Incoming::Help,
        "bot" => {
            let (name, text) = args.split_once(char::is_whitespace).map(|(n, t)| (n, t.trim())).unwrap_or((args, ""));
            if name.is_empty() { Incoming::Help } else if text.is_empty() { Incoming::Empty(name.to_ascii_lowercase()) } else { Incoming::Bot { name: name.to_ascii_lowercase(), text: text.to_string() } }
        }
        _ if args.is_empty() => Incoming::Empty(cmd),
        _ => Incoming::Bot { name: cmd, text: args.to_string() },
    }
}

/// Telegram command names have no dashes: `/repo_scout` and `/reposcout` both find the bot `repo-scout`.
fn same_name(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
    !norm(a).is_empty() && norm(a) == norm(b)
}

/// The workspace's bots, oldest first.
fn bots_of<'a>(bots: &'a [Bot], workspace: &str) -> Vec<&'a Bot> {
    let mut mine: Vec<&Bot> = bots.iter().filter(|b| b.workspace_id == workspace).collect();
    mine.sort_by_key(|b| b.created_at);
    mine
}

/// Where plain text goes, the rule of the top-bar Ask (`askTarget` in `web/src/lib/bots.ts`): the workspace's
/// oldest bot with admin Launchpad CLI access, else its oldest bot.
pub fn ask_target<'a>(bots: &'a [Bot], workspace: &str) -> Option<&'a Bot> {
    let mine = bots_of(bots, workspace);
    mine.iter().find(|b| b.settings.launchpad == LaunchpadAccess::Admin).or(mine.first()).copied()
}

/// The workspace's bot a `/<name>` command names (its slug or its name).
fn find_bot<'a>(bots: &'a [Bot], workspace: &str, name: &str) -> Option<&'a Bot> {
    bots_of(bots, workspace).into_iter().find(|b| same_name(&b.slug, name) || same_name(&b.settings.name, name))
}

/// The help text: the workspace's bots only.
fn help_text(bots: &[Bot], workspace: &str) -> String {
    let mine = bots_of(bots, workspace);
    let list = if mine.is_empty() { "no bots yet".to_string() } else { mine.iter().map(|b| format!("/{} ({})", b.slug.replace('-', "_"), b.settings.name)).collect::<Vec<_>>().join(", ") };
    let first = ask_target(bots, workspace).map(|b| format!("Write a message to talk to {}. ", b.settings.name)).unwrap_or_default();
    format!("{first}To give a bot a task: /<bot> <text> or /bot <name> <text>. Bots: {list}.")
}

async fn help(app: &App, workspace: &str) -> String {
    help_text(&launchpad_core::bots::list(app).await.unwrap_or_default(), workspace)
}

pub(crate) const FROM_TELEGRAM: &str = "\n\n(Sent from Telegram. Your final message of this turn goes back to the owner's chat as plain text: keep it short.)";

async fn handle(app: &App, workspace: &str, msg: &Value) -> Result<()> {
    let Some(chat) = msg["chat"]["id"].as_i64() else { return Ok(()) };
    let text = msg["text"].as_str().unwrap_or("");
    let st = load(app, workspace).await;
    if let Incoming::Start(Some(c)) = parse(text)
        && msg["chat"]["type"] == "private"
        && st.code.as_deref().is_some_and(|code| code.eq_ignore_ascii_case(&c))
        && st.code_expires_at.is_some_and(|e| e > now_ms())
    {
        let name = msg["chat"]["username"].as_str().map(|u| format!("@{u}")).or_else(|| msg["chat"]["first_name"].as_str().map(str::to_string));
        // Single use: the code is spent with the binding, under the write lock (a new code meanwhile wins).
        let bound = modify(app, workspace, |st| {
            if st.code.as_deref().is_some_and(|code| code.eq_ignore_ascii_case(&c)) {
                st.chat_id = Some(chat);
                st.chat_name = name;
                st.code = None;
                st.code_expires_at = None;
            }
        })
        .await?
        .is_some_and(|st| st.chat_id == Some(chat) && st.code.is_none());
        if !bound {
            return Ok(());
        }
        changed(app, workspace);
        let name = launchpad_core::workspace::get(app, workspace).await.map(|w| w.name).unwrap_or_default();
        return say(app, workspace, &format!("This chat is now bound to the Launchpad workspace {name}. Only this chat is answered.\n\n{}", help(app, workspace).await)).await;
    }
    // Every other chat is ignored, without an answer.
    if st.chat_id != Some(chat) {
        return Ok(());
    }
    if text.trim().is_empty() {
        return say(app, workspace, "Launchpad reads text messages only.").await;
    }
    let bots = launchpad_core::bots::list(app).await?;
    let (bot, body) = match parse(text) {
        Incoming::Start(_) | Incoming::Help => return say(app, workspace, &help_text(&bots, workspace)).await,
        Incoming::Empty(name) => return say(app, workspace, &format!("Add the task after the command, for example /{name} check the open issues.")).await,
        // A plain message goes to the workspace's Ask target.
        Incoming::Ask(t) => match ask_target(&bots, workspace) {
            Some(b) => (b, t),
            None => return say(app, workspace, "This workspace has no bot yet: create one in Launchpad (Bots), then write again.").await,
        },
        Incoming::Bot { name, text } => match find_bot(&bots, workspace, &name) {
            Some(b) => (b, text),
            None => return say(app, workspace, &format!("No bot named {name} in this workspace. {}", help_text(&bots, workspace))).await,
        },
    };
    let name = bot.settings.name.clone();
    let sent = crate::hooks::deliver(app, crate::hooks::Target::Bot { id: bot.id.clone() }, format!("{body}{FROM_TELEGRAM}"), false, true).await;
    match sent {
        Ok(d) if d.queued => {
            let link = match d.session {
                Some(s) => format!("\n{}", session_link(app, &s.id, workspace).await),
                None => String::new(),
            };
            say(app, workspace, &format!("Queued for {name}: it is busy; your message is sent when its current turn ends, and the answer comes here.{link}")).await
        }
        Ok(d) => {
            let s = d.session.context("no session")?;
            await_reply(app, workspace, &s).await?;
            say(app, workspace, &format!("Sent to {name}. Its answer comes here when the turn ends.\n{}", session_link(app, &s.id, workspace).await)).await
        }
        Err(e) => say(app, workspace, &format!("Launchpad could not send it: {e:#}")).await,
    }
}

/// The workspace of a bot, whose Telegram chat answers for the messages sent to it from there.
pub(crate) async fn bot_workspace(app: &App, bot_id: &str) -> Option<String> {
    launchpad_core::bots::get(app, bot_id).await.ok().map(|b| b.workspace_id)
}

// ---------------------------------------------------------------- answers to the chat

/// Sessions whose current turn came from Telegram (`telegram_answer`): Home sends their final message to the
/// chat of the workspace it came from.
static PENDING: std::sync::LazyLock<Mutex<HashSet<String>>> = std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) async fn await_reply(app: &App, workspace: &str, s: &Session) -> Result<()> {
    let (id, ws, after) = (s.id.clone(), workspace.to_string(), s.last_seq);
    app.state_db
        .call(move |c| Ok(c.execute("INSERT INTO telegram_answer (session_id, workspace_id, after_seq, at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (session_id) DO UPDATE SET workspace_id = ?2, after_seq = ?3, at = ?4", rusqlite::params![id, ws, after, now_ms()])?))
        .await?;
    PENDING.lock().unwrap().insert(s.id.clone());
    Ok(())
}

/// When the turn has ended (a non-running status event after the message), the reply is claimed (row deleted)
/// and sent once.
async fn check_reply(app: &App, id: &str) -> Result<()> {
    let key = id.to_string();
    let rows: Option<(String, Vec<(String, String)>)> = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let Some((ws, after)) = c.query_row("SELECT workspace_id, after_seq FROM telegram_answer WHERE session_id = ?1", [&key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).optional()? else { return Ok(None) };
            let mut st = c.prepare("SELECT kind, json FROM session_event WHERE session_id = ?1 AND seq > ?2 AND kind IN ('status', 'assistant_message') ORDER BY seq")?;
            let rows = st.query_map(rusqlite::params![key, after], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((ws, rows)))
        })
        .await?;
    let Some((workspace, rows)) = rows else {
        PENDING.lock().unwrap().remove(id);
        return Ok(());
    };
    let (mut ended, mut last) = (None, None);
    for (kind, json) in rows {
        let v: Value = serde_json::from_str(&json).unwrap_or_default();
        if kind == "status" {
            let status: Option<SessionStatus> = serde_json::from_value(v["status"].clone()).ok();
            ended = status.filter(|s| !s.is_running());
        } else if let Some(t) = v["text"].as_str() {
            last = Some(t.to_string());
        }
    }
    let Some(status) = ended else { return Ok(()) };
    let Some(s) = app.sessions.get(id).filter(|s| !s.status.is_running()) else { return Ok(()) };
    let key = id.to_string();
    let claimed = app.state_db.call(move |c| Ok(c.execute("DELETE FROM telegram_answer WHERE session_id = ?1", [key])?)).await? > 0;
    PENDING.lock().unwrap().remove(id);
    if !claimed {
        return Ok(());
    }
    let name = session_name(&s);
    let link = session_link(app, id, &workspace).await;
    let text = match (status, last) {
        (SessionStatus::Waiting, Some(t)) => format!("{name}:\n{}\n\n{link}", truncate(t.trim(), MAX_TEXT - 300)),
        (SessionStatus::Waiting, None) => format!("{name} finished its turn without a message.\n{link}"),
        (other, Some(t)) => format!("{name} · {}:\n{}\n\n{link}", label_of(other), truncate(t.trim(), MAX_TEXT - 300)),
        (other, None) => format!("{name} · {}.\n{link}", label_of(other)),
    };
    say(app, &workspace, &text).await
}

fn label_of(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Error => "Failed",
        SessionStatus::Interrupted => "Interrupted",
        SessionStatus::Stopped => "Stopped",
        SessionStatus::QuotaPaused => "Quota paused",
        SessionStatus::Closed => "Archived",
        _ => "Done",
    }
}

/// Watches the bus for the end of turns started from Telegram, and for workspace changes (a deleted workspace
/// loses its Telegram bot).
fn watch_bus(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let ids: Vec<String> = app
            .state_db
            .call(|c| {
                let mut st = c.prepare("SELECT session_id FROM telegram_answer")?;
                let rows = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap_or_default();
        PENDING.lock().unwrap().extend(ids.iter().cloned());
        for id in ids {
            let _ = check_reply(&app, &id).await;
        }
        let mut rx = app.bus.subscribe(None).rx;
        loop {
            let ids: Vec<String> = match rx.recv().await {
                Ok(d) => {
                    let v: Value = serde_json::from_str(&d.text).unwrap_or_default();
                    let topic = v["topic"].as_str().unwrap_or("");
                    if topic == "workspaces" {
                        SUPERVISE.notify_one();
                        continue;
                    }
                    if PENDING.lock().unwrap().is_empty() { continue; }
                    let id = if topic == "sessions" && v["type"] == "updated" {
                        v["payload"]["id"].as_str().map(str::to_string)
                    } else if topic.starts_with("session/") && v["payload"]["body"]["kind"] == "status" {
                        topic.strip_prefix("session/").map(str::to_string)
                    } else {
                        None
                    };
                    id.into_iter().filter(|id| PENDING.lock().unwrap().contains(id)).collect()
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    SUPERVISE.notify_one();
                    PENDING.lock().unwrap().iter().cloned().collect()
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            for id in ids {
                if let Err(e) = check_reply(&app, &id).await {
                    tracing::warn!("telegram answer: {e:#}");
                }
            }
        }
    });
}

// ---------------------------------------------------------------- polling

/// Once, at Home's first start with per-workspace Telegram: the single token and chat binding of the time when one
/// Telegram bot served every workspace are deleted, not assigned to a workspace (owner decision). Claimed in
/// `seeded` first.
async fn drop_global_telegram(app: &App) -> Result<()> {
    const KEY: &str = "telegram-per-workspace";
    let claimed = app.state_db.call(|c| Ok(c.execute("INSERT OR IGNORE INTO seeded (key, at) VALUES (?1, ?2)", rusqlite::params![KEY, now_ms()])? == 1)).await?;
    if !claimed {
        return Ok(());
    }
    let had = launchpad_core::vault::secret(app, OLD_TOKEN_ID).await?.is_some();
    launchpad_core::vault::delete(app, OLD_TOKEN_ID).await?;
    app.state_db.call(|c| Ok(c.execute_batch("DROP TABLE IF EXISTS telegram; DROP TABLE IF EXISTS telegram_reply;")?)).await?;
    if had {
        tracing::info!("telegram: the single Telegram bot of every workspace was removed; add one per workspace in Settings → Workspace");
    }
    Ok(())
}

/// Starts a poll loop for each workspace with a Telegram state, and ends the loops of workspaces without one. A
/// deleted workspace's token and chat are removed.
async fn supervise(app: &App) {
    let workspaces: HashSet<String> = match launchpad_core::workspace::list(app).await {
        Ok(l) => l.into_iter().map(|w| w.id).collect(),
        Err(_) => return,
    };
    let Ok(states) = states(app).await else { return };
    for (ws, _) in states.iter().filter(|(ws, _)| !workspaces.contains(ws)) {
        if let Err(e) = remove(app, ws).await {
            tracing::warn!("telegram of a deleted workspace: {e:#}");
        }
    }
    let wanted: HashSet<String> = states.into_iter().map(|(ws, _)| ws).filter(|ws| workspaces.contains(ws)).collect();
    let mut loops = LOOPS.lock().unwrap();
    loops.retain(|ws, wake| {
        let keep = wanted.contains(ws);
        if !keep {
            wake.notify_one();
        }
        keep
    });
    for ws in wanted {
        if loops.contains_key(&ws) {
            continue;
        }
        let wake = Arc::new(tokio::sync::Notify::new());
        loops.insert(ws.clone(), wake.clone());
        poll(app, ws, wake);
    }
}

/// Home: the supervisor of the poll loops, and the watcher of answers to Telegram.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = drop_global_telegram(&app).await {
            tracing::warn!("telegram: dropping the former single Telegram bot: {e:#}");
        }
        watch_bus(&app);
        loop {
            supervise(&app).await;
            tokio::select! { _ = SUPERVISE.notified() => {}, _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {} }
        }
    });
}

/// Whether this loop is still the one of its workspace.
fn current_loop(workspace: &str, wake: &Arc<tokio::sync::Notify>) -> bool {
    LOOPS.lock().unwrap().get(workspace).is_some_and(|w| Arc::ptr_eq(w, wake))
}

/// Home: long-polls Telegram for one workspace while its token is saved.
fn poll(app: &App, workspace: String, wake: Arc<tokio::sync::Notify>) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            if !current_loop(&workspace, &wake) {
                return;
            }
            let Some(token) = token(&app, &workspace).await else {
                tokio::select! { _ = wake.notified() => {}, _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {} }
                continue;
            };
            let st = load(&app, &workspace).await;
            let poll = call(&token, "getUpdates", json!({"offset": st.offset, "timeout": 50, "allowed_updates": ["message"]}), 65);
            let result = tokio::select! {
                r = poll => r,
                // The token changed or was removed: start again.
                _ = wake.notified() => continue,
            };
            match result {
                Ok(updates) => {
                    if st.error.is_some() {
                        let _ = modify(&app, &workspace, |st| st.error = None).await;
                        crate::channels::changed(&app);
                    }
                    for u in updates.as_array().cloned().unwrap_or_default() {
                        let Some(update_id) = u["update_id"].as_i64() else { continue };
                        if !current_loop(&workspace, &wake) {
                            return;
                        }
                        // Claimed before it is handled: a crash never runs a message twice.
                        if !matches!(modify(&app, &workspace, |st| st.offset = st.offset.max(update_id + 1)).await, Ok(Some(_))) {
                            break;
                        }
                        if let Err(e) = handle(&app, &workspace, &u["message"]).await {
                            tracing::warn!("telegram message: {e:#}");
                        }
                    }
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    if load(&app, &workspace).await.error.as_deref() != Some(message.as_str()) {
                        tracing::warn!("telegram ({workspace}): {message}");
                        let _ = modify(&app, &workspace, |st| st.error = Some(message)).await;
                        crate::channels::changed(&app);
                    }
                    tokio::select! { _ = wake.notified() => {}, _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {} }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_commands() {
        assert_eq!(parse("/start ABC123"), Incoming::Start(Some("ABC123".into())));
        assert_eq!(parse("/scout@my_bot look at #12"), Incoming::Bot { name: "scout".into(), text: "look at #12".into() });
        assert_eq!(parse("/bot Scout  look"), Incoming::Bot { name: "scout".into(), text: "look".into() });
        assert_eq!(parse("/scout"), Incoming::Empty("scout".into()));
        assert_eq!(parse("what needs me?"), Incoming::Ask("what needs me?".into()));
        assert_eq!(parse("/help"), Incoming::Help);
        assert!(same_name("repo-scout", "repo_scout") && !same_name("scout", "scouts"));
    }

    fn bot(id: &str, ws: &str, slug: &str, created_at: i64, admin: bool) -> Bot {
        let mut b = Bot {
            id: id.into(),
            workspace_id: ws.into(),
            slug: slug.into(),
            created_at,
            settings: launchpad_core::bots::BotSettings { name: slug.into(), ..Default::default() },
            instructions: String::new(),
            instructions_version: String::new(),
            settings_text: String::new(),
            settings_error: None,
            own_folder: false,
            paused: false,
            version: 0,
            memory: vec![],
            memory_loaded: Default::default(),
            routines: Default::default(),
        };
        if admin {
            b.settings.launchpad = LaunchpadAccess::Admin;
        }
        b
    }

    #[test]
    fn ask_target_is_the_workspace_oldest_admin_bot_else_its_oldest() {
        let bots = vec![bot("w2-ask", "w2", "ask-launchpad", 1, true), bot("w1-scout", "w1", "scout", 2, false), bot("w1-ask", "w1", "ask-launchpad", 3, true), bot("w1-old", "w1", "old", 1, false)];
        assert_eq!(ask_target(&bots, "w1").map(|b| b.id.as_str()), Some("w1-ask"));
        assert_eq!(ask_target(&bots, "w2").map(|b| b.id.as_str()), Some("w2-ask"));
        let plain = vec![bot("a", "w1", "a", 5, false), bot("b", "w1", "b", 4, false)];
        assert_eq!(ask_target(&plain, "w1").map(|b| b.id.as_str()), Some("b"));
        assert!(ask_target(&bots, "w3").is_none());
    }

    #[test]
    fn commands_and_help_stay_in_the_workspace() {
        let bots = vec![bot("w1-ask", "w1", "ask-launchpad", 1, true), bot("w2-ask", "w2", "ask-launchpad", 1, true), bot("w2-scout", "w2", "repo-scout", 2, false)];
        assert_eq!(find_bot(&bots, "w2", "ask_launchpad").map(|b| b.id.as_str()), Some("w2-ask"));
        assert_eq!(find_bot(&bots, "w1", "ask_launchpad").map(|b| b.id.as_str()), Some("w1-ask"));
        assert!(find_bot(&bots, "w1", "repo_scout").is_none());
        let help = help_text(&bots, "w1");
        assert!(help.contains("/ask_launchpad") && !help.contains("repo_scout"), "{help}");
        assert_eq!(help_text(&bots, "w2").matches("/ask_launchpad").count(), 1);
    }

    #[test]
    fn notifications_go_only_to_a_bound_chat_of_the_session_workspace() {
        let bound = State { chat_id: Some(7), notify: true, ..Default::default() };
        assert_eq!(notification_chat(Some(&bound), true), Some(7));
        assert_eq!(notification_chat(Some(&bound), false), None);
        assert_eq!(notification_chat(Some(&State { notify: false, ..bound.clone() }), true), None);
        assert_eq!(notification_chat(Some(&State { notify: true, ..Default::default() }), true), None);
        assert_eq!(notification_chat(None, true), None);
    }

    #[test]
    fn one_workspace_per_telegram_bot() {
        assert_eq!(token_bot_id("123456:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ"), Some(123456));
        assert_eq!(token_bot_id("abc:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ"), None);
        assert_eq!(token_bot_id("123456:short"), None);
        let states = vec![("w1".to_string(), State { bot_id: Some(1), ..Default::default() })];
        assert_eq!(used_elsewhere(&states, "w2", 1), Some("w1"));
        assert_eq!(used_elsewhere(&states, "w1", 1), None);
        assert_eq!(used_elsewhere(&states, "w2", 2), None);
    }
}

//! The owner's Telegram chat, on Home. The owner pastes a bot token from @BotFather (vault entry
//! `channel-telegram`, write-only), then binds their own chat with a single-use code (`/start <code>`). Only that
//! chat is ever answered; other chats are ignored. Home long-polls `getUpdates` (a private Space has no inbound
//! endpoint for Telegram).
//!
//! Messages to the owner's own chat go out directly (owner decision): notifications (the desktop notification events, never
//! code), the final message of turns started from Telegram, and `launchpad send telegram`. Incoming text goes to
//! a bot's thread (`/<bot> text`, `/bot <name> text`), plain text to the first bot; launches from Telegram never allow paid
//! usage. An update is marked handled before it is acted on, so a crash never runs a message twice.

use std::collections::HashSet;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::App;
use crate::model::{Session, SessionStatus};
use crate::util::now_ms;

const TOKEN_ID: &str = "channel-telegram";
const CODE_TTL_MS: i64 = 30 * 60 * 1000;
/// Telegram's limit is 4096 characters.
const MAX_TEXT: usize = 3500;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
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

/// Settings → Channels → Telegram.
#[derive(Clone, Debug, Serialize, TS)]
pub struct TelegramStatus {
    /// A bot token is saved.
    pub configured: bool,
    pub bot_username: Option<String>,
    pub bound: bool,
    pub chat_name: Option<String>,
    /// The binding code while no chat is bound (or a new one was asked for), with its `t.me` link.
    pub code: Option<String>,
    pub link: Option<String>,
    #[ts(type = "number | null")]
    pub code_expires_at: Option<i64>,
    /// Notifications to the chat (the same events as desktop notifications).
    pub notify: bool,
    /// Include session titles in notifications.
    pub titles: bool,
    /// The last polling error, if Telegram is not reachable or refused the token.
    pub error: Option<String>,
}

async fn load(app: &App) -> State {
    app.state_db
        .call(|c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT json FROM telegram WHERE id = 1", [], |r| r.get::<_, String>(0)).optional()?)
        })
        .await
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

async fn store(app: &App, st: &State) -> Result<()> {
    let json = serde_json::to_string(st)?;
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO telegram (id, json) VALUES (1, ?1) ON CONFLICT (id) DO UPDATE SET json = ?1", [json])?)).await?;
    Ok(())
}

async fn token(app: &App) -> Option<String> {
    crate::vault::secret(app, TOKEN_ID).await.ok().flatten().map(|(_, t)| t)
}

fn code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    rand::random::<[u8; 8]>().iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

pub async fn status(app: &App) -> TelegramStatus {
    let st = load(app).await;
    let configured = token(app).await.is_some();
    let code = st.code.clone().filter(|_| st.code_expires_at.is_some_and(|e| e > now_ms()));
    TelegramStatus {
        configured,
        link: match (&st.bot_username, &code) {
            (Some(b), Some(c)) => Some(format!("https://t.me/{b}?start={c}")),
            _ => None,
        },
        code_expires_at: code.as_ref().and(st.code_expires_at),
        code,
        bound: st.chat_id.is_some(),
        bot_username: st.bot_username,
        chat_name: st.chat_name,
        notify: st.notify,
        titles: st.titles,
        error: if configured { st.error } else { None },
    }
}

pub async fn bound(app: &App) -> bool {
    load(app).await.chat_id.is_some() && token(app).await.is_some()
}

static WAKE: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

fn changed(app: &App) {
    WAKE.notify_one();
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

/// Owner click: saves the token after Telegram accepted it, and shows a binding code.
pub async fn set_token(app: &App, token: &str) -> Result<TelegramStatus> {
    crate::channels::ensure_home(app)?;
    let token = token.trim();
    ensure!(token.split_once(':').is_some_and(|(id, rest)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && rest.len() >= 20 && !rest.contains(char::is_whitespace)), "paste the token @BotFather gave you (digits, a colon, then letters)");
    let me = call(token, "getMe", json!({}), 20).await?;
    let username = me["username"].as_str().context("Telegram did not return the bot's name")?.to_string();
    crate::vault::put(app, TOKEN_ID, crate::vault::Kind::Token, "Telegram bot", token).await?;
    let st = State { bot_username: Some(username), code: Some(code()), code_expires_at: Some(now_ms() + CODE_TTL_MS), notify: true, ..Default::default() };
    store(app, &st).await?;
    changed(app);
    Ok(status(app).await)
}

/// A new binding code: the next `/start <code>` binds that chat instead of the current one.
pub async fn new_code(app: &App) -> Result<TelegramStatus> {
    crate::channels::ensure_home(app)?;
    ensure!(token(app).await.is_some(), "add the bot token first");
    let mut st = load(app).await;
    st.code = Some(code());
    st.code_expires_at = Some(now_ms() + CODE_TTL_MS);
    store(app, &st).await?;
    changed(app);
    Ok(status(app).await)
}

#[derive(Deserialize)]
pub struct Patch {
    pub notify: Option<bool>,
    pub titles: Option<bool>,
}

pub async fn patch(app: &App, p: Patch) -> Result<TelegramStatus> {
    crate::channels::ensure_home(app)?;
    let mut st = load(app).await;
    if let Some(v) = p.notify { st.notify = v; }
    if let Some(v) = p.titles { st.titles = v; }
    store(app, &st).await?;
    changed(app);
    Ok(status(app).await)
}

/// Removes the token and the bound chat.
pub async fn remove(app: &App) -> Result<()> {
    crate::channels::ensure_home(app)?;
    crate::vault::delete(app, TOKEN_ID).await?;
    app.state_db.call(|c| Ok(c.execute_batch("DELETE FROM telegram; DELETE FROM telegram_reply;")?)).await?;
    PENDING.lock().unwrap().clear();
    changed(app);
    Ok(())
}

/// Sends text to the owner's chat (plain text, no link previews).
pub(crate) async fn say(app: &App, text: &str) -> Result<()> {
    let st = load(app).await;
    let chat = st.chat_id.context("no Telegram chat is bound: bind yours in Settings → Channels")?;
    let token = token(app).await.context("no Telegram bot token: add it in Settings → Channels")?;
    call(&token, "sendMessage", json!({"chat_id": chat, "text": truncate(text, MAX_TEXT), "disable_web_page_preview": true}), 20).await.map(|_| ())
}

pub async fn test(app: &App) -> Result<()> {
    crate::channels::ensure_home(app)?;
    say(app, "Test message from Launchpad: this chat receives your notifications and answers.").await
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}… (truncated; the full text is in Launchpad)", text.chars().take(max).collect::<String>().trim_end())
}

fn session_link(app: &App, id: &str) -> String {
    format!("{}/agents?session={id}", crate::channels::home_origin(app))
}

fn session_name(s: &Session) -> String {
    match &s.launch_spec.bot {
        Some(b) => b.name.clone(),
        None => s.harness.display().to_string(),
    }
}

/// `launchpad send telegram`: an agent's message to the owner's own chat, sent directly.
pub(crate) async fn send_from_agent(app: &App, session_id: Option<&str>, title: Option<&str>, body: &str) -> Result<()> {
    ensure!(!body.trim().is_empty(), "the message is empty");
    let (from, link) = match session_id {
        Some(id) => (session_name(&app.sessions.get(id).context("session not found")?), format!("\n\n{}", session_link(app, id))),
        None => ("An agent".to_string(), String::new()),
    };
    let head = match title {
        Some(t) => format!("{from}: {t}\n\n"),
        None => format!("{from}:\n"),
    };
    say(app, &format!("{head}{}{link}", truncate(body, MAX_TEXT - 300))).await
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

/// From `notify::session`: an eligible notification. A runner forwards it to Home, which owns the chat.
pub(crate) fn notify(app: &App, s: &Session, event: &'static str) {
    let (app, id) = (app.clone(), s.id.clone());
    tokio::spawn(async move {
        let result = if crate::vault::here(&app) {
            notification(&app, &id, event).await
        } else {
            crate::home_link::home_request("POST", "/api/channels/telegram/notify", Some(json!({"session_id": id, "event": event}))).await.map(|_| ())
        };
        if let Err(e) = result {
            tracing::debug!("telegram notification: {e:#}");
        }
    });
}

/// Home: sends a notification to the bound chat when they are on. Never contains code or messages.
pub async fn notification(app: &App, session_id: &str, event: &str) -> Result<()> {
    crate::channels::ensure_home(app)?;
    let st = load(app).await;
    if !st.notify || st.chat_id.is_none() || PENDING.lock().unwrap().contains(session_id) {
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
    say(app, &format!("{} in {project} · {}{title}\n{}", label(event), session_name(&s), session_link(app, session_id))).await
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

async fn help(app: &App) -> String {
    let bots = crate::bots::list(app).await.unwrap_or_default();
    let list = if bots.is_empty() { "no bots yet".to_string() } else { bots.iter().map(|b| format!("/{} ({})", b.slug.replace('-', "_"), b.settings.name)).collect::<Vec<_>>().join(", ") };
    let first = bots.first().map(|b| format!("Write a message to talk to {}, your first bot. ", b.settings.name)).unwrap_or_default();
    format!("{first}To give a bot a task: /<bot> <text> or /bot <name> <text>. Bots: {list}.")
}

const FROM_TELEGRAM: &str = "\n\n(Sent from Telegram. Your final message of this turn goes back to the owner's chat as plain text: keep it short.)";

async fn handle(app: &App, msg: &Value) -> Result<()> {
    let Some(chat) = msg["chat"]["id"].as_i64() else { return Ok(()) };
    let text = msg["text"].as_str().unwrap_or("");
    let mut st = load(app).await;
    if let Incoming::Start(Some(c)) = parse(text)
        && msg["chat"]["type"] == "private"
        && st.code.as_deref().is_some_and(|code| code.eq_ignore_ascii_case(&c))
        && st.code_expires_at.is_some_and(|e| e > now_ms())
    {
        st.chat_id = Some(chat);
        st.chat_name = msg["chat"]["username"].as_str().map(|u| format!("@{u}")).or_else(|| msg["chat"]["first_name"].as_str().map(str::to_string));
        st.code = None;
        st.code_expires_at = None;
        store(app, &st).await?;
        changed(app);
        return say(app, &format!("This chat is now bound to Launchpad. Only this chat is answered.\n\n{}", help(app).await)).await;
    }
    // Every other chat is ignored, without an answer.
    if st.chat_id != Some(chat) {
        return Ok(());
    }
    if text.trim().is_empty() {
        return say(app, "Launchpad reads text messages only.").await;
    }
    let sent = match parse(text) {
        Incoming::Start(_) | Incoming::Help => return say(app, &help(app).await).await,
        Incoming::Empty(name) => return say(app, &format!("Add the task after the command, for example /{name} check the open issues.")).await,
        // A plain message goes to the first bot (Ask Launchpad of the first workspace, unless the owner deleted it).
        Incoming::Ask(t) => match crate::bots::list(app).await?.into_iter().next() {
            Some(b) => crate::hooks::deliver(app, crate::hooks::Target::Bot { id: b.id.clone() }, format!("{t}{FROM_TELEGRAM}"), false, true).await.map(|d| (b.settings.name.clone(), d)),
            None => return say(app, "There is no bot yet: create one in Launchpad (Bots), then write again.").await,
        },
        Incoming::Bot { name, text } => {
            let bots = crate::bots::list(app).await?;
            match bots.iter().find(|b| same_name(&b.slug, &name) || same_name(&b.settings.name, &name)) {
                Some(b) => crate::hooks::deliver(app, crate::hooks::Target::Bot { id: b.id.clone() }, format!("{text}{FROM_TELEGRAM}"), false, true).await.map(|d| (b.settings.name.clone(), d)),
                None => return say(app, &format!("No bot named {name}. {}", help(app).await)).await,
            }
        }
    };
    match sent {
        Ok((name, d)) if d.queued => {
            let link = d.session.map(|s| format!("\n{}", session_link(app, &s.id))).unwrap_or_default();
            say(app, &format!("Queued for {name}: it is busy; your message is sent when its current turn ends, and the answer comes here.{link}")).await
        }
        Ok((name, d)) => {
            let s = d.session.context("no session")?;
            await_reply(app, &s).await?;
            say(app, &format!("Sent to {name}. Its answer comes here when the turn ends.\n{}", session_link(app, &s.id))).await
        }
        Err(e) => say(app, &format!("Launchpad could not send it: {e:#}")).await,
    }
}

// ---------------------------------------------------------------- answers to the chat

/// Sessions whose current turn came from Telegram (`telegram_reply`): Home sends their final message to the chat.
static PENDING: std::sync::LazyLock<Mutex<HashSet<String>>> = std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) async fn await_reply(app: &App, s: &Session) -> Result<()> {
    let (id, after) = (s.id.clone(), s.last_seq);
    app.state_db
        .call(move |c| Ok(c.execute("INSERT INTO telegram_reply (session_id, after_seq, at) VALUES (?1, ?2, ?3) ON CONFLICT (session_id) DO UPDATE SET after_seq = ?2, at = ?3", rusqlite::params![id, after, now_ms()])?))
        .await?;
    PENDING.lock().unwrap().insert(s.id.clone());
    Ok(())
}

/// When the turn has ended (a non-running status event after the message), the reply is claimed (row deleted)
/// and sent once.
async fn check_reply(app: &App, id: &str) -> Result<()> {
    let key = id.to_string();
    let rows: Option<Vec<(String, String)>> = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let Some(after) = c.query_row("SELECT after_seq FROM telegram_reply WHERE session_id = ?1", [&key], |r| r.get::<_, i64>(0)).optional()? else { return Ok(None) };
            let mut st = c.prepare("SELECT kind, json FROM session_event WHERE session_id = ?1 AND seq > ?2 AND kind IN ('status', 'assistant_message') ORDER BY seq")?;
            let rows = st.query_map(rusqlite::params![key, after], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some(rows))
        })
        .await?;
    let Some(rows) = rows else {
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
    let claimed = app.state_db.call(move |c| Ok(c.execute("DELETE FROM telegram_reply WHERE session_id = ?1", [key])?)).await? > 0;
    PENDING.lock().unwrap().remove(id);
    if !claimed {
        return Ok(());
    }
    let name = session_name(&s);
    let link = session_link(app, id);
    let text = match (status, last) {
        (SessionStatus::Waiting, Some(t)) => format!("{name}:\n{}\n\n{link}", truncate(t.trim(), MAX_TEXT - 300)),
        (SessionStatus::Waiting, None) => format!("{name} finished its turn without a message.\n{link}"),
        (other, Some(t)) => format!("{name} · {}:\n{}\n\n{link}", label_of(other), truncate(t.trim(), MAX_TEXT - 300)),
        (other, None) => format!("{name} · {}.\n{link}", label_of(other)),
    };
    say(app, &text).await
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

/// Watches the bus for the end of turns started from Telegram.
fn watch_replies(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let ids: Vec<String> = app
            .state_db
            .call(|c| {
                let mut st = c.prepare("SELECT session_id FROM telegram_reply")?;
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
                    if PENDING.lock().unwrap().is_empty() { continue; }
                    let v: Value = serde_json::from_str(&d.text).unwrap_or_default();
                    let topic = v["topic"].as_str().unwrap_or("");
                    let id = if topic == "sessions" && v["type"] == "updated" {
                        v["payload"]["id"].as_str().map(str::to_string)
                    } else if topic.starts_with("session/") && v["payload"]["body"]["kind"] == "status" {
                        topic.strip_prefix("session/").map(str::to_string)
                    } else {
                        None
                    };
                    id.into_iter().filter(|id| PENDING.lock().unwrap().contains(id)).collect()
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => PENDING.lock().unwrap().iter().cloned().collect(),
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

/// Home: long-polls Telegram while a token is saved.
pub(crate) fn spawn(app: &App) {
    watch_replies(app);
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let Some(token) = token(&app).await else {
                tokio::select! { _ = WAKE.notified() => {}, _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {} }
                continue;
            };
            let st = load(&app).await;
            let poll = call(&token, "getUpdates", json!({"offset": st.offset, "timeout": 50, "allowed_updates": ["message"]}), 65);
            let result = tokio::select! {
                r = poll => r,
                // The token changed or was removed: start again.
                _ = WAKE.notified() => continue,
            };
            match result {
                Ok(updates) => {
                    if st.error.is_some() {
                        let mut st = load(&app).await;
                        st.error = None;
                        let _ = store(&app, &st).await;
                        crate::channels::changed(&app);
                    }
                    for u in updates.as_array().cloned().unwrap_or_default() {
                        let Some(update_id) = u["update_id"].as_i64() else { continue };
                        // Claimed before it is handled: a crash never runs a message twice.
                        let mut st = load(&app).await;
                        st.offset = st.offset.max(update_id + 1);
                        if store(&app, &st).await.is_err() {
                            break;
                        }
                        if let Err(e) = handle(&app, &u["message"]).await {
                            tracing::warn!("telegram message: {e:#}");
                        }
                    }
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    let mut st = load(&app).await;
                    if st.error.as_deref() != Some(message.as_str()) {
                        tracing::warn!("telegram: {message}");
                        st.error = Some(message);
                        let _ = store(&app, &st).await;
                        crate::channels::changed(&app);
                    }
                    tokio::select! { _ = WAKE.notified() => {}, _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {} }
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
}

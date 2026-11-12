//! Channels on Home: outgoing webhooks (Slack, Discord or a generic JSON endpoint) and the Outbox.
//!
//! A webhook URL is a credential: it is stored in the vault (`channel-<id>`, never listed in Credentials and
//! tools, never lent to sessions) and never shown again. Agents draft messages for other people with
//! `launchpad send <channel>`; the draft waits in the Outbox until the owner clicks Send. A send is claimed
//! (`sending`) before the request and never retried: a send that did not finish locally becomes `uncertain`.
//! Telegram (the owner's own chat of each workspace, direct) is in `telegram.rs`, incoming hooks in `hooks.rs`.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::util::now_ms;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// A Slack incoming webhook: `{"text": …}`.
    Slack,
    /// A Discord webhook: `{"content": …}`.
    Discord,
    /// Any URL receiving JSON: `{"text": …, "title": …, "session_id": …}`.
    Generic,
}

/// An outgoing webhook channel as the UI sees it: never the URL.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Channel {
    pub id: String,
    /// The name agents use: `launchpad send <name>`.
    pub name: String,
    pub kind: ChannelKind,
    /// The webhook's host, for recognition (the path is the secret).
    pub host: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Deserialize)]
pub struct NewChannel {
    pub name: String,
    pub kind: ChannelKind,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    Draft,
    /// Claimed: the request is in flight (or the daemon stopped during it).
    Sending,
    Sent,
    /// The send did not complete locally: check the destination; never retried.
    Uncertain,
    Discarded,
}

/// A message an agent drafted for an outgoing channel.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct OutboxMessage {
    pub id: String,
    pub channel_id: String,
    pub channel: String,
    pub kind: ChannelKind,
    pub title: Option<String>,
    pub body: String,
    pub session_id: Option<String>,
    /// Who drafted it: the bot's name or the session's harness.
    pub author: String,
    #[ts(type = "number")]
    pub created_at: i64,
    pub status: OutboxStatus,
    #[ts(type = "number | null")]
    pub sent_at: Option<i64>,
    /// Why the last send failed (the destination refused it, or it is uncertain).
    pub error: Option<String>,
}

/// `launchpad send`: a message for a channel, from a session.
#[derive(Deserialize)]
pub struct Draft {
    pub channel: String,
    pub body: String,
    pub title: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Serialize)]
pub struct DraftResult {
    /// The Outbox draft (webhook channels).
    pub outbox: Option<OutboxMessage>,
    /// Sent directly (Telegram, the owner's own chat).
    pub sent: bool,
    pub message: String,
}

#[derive(Deserialize)]
pub struct Edit {
    pub body: String,
    pub title: Option<String>,
}

pub(crate) fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::vault::here(app), "channels are kept on Home: change them there");
    Ok(())
}

/// Channel names are what agents type: lowercase letters, digits and dashes. `telegram` is the owner's chat of
/// the session's workspace.
pub fn check_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.starts_with('-'), "a channel name uses lowercase letters, digits and dashes (at most 40)");
    ensure!(name != "telegram", "telegram is the name of your own Telegram chat");
    Ok(())
}

/// Checks a webhook URL for its kind and returns its host.
pub fn check_url(kind: ChannelKind, url: &str) -> Result<String> {
    let u = reqwest::Url::parse(url.trim()).context("this is not a valid URL")?;
    ensure!(u.scheme() == "https", "the webhook URL must start with https://");
    let host = u.host_str().context("the URL has no host")?.to_ascii_lowercase();
    match kind {
        ChannelKind::Slack => ensure!(host == "hooks.slack.com" && u.path().starts_with("/services/"), "a Slack incoming webhook starts with https://hooks.slack.com/services/"),
        ChannelKind::Discord => ensure!(matches!(host.as_str(), "discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com") && u.path().starts_with("/api/webhooks/"), "a Discord webhook starts with https://discord.com/api/webhooks/"),
        ChannelKind::Generic => {}
    }
    Ok(host)
}

/// Discord refuses longer messages; the others accept much more.
fn max_body(kind: ChannelKind) -> usize {
    if kind == ChannelKind::Discord { 2000 } else { 30_000 }
}

fn check_body(kind: ChannelKind, body: &str, title: Option<&str>) -> Result<()> {
    ensure!(!body.trim().is_empty(), "the message is empty");
    let total = body.chars().count() + title.map(|t| t.chars().count() + 4).unwrap_or(0);
    ensure!(total <= max_body(kind), "the message is too long for this channel (at most {} characters)", max_body(kind));
    ensure!(title.is_none_or(|t| t.chars().count() <= 200 && !t.contains('\n')), "a title is one line of at most 200 characters");
    Ok(())
}

/// The JSON body of a webhook request.
pub fn payload(kind: ChannelKind, title: Option<&str>, body: &str, session_id: Option<&str>) -> Value {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    match kind {
        ChannelKind::Slack => json!({"text": match title { Some(t) => format!("*{t}*\n{body}"), None => body.to_string() }}),
        // No mentions: a draft never pings @everyone or roles.
        ChannelKind::Discord => json!({"content": match title { Some(t) => format!("**{t}**\n{body}"), None => body.to_string() }, "allowed_mentions": {"parse": []}}),
        ChannelKind::Generic => json!({"text": body, "title": title, "session_id": session_id}),
    }
}

/// A message ending with the attribution line of the agent that drafted it, in the channel's link syntax.
pub fn signed(kind: ChannelKind, body: &str, line: &str) -> String {
    use launchpad_core::attribution as a;
    let line = if kind == ChannelKind::Slack { a::slack(line) } else { line.to_string() };
    a::sign(body, &line, a::Separator::Blank)
}

/// Home's address in links sent to chats: the configured public origin, the Space's host, or the local port.
pub fn home_origin(app: &App) -> String {
    if let Ok(Some(o)) = app.config.get().server.origin() {
        return o;
    }
    if let Ok(h) = std::env::var("SPACE_HOST")
        && !h.trim().is_empty()
    {
        return format!("https://{}", h.trim());
    }
    app.paths.base_url()
}

fn secret_id(id: &str) -> String {
    format!("channel-{id}")
}

// ---------------------------------------------------------------- channels

pub async fn list(app: &App) -> Result<Vec<Channel>> {
    let mut all: Vec<Channel> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM channel")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    all.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(all)
}

pub async fn add(app: &App, n: NewChannel) -> Result<Channel> {
    ensure_home(app)?;
    let name = n.name.trim().to_ascii_lowercase();
    check_name(&name)?;
    let host = check_url(n.kind, &n.url)?;
    ensure!(!list(app).await?.iter().any(|c| c.name == name), "a channel named {name} already exists");
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    launchpad_core::vault::put(app, &secret_id(&id), launchpad_core::vault::Kind::Token, &format!("Channel {name}"), n.url.trim()).await?;
    let ch = Channel { id: id.clone(), name, kind: n.kind, host, created_at: now_ms() };
    let json = serde_json::to_string(&ch)?;
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO channel (id, json) VALUES (?1, ?2)", [id, json])?)).await?;
    changed(app);
    Ok(ch)
}

/// Removes a channel and its URL. Its drafts stay in the Outbox but cannot be sent any more.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    launchpad_core::vault::delete(app, &secret_id(id)).await?;
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM channel WHERE id = ?1", [key])?)).await?;
    changed(app);
    Ok(())
}

/// Channel names for the session prompt, kept in memory (the prompt is built synchronously).
static NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn changed(app: &App) {
    app.bus.publish("channels", "changed", Value::Null);
    let app = app.clone();
    tokio::spawn(async move { refresh_names(&app).await });
}

async fn refresh_names(app: &App) {
    let names = if launchpad_core::vault::here(app) {
        let mut names: Vec<String> = list(app).await.map(|l| l.into_iter().map(|c| c.name).collect()).unwrap_or_default();
        if crate::telegram::any_bound(app).await {
            names.insert(0, "telegram".into());
        }
        names
    } else {
        // A runner asks Home (its sessions' prompts name the channels too).
        match launchpad_core::home_link::home_request("GET", "/api/channels/names", None).await {
            Ok(v) => v["names"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            Err(_) => return,
        }
    };
    *NAMES.lock().unwrap() = names;
}

/// Channel names agents can use, for `GET /api/channels`.
pub async fn names(app: &App) -> Vec<String> {
    refresh_names(app).await;
    NAMES.lock().unwrap().clone()
}

/// The part of a session's prompt about channels, when there is one.
pub fn instruction(app: &App, session: &str) -> Option<String> {
    let names = NAMES.lock().unwrap().clone();
    if names.is_empty() {
        return None;
    }
    let cli = app.paths.cli_command();
    Some(format!(
        "To send a message through one of the owner's channels ({}), write it to a file and run `{cli} send <channel> --body-file <file> [--title \"…\"] --session-id {session}`. `telegram` goes directly to the owner's own chat of your workspace (when it has one); other channels reach other people, so the message waits in the owner's Outbox until they click Send. Send messages only when the task asks for it.",
        names.join(", ")
    ))
}

// ---------------------------------------------------------------- outbox

pub async fn outbox(app: &App) -> Result<Vec<OutboxMessage>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM outbox_message ORDER BY rowid DESC LIMIT 1000")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await
}

async fn get(app: &App, id: &str) -> Result<OutboxMessage> {
    let key = id.to_string();
    let json: Option<String> = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT json FROM outbox_message WHERE id = ?1", [key], |r| r.get(0)).optional()?)
        })
        .await?;
    Ok(serde_json::from_str(&json.context("no such Outbox message")?)?)
}

async fn save(app: &App, m: &OutboxMessage) -> Result<()> {
    let (id, json) = (m.id.clone(), serde_json::to_string(m)?);
    app.state_db
        .call(move |c| Ok(c.execute("INSERT INTO outbox_message (id, json) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET json = ?2", [id, json])?))
        .await?;
    app.bus.publish("outbox", "changed", serde_json::to_value(m)?);
    Ok(())
}

/// Who drafted a message, from Home's session record.
pub(crate) fn author(app: &App, session_id: Option<&str>) -> Result<String> {
    Ok(match session_id {
        Some(id) => {
            let s = app.sessions.get(id).context("session not found")?;
            s.launch_spec.bot.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| s.harness.display().to_string())
        }
        None => "Agent".into(),
    })
}

/// `launchpad send`: a draft in the Outbox for a webhook channel, or a direct message to the Telegram chat of the
/// session's workspace.
pub async fn draft(app: &App, d: Draft) -> Result<DraftResult> {
    ensure_home(app)?;
    let name = d.channel.trim().to_ascii_lowercase();
    let title = d.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if name == "telegram" {
        crate::telegram::send_from_agent(app, d.session_id.as_deref(), title.as_deref(), &d.body).await?;
        return Ok(DraftResult { outbox: None, sent: true, message: "Sent to the Telegram chat of the session's workspace.".into() });
    }
    let channels = list(app).await?;
    let Some(ch) = channels.iter().find(|c| c.name == name) else {
        let names: Vec<_> = channels.iter().map(|c| c.name.as_str()).collect();
        bail!("no channel named {name}; channels: {}", if names.is_empty() { "none (the owner adds them in Settings → Channels)".to_string() } else { names.join(", ") });
    };
    check_body(ch.kind, &d.body, title.as_deref())?;
    let m = OutboxMessage {
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: ch.id.clone(),
        channel: ch.name.clone(),
        kind: ch.kind,
        title,
        body: d.body,
        author: author(app, d.session_id.as_deref())?,
        session_id: d.session_id,
        created_at: now_ms(),
        status: OutboxStatus::Draft,
        sent_at: None,
        error: None,
    };
    save(app, &m).await?;
    Ok(DraftResult { message: format!("Saved in the owner's Outbox (id {}). Nothing was sent: the owner reviews it and clicks Send.", m.id), outbox: Some(m), sent: false })
}

fn lock(app: &App, id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    app.suggestion_locks.get(&format!("outbox:{id}"))
}

pub async fn edit(app: &App, id: &str, e: Edit) -> Result<OutboxMessage> {
    ensure_home(app)?;
    let l = lock(app, id);
    let _g = l.lock().await;
    let mut m = get(app, id).await?;
    ensure!(m.status == OutboxStatus::Draft, "only drafts can be edited");
    let title = e.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    check_body(m.kind, &e.body, title.as_deref())?;
    m.body = e.body;
    m.title = title;
    save(app, &m).await?;
    Ok(m)
}

pub async fn discard(app: &App, id: &str) -> Result<OutboxMessage> {
    ensure_home(app)?;
    let l = lock(app, id);
    let _g = l.lock().await;
    let mut m = get(app, id).await?;
    ensure!(matches!(m.status, OutboxStatus::Draft | OutboxStatus::Uncertain), "only drafts and uncertain messages can be discarded");
    m.status = OutboxStatus::Discarded;
    save(app, &m).await?;
    Ok(m)
}

/// The owner checked the destination of an uncertain message and found it there.
pub async fn mark_sent(app: &App, id: &str) -> Result<OutboxMessage> {
    ensure_home(app)?;
    let l = lock(app, id);
    let _g = l.lock().await;
    let mut m = get(app, id).await?;
    ensure!(m.status == OutboxStatus::Uncertain, "only uncertain messages can be marked as sent");
    m.status = OutboxStatus::Sent;
    m.sent_at = Some(now_ms());
    save(app, &m).await?;
    Ok(m)
}

/// Owner click: one request to the channel's webhook. Never retried.
pub async fn send(app: &App, id: &str) -> Result<OutboxMessage> {
    ensure_home(app)?;
    let l = lock(app, id);
    let _g = l.lock().await;
    let mut m = get(app, id).await?;
    ensure!(m.status == OutboxStatus::Draft, "a send was already attempted; check the destination before discarding");
    let (_, url) = launchpad_core::vault::secret(app, &secret_id(&m.channel_id)).await?.context("this channel was removed: discard the message")?;
    let text = match launchpad_core::attribution::for_session(app, m.session_id.as_deref()) {
        Some(line) => signed(m.kind, &m.body, &line),
        None => m.body.clone(),
    };
    let body = payload(m.kind, m.title.as_deref(), &text, m.session_id.as_deref());
    m.status = OutboxStatus::Sending;
    m.error = None;
    save(app, &m).await?;
    let result = http().post(url.trim()).json(&body).send().await;
    match result {
        Ok(r) if r.status().is_success() => {
            m.status = OutboxStatus::Sent;
            m.sent_at = Some(now_ms());
        }
        // A clear refusal: nothing was delivered, the draft stays editable.
        Ok(r) if r.status().is_client_error() => {
            let status = r.status();
            let text: String = r.text().await.unwrap_or_default().chars().take(200).collect();
            m.status = OutboxStatus::Draft;
            m.error = Some(format!("The channel refused the message ({status}){}", if text.trim().is_empty() { String::new() } else { format!(": {}", text.trim()) }));
        }
        Ok(r) => {
            m.status = OutboxStatus::Uncertain;
            m.error = Some(format!("The channel answered {}. Check the destination before discarding; this message is not sent again.", r.status()));
        }
        Err(e) if e.is_connect() => {
            m.status = OutboxStatus::Draft;
            m.error = Some(format!("Could not reach {}: nothing was sent.", e.url().and_then(|u| u.host_str()).unwrap_or("the channel")));
        }
        Err(_) => {
            m.status = OutboxStatus::Uncertain;
            m.error = Some("The send did not complete. Check the destination before discarding; this message is not sent again.".into());
        }
    }
    save(app, &m).await?;
    if let Some(e) = m.error.as_ref().filter(|_| m.status != OutboxStatus::Sent) {
        bail!("{e}");
    }
    Ok(m)
}

pub(crate) fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent("launchpad").build().unwrap_or_default()
}

/// Home start: a send that was in flight when the daemon stopped is uncertain, never retried.
pub fn spawn(app: &App) {
    let app = app.clone();
    if !launchpad_core::vault::here(&app) {
        tokio::spawn(async move {
            loop {
                refresh_names(&app).await;
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
        return;
    }
    tokio::spawn(async move {
        if let Ok(all) = outbox(&app).await {
            for mut m in all.into_iter().filter(|m| m.status == OutboxStatus::Sending) {
                m.status = OutboxStatus::Uncertain;
                m.error = Some("Launchpad stopped during the send. Check the destination before discarding; this message is not sent again.".into());
                let _ = save(&app, &m).await;
            }
        }
        refresh_names(&app).await;
        crate::telegram::spawn(&app);
        crate::hooks::spawn(&app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_follow_each_kind() {
        assert_eq!(payload(ChannelKind::Slack, Some("Build"), "ok", None), json!({"text": "*Build*\nok"}));
        assert_eq!(payload(ChannelKind::Discord, None, "ok", None)["content"], "ok");
        assert_eq!(payload(ChannelKind::Generic, Some(" "), "ok", Some("s1")), json!({"text": "ok", "title": null, "session_id": "s1"}));
    }

    #[test]
    fn messages_end_with_the_attribution_line_once() {
        let line = launchpad_core::attribution::text(Some("HFJS Bot"), "Claude Code");
        let slack = signed(ChannelKind::Slack, "Released.", &line);
        assert_eq!(slack, "Released.\n\n_Written by HFJS Bot via <https://github.com/Wauplin/launchpad|Launchpad>_");
        assert_eq!(signed(ChannelKind::Slack, &slack, &line), slack);
        assert_eq!(signed(ChannelKind::Discord, "Released.", &line), format!("Released.\n\n{line}"));
    }

    #[test]
    fn urls_and_names_are_checked() {
        assert!(check_url(ChannelKind::Slack, "https://hooks.slack.com/services/T/B/x").is_ok());
        assert!(check_url(ChannelKind::Slack, "https://example.com/services/x").is_err());
        assert!(check_url(ChannelKind::Discord, "https://discord.com/api/webhooks/1/x").is_ok());
        assert!(check_url(ChannelKind::Generic, "http://example.com/x").is_err());
        assert!(check_name("team-chat").is_ok());
        assert!(check_name("telegram").is_err() && check_name("Team").is_err());
        assert!(check_body(ChannelKind::Discord, &"x".repeat(2001), None).is_err());
    }
}

//! Bot channels and DMs on Home: named conversations of a workspace whose members are bots (the owner is a member of
//! every one), and DMs, a channel of exactly two bots per pair (`dm:<bot id>:<bot id>`, sorted), created on first
//! use. Messages are short records (Markdown, at most 8 KB), not sessions. Home stamps every author from its own
//! records: the owner's window, or a bot's session (`LaunchSpec.bot`) that is a member; any other agent is refused.
//! Each post is delivered to the member bots that should react (`delivery`), through the bot message path. A bot
//! speaks in a channel only by posting (`launchpad channel post`, `launchpad dm`): nothing at a turn's end posts.
//! Channels never cross workspaces.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::util::now_ms;

/// The longest message text, in bytes.
pub const MAX_TEXT: usize = 8 * 1024;
/// The mention and author id of the owner (`@owner`, `@you`).
pub const OWNER: &str = "owner";
/// What Allow more adds to a channel's allowance.
pub const ALLOW_MORE: u32 = 10;
pub const DEFAULT_HOP_LIMIT: u32 = 3;
pub const DEFAULT_HOURLY_LIMIT: u32 = 30;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ChannelKind")]
pub enum Kind {
    Channel,
    Dm,
}

/// When a member bot is woken: every message, only when mentioned, once a day with a digest, or never.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ChannelMemberMode")]
pub enum Mode {
    All,
    Mentions,
    Digest,
    Muted,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ChannelAuthorKind")]
pub enum AuthorKind {
    Owner,
    Bot,
    External,
}

fn text_of<T: Serialize>(v: T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}
fn parse<T: for<'de> Deserialize<'de>>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_string())).ok()
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "ChannelMember")]
pub struct Member {
    pub bot_id: String,
    /// The bot's name and slug now ("deleted bot" when it is gone).
    pub name: String,
    pub slug: String,
    pub mode: Mode,
    /// The last message the bot read.
    #[ts(type = "number")]
    pub last_read: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct Channel {
    pub id: String,
    pub workspace_id: String,
    pub kind: Kind,
    /// A named channel's name (`release-train`); a DM's pair key (`dm:<bot id>:<bot id>`).
    pub name: String,
    /// What the UI shows: "#release-train", or "Maintainer ↔ Release" for a DM.
    pub title: String,
    pub purpose: String,
    /// No delivery while paused; messages still post.
    pub paused: bool,
    /// A DM the owner blocked: its bots cannot post.
    pub blocked: bool,
    pub hop_limit: u32,
    pub hourly_limit: u32,
    /// Deliveries the limits would skip that still go (Allow more).
    pub allowance: u32,
    /// The owner's last message: the hourly count starts there.
    #[ts(type = "number")]
    pub reset_at: i64,
    pub archived: bool,
    /// "owner", "bot:<id>" or "session:<id>".
    pub created_by: String,
    #[ts(type = "number")]
    pub created_at: i64,
    /// The last message the owner read.
    #[ts(type = "number")]
    pub owner_read: i64,
    pub members: Vec<Member>,
    #[ts(type = "number | null")]
    pub last_message: Option<i64>,
    #[ts(type = "number | null")]
    pub last_at: Option<i64>,
    /// The newest message for lists: its author ("you" for the owner) and its first line, cut to 160 characters.
    pub last_by: Option<String>,
    pub last_text: Option<String>,
    /// Messages after `owner_read`, and those of them that mention the owner or ask the owner (an amber count).
    pub unread: u32,
    pub attention: u32,
    /// Bot messages in the hourly window (since an hour ago, or the owner's last message when later), and the newest
    /// message's hop: the loop-guard footer.
    pub hour_count: u32,
    pub last_hops: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "ChannelAuthor")]
pub struct Author {
    pub kind: AuthorKind,
    /// "owner", the bot's id, or the outside source.
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ChannelMessage {
    #[ts(type = "number")]
    pub id: i64,
    pub channel_id: String,
    pub author: Author,
    /// The bot session and the turn (the `seq` of the message its turn answers) that wrote it.
    pub session_id: Option<String>,
    #[ts(type = "number | null")]
    pub turn: Option<i64>,
    pub text: String,
    /// The bots it mentions (ids), and "owner".
    pub mentions: Vec<String>,
    #[ts(type = "number | null")]
    pub reply_to: Option<i64>,
    pub hops: u32,
    pub ask_owner: bool,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ChannelDeliveryState")]
pub enum DeliveryState {
    /// Claimed: being sent, or waiting for the bot's turn to end (or its quota).
    Queued,
    Sent,
    /// Not delivered, with the reason.
    Skipped,
    /// Joined an earlier delivery of the same channel that still waited (`joined`).
    Combined,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ChannelDelivery {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub message_id: i64,
    pub bot_id: String,
    pub state: DeliveryState,
    /// Why it was skipped, or why it waits.
    pub reason: Option<String>,
    /// The session it went to.
    pub session_id: Option<String>,
    #[ts(type = "number | null")]
    pub joined: Option<i64>,
    #[ts(type = "number")]
    pub at: i64,
    /// A delivery of one message (and the ones that joined it), or a member's daily digest (`digest`), shown under
    /// the newest message it lists (`message_id`).
    pub kind: DeliveryKind,
    /// A digest's first listed message and how many it lists.
    #[ts(type = "number | null")]
    pub first_message: Option<i64>,
    pub count: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ChannelDeliveryKind")]
pub enum DeliveryKind {
    Message,
    Digest,
}

/// A channel with a page of its messages (oldest first) and their deliveries.
#[derive(Serialize, TS)]
pub struct ChannelPage {
    pub channel: Channel,
    pub messages: Vec<ChannelMessage>,
    pub deliveries: Vec<ChannelDelivery>,
    /// Older messages exist before this page.
    pub more: bool,
}

/// A posted message and its deliveries.
#[derive(Serialize, TS)]
pub struct ChannelPosted {
    pub message: ChannelMessage,
    pub deliveries: Vec<ChannelDelivery>,
}

// ---------------------------------------------------------------- records

/// A channel row, without its members.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub id: String,
    pub workspace_id: String,
    pub kind: Kind,
    pub name: String,
    pub purpose: String,
    pub paused: bool,
    pub blocked: bool,
    pub hop_limit: u32,
    pub hourly_limit: u32,
    pub allowance: u32,
    pub reset_at: i64,
    pub archived: bool,
    pub created_by: String,
    pub created_at: i64,
    pub owner_read: i64,
}

pub(crate) const ROW: &str = "id, workspace_id, kind, name, purpose, paused, blocked, hop_limit, hourly_limit, allowance, reset_at, archived, created_by, created_at, owner_read";

pub(crate) fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        kind: parse(&r.get::<_, String>(2)?).unwrap_or(Kind::Channel),
        name: r.get(3)?,
        purpose: r.get(4)?,
        paused: r.get(5)?,
        blocked: r.get(6)?,
        hop_limit: r.get(7)?,
        hourly_limit: r.get(8)?,
        allowance: r.get(9)?,
        reset_at: r.get(10)?,
        archived: r.get(11)?,
        created_by: r.get(12)?,
        created_at: r.get(13)?,
        owner_read: r.get(14)?,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct MemberRow {
    pub bot_id: String,
    pub mode: Mode,
    pub last_read: i64,
}

/// What a channel needs of a bot.
#[derive(Clone, Debug)]
pub(crate) struct BotInfo {
    pub name: String,
    pub slug: String,
    pub workspace_id: String,
    pub paused: bool,
}

pub(crate) async fn bots(app: &App) -> Result<HashMap<String, BotInfo>> {
    Ok(launchpad_core::bots::list(app)
        .await?
        .into_iter()
        .map(|b| (b.id.clone(), BotInfo { name: b.settings.name.clone(), slug: b.slug.clone(), workspace_id: b.workspace_id.clone(), paused: b.paused }))
        .collect())
}

pub(crate) fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::vault::here(app), "bot channels are kept on Home");
    Ok(())
}

pub(crate) async fn load(app: &App, id: &str) -> Result<Row> {
    let id = id.to_string();
    app.state_db
        .call(move |c| Ok(c.query_row(&format!("SELECT {ROW} FROM channel WHERE id = ?1"), [id], row).optional()?))
        .await?
        .context("no such channel")
}

pub(crate) async fn members_of(app: &App, id: &str) -> Result<Vec<MemberRow>> {
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT bot_id, mode, last_read FROM channel_member WHERE channel_id = ?1 ORDER BY bot_id")?;
            let rows = st
                .query_map([id], |r| Ok(MemberRow { bot_id: r.get(0)?, mode: parse(&r.get::<_, String>(1)?).unwrap_or(Mode::Mentions), last_read: r.get(2)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

/// The start of the hourly window: an hour ago, or the owner's last message when it is later (an owner message
/// resets the count).
pub fn window_start(now: i64, reset_at: i64) -> i64 {
    (now - 3_600_000).max(reset_at)
}

/// Bot messages of the channel since `since`.
pub(crate) async fn bot_messages_since(app: &App, id: &str, since: i64) -> Result<u32> {
    let id = id.to_string();
    app.state_db
        .call(move |c| Ok(c.query_row("SELECT count(*) FROM channel_message WHERE channel_id = ?1 AND author_kind = 'bot' AND created_at >= ?2", params![id, since], |r| r.get(0))?))
        .await
}

/// A DM's pair key: the two bot IDs sorted, so both bots find the same DM.
pub fn dm_key(a: &str, b: &str) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    format!("dm:{lo}:{hi}")
}

/// A named channel's name: lowercase letters, digits and dashes, starting with a letter or digit, at most 40.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && b.len() <= 40 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The UI's title: "#name", or "A ↔ B" for a DM (its bots' names now, sorted).
fn title(r: &Row, members: &[Member]) -> String {
    match r.kind {
        Kind::Channel => format!("#{}", r.name),
        Kind::Dm => {
            let mut names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
            names.sort_unstable();
            names.join(" ↔ ")
        }
    }
}

/// The text's mentions among `slugs` (bot slug → id): `@<slug>` at the start or after a character that is not part
/// of a name, outside code spans and fenced blocks; `@owner` and `@you` mention the owner. Each once, in order.
pub fn mentions(text: &str, slugs: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut code = false;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '`' {
                code = !code;
            } else if c == '@' && !code && (i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '_' | '-' | '.' | '@'))) {
                let word: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
                let word = word.trim_end_matches(['-', '_']).to_ascii_lowercase();
                let hit = if word == OWNER || word == "you" { Some(OWNER.to_string()) } else { slugs.get(&word).cloned() };
                if let Some(h) = hit
                    && !out.contains(&h)
                {
                    out.push(h);
                }
                i += 1 + word.len();
                continue;
            }
            i += 1;
        }
    }
    out
}

/// A message text: not empty, at most `MAX_TEXT` bytes.
pub fn check_text(text: &str) -> Result<()> {
    ensure!(!text.trim().is_empty(), "the message is empty");
    ensure!(text.len() <= MAX_TEXT, "the message is {} bytes; a channel message holds at most 8 KB: post a summary and put the rest in a file or the knowledge base", text.len());
    Ok(())
}

pub(crate) fn changed(app: &App, id: &str, workspace: &str) {
    app.bus.publish("channels", "changed", json!({"id": id, "workspace_id": workspace}));
}

// ---------------------------------------------------------------- views

async fn view(app: &App, r: Row, bots: &HashMap<String, BotInfo>) -> Result<Channel> {
    let members: Vec<Member> = members_of(app, &r.id)
        .await?
        .into_iter()
        .map(|m| {
            let b = bots.get(&m.bot_id);
            Member { name: b.map_or("deleted bot".into(), |b| b.name.clone()), slug: b.map(|b| b.slug.clone()).unwrap_or_default(), bot_id: m.bot_id, mode: m.mode, last_read: m.last_read }
        })
        .collect();
    let (id, read, since) = (r.id.clone(), r.owner_read, window_start(now_ms(), r.reset_at));
    type Last = (i64, i64, String, String, String);
    let (last, unread, attention, hour, hops): (Option<Last>, u32, u32, u32, u32) = app
        .state_db
        .call(move |c| {
            let last = c
                .query_row("SELECT id, created_at, author_kind, author_name, text FROM channel_message WHERE channel_id = ?1 ORDER BY id DESC LIMIT 1", [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
                .optional()?;
            let unread = c.query_row("SELECT count(*) FROM channel_message WHERE channel_id = ?1 AND id > ?2", params![id, read], |r| r.get(0))?;
            let attention = c.query_row(
                "SELECT count(*) FROM channel_message WHERE channel_id = ?1 AND id > ?2 AND author_kind != 'owner' AND (ask_owner = 1 OR EXISTS (SELECT 1 FROM json_each(mentions) WHERE value = 'owner'))",
                params![id, read],
                |r| r.get(0),
            )?;
            let hour = c.query_row("SELECT count(*) FROM channel_message WHERE channel_id = ?1 AND author_kind = 'bot' AND created_at >= ?2", params![id, since], |r| r.get(0))?;
            let hops = c.query_row("SELECT hops FROM channel_message WHERE channel_id = ?1 ORDER BY id DESC LIMIT 1", [&id], |r| r.get(0)).optional()?.unwrap_or(0);
            Ok((last, unread, attention, hour, hops))
        })
        .await?;
    Ok(Channel {
        title: title(&r, &members),
        id: r.id,
        workspace_id: r.workspace_id,
        kind: r.kind,
        name: r.name,
        purpose: r.purpose,
        paused: r.paused,
        blocked: r.blocked,
        hop_limit: r.hop_limit,
        hourly_limit: r.hourly_limit,
        allowance: r.allowance,
        reset_at: r.reset_at,
        archived: r.archived,
        created_by: r.created_by,
        created_at: r.created_at,
        owner_read: r.owner_read,
        members,
        last_message: last.as_ref().map(|l| l.0),
        last_at: last.as_ref().map(|l| l.1),
        last_by: last.as_ref().map(|l| if l.2 == "owner" { "you".to_string() } else { l.3.clone() }),
        last_text: last.as_ref().map(|l| preview(&l.4)),
        unread,
        attention,
        hour_count: hour,
        last_hops: hops,
    })
}

/// A message's first non-empty line, cut to 160 characters: what a list row shows.
pub fn preview(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > 160 { format!("{}…", line.chars().take(159).collect::<String>()) } else { line.to_string() }
}

/// The channels and DMs of a workspace (every workspace when None), newest activity first.
pub async fn list(app: &App, workspace: Option<&str>) -> Result<Vec<Channel>> {
    ensure_home(app)?;
    let ws = workspace.map(str::to_string);
    let rows: Vec<Row> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare(&format!("SELECT {ROW} FROM channel WHERE ?1 IS NULL OR workspace_id = ?1"))?;
            let rows = st.query_map([ws], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let bots = bots(app).await?;
    let mut out = vec![];
    for r in rows {
        out.push(view(app, r, &bots).await?);
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.last_at.unwrap_or(c.created_at)));
    Ok(out)
}

pub async fn get(app: &App, id: &str) -> Result<Channel> {
    ensure_home(app)?;
    let r = load(app, id).await?;
    view(app, r, &bots(app).await?).await
}

pub(crate) fn message_row(r: &rusqlite::Row) -> rusqlite::Result<ChannelMessage> {
    Ok(ChannelMessage {
        id: r.get(0)?,
        channel_id: r.get(1)?,
        author: Author { kind: parse(&r.get::<_, String>(2)?).unwrap_or(AuthorKind::External), id: r.get(3)?, name: r.get(4)? },
        session_id: r.get(5)?,
        turn: r.get(6)?,
        text: r.get(7)?,
        mentions: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
        reply_to: r.get(9)?,
        hops: r.get(10)?,
        ask_owner: r.get(11)?,
        created_at: r.get(12)?,
    })
}
pub(crate) const MESSAGE: &str = "id, channel_id, author_kind, author_id, author_name, session_id, turn, text, mentions, reply_to, hops, ask_owner, created_at";

/// Messages of a channel, oldest first: after `since` (the oldest first), or else the newest ones before `before`;
/// at most `limit` (1 to 200, 50 by default). Also says whether older ones exist.
pub async fn messages(app: &App, id: &str, since: Option<i64>, before: Option<i64>, limit: Option<u32>) -> Result<(Vec<ChannelMessage>, bool)> {
    ensure_home(app)?;
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            let mut list: Vec<ChannelMessage> = if let Some(s) = since {
                let mut st = c.prepare(&format!("SELECT {MESSAGE} FROM channel_message WHERE channel_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3"))?;
                st.query_map(params![id, s, limit], message_row)?.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                let mut st = c.prepare(&format!("SELECT {MESSAGE} FROM channel_message WHERE channel_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"))?;
                let mut l = st.query_map(params![id, before.unwrap_or(i64::MAX), limit], message_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
                l.reverse();
                l
            };
            list.sort_by_key(|m| m.id);
            let more = match list.first() {
                Some(m) => c.query_row("SELECT 1 FROM channel_message WHERE channel_id = ?1 AND id < ?2 LIMIT 1", params![id, m.id], |_| Ok(())).optional()?.is_some(),
                None => false,
            };
            Ok((list, more))
        })
        .await
}

/// The deliveries of these messages.
pub async fn deliveries(app: &App, message_ids: Vec<i64>) -> Result<Vec<ChannelDelivery>> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    app.state_db
        .call(move |c| {
            let ids = serde_json::to_string(&message_ids).unwrap_or_default();
            let mut st = c.prepare("SELECT id, message_id, bot_id, state, reason, session_id, joined, at, kind, first_message, count FROM channel_delivery WHERE message_id IN (SELECT value FROM json_each(?1)) ORDER BY id")?;
            let rows = st
                .query_map([ids], |r| {
                    Ok(ChannelDelivery {
                        id: r.get(0)?,
                        message_id: r.get(1)?,
                        bot_id: r.get(2)?,
                        state: parse(&r.get::<_, String>(3)?).unwrap_or(DeliveryState::Skipped),
                        reason: r.get(4)?,
                        session_id: r.get(5)?,
                        joined: r.get(6)?,
                        at: r.get(7)?,
                        kind: parse(&r.get::<_, String>(8)?).unwrap_or(DeliveryKind::Message),
                        first_message: r.get(9)?,
                        count: r.get(10)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

/// Whether the message is one of the channel's.
pub async fn has_message(app: &App, id: &str, message: i64) -> Result<bool> {
    let cid = id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT 1 FROM channel_message WHERE id = ?1 AND channel_id = ?2", params![message, cid], |_| Ok(())).optional()?.is_some())).await
}

/// A channel page for the owner's window: the channel, a page of messages and their deliveries.
pub async fn page(app: &App, id: &str, before: Option<i64>, limit: Option<u32>) -> Result<ChannelPage> {
    let channel = get(app, id).await?;
    let (messages, more) = messages(app, id, None, before, limit).await?;
    let deliveries = deliveries(app, messages.iter().map(|m| m.id).collect()).await?;
    Ok(ChannelPage { channel, messages, deliveries, more })
}

// ---------------------------------------------------------------- posting

/// Who posts: the owner (a window), or an agent session (Home's record of the CLI token's session).
pub enum Poster {
    Owner,
    Session(String),
}

#[derive(Debug, Default, Deserialize)]
pub struct Post {
    pub text: String,
    #[serde(default)]
    pub reply_to: Option<i64>,
    /// A bot asks the owner (CHN-07).
    #[serde(default)]
    pub ask_owner: bool,
}

/// The seq of the message the session's current turn answers.
async fn turn_of(app: &App, session: &str) -> Option<i64> {
    let id = session.to_string();
    app.state_db
        .call(move |c| Ok(c.query_row("SELECT seq FROM session_event WHERE session_id = ?1 AND kind = 'user_message' ORDER BY seq DESC LIMIT 1", [id], |r| r.get(0)).optional()?))
        .await
        .ok()
        .flatten()
}

/// The refusal for a bot that is not a member: a named channel points to the join card.
pub(crate) fn not_member(ch: &Row) -> String {
    match ch.kind {
        Kind::Channel => format!("not a member of #{}: propose joining with `launchpad channel join {}` (a card the owner accepts)", ch.name, ch.name),
        Kind::Dm => "not a member of this DM".into(),
    }
}

/// The bot a session posts as: its bot (`LaunchSpec.bot`, Home's record), a member of the channel, of its workspace.
pub(crate) async fn poster_bot(app: &App, session: &str) -> Result<String> {
    let s = app.sessions.get(session).context("no such session")?;
    let bot = s.launch_spec.bot.as_ref().context("only bots post in channels: this agent is not a bot's session")?;
    Ok(bot.id.clone())
}

/// Posts a message and delivers it (`delivery::deliver`). A bot's session posts only in a channel it is a member of,
/// never in a DM the owner blocked; its hop is one after the message its turn answers. The owner's message has hop 0
/// and restarts the hourly count.
pub async fn post(app: &App, id: &str, poster: Poster, p: Post) -> Result<ChannelPosted> {
    ensure_home(app)?;
    check_text(&p.text)?;
    let ch = load(app, id).await?;
    let label = match ch.kind {
        Kind::Channel => format!("#{}", ch.name),
        Kind::Dm => "this DM".into(),
    };
    ensure!(!ch.archived, "{label} is archived");
    let members = members_of(app, id).await?;
    let bots = bots(app).await?;
    let now = now_ms();
    let (author, session, turn, hops) = match &poster {
        Poster::Owner => (Author { kind: AuthorKind::Owner, id: OWNER.into(), name: "you".into() }, None, None, 0),
        Poster::Session(s) => {
            let bot = poster_bot(app, s).await?;
            let info = bots.get(&bot).context("the session's bot no longer exists")?;
            ensure!(info.workspace_id == ch.workspace_id, "{label} is in another workspace");
            ensure!(members.iter().any(|m| m.bot_id == bot), "{}", not_member(&ch));
            ensure!(!ch.blocked, "the owner blocked this DM");
            let hops = launchpad_core::senders::hops_of(app, s).await + 1;
            (Author { kind: AuthorKind::Bot, id: bot, name: info.name.clone() }, Some(s.clone()), turn_of(app, s).await, hops)
        }
    };
    let slugs: HashMap<String, String> = members.iter().filter_map(|m| bots.get(&m.bot_id).map(|b| (b.slug.clone(), m.bot_id.clone()))).collect();
    let found = mentions(&p.text, &slugs);
    if let Some(r) = p.reply_to {
        ensure!(has_message(app, id, r).await?, "message {r} is not in {label}");
    }
    let owner = author.kind == AuthorKind::Owner;
    let msg = ChannelMessage {
        id: 0,
        channel_id: id.to_string(),
        author,
        session_id: session,
        turn,
        text: p.text,
        mentions: found,
        reply_to: p.reply_to,
        hops,
        ask_owner: p.ask_owner && !owner,
        created_at: now,
    };
    let m = msg.clone();
    let new_id: i64 = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO channel_message (channel_id, author_kind, author_id, author_name, session_id, turn, text, mentions, reply_to, hops, ask_owner, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![m.channel_id, text_of(m.author.kind), m.author.id, m.author.name, m.session_id, m.turn, m.text, serde_json::to_string(&m.mentions).unwrap_or_default(), m.reply_to, m.hops, m.ask_owner, m.created_at],
            )?;
            let id = tx.last_insert_rowid();
            if owner {
                // The owner steps in: the hourly count starts again, and the owner has read up to here.
                tx.execute("UPDATE channel SET reset_at = ?2, owner_read = max(owner_read, ?3) WHERE id = ?1", params![m.channel_id, m.created_at, id])?;
            }
            tx.commit()?;
            Ok(id)
        })
        .await?;
    let msg = ChannelMessage { id: new_id, ..msg };
    let ch = load(app, id).await?;
    changed(app, id, &ch.workspace_id);
    let ids = crate::delivery::deliver(app, &ch, &msg, &members, &bots).await;
    let deliveries = deliveries(app, vec![msg.id]).await?.into_iter().filter(|d| ids.contains(&d.id)).collect();
    changed(app, id, &ch.workspace_id);
    Ok(ChannelPosted { message: msg, deliveries })
}

// ---------------------------------------------------------------- DMs and named channels

/// The DM of two bots of the same workspace, created on first use with both bots as members in mode `all`.
pub async fn dm(app: &App, a: &str, b: &str, by: &str) -> Result<Channel> {
    ensure_home(app)?;
    ensure!(a != b, "a DM is between two different bots");
    let bots = bots(app).await?;
    let (ba, bb) = (bots.get(a).context("no such bot")?, bots.get(b).context("no such bot")?);
    ensure!(ba.workspace_id == bb.workspace_id, "DMs never cross workspaces: {} and {} are in different workspaces", ba.name, bb.name);
    let (ws, key, by) = (ba.workspace_id.clone(), dm_key(a, b), by.to_string());
    let (a, b, now, new_id) = (a.to_string(), b.to_string(), now_ms(), uuid::Uuid::new_v4().to_string());
    let id: String = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT OR IGNORE INTO channel (id, workspace_id, kind, name, created_by, created_at, hop_limit, hourly_limit) VALUES (?1, ?2, 'dm', ?3, ?4, ?5, ?6, ?7)",
                params![new_id, ws, key, by, now, DEFAULT_HOP_LIMIT, DEFAULT_HOURLY_LIMIT],
            )?;
            let id: String = tx.query_row("SELECT id FROM channel WHERE workspace_id = ?1 AND name = ?2", params![ws, key], |r| r.get(0))?;
            for bot in [&a, &b] {
                tx.execute("INSERT OR IGNORE INTO channel_member (channel_id, bot_id, mode) VALUES (?1, ?2, 'all')", params![id, bot])?;
            }
            tx.commit()?;
            Ok(id)
        })
        .await?;
    let ch = get(app, &id).await?;
    changed(app, &id, &ch.workspace_id);
    Ok(ch)
}

#[derive(Debug, Deserialize)]
pub struct NewChannel {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub purpose: String,
    /// Member bots of the workspace: a slug or ID (mode `mentions`), or `{"bot": "release", "mode": "all"}`.
    #[serde(default, deserialize_with = "new_members")]
    pub members: Vec<NewMember>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct NewMember {
    pub bot: String,
    #[serde(default = "mentions_mode")]
    pub mode: Mode,
}

fn mentions_mode() -> Mode {
    Mode::Mentions
}

/// `["release", {"bot": "docs", "mode": "all"}]`.
fn new_members<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<NewMember>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum One {
        Key(String),
        Full(NewMember),
    }
    Ok(Vec::<One>::deserialize(d)?.into_iter().map(|o| match o { One::Key(bot) => NewMember { bot, mode: Mode::Mentions }, One::Full(m) => m }).collect())
}

/// Why a new channel cannot have this name in the workspace (taken by a live or an archived channel), checked
/// before the insert for a clear message; the unique index decides.
pub(crate) async fn name_taken(app: &App, workspace: &str, name: &str, except: Option<&str>) -> Result<Option<String>> {
    let (ws, n, ex) = (workspace.to_string(), name.to_string(), except.unwrap_or_default().to_string());
    let archived: Option<bool> = app.state_db.call(move |c| Ok(c.query_row("SELECT archived FROM channel WHERE workspace_id = ?1 AND name = ?2 AND id != ?3", params![ws, n, ex], |r| r.get(0)).optional()?)).await?;
    Ok(archived.map(|a| if a { format!("#{name} already exists in this workspace, archived: bring it back (Bots → Archived channels) or delete it first") } else { format!("#{name} already exists in this workspace") }))
}

/// Checks a new channel: its name (valid, free in the workspace), workspace and member bots (each once, of the
/// workspace). Returns the trimmed name and the members by bot ID.
pub(crate) async fn check_new(app: &App, n: &NewChannel) -> Result<(String, Vec<(String, Mode)>)> {
    let name = n.name.trim().trim_start_matches('#').to_string();
    ensure!(valid_name(&name), "a channel name has lowercase letters, digits and dashes (at most 40), like release-train");
    launchpad_core::workspace::get(app, &n.workspace_id).await?;
    if let Some(why) = name_taken(app, &n.workspace_id, &name, None).await? {
        anyhow::bail!(why);
    }
    let bots = bots(app).await?;
    let mut members: Vec<(String, Mode)> = vec![];
    for m in &n.members {
        let id = resolve_bot(&bots, &n.workspace_id, &m.bot)?;
        if !members.iter().any(|(b, _)| *b == id) {
            members.push((id, m.mode));
        }
    }
    Ok((name, members))
}

/// A named channel of a workspace. Members are woken only when mentioned (mode `mentions`) unless the request names
/// another mode.
pub async fn create(app: &App, n: NewChannel, by: &str) -> Result<Channel> {
    ensure_home(app)?;
    let (name, members) = check_new(app, &n).await?;
    let purpose = one_line(&n.purpose, 200);
    let (id, ws, by, now) = (uuid::Uuid::new_v4().to_string(), n.workspace_id.clone(), by.to_string(), now_ms());
    let (i, nm) = (id.clone(), name.clone());
    let made = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let n = tx.execute(
                "INSERT OR IGNORE INTO channel (id, workspace_id, kind, name, purpose, created_by, created_at, hop_limit, hourly_limit) VALUES (?1, ?2, 'channel', ?3, ?4, ?5, ?6, ?7, ?8)",
                params![i, ws, nm, purpose, by, now, DEFAULT_HOP_LIMIT, DEFAULT_HOURLY_LIMIT],
            )?;
            if n > 0 {
                for (bot, mode) in &members {
                    tx.execute("INSERT OR IGNORE INTO channel_member (channel_id, bot_id, mode) VALUES (?1, ?2, ?3)", params![i, bot, text_of(*mode)])?;
                }
            }
            tx.commit()?;
            Ok(n > 0)
        })
        .await?;
    ensure!(made, "#{name} already exists in this workspace");
    let ch = get(app, &id).await?;
    changed(app, &id, &ch.workspace_id);
    Ok(ch)
}

fn one_line(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// A bot of the workspace by ID or slug.
pub(crate) fn resolve_bot(bots: &HashMap<String, BotInfo>, workspace: &str, key: &str) -> Result<String> {
    let key = key.trim().trim_start_matches('@');
    bots.iter()
        .find(|(id, b)| b.workspace_id == workspace && (id.as_str() == key || b.slug == key))
        .map(|(id, _)| id.clone())
        .with_context(|| format!("no bot {key} in this workspace"))
}

/// A bot's membership: its mode, None when it is not a member (what an admin change's Undo puts back).
pub async fn member(app: &App, id: &str, bot: &str) -> Result<Option<Mode>> {
    ensure_home(app)?;
    load(app, id).await?;
    Ok(members_of(app, id).await?.into_iter().find(|m| m.bot_id == bot).map(|m| m.mode))
}

/// Adds a bot of the channel's workspace or changes its mode; None removes it. A DM's members are its two bots.
pub async fn set_member(app: &App, id: &str, bot: &str, mode: Option<Mode>) -> Result<Channel> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    ensure!(ch.kind == Kind::Channel, "a DM's members are its two bots");
    let (cid, b) = (id.to_string(), bot.to_string());
    match mode {
        Some(m) => {
            let bots = bots(app).await?;
            let info = bots.get(bot).context("no such bot")?;
            ensure!(info.workspace_id == ch.workspace_id, "{} is in another workspace", info.name);
            let m = text_of(m);
            app.state_db
                .call(move |c| Ok(c.execute("INSERT INTO channel_member (channel_id, bot_id, mode) VALUES (?1, ?2, ?3) ON CONFLICT (channel_id, bot_id) DO UPDATE SET mode = ?3", params![cid, b, m])?))
                .await?;
        }
        None => {
            app.state_db.call(move |c| Ok(c.execute("DELETE FROM channel_member WHERE channel_id = ?1 AND bot_id = ?2", params![cid, b])?)).await?;
        }
    }
    changed(app, id, &ch.workspace_id);
    get(app, id).await
}

/// Changes the mode of a bot that is already a member of a named channel (`channel.set-mode`).
pub async fn set_mode(app: &App, id: &str, bot: &str, mode: Mode) -> Result<Channel> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    ensure!(ch.kind == Kind::Channel, "a DM's bots are always in mode all");
    ensure!(members_of(app, id).await?.iter().any(|m| m.bot_id == bot), "this bot is not a member of #{}: add it first", ch.name);
    set_member(app, id, bot, Some(mode)).await
}

/// A bot's session read the channel up to `through`.
pub async fn mark_read_bot(app: &App, id: &str, bot: &str, through: i64) -> Result<()> {
    let (cid, b) = (id.to_string(), bot.to_string());
    app.state_db.call(move |c| Ok(c.execute("UPDATE channel_member SET last_read = max(last_read, ?3) WHERE channel_id = ?1 AND bot_id = ?2", params![cid, b, through])?)).await?;
    Ok(())
}

/// The owner read the channel up to `through` (its newest message by default).
pub async fn mark_read(app: &App, id: &str, through: Option<i64>) -> Result<Channel> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    let cid = id.to_string();
    app.state_db
        .call(move |c| {
            let newest: i64 = c.query_row("SELECT coalesce(max(id), 0) FROM channel_message WHERE channel_id = ?1", [&cid], |r| r.get(0))?;
            Ok(c.execute("UPDATE channel SET owner_read = max(owner_read, ?2) WHERE id = ?1", params![cid, through.unwrap_or(newest).min(newest)])?)
        })
        .await?;
    changed(app, id, &ch.workspace_id);
    get(app, id).await
}

async fn set_flag(app: &App, id: &str, column: &'static str, value: bool) -> Result<Channel> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    let cid = id.to_string();
    app.state_db.call(move |c| Ok(c.execute(&format!("UPDATE channel SET {column} = ?2 WHERE id = ?1"), params![cid, value])?)).await?;
    changed(app, id, &ch.workspace_id);
    get(app, id).await
}

/// Paused: messages still post, nobody is woken.
pub async fn pause(app: &App, id: &str, paused: bool) -> Result<Channel> {
    set_flag(app, id, "paused", paused).await
}

/// Blocks a DM: its bots can no longer post; the owner still can.
pub async fn block(app: &App, id: &str, blocked: bool) -> Result<Channel> {
    ensure!(load(app, id).await?.kind == Kind::Dm, "only a DM between two bots can be blocked; pause a channel instead");
    set_flag(app, id, "blocked", blocked).await
}

pub async fn archive(app: &App, id: &str, archived: bool) -> Result<Channel> {
    set_flag(app, id, "archived", archived).await
}

#[derive(Debug, Default, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
}

/// Renames a named channel or changes its purpose (a DM is named after its bots).
pub async fn edit(app: &App, id: &str, e: Edit) -> Result<Channel> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    let name = match e.name {
        Some(n) => {
            ensure!(ch.kind == Kind::Channel, "a DM is named after its bots");
            let n = n.trim().trim_start_matches('#').to_string();
            ensure!(valid_name(&n), "a channel name has lowercase letters, digits and dashes (at most 40), like release-train");
            n
        }
        None => ch.name.clone(),
    };
    let purpose = e.purpose.map_or(ch.purpose.clone(), |p| one_line(&p, 200));
    if let Some(why) = name_taken(app, &ch.workspace_id, &name, Some(id)).await? {
        anyhow::bail!(why);
    }
    let (cid, ws, n2) = (id.to_string(), ch.workspace_id.clone(), name.clone());
    let ok = app
        .state_db
        .call(move |c| {
            let taken = c.query_row("SELECT 1 FROM channel WHERE workspace_id = ?1 AND name = ?2 AND id != ?3", params![ws, n2, cid], |_| Ok(())).optional()?.is_some();
            if taken {
                return Ok(false);
            }
            c.execute("UPDATE channel SET name = ?2, purpose = ?3 WHERE id = ?1", params![cid, n2, purpose])?;
            Ok(true)
        })
        .await?;
    ensure!(ok, "#{name} already exists in this workspace");
    changed(app, id, &ch.workspace_id);
    get(app, id).await
}

/// Deletes a channel with its messages and deliveries.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    let cid = id.to_string();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for t in ["channel_delivery", "channel_message", "channel_member"] {
                tx.execute(&format!("DELETE FROM {t} WHERE channel_id = ?1"), [&cid])?;
            }
            tx.execute("DELETE FROM channel WHERE id = ?1", [&cid])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    changed(app, id, &ch.workspace_id);
    Ok(())
}

/// A deleted bot leaves its channels; its DMs are archived (their history stays readable).
pub async fn bot_deleted(app: &App, bot: &str) -> Result<()> {
    let b = bot.to_string();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE channel SET archived = 1 WHERE kind = 'dm' AND id IN (SELECT channel_id FROM channel_member WHERE bot_id = ?1)", [&b])?;
            tx.execute("DELETE FROM channel_member WHERE bot_id = ?1 AND channel_id IN (SELECT id FROM channel WHERE kind = 'channel')", [&b])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("channels", "changed", Value::Null);
    Ok(())
}

/// Adds `ALLOW_MORE` to the channel's allowance and delivers again the loop-guard skips of its newest message.
pub async fn allow_more(app: &App, id: &str) -> Result<ChannelPage> {
    ensure_home(app)?;
    let ch = load(app, id).await?;
    let cid = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("UPDATE channel SET allowance = allowance + ?2 WHERE id = ?1", params![cid, ALLOW_MORE])?)).await?;
    crate::delivery::redeliver_latest(app, id).await?;
    changed(app, id, &ch.workspace_id);
    page(app, id, None, None).await
}

/// The workspace of a channel (for the agent boundary).
pub async fn workspace_of(app: &App, id: &str) -> Option<String> {
    load(app, id).await.ok().map(|c| c.workspace_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_find_member_slugs_and_the_owner() {
        let slugs: HashMap<String, String> = [("release".to_string(), "b1".to_string()), ("docs-bot".to_string(), "b2".to_string())].into();
        assert_eq!(mentions("@Release please, then @docs-bot.", &slugs), ["b1", "b2"]);
        assert_eq!(mentions("@release @release @you", &slugs), ["b1", "owner"]);
        assert_eq!(mentions("mail me at a@release.com", &slugs), Vec::<String>::new());
        assert_eq!(mentions("`@release` and\n```\n@docs-bot\n```\n@owner", &slugs), ["owner"]);
        assert_eq!(mentions("@nobody (@docs-bot)", &slugs), ["b2"]);
        assert_eq!(mentions("@docs-bot- trailing dash", &slugs), ["b2"]);
    }

    #[test]
    fn dm_keys_are_stable_per_pair_and_names_are_slugs() {
        assert_eq!(dm_key("b", "a"), dm_key("a", "b"));
        assert_eq!(dm_key("a", "b"), "dm:a:b");
        assert!(valid_name("release-train") && valid_name("2024"));
        assert!(!valid_name("") && !valid_name("-x") && !valid_name("Release") && !valid_name("dm:a:b") && !valid_name(&"a".repeat(41)));
    }

    #[test]
    fn new_channel_members_are_slugs_or_bots_with_a_mode() {
        let n: NewChannel = serde_json::from_value(json!({"workspace_id": "w", "name": "x", "members": ["release", {"bot": "docs", "mode": "all"}, {"bot": "qa"}]})).unwrap();
        assert_eq!(n.members, [NewMember { bot: "release".into(), mode: Mode::Mentions }, NewMember { bot: "docs".into(), mode: Mode::All }, NewMember { bot: "qa".into(), mode: Mode::Mentions }]);
        assert!(serde_json::from_value::<NewChannel>(json!({"workspace_id": "w", "name": "x", "members": [{"bot": "docs", "mode": "loud"}]})).is_err());
    }

    #[test]
    fn messages_hold_at_most_8_kb() {
        assert!(check_text(&"a".repeat(MAX_TEXT)).is_ok());
        assert!(check_text(&"a".repeat(MAX_TEXT + 1)).is_err());
        // Bytes, not characters.
        assert!(check_text(&"é".repeat(MAX_TEXT / 2 + 1)).is_err());
        assert!(check_text(" \n").is_err());
    }

    #[test]
    fn the_hourly_window_starts_at_the_owners_last_message() {
        let now = 10_000_000;
        assert_eq!(window_start(now, 0), now - 3_600_000);
        assert_eq!(window_start(now, now - 60_000), now - 60_000);
    }
}

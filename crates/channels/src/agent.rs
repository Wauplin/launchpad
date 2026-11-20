//! What an agent's CLI does in bot channels and DMs (`launchpad channel list|read|post`, `launchpad dm`), on Home.
//! The caller is the session of its CLI token (Home's record, never the body): its bot (`LaunchSpec.bot`) and its
//! workspace. Only member bots read and post; a session with admin access also reads every channel of its
//! workspace (it reads those records anyway). Channels are named by name (`release-train`, `#release-train`), by ID,
//! or `@<bot>` for the caller's DM with that bot (created on the first post). Never across workspaces.

use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use launchpad_core::App;

use crate::channel::{self, BotInfo, ChannelFile, Kind, MemberRow, NewFile, Post, Poster, Row};

/// A channel as an agent names it.
#[derive(Debug, PartialEq, Eq)]
pub enum Target<'a> {
    /// A named channel's name, or a channel ID.
    Named(&'a str),
    /// The caller's DM with this bot (slug, name or ID).
    Dm(&'a str),
}

/// Pure: `@release` is the DM with Release; `#release-train` and `release-train` a named channel (or an ID).
pub fn target(key: &str) -> Result<Target<'_>> {
    let key = key.trim();
    let t = match key.strip_prefix('@') {
        Some(bot) => Target::Dm(bot.trim()),
        None => Target::Named(key.trim_start_matches('#')),
    };
    ensure!(!matches!(t, Target::Named("") | Target::Dm("")), "name a channel (release-train) or a bot (@release)");
    Ok(t)
}

/// Pure: the bot of `workspace` that `key` names: its slug or ID, else its slug or name in any case when only one
/// bot of the workspace has it (Err: none, or several).
pub(crate) fn find_bot<'a>(bots: &'a HashMap<String, BotInfo>, workspace: &str, key: &str) -> Result<&'a str> {
    let key = key.trim().trim_start_matches('@');
    let mine = || bots.iter().filter(|(_, b)| b.workspace_id == workspace);
    if let Some((id, _)) = mine().find(|(id, b)| b.slug == key || id.as_str() == key) {
        return Ok(id.as_str());
    }
    let found: Vec<(&String, &BotInfo)> = mine().filter(|(_, b)| b.slug.eq_ignore_ascii_case(key) || b.name.eq_ignore_ascii_case(key)).collect();
    match found.as_slice() {
        [(id, _)] => Ok(id.as_str()),
        [] => bail!("no bot {key} in this workspace (`launchpad bots list`); DMs never cross workspaces"),
        _ => bail!("several bots of this workspace are named {key}: name one by its slug ({})", found.iter().map(|(_, b)| b.slug.as_str()).collect::<Vec<_>>().join(", ")),
    }
}

/// The calling session: its bot (None for an agent that is not a bot), workspace and admin access.
struct Caller {
    session: String,
    bot: Option<String>,
    workspace: String,
    admin: bool,
}

async fn caller(app: &App, session: &str) -> Result<Caller> {
    channel::ensure_home(app)?;
    let s = app.sessions.get(session).context("Home has no record of this session")?;
    let workspace = launchpad_core::scope::of_session(app, session).await.context("this session has no workspace, so it has no channels")?;
    let admin = launchpad_core::vault::session_access(app, session).await.ok().flatten() == Some(launchpad_core::vault::LaunchpadAccess::Admin);
    Ok(Caller { session: session.to_string(), bot: s.launch_spec.bot.as_ref().map(|b| b.id.clone()), workspace, admin })
}

/// Refusal for an agent that is not a bot (CHN-03: only bots speak in channels).
const NOT_A_BOT: &str = "only bots post in channels: this agent is not a bot's session";

async fn by_name(app: &App, workspace: &str, name: &str) -> Result<Option<String>> {
    let (ws, name) = (workspace.to_string(), name.to_string());
    app.state_db.call(move |c| Ok(c.query_row("SELECT id FROM channel WHERE workspace_id = ?1 AND name = ?2", params![ws, name], |r| r.get(0)).optional()?)).await
}

/// The channel `key` names for this caller. A DM that does not exist yet is created when `create` (a post).
async fn resolve(app: &App, me: &Caller, key: &str, bots: &HashMap<String, BotInfo>, create: bool) -> Result<Row> {
    match target(key)? {
        Target::Dm(other) => {
            let mine = me.bot.as_deref().context(if create { NOT_A_BOT } else { "a DM is between two bots: only a bot's session has DMs (name a channel instead)" })?;
            let id = find_bot(bots, &me.workspace, other)?;
            ensure!(id != mine, "@{other} is you: a DM is with another bot");
            let found = by_name(app, &me.workspace, &channel::dm_key(mine, id)).await?;
            let cid = match found {
                Some(c) => c,
                None if create => channel::dm(app, mine, id, &format!("session:{}", me.session)).await?.id,
                None => bail!("no DM with {other} yet: start it with `launchpad dm {other} --body-file <file>`"),
            };
            channel::load(app, &cid).await
        }
        Target::Named(name) => {
            if let Some(cid) = by_name(app, &me.workspace, name).await? {
                return channel::load(app, &cid).await;
            }
            match channel::load(app, name).await {
                Ok(r) if r.workspace_id == me.workspace => Ok(r),
                Ok(_) => bail!("channel {name} belongs to another workspace: agents read and change only the records of their own workspace"),
                Err(_) => bail!("no channel #{name} in this workspace (`launchpad channel list` shows them)"),
            }
        }
    }
}

/// How an agent names the channel: `release-train`, or `@<other bot>` for a DM (as seen by `bot`).
fn handle(r: &Row, members: &[MemberRow], bots: &HashMap<String, BotInfo>, bot: Option<&str>) -> String {
    match r.kind {
        Kind::Channel => r.name.clone(),
        Kind::Dm => {
            let slugs: Vec<String> = members.iter().filter(|m| Some(m.bot_id.as_str()) != bot).map(|m| format!("@{}", bots.get(&m.bot_id).map_or("deleted-bot", |b| b.slug.as_str()))).collect();
            slugs.join(" ↔ ")
        }
    }
}

/// The messages after `since` that the caller's bot did not write.
async fn unread(app: &App, id: &str, bot: &str, since: i64) -> Result<u32> {
    let (id, bot) = (id.to_string(), bot.to_string());
    app.state_db
        .call(move |c| Ok(c.query_row("SELECT count(*) FROM channel_message WHERE channel_id = ?1 AND id > ?2 AND NOT (author_kind = 'bot' AND author_id = ?3)", params![id, since, bot], |r| r.get(0))?))
        .await
}

/// `launchpad channel list`: the named channels of the caller's workspace and its bot's DMs (every DM at admin
/// access), each with the bot's membership, mode and unread messages.
pub async fn list(app: &App, session: &str) -> Result<Value> {
    let me = caller(app, session).await?;
    let bots = channel::bots(app).await?;
    let ws = me.workspace.clone();
    let rows: Vec<Row> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare(&format!("SELECT {} FROM channel WHERE workspace_id = ?1 ORDER BY kind, name", channel::ROW))?;
            let rows = st.query_map([ws], channel::row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let mut out = vec![];
    for r in rows {
        let members = channel::members_of(app, &r.id).await?;
        let mine = me.bot.as_deref().and_then(|b| members.iter().find(|m| m.bot_id == b));
        if r.kind == Kind::Dm && mine.is_none() && !me.admin {
            continue;
        }
        let names: Vec<&str> = members.iter().map(|m| bots.get(&m.bot_id).map_or("deleted-bot", |b| b.slug.as_str())).collect();
        let mut v = json!({
            "name": handle(&r, &members, &bots, me.bot.as_deref()),
            "id": r.id,
            "kind": r.kind,
            "members": names,
            "member": mine.is_some(),
        });
        if !r.purpose.is_empty() {
            v["purpose"] = json!(r.purpose);
        }
        for (flag, on) in [("paused", r.paused), ("blocked", r.blocked), ("archived", r.archived)] {
            if on {
                v[flag] = json!(true);
            }
        }
        if let (Some(m), Some(bot)) = (mine, me.bot.as_deref()) {
            v["mode"] = json!(m.mode);
            v["last_read"] = json!(m.last_read);
            v["unread"] = json!(unread(app, &r.id, bot, m.last_read).await?);
        }
        out.push(v);
    }
    Ok(json!({"bot": me.bot.as_deref().and_then(|b| bots.get(b)).map(|b| b.slug.clone()), "channels": out}))
}

/// `launchpad channel read`: messages after `since`, else the newest `limit`, oldest first. A member bot's read
/// moves its own last-read mark (never the owner's).
pub async fn read(app: &App, session: &str, key: &str, since: Option<i64>, limit: Option<u32>) -> Result<Value> {
    let me = caller(app, session).await?;
    let bots = channel::bots(app).await?;
    let r = resolve(app, &me, key, &bots, false).await?;
    let members = channel::members_of(app, &r.id).await?;
    let mine = me.bot.as_deref().and_then(|b| members.iter().find(|m| m.bot_id == b)).cloned();
    if mine.is_none() && !me.admin {
        match me.bot {
            None => bail!("only member bots read channels: this agent is not a bot's session"),
            Some(_) => bail!("{}", channel::not_member(&r)),
        }
    }
    let (messages, more) = channel::messages(app, &r.id, since, None, limit).await?;
    if let (Some(m), Some(last)) = (&mine, messages.last()) {
        channel::mark_read_bot(app, &r.id, &m.bot_id, last.id).await?;
    }
    let slug = |id: &str| if id == channel::OWNER { "owner".to_string() } else { bots.get(id).map_or(id.to_string(), |b| b.slug.clone()) };
    let list: Vec<Value> = messages
        .iter()
        .map(|m| {
            let mut v = json!({"id": m.id, "from": if m.author.kind == channel::AuthorKind::Owner { "owner".to_string() } else { m.author.name.clone() }, "created_at": m.created_at, "text": m.text, "hops": m.hops});
            if !m.mentions.is_empty() {
                v["mentions"] = json!(m.mentions.iter().map(|x| slug(x)).collect::<Vec<_>>());
            }
            if let Some(r) = m.reply_to {
                v["reply_to"] = json!(r);
            }
            if m.ask_owner {
                v["ask_owner"] = json!(true);
            }
            if !m.files.is_empty() {
                v["files"] = json!(m.files.iter().map(|f| json!({"name": f.name, "mime": f.mime, "size": f.size})).collect::<Vec<_>>());
            }
            v
        })
        .collect();
    let handle = handle(&r, &members, &bots, me.bot.as_deref());
    let with_files = messages.iter().any(|m| !m.files.is_empty());
    let mut out = json!({"channel": handle, "id": r.id, "kind": r.kind, "messages": list, "more_before": more});
    if with_files {
        out["files_hint"] = json!(format!("download a message's files with `launchpad channel files {handle} <message id>`; forward them with `--forward <message id>`"));
    }
    if !r.purpose.is_empty() {
        out["purpose"] = json!(r.purpose);
    }
    if let Some(m) = mine {
        out["mode"] = json!(m.mode);
        out["last_read_before"] = json!(m.last_read);
    }
    for (flag, on) in [("paused", r.paused), ("blocked", r.blocked), ("archived", r.archived)] {
        if on {
            out[flag] = json!(true);
        }
    }
    Ok(out)
}

/// How long a file an agent uploaded (`upload`) may be attached to its posts.
const UPLOAD_MS: i64 = 3600 * 1000;

/// `(session, sha256)` → when the session uploaded it: a post attaches only its own uploads (or forwarded files).
static UPLOADED: std::sync::LazyLock<std::sync::Mutex<HashMap<(String, String), i64>>> = std::sync::LazyLock::new(Default::default);

fn uploaded_by(session: &str, reference: &str) -> bool {
    let Some(hash) = launchpad_core::attachments::hash_of(reference) else { return false };
    let now = launchpad_core::util::now_ms();
    UPLOADED.lock().unwrap().get(&(session.to_string(), hash.to_string())).is_some_and(|at| now - *at < UPLOAD_MS)
}

/// `launchpad channel post --attach` and `launchpad dm --attach`: one file of the bot's next post, stored on Home
/// (`attachments::store`: at most 5 MB). Only a bot's session uploads; the reference it gets back is attachable only
/// by that session, for an hour.
pub async fn upload(app: &App, session: &str, name: &str, mime: &str, data: &str) -> Result<Value> {
    use base64::Engine;
    let me = caller(app, session).await?;
    ensure!(me.bot.is_some(), NOT_A_BOT);
    ensure!(data.len() <= 7 * 1024 * 1024, "each file must be 5 MB or smaller");
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).context("the file is not valid base64")?;
    let reference = launchpad_core::attachments::store(&bytes, mime).await?;
    let hash = launchpad_core::attachments::hash_of(&reference).context("invalid attachment reference")?.to_string();
    let now = launchpad_core::util::now_ms();
    let mut up = UPLOADED.lock().unwrap();
    up.retain(|_, at| now - *at < UPLOAD_MS);
    up.insert((me.session.clone(), hash), now);
    Ok(json!({"name": channel::file_name(name), "reference": reference, "size": bytes.len()}))
}

/// The files of message `id` that the caller may forward: a message of a channel of its workspace that it reads
/// (its bot is a member, or it has admin access).
async fn forwardable(app: &App, me: &Caller, id: i64) -> Result<Vec<ChannelFile>> {
    let refused = format!("message {id} is not in a channel you read: forward files only from your own channels and DMs");
    let m = channel::message(app, id).await?.with_context(|| refused.clone())?;
    let r = channel::load(app, &m.channel_id).await.with_context(|| refused.clone())?;
    ensure!(r.workspace_id == me.workspace, "{refused}");
    let member = match &me.bot {
        Some(bot) => channel::members_of(app, &r.id).await?.iter().any(|x| &x.bot_id == bot),
        None => false,
    };
    ensure!(member || me.admin, "{refused}");
    ensure!(!m.files.is_empty(), "message {id} has no files");
    Ok(m.files)
}

/// Pure: whether every file a post names is one the session may attach (its own upload, or a forwarded one).
fn attachable(files: &[NewFile], allowed: impl Fn(&str) -> bool) -> Result<()> {
    for f in files {
        ensure!(allowed(&f.reference), "{} was not uploaded by this session: attach files with `--attach <path>` or forward them with `--forward <message id>`", channel::file_name(&f.name));
    }
    Ok(())
}

/// `launchpad channel post` and `launchpad dm`: the caller's bot posts (a DM is created on first use). Home stamps
/// the author from its own session record; `channel::post` checks membership, blocks and the archive. `p.files` are
/// the session's own uploads (`upload`); `forward` adds the files of earlier messages it reads (`forwardable`).
/// With `at` (Unix ms), the post is scheduled instead (`scheduled::insert`): checked now like a post, posted once due.
pub async fn post(app: &App, session: &str, key: &str, mut p: Post, forward: &[i64], at: Option<i64>) -> Result<Value> {
    let me = caller(app, session).await?;
    ensure!(me.bot.is_some(), NOT_A_BOT);
    attachable(&p.files, |r| uploaded_by(&me.session, r))?;
    for id in forward {
        p.files.extend(forwardable(app, &me, *id).await?.into_iter().map(|f| NewFile { name: f.name, reference: f.reference }));
    }
    if p.files.is_empty() {
        channel::check_text(&p.text)?;
    }
    let bots = channel::bots(app).await?;
    let r = resolve(app, &me, key, &bots, true).await?;
    let members = channel::members_of(app, &r.id).await?;
    if let Some(at) = at {
        let bot = me.bot.as_deref().unwrap_or_default();
        ensure!(members.iter().any(|m| m.bot_id == bot), "{}", channel::not_member(&r));
        if let Some(reply) = p.reply_to {
            ensure!(channel::has_message(app, &r.id, reply).await?, "message {reply} is not in this channel");
        }
        let s = crate::scheduled::insert(app, &r, bot, &me.session, at, p).await?;
        return Ok(json!({"scheduled": true, "n": s.n, "channel": handle(&r, &members, &bots, me.bot.as_deref()), "due_at": s.due_at, "hint": "`launchpad channel scheduled` lists your bot's scheduled posts; `launchpad channel cancel-scheduled <n>` cancels one"}));
    }
    let posted = channel::post(app, &r.id, Poster::Session(me.session.clone()), p).await?;
    let slug = |id: &str| if id == channel::OWNER { "owner".to_string() } else { bots.get(id).map_or(id.to_string(), |b| b.slug.clone()) };
    let deliveries: Vec<Value> = posted
        .deliveries
        .iter()
        .map(|d| {
            let mut v = json!({"bot": slug(&d.bot_id), "state": d.state});
            if let Some(why) = &d.reason {
                v["reason"] = json!(why);
            }
            v
        })
        .collect();
    let m = &posted.message;
    let mut out = json!({
        "channel": handle(&r, &members, &bots, me.bot.as_deref()),
        "message": m.id,
        "hops": m.hops,
        "mentions": m.mentions.iter().map(|x| slug(x)).collect::<Vec<_>>(),
        "deliveries": deliveries,
    });
    if !m.files.is_empty() {
        out["files"] = json!(m.files.iter().map(|f| json!({"name": f.name, "size": f.size})).collect::<Vec<_>>());
    }
    Ok(out)
}

/// `launchpad channel scheduled`: the scheduled posts of the caller's bot that wait (any of its sessions), soonest
/// first, each with how the bot names its channel.
pub async fn scheduled(app: &App, session: &str) -> Result<Value> {
    let me = caller(app, session).await?;
    let bot = me.bot.as_deref().context(NOT_A_BOT)?;
    let bots = channel::bots(app).await?;
    let mut out = vec![];
    for s in crate::scheduled::list(app, None, Some(bot)).await? {
        let name = match channel::load(app, &s.channel_id).await {
            Ok(r) => handle(&r, &channel::members_of(app, &r.id).await?, &bots, Some(bot)),
            Err(_) => s.channel_id.clone(),
        };
        let mut v = json!({"n": s.n, "channel": name, "due_at": s.due_at, "created_at": s.created_at, "text": s.text});
        if let Some(r) = s.reply_to {
            v["reply_to"] = json!(r);
        }
        if s.files > 0 {
            v["files"] = json!(s.files);
        }
        out.push(v);
    }
    Ok(json!({"scheduled": out}))
}

/// `launchpad channel cancel-scheduled <n>`: cancels one of the caller's bot's scheduled posts (never another bot's).
pub async fn cancel_scheduled(app: &App, session: &str, n: i64) -> Result<Value> {
    let me = caller(app, session).await?;
    let bot = me.bot.as_deref().context(NOT_A_BOT)?;
    crate::scheduled::cancel(app, n, None, Some(bot)).await?;
    Ok(json!({"cancelled": n}))
}

/// `launchpad channel files`: file `index` of message `message` in a channel the caller reads (like `read`), with
/// its bytes in base64 and the message's file count. One file per call, so each answer stays small.
pub async fn file(app: &App, session: &str, key: &str, message: i64, index: usize) -> Result<Value> {
    use base64::Engine;
    let me = caller(app, session).await?;
    let bots = channel::bots(app).await?;
    let r = resolve(app, &me, key, &bots, false).await?;
    let members = channel::members_of(app, &r.id).await?;
    let member = me.bot.as_deref().is_some_and(|b| members.iter().any(|m| m.bot_id == b));
    if !member && !me.admin {
        match me.bot {
            None => bail!("only member bots read channels: this agent is not a bot's session"),
            Some(_) => bail!("{}", channel::not_member(&r)),
        }
    }
    let m = channel::message(app, message).await?.filter(|m| m.channel_id == r.id).with_context(|| format!("message {message} is not in this channel"))?;
    ensure!(!m.files.is_empty(), "message {message} has no files");
    let f: &ChannelFile = m.files.get(index).with_context(|| format!("message {message} has {} files", m.files.len()))?;
    let bytes = launchpad_core::attachments::read(&f.reference)?;
    Ok(json!({"message": message, "index": index, "count": m.files.len(), "name": f.name, "mime": f.mime, "size": bytes.len(), "data": base64::engine::general_purpose::STANDARD.encode(bytes)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_attaches_only_the_sessions_own_uploads() {
        let f = |r: &str| NewFile { name: "a/b/report.pdf".into(), reference: r.into() };
        let mine = format!("upload:application/pdf;{}", "a".repeat(64));
        let other = format!("upload:application/pdf;{}", "b".repeat(64));
        assert!(attachable(&[f(&mine)], |r| r == mine).is_ok());
        let e = attachable(&[f(&mine), f(&other)], |r| r == mine).unwrap_err().to_string();
        assert!(e.starts_with("report.pdf was not uploaded by this session"), "{e}");
        // The in-memory record names the session and the bytes, never only the reference text.
        let now = launchpad_core::util::now_ms();
        UPLOADED.lock().unwrap().insert(("s1".into(), "a".repeat(64)), now);
        assert!(uploaded_by("s1", &mine) && uploaded_by("s1", &format!("upload:image/png;{}", "a".repeat(64))));
        assert!(!uploaded_by("s2", &mine) && !uploaded_by("s1", &other) && !uploaded_by("s1", "upload:x"));
        UPLOADED.lock().unwrap().insert(("s3".into(), "a".repeat(64)), now - UPLOAD_MS);
        assert!(!uploaded_by("s3", &mine));
    }

    #[test]
    fn agents_name_channels_and_dms() {
        assert_eq!(target("release-train").unwrap(), Target::Named("release-train"));
        assert_eq!(target(" #release-train").unwrap(), Target::Named("release-train"));
        assert_eq!(target("@release").unwrap(), Target::Dm("release"));
        assert!(target("@").is_err() && target("#").is_err() && target(" ").is_err());
    }

    #[test]
    fn bots_are_found_by_slug_id_or_name_in_their_workspace_only() {
        let b = |name: &str, slug: &str, ws: &str| BotInfo { name: name.into(), slug: slug.into(), workspace_id: ws.into(), paused: false };
        let bots: HashMap<String, BotInfo> = [("b1".to_string(), b("Release", "release", "w1")), ("b2".to_string(), b("Docs Bot", "docs-bot", "w1")), ("b3".to_string(), b("Other", "other", "w2"))].into();
        let find = |ws: &str, key: &str| find_bot(&bots, ws, key).ok();
        assert_eq!(find("w1", "release"), Some("b1"));
        assert_eq!(find("w1", "@Release"), Some("b1"));
        assert_eq!(find("w1", "docs bot"), Some("b2"));
        assert_eq!(find("w1", "b2"), Some("b2"));
        assert_eq!(find("w1", "other"), None);
        assert_eq!(find("w2", "b1"), None);
        // Two bots of one name: only their slugs name them.
        let mut twins = bots.clone();
        twins.insert("b4".into(), b("release", "release-2", "w1"));
        assert_eq!(find_bot(&twins, "w1", "release").ok(), Some("b1"));
        assert!(find_bot(&twins, "w1", "RELEASE").unwrap_err().to_string().contains("several bots"));
    }
}

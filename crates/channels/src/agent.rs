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

use crate::channel::{self, BotInfo, Kind, MemberRow, Post, Poster, Row};

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

/// Pure: the bot of `workspace` that `key` names: its slug, ID or name (any case), with or without `@`.
pub(crate) fn find_bot<'a>(bots: &'a HashMap<String, BotInfo>, workspace: &str, key: &str) -> Option<&'a str> {
    let key = key.trim().trim_start_matches('@');
    let mine = || bots.iter().filter(|(_, b)| b.workspace_id == workspace);
    mine()
        .find(|(id, b)| b.slug == key || id.as_str() == key)
        .or_else(|| mine().find(|(_, b)| b.slug.eq_ignore_ascii_case(key) || b.name.eq_ignore_ascii_case(key)))
        .map(|(id, _)| id.as_str())
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
            let id = find_bot(bots, &me.workspace, other).with_context(|| format!("no bot {other} in this workspace (`launchpad bots list`); DMs never cross workspaces"))?;
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

fn label(r: &Row) -> String {
    match r.kind {
        Kind::Channel => format!("#{}", r.name),
        Kind::Dm => "this DM".into(),
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
            Some(_) => bail!("not a member of {}: ask the owner to add you", label(&r)),
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
            v
        })
        .collect();
    let mut out = json!({"channel": handle(&r, &members, &bots, me.bot.as_deref()), "id": r.id, "kind": r.kind, "messages": list, "more_before": more});
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

/// `launchpad channel post` and `launchpad dm`: the caller's bot posts (a DM is created on first use). Home stamps
/// the author from its own session record; `channel::post` checks membership, blocks and the archive.
pub async fn post(app: &App, session: &str, key: &str, p: Post) -> Result<Value> {
    let me = caller(app, session).await?;
    ensure!(me.bot.is_some(), NOT_A_BOT);
    channel::check_text(&p.text)?;
    let bots = channel::bots(app).await?;
    let r = resolve(app, &me, key, &bots, true).await?;
    let members = channel::members_of(app, &r.id).await?;
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
    Ok(json!({
        "channel": handle(&r, &members, &bots, me.bot.as_deref()),
        "message": m.id,
        "hops": m.hops,
        "mentions": m.mentions.iter().map(|x| slug(x)).collect::<Vec<_>>(),
        "deliveries": deliveries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(find_bot(&bots, "w1", "release"), Some("b1"));
        assert_eq!(find_bot(&bots, "w1", "@Release"), Some("b1"));
        assert_eq!(find_bot(&bots, "w1", "docs bot"), Some("b2"));
        assert_eq!(find_bot(&bots, "w1", "b2"), Some("b2"));
        assert_eq!(find_bot(&bots, "w1", "other"), None);
        assert_eq!(find_bot(&bots, "w2", "b1"), None);
    }
}

//! Scheduled posts in bot channels and DMs (`launchpad channel post|dm --in|--at`), on Home. A bot's session
//! schedules a post; Home stores it in `channel_scheduled` (it survives restarts and Home updates) with the hop count
//! and turn of the turn that scheduled it, and once due posts it exactly like that post sent then (`channel::post_as`:
//! same author, membership checks, mentions, deliveries and loop guard, one hop after the scheduling turn). The row is
//! deleted in the message's own transaction, so a post goes out once at most and a cancelled one never.
//!
//! While the posting bot or the channel is paused it waits; a deleted channel or bot, an archived channel, a blocked
//! DM or a bot that is no longer a member drops it (logged). The bot lists its own with `launchpad channel scheduled`
//! and cancels one with `launchpad channel cancel-scheduled <n>`; the owner sees them on the channel's page with
//! Cancel (`channel.cancel-scheduled`).

use anyhow::{Result, ensure};
use rusqlite::params;
use serde::Serialize;
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::util::now_ms;

use crate::channel::{self, Frozen, Post, Poster, Row};

/// How long after its due time a post whose posting keeps failing is given up (pauses do not count: they wait).
const GIVE_UP_MS: i64 = 3_600_000;

/// A scheduled post that waits, as `launchpad channel scheduled` and the channel page list it.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "ScheduledPost")]
pub struct ScheduledPost {
    /// Its number: `launchpad channel cancel-scheduled <n>` (or the owner's Cancel) cancels it.
    #[ts(type = "number")]
    pub n: i64,
    pub channel_id: String,
    /// The bot that posts it, and the session that scheduled it.
    pub bot_id: String,
    pub session_id: String,
    #[ts(type = "number")]
    pub due_at: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    pub text: String,
    #[ts(type = "number | null")]
    pub reply_to: Option<i64>,
    /// The number of files it carries.
    pub files: u32,
}

/// Home: stores bot `bot`'s post in channel `ch` (already checked by the caller: a member, of its workspace) for
/// `at` (Unix ms), scheduled by `session` now: its hop count and turn are taken now.
pub(crate) async fn insert(app: &App, ch: &Row, bot: &str, session: &str, at: i64, p: Post) -> Result<ScheduledPost> {
    let now = now_ms();
    // Like a post now: never signed (the note goes after the text).
    let p = Post { text: channel::without_attribution(&p.text), ..p };
    crate::hooks::check_due(at, now)?;
    ensure!(!ch.archived, "this channel is archived");
    ensure!(!ch.blocked, "the owner blocked this DM");
    // The post as it goes out (with its note), checked now so it cannot fail for its size later.
    let files = channel::resolve_files(p.files.clone())?;
    let full = with_note(&p.text, now, at);
    if files.is_empty() && p.text.trim().is_empty() {
        channel::check_text(&p.text)?;
    }
    channel::check_text(&full)?;
    let waiting = list(app, None, Some(bot)).await?.len();
    ensure!(waiting < crate::hooks::SCHEDULE_MAX_WAITING, "{waiting} scheduled posts of this bot already wait: cancel one first (`launchpad channel scheduled`)");
    let hops = launchpad_core::senders::hops_of(app, session).await + 1;
    let turn = channel::turn_of(app, session).await;
    let json = serde_json::to_string(&p)?;
    let (cid, b, s) = (ch.id.clone(), bot.to_string(), session.to_string());
    let n = app
        .state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO channel_scheduled (channel_id, bot_id, session_id, hops, turn, due_at, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![cid, b, s, hops, turn, at, now, json],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await?;
    channel::changed(app, &ch.id, &ch.workspace_id);
    Ok(ScheduledPost { n, channel_id: ch.id.clone(), bot_id: bot.into(), session_id: session.into(), due_at: at, created_at: now, text: p.text, reply_to: p.reply_to, files: files.len() as u32 })
}

/// Pure: the text a scheduled post goes out with: its text, then the line that says when it was written and due.
fn with_note(text: &str, created_at: i64, at: i64) -> String {
    let note = crate::hooks::note_of(created_at, at);
    if text.trim().is_empty() { note } else { format!("{}\n\n{note}", text.trim_end()) }
}

struct Waiting {
    n: i64,
    channel_id: String,
    bot_id: String,
    session_id: String,
    hops: u32,
    turn: Option<i64>,
    due_at: i64,
    created_at: i64,
    post: Post,
}

async fn rows(app: &App, due_by: Option<i64>) -> Result<Vec<Waiting>> {
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT n, channel_id, bot_id, session_id, hops, turn, due_at, created_at, json FROM channel_scheduled WHERE due_at <= ?1 ORDER BY due_at, n")?;
            let rows = st
                .query_map([due_by.unwrap_or(i64::MAX)], |r| {
                    // A row whose post no longer reads is skipped (never posted).
                    let Ok(post) = serde_json::from_str::<Post>(&r.get::<_, String>(8)?) else { return Ok(None) };
                    Ok(Some(Waiting { n: r.get(0)?, channel_id: r.get(1)?, bot_id: r.get(2)?, session_id: r.get(3)?, hops: r.get(4)?, turn: r.get(5)?, due_at: r.get(6)?, created_at: r.get(7)?, post }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().flatten().collect())
        })
        .await
}

/// The scheduled posts that wait, soonest first: of one channel, of one bot, or every one.
pub async fn list(app: &App, channel: Option<&str>, bot: Option<&str>) -> Result<Vec<ScheduledPost>> {
    Ok(rows(app, None)
        .await?
        .into_iter()
        .filter(|w| channel.is_none_or(|c| w.channel_id == c) && bot.is_none_or(|b| w.bot_id == b))
        .map(|w| ScheduledPost { n: w.n, channel_id: w.channel_id, bot_id: w.bot_id, session_id: w.session_id, due_at: w.due_at, created_at: w.created_at, text: w.post.text, reply_to: w.post.reply_to, files: w.post.files.len() as u32 })
        .collect())
}

/// Cancels scheduled post `n`, only when it is of `channel` (the owner's channel page) or of `bot` (the bot's own).
pub async fn cancel(app: &App, n: i64, channel: Option<&str>, bot: Option<&str>) -> Result<()> {
    let (c, b) = (channel.map(str::to_string), bot.map(str::to_string));
    let gone: Option<String> = app
        .state_db
        .call(move |db| {
            let tx = db.transaction()?;
            let found: Option<String> = rusqlite::OptionalExtension::optional(tx.query_row(
                "SELECT channel_id FROM channel_scheduled WHERE n = ?1 AND (?2 IS NULL OR channel_id = ?2) AND (?3 IS NULL OR bot_id = ?3)",
                params![n, c, b],
                |r| r.get(0),
            ))?;
            if found.is_some() {
                tx.execute("DELETE FROM channel_scheduled WHERE n = ?1", [n])?;
            }
            tx.commit()?;
            Ok(found)
        })
        .await?;
    let Some(cid) = gone else { anyhow::bail!("no scheduled post {n} waits here (`launchpad channel scheduled` lists them)") };
    if let Ok(ch) = channel::load(app, &cid).await {
        channel::changed(app, &cid, &ch.workspace_id);
    }
    Ok(())
}

/// What a due post does now.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Post,
    Wait,
    Drop(&'static str),
}

/// The state a due post meets.
struct Seen {
    /// The channel (None: deleted): archived, paused, blocked.
    channel: Option<(bool, bool, bool)>,
    /// The posting bot (None: deleted): paused.
    bot: Option<bool>,
    /// The scheduling session still exists, and its bot is a member of the channel, in the channel's workspace.
    session: bool,
    member: bool,
}

/// Pure: a paused bot or channel makes it wait; what would refuse the post forever drops it.
fn step(s: &Seen) -> Step {
    let Some((archived, paused, blocked)) = s.channel else { return Step::Drop("the channel was deleted") };
    let Some(bot_paused) = s.bot else { return Step::Drop("its bot was deleted") };
    if archived {
        return Step::Drop("the channel was archived");
    }
    if blocked {
        return Step::Drop("the owner blocked this DM");
    }
    if !s.session {
        return Step::Drop("the session that scheduled it is gone");
    }
    if !s.member {
        return Step::Drop("its bot is no longer a member");
    }
    if bot_paused || paused { Step::Wait } else { Step::Post }
}

async fn drop_row(app: &App, w: &Waiting, why: &str) -> Result<()> {
    tracing::warn!("scheduled channel post {} of bot {} in {} is dropped: {why}", w.n, w.bot_id, w.channel_id);
    let n = w.n;
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM channel_scheduled WHERE n = ?1", [n])?)).await?;
    if let Ok(ch) = channel::load(app, &w.channel_id).await {
        channel::changed(app, &ch.id, &ch.workspace_id);
    }
    Ok(())
}

/// One pass: posts every due post that may go now.
async fn run(app: &App) -> Result<()> {
    let due = rows(app, Some(now_ms())).await?;
    if due.is_empty() {
        return Ok(());
    }
    let bots = channel::bots(app).await?;
    for w in due {
        let ch: Option<Row> = channel::load(app, &w.channel_id).await.ok();
        let bot = bots.get(&w.bot_id);
        let member = match (&ch, bot) {
            (Some(c), Some(b)) => b.workspace_id == c.workspace_id && channel::members_of(app, &c.id).await?.iter().any(|m| m.bot_id == w.bot_id),
            _ => false,
        };
        let session = app.sessions.get(&w.session_id).is_some_and(|s| s.launch_spec.bot.as_ref().is_some_and(|b| b.id == w.bot_id));
        let seen = Seen { channel: ch.as_ref().map(|c| (c.archived, c.paused, c.blocked)), bot: bot.map(|b| b.paused), session, member };
        match step(&seen) {
            Step::Wait => continue,
            Step::Drop(why) => drop_row(app, &w, why).await?,
            Step::Post => {
                let post = Post { text: with_note(&w.post.text, w.created_at, w.due_at), ..w.post.clone() };
                let frozen = Frozen { n: w.n, hops: w.hops, turn: w.turn };
                if let Err(e) = Box::pin(channel::post_as(app, &w.channel_id, Poster::Session(w.session_id.clone()), post, Some(frozen))).await {
                    if now_ms() > w.due_at + GIVE_UP_MS {
                        drop_row(app, &w, &format!("{e:#}")).await?;
                    } else {
                        tracing::warn!("scheduled channel post {}: {e:#}", w.n);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Home: checks every 5 seconds for scheduled posts that are due.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if let Err(e) = run(&app).await {
                tracing::warn!("scheduled channel posts: {e:#}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_due_post_waits_while_paused_and_drops_what_would_refuse_it_forever() {
        let ok = Seen { channel: Some((false, false, false)), bot: Some(false), session: true, member: true };
        assert_eq!(step(&ok), Step::Post);
        assert_eq!(step(&Seen { bot: Some(true), ..ok }), Step::Wait);
        assert_eq!(step(&Seen { channel: Some((false, true, false)), ..ok }), Step::Wait);
        assert_eq!(step(&Seen { channel: None, ..ok }), Step::Drop("the channel was deleted"));
        assert_eq!(step(&Seen { bot: None, ..ok }), Step::Drop("its bot was deleted"));
        assert_eq!(step(&Seen { channel: Some((true, true, false)), ..ok }), Step::Drop("the channel was archived"));
        assert_eq!(step(&Seen { channel: Some((false, false, true)), ..ok }), Step::Drop("the owner blocked this DM"));
        assert_eq!(step(&Seen { member: false, bot: Some(true), ..ok }), Step::Drop("its bot is no longer a member"));
        assert_eq!(step(&Seen { session: false, ..ok }), Step::Drop("the session that scheduled it is gone"));
    }

    #[test]
    fn a_scheduled_post_keeps_its_text_reply_and_files_and_ends_with_its_note() {
        let p = Post { text: "Release is out\n".into(), reply_to: Some(41), ask_owner: true, files: vec![channel::NewFile { name: "notes.md".into(), reference: format!("upload:text/markdown;{}", "a".repeat(64)) }] };
        // Stored as JSON in state.db: read back the same after a restart.
        let back: Post = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!((back.text.as_str(), back.reply_to, back.ask_owner, back.files.len()), ("Release is out\n", Some(41), true, 1));
        assert_eq!(with_note(&back.text, 0, 7_200_000), "Release is out\n\n(Scheduled message: written 1970-01-01 00:00 UTC, due 1970-01-01 02:00 UTC.)");
        assert_eq!(with_note(" ", 0, 60_000), "(Scheduled message: written 1970-01-01 00:00 UTC, due 1970-01-01 00:01 UTC.)");
    }
}

//! Daily digests of bot channels: a member in mode `digest` gets no delivery per message (only when mentioned);
//! instead, every day at `HOUR`:00 in Home's time zone (`[home] time_zone`, else the host's), it gets one message per
//! channel listing the messages since its last read (author, time, first line, IDs), with the
//! `launchpad channel read` hint. Nothing new: nothing is sent and no row is written.
//!
//! It is automatic work: it goes through the bot message path like other channel deliveries (`hooks`: it waits
//! while the bot works or its quota is not confirmed), under the channel's loop guard with the hop of the newest bot
//! message it lists. The day's run is claimed first (`channel_digest_run`), each digest is a `channel_delivery` row
//! of kind `digest` (its range in `first_message`..`message_id`) written before it is sent, never retried. A run more
//! than an hour late (Home was not running at `HOUR`:00) never catches up: its digests are recorded as skipped with
//! the reason, and the next digest lists their messages again (a skipped digest does not move the bot's place).
//! Home only.

use std::collections::HashMap;

use anyhow::Result;
use chrono::TimeZone;
use rusqlite::{OptionalExtension, params};

use launchpad_core::App;
use launchpad_core::model::{MessageFrom, SenderKind};
use launchpad_core::senders::{HEADER_START, fence};
use launchpad_core::util::now_ms;

use crate::channel::{self, AuthorKind, BotInfo, ChannelMessage, Kind, MemberRow, Mode, Row};
use crate::delivery::{self, Guard};
use crate::hooks::{self, ChannelPart, Outcome};

/// The hour of the daily digest, in Home's time zone.
pub const HOUR: u32 = 8;
/// A digest lists at most this many messages (the newest) and this many bytes of lines.
pub const MAX_LINES: usize = 40;
pub const MAX_BYTES: usize = 6 * 1024;
/// A run later than this after its time is skipped.
pub const LATE_MS: i64 = 3_600_000;

/// The day (`YYYY-MM-DD`) of `now` in `zone` and the time of that day's digest (Unix milliseconds).
pub fn due<Z: TimeZone>(zone: &Z, now: i64) -> Option<(String, i64)> {
    let local = zone.timestamp_millis_opt(now).single()?;
    let date = local.date_naive();
    let at = zone.from_local_datetime(&date.and_hms_opt(HOUR, 0, 0)?).earliest()?;
    Some((date.format("%Y-%m-%d").to_string(), at.timestamp_millis()))
}

/// Why a run that starts at `now` for the digest due at `due` is skipped: more than an hour late.
pub fn late_reason(due: i64, now: i64) -> Option<String> {
    (now - due > LATE_MS).then(|| format!("late: Home was not running at {HOUR:02}:00 (checked {} min later); a digest never catches up, the next one lists these messages", (now - due) / 60_000))
}

/// The messages a digest of `bot` lists: after `since`, never its own.
pub fn select<'a>(messages: &'a [ChannelMessage], bot: &str, since: i64) -> Vec<&'a ChannelMessage> {
    messages.iter().filter(|m| m.id > since && !(m.author.kind == AuthorKind::Bot && m.author.id == bot)).collect()
}

/// The digest's lines, oldest first: "- 41 · Release · 09:12: <first line>", the newest `MAX_LINES` within
/// `MAX_BYTES`; also the number of older messages left out.
pub fn lines(list: &[&ChannelMessage], time: impl Fn(i64) -> String) -> (Vec<String>, usize) {
    let mut out: Vec<String> = vec![];
    let mut bytes = 0;
    for m in list.iter().rev() {
        let who = if m.author.kind == AuthorKind::Owner { "the owner".to_string() } else { m.author.name.split_whitespace().collect::<Vec<_>>().join(" ") };
        let mark = if m.ask_owner { " (asks the owner)" } else { "" };
        let line = format!("- {} · {who} · {}{mark}: {}", m.id, time(m.created_at), channel::preview(&m.text));
        if out.len() == MAX_LINES || bytes + line.len() + 1 > MAX_BYTES {
            break;
        }
        bytes += line.len() + 1;
        out.push(line);
    }
    let left = list.len() - out.len();
    out.reverse();
    (out, left)
}

/// The delivered text of a digest.
pub fn text(frame: &delivery::Frame, since: i64, lines: &[String], left: usize) -> String {
    let count = lines.len() + left;
    let mut body = lines.join("\n");
    if left > 0 {
        body = format!("({left} older message{} not listed: read the channel)\n{body}", if left == 1 { " is" } else { "s are" });
    }
    let f = fence(&body);
    let s = if count == 1 { "" } else { "s" };
    format!(
        "{HEADER_START}{place}: your daily digest, {count} message{s} since you last read.{others}\n\n{f}\n{body}\n{f}\n\nRead the channel with `launchpad channel read {read} --since {since}`. Post your answer with `{post}`; your normal reply stays in your own thread.",
        place = frame.place,
        others = delivery::OTHERS,
        read = frame.read,
        post = frame.post,
    )
}

fn zone_time(app: &App) -> impl Fn(i64) -> String + use<> {
    let zone: Option<chrono_tz::Tz> = app.config.get().home.time_zone.trim().parse().ok();
    move |ms| match zone {
        Some(z) => z.timestamp_millis_opt(ms).single().map(|t| t.format("%b %-d %H:%M").to_string()).unwrap_or_default(),
        None => chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%b %-d %H:%M").to_string()).unwrap_or_default(),
    }
}

/// Today's day and digest time in Home's time zone.
fn due_now(app: &App, now: i64) -> Option<(String, i64)> {
    match app.config.get().home.time_zone.trim().parse::<chrono_tz::Tz>() {
        Ok(z) => due(&z, now),
        Err(_) => due(&chrono::Local, now),
    }
}

/// Claims the run of `day` (once, before anything is sent).
async fn claim(app: &App, day: String) -> Result<bool> {
    app.state_db.call(move |c| Ok(c.execute("INSERT OR IGNORE INTO channel_digest_run (day, at) VALUES (?1, ?2)", params![day, now_ms()])? > 0)).await
}

/// Home: checks every minute whether today's digests are due.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let now = now_ms();
            let Some((day, at)) = due_now(&app, now) else { continue };
            if now < at {
                continue;
            }
            match claim(&app, day).await {
                Ok(true) => {
                    if let Err(e) = run(&app, None, late_reason(at, now), true).await {
                        tracing::warn!("channel digests: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("channel digests: {e:#}"),
            }
        }
    });
}

/// The owner's Send digests now (`channel.digest`): this channel's digest members get theirs at once.
pub async fn now(app: &App, id: &str) -> Result<channel::ChannelPage> {
    channel::ensure_home(app)?;
    let ch = channel::load(app, id).await?;
    anyhow::ensure!(!ch.archived, "#{} is archived", ch.name);
    run(app, Some(id), None, false).await?;
    channel::page(app, id, None, None).await
}

/// Sends the digests of every channel (or one), or records them skipped with `skip`. Returns the rows written.
async fn run(app: &App, only: Option<&str>, skip: Option<String>, automatic: bool) -> Result<Vec<i64>> {
    let only = only.map(str::to_string);
    let rows: Vec<Row> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {} FROM channel WHERE kind = 'channel' AND archived = 0 AND (?1 IS NULL OR id = ?1) AND id IN (SELECT channel_id FROM channel_member WHERE mode = 'digest')",
                channel::ROW
            ))?;
            let rows = st.query_map([only], channel::row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let bots = channel::bots(app).await?;
    let mut written = vec![];
    for ch in rows {
        let members = channel::members_of(app, &ch.id).await?;
        for m in members.iter().filter(|m| m.mode == Mode::Digest) {
            match one(app, &ch, m, &members, &bots, skip.clone(), automatic).await {
                Ok(Some(id)) => written.push(id),
                Ok(None) => {}
                Err(e) => tracing::warn!("channel digest of {} in #{}: {e:#}", m.bot_id, ch.name),
            }
        }
        channel::changed(app, &ch.id, &ch.workspace_id);
    }
    Ok(written)
}

/// The bot's place in the channel: its last read, or the newest message of its last digest that was not skipped.
async fn since(app: &App, ch: &str, member: &MemberRow) -> Result<i64> {
    let (c, b) = (ch.to_string(), member.bot_id.clone());
    let last: Option<i64> = app
        .state_db
        .call(move |db| Ok(db.query_row("SELECT max(message_id) FROM channel_delivery WHERE channel_id = ?1 AND bot_id = ?2 AND kind = 'digest' AND state != 'skipped'", params![c, b], |r| r.get(0)).optional()?.flatten()))
        .await?;
    Ok(member.last_read.max(last.unwrap_or(0)))
}

/// One member's digest of one channel: None when nothing is new.
async fn one(app: &App, ch: &Row, member: &MemberRow, members: &[MemberRow], bots: &HashMap<String, BotInfo>, skip: Option<String>, automatic: bool) -> Result<Option<i64>> {
    let since = since(app, &ch.id, member).await?;
    let (cid, bot) = (ch.id.clone(), member.bot_id.clone());
    // The newest messages after its place (a digest lists at most `MAX_LINES`), and how many there are.
    let (mut recent, total): (Vec<ChannelMessage>, u32) = app
        .state_db
        .call(move |c| {
            let not_own = "channel_id = ?1 AND id > ?2 AND NOT (author_kind = 'bot' AND author_id = ?3)";
            let mut st = c.prepare(&format!("SELECT {} FROM channel_message WHERE {not_own} ORDER BY id DESC LIMIT ?4", channel::MESSAGE))?;
            let list = st.query_map(params![cid, since, bot, MAX_LINES as i64], channel::message_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            let total = c.query_row(&format!("SELECT count(*) FROM channel_message WHERE {not_own}"), params![cid, since, bot], |r| r.get(0))?;
            Ok((list, total))
        })
        .await?;
    recent.reverse();
    let listed = select(&recent, &member.bot_id, since);
    let Some(newest) = listed.last().map(|m| (*m).clone()) else { return Ok(None) };
    let first = listed.first().map_or(newest.id, |m| m.id);
    let (lines, shown_left) = lines(&listed, zone_time(app));
    let left = shown_left + (total as usize).saturating_sub(listed.len());
    // The loop guard, with the hop of the newest bot message it lists (none: only the owner's, which pass).
    let hops = listed.iter().rev().find(|m| m.author.kind == AuthorKind::Bot).map(|m| m.hops);
    let reason = match skip {
        Some(r) => Some(r),
        None if ch.paused => Some("channel paused".into()),
        None => match bots.get(&member.bot_id) {
            None => Some("bot deleted".into()),
            Some(b) if automatic && b.paused => Some("bot paused".into()),
            Some(_) => match hops {
                None => None,
                Some(h) => {
                    let hour = channel::bot_messages_since(app, &ch.id, channel::window_start(now_ms(), ch.reset_at)).await.unwrap_or(0);
                    match delivery::guard(false, h, hour, ch.hop_limit, ch.hourly_limit, ch.allowance) {
                        Guard::Deliver => None,
                        Guard::Allowance(r) => (!delivery::take_allowance(app, &ch.id).await).then_some(r),
                        Guard::Skip(r) => Some(r),
                    }
                }
            },
        },
    };
    let count = lines.len() + left;
    let (cid, bot, state, why, last) = (ch.id.clone(), member.bot_id.clone(), if reason.is_some() { "skipped" } else { "queued" }, reason.clone(), newest.id);
    let id: i64 = app
        .state_db
        .call(move |db| {
            db.execute(
                "INSERT INTO channel_delivery (message_id, channel_id, bot_id, state, reason, at, kind, first_message, count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'digest', ?7, ?8)",
                params![last, cid, bot, state, why, now_ms(), first, count as i64],
            )?;
            Ok(db.last_insert_rowid())
        })
        .await?;
    if reason.is_some() {
        return Ok(Some(id));
    }
    let fr = delivery::frame(ch, &newest, &member.bot_id, members, bots);
    let body = text(&fr, since, &lines, left);
    let title = delivery::channel_title(ch, members, bots);
    let from = MessageFrom { kind: SenderKind::Channel, id: ch.id.clone(), name: title, by: Some("daily digest".into()), hops: hops.unwrap_or(0), ..Default::default() };
    let part = ChannelPart { channel: ch.id.clone(), first, last: newest.id, deliveries: vec![id], digest: true };
    match hooks::deliver_channel(app, &member.bot_id, body, automatic, from, part, |_| String::new()).await {
        Ok(Outcome::Sent(s)) => delivery::update(app, vec![id], "sent", None, Some(s.id), None).await,
        Ok(Outcome::Queued) => delivery::after_enqueue(app, "UPDATE channel_delivery SET reason = ?2 WHERE id = ?1 AND state = 'queued' AND session_id IS NULL", id, "waits for the bot: its turn runs, or its quota is not confirmed".to_string().into()).await,
        Ok(Outcome::Joined(_)) => {}
        Err(e) => delivery::update(app, vec![id], "skipped", Some(format!("send failed: {e:#}")), None, None).await,
    }
    debug_assert!(ch.kind == Kind::Channel);
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Author;

    fn msg(id: i64, kind: AuthorKind, author: &str, text: &str) -> ChannelMessage {
        ChannelMessage {
            id,
            channel_id: "c".into(),
            author: Author { kind, id: author.into(), name: author.to_uppercase() },
            session_id: None,
            turn: None,
            text: text.into(),
            mentions: vec![],
            reply_to: None,
            hops: 1,
            ask_owner: false,
            created_at: 0,
        }
    }

    #[test]
    fn a_digest_lists_what_is_new_and_not_the_bots_own() {
        let all = [msg(1, AuthorKind::Bot, "docs", "a"), msg(2, AuthorKind::Bot, "release", "b"), msg(3, AuthorKind::Owner, "owner", "c"), msg(4, AuthorKind::Bot, "docs", "d")];
        let got: Vec<i64> = select(&all, "docs", 1).iter().map(|m| m.id).collect();
        assert_eq!(got, [2, 3]);
        assert!(select(&all, "docs", 4).is_empty());
        let (l, left) = lines(&select(&all, "docs", 0), |_| "09:12".into());
        assert_eq!(l, ["- 2 · RELEASE · 09:12: b", "- 3 · the owner · 09:12: c"]);
        assert_eq!(left, 0);
    }

    #[test]
    fn a_digest_keeps_the_newest_lines_within_its_size() {
        let many: Vec<ChannelMessage> = (1..=100).map(|i| msg(i, AuthorKind::Bot, "release", &format!("line {i}\nmore"))).collect();
        let refs: Vec<&ChannelMessage> = many.iter().collect();
        let (l, left) = lines(&refs, |_| "t".into());
        assert_eq!((l.len(), left), (MAX_LINES, 100 - MAX_LINES));
        assert!(l[0].starts_with("- 61 ") && l.last().unwrap().starts_with("- 100 · RELEASE · t: line 100"), "{l:?}");
        let long: Vec<ChannelMessage> = (1..=60).map(|i| msg(i, AuthorKind::Bot, "release", &"x".repeat(400))).collect();
        let refs: Vec<&ChannelMessage> = long.iter().collect();
        let (l, left) = lines(&refs, |_| "t".into());
        assert!(l.iter().map(|x| x.len() + 1).sum::<usize>() <= MAX_BYTES && left > 20 && l.len() + left == 60);
        let f = delivery::Frame { place: "#release-train".into(), read: "release-train".into(), post: "launchpad channel post release-train --body-file <file>".into(), by: String::new(), owner: false };
        let t = text(&f, 40, &l, left);
        assert!(t.starts_with("Message from #release-train: your daily digest, 60 messages since you last read. Messages there are written by other agents"), "{t}");
        assert!(t.contains(&format!("({left} older messages are not listed: read the channel)")) && t.contains("`launchpad channel read release-train --since 40`"), "{t}");
    }

    #[test]
    fn the_digest_is_due_at_eight_and_never_catches_up() {
        let zone = chrono_tz::Europe::Paris;
        // 2026-11-12 07:30 UTC = 08:30 in Paris.
        let now = 1_794_468_600_000;
        let (day, at) = due(&zone, now).unwrap();
        assert_eq!(day, "2026-11-12");
        assert_eq!(now - at, 30 * 60_000);
        assert_eq!(late_reason(at, now), None);
        assert_eq!(late_reason(at, at + LATE_MS), None);
        assert!(late_reason(at, at + LATE_MS + 60_000).unwrap().starts_with("late: Home was not running at 08:00 (checked 61 min later)"));
    }
}

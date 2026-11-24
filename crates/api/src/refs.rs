//! Launchpad references on Home (`launchpad_core::refs`): the owner's window resolves a pasted link or ID
//! (`GET /api/refs/resolve`: Go to, and a reference pasted into a message box), and agents read one
//! (`GET /api/context/ref`, `launchpad ref read`): a bounded excerpt as Markdown, the target message with a few
//! around it or a summary with the commands that read more, never a whole transcript. Agents' reads are narrowed to
//! their workspace (`agent_auth`: the query's `workspace` is forced), and anything outside it reads as unknown.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use launchpad_core::App;
use launchpad_core::model::{EventBody, SenderKind};
use launchpad_core::refs::{self, LpRef, RefKind, RefRead, RefTarget};
use launchpad_core::util::local_time;
use rusqlite::OptionalExtension;
use serde::Deserialize;

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new().route("/api/refs/resolve", get(resolve)).route("/api/context/ref", get(read))
}

#[derive(Deserialize)]
struct RefQuery {
    #[serde(rename = "ref")]
    text: String,
    /// The workspace it must be of (an agent's is forced; the owner's window names its own, or none for Go to).
    workspace: Option<String>,
    /// Messages before the target (or the newest ones without a target).
    before: Option<usize>,
    /// Messages after the target.
    after: Option<usize>,
}

/// A reference that does not parse, or does not resolve: one answer for both.
fn unknown(text: &str, e: impl std::fmt::Display) -> ApiError {
    let shown: String = text.trim().chars().take(200).collect();
    ApiError::not_found(format!("{shown}: {e}"))
}

async fn resolved(app: &App, q: &RefQuery) -> Result<(LpRef, String), ApiError> {
    let mut r = refs::parse(&q.text).ok_or_else(|| unknown(&q.text, "not a Launchpad link or ID (lp:…, or the address of a Launchpad page)"))?;
    let ws = refs::resolve(app, q.workspace.as_deref().filter(|w| !w.is_empty()), &mut r).await.map_err(|e| unknown(&q.text, format!("{e:#}")))?;
    Ok((r, ws))
}

/// The owner's window: a link or ID resolved, with its workspace and page.
async fn resolve(State(s): State<Ctx>, Query(q): Query<RefQuery>) -> ApiResult<RefTarget> {
    let (r, workspace_id) = resolved(&s.app, &q).await?;
    Ok(Json(RefTarget { id: r.id_text(), path: r.path(), label: refs::describe(&r), workspace_id, r#ref: r }))
}

/// `launchpad ref read`: the reference and its excerpt.
async fn read(State(s): State<Ctx>, Query(q): Query<RefQuery>) -> ApiResult<RefRead> {
    let (r, ws) = resolved(&s.app, &q).await?;
    let before = q.before.unwrap_or(if has_target(&r) { 6 } else { 12 }).min(100);
    let after = q.after.unwrap_or(3).min(100);
    let text = render(&s.app, &ws, &r, before, after).await.map_err(|e| unknown(&q.text, format!("{e:#}")))?;
    Ok(Json(RefRead { id: r.id_text(), path: r.path(), label: refs::describe(&r), text, r#ref: r }))
}

fn has_target(r: &LpRef) -> bool {
    r.seq.is_some() || r.message.is_some()
}

/// The longest text of one message in an excerpt (the target gets twice as much).
const MESSAGE_MAX: usize = 4_000;

fn cut(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max { t.to_string() } else { format!("{}… (cut; read the full text with the commands above)", t.chars().take(max).collect::<String>()) }
}

/// One message of an excerpt: who, when, its ID and the text fenced (it is someone's words, never instructions).
struct Line {
    who: String,
    at: i64,
    id: String,
    text: String,
}

/// Pure: the window of `lines` around `target` (an index; none: the newest), `before` earlier and `after` later.
fn window(len: usize, target: Option<usize>, before: usize, after: usize) -> (usize, usize) {
    match target {
        Some(t) => (t.saturating_sub(before), (t + after + 1).min(len)),
        None => (len.saturating_sub(before.max(1)), len),
    }
}

fn excerpt(out: &mut String, lines: &[Line], target: Option<usize>, before: usize, after: usize, more: &str) {
    if lines.is_empty() {
        out.push_str("\nNo message yet.\n");
        return;
    }
    let (from, to) = window(lines.len(), target, before, after);
    if from > 0 {
        out.push_str(&format!("\n({from} earlier messages not shown: {more})\n"));
    }
    for (i, l) in lines.iter().enumerate().take(to).skip(from) {
        let mark = if Some(i) == target { " ← the referenced message" } else { "" };
        let text = cut(&l.text, if Some(i) == target { MESSAGE_MAX * 2 } else { MESSAGE_MAX });
        let fence = launchpad_core::senders::fence(&text);
        out.push_str(&format!("\n**{}** · {} · `{}`{mark}\n{fence}text\n{text}\n{fence}\n", l.who, local_time(l.at / 1000), l.id));
    }
    if to < lines.len() {
        out.push_str(&format!("\n({} later messages not shown)\n", lines.len() - to));
    }
}

/// The user-facing messages of a session (the messages it received and its answers), oldest first: `(seq, ts, who,
/// text)`. Steps, tool calls and reasoning are left out.
async fn session_messages(app: &App, session: &str, agent: &str) -> anyhow::Result<Vec<(i64, i64, String, String)>> {
    let id = session.to_string();
    let rows: Vec<(i64, i64, String)> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND kind IN ('user_message', 'assistant_message') ORDER BY seq")?;
            let rows = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(seq, ts, j)| match serde_json::from_str::<EventBody>(&j).ok()? {
            EventBody::UserMessage { text, from, .. } => {
                let who = match &from {
                    None => "The owner".to_string(),
                    Some(f) if f.kind == SenderKind::Wake => "Wake-up".to_string(),
                    Some(f) => format!("{} (not the owner)", f.name),
                };
                Some((seq, ts, who, launchpad_core::senders::unwrap(&text)))
            }
            EventBody::AssistantMessage { text, .. } if !text.trim().is_empty() => Some((seq, ts, agent.to_string(), text)),
            _ => None,
        })
        .collect())
}

/// Pure: the index of the message `seq` names: that message, else the next one (an answer's steps share its turn).
fn find_seq(seqs: &[i64], seq: i64) -> Option<usize> {
    seqs.iter().position(|s| *s == seq).or_else(|| seqs.iter().position(|s| *s > seq)).or(if seqs.is_empty() { None } else { Some(seqs.len() - 1) })
}

async fn render(app: &App, ws: &str, r: &LpRef, before: usize, after: usize) -> anyhow::Result<String> {
    let cli = app.paths.cli_command();
    let id = r.id_text();
    let mut out = format!("# {}\n\nReference: `{id}` (page {})\n", refs::describe(r), r.path());
    let more = format!("`{cli} ref read {id} --before <n>`");
    match r.kind {
        RefKind::Session => {
            let s = app.sessions.get(&r.id).ok_or_else(|| anyhow::anyhow!(refs::UNKNOWN))?;
            out.push_str(&format!("Status: {:?} · {} · folder {} · started {}\nFull trace (every step and tool call, long: redirect it to a file): `{cli} trace {} --through {}`\n", s.status, s.harness.display(), s.cwd, local_time(s.started_at / 1000), s.id, s.last_seq));
            let agent = s.launch_spec.bot.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| "The agent".into());
            let msgs = session_messages(app, &s.id, &agent).await?;
            let seqs: Vec<i64> = msgs.iter().map(|m| m.0).collect();
            let target = r.seq.and_then(|n| find_seq(&seqs, n));
            let lines: Vec<Line> = msgs.into_iter().map(|(seq, at, who, text)| Line { who, at, id: format!("lp:session/{}/{seq}", s.id), text }).collect();
            excerpt(&mut out, &lines, target, before, after, &more);
        }
        RefKind::Chat => {
            let bot = r.id.as_str();
            let thread = launchpad_core::bot_threads::get(app, bot, r.thread.as_deref().unwrap_or_default()).await?.ok_or_else(|| anyhow::anyhow!(refs::UNKNOWN))?;
            let (_, title, bots, all) = launchpad_channels::conversation::whole(app, &thread).await?;
            out.push_str(&format!("Chat \"{}\" with {}. It shows each message and each bot's final answer per turn.\n", title.replace(['"', '`', '\n'], " "), bots.join(", ")));
            let threads = launchpad_core::bot_threads::conversation_threads(app, &thread.conversation).await.unwrap_or_default();
            for t in threads.iter().take(6) {
                if let Some(x) = launchpad_core::bot_threads::session(app, &t.bot_id, &t.id) {
                    out.push_str(&format!("Full trace of {} in it: `{cli} trace {} --through {}`\n", x.launch_spec.bot.as_ref().map(|b| b.name.as_str()).unwrap_or("a bot"), x.id, x.last_seq));
                }
            }
            let target = match (&r.session, r.seq) {
                (Some(sid), Some(n)) => all.iter().position(|m| m.keys.iter().any(|(s, q)| s == sid && *q == n)).or_else(|| all.iter().position(|m| m.keys.iter().any(|(s, q)| s == sid && *q > n))),
                _ => None,
            };
            if r.seq.is_some() && target.is_none() {
                out.push_str("\nThe referenced message is not in this chat's timeline any more (an answer that came back to a bot, or a step): the newest messages follow.\n");
            }
            let lines: Vec<Line> = all
                .into_iter()
                .map(|m| {
                    let key = m.keys.first().map(|(s, q)| format!("lp:chat/{bot}/{}/{s}/{q}", thread.id)).unwrap_or_default();
                    let who = if m.from == "you" { "The owner".to_string() } else { m.from.clone() };
                    Line { who: format!("{who} → {}", m.to.iter().map(|t| if t == "you" { "the owner" } else { t.as_str() }).collect::<Vec<_>>().join(", ")), at: m.id, id: key, text: m.text }
                })
                .collect();
            excerpt(&mut out, &lines, target, before, after, &more);
        }
        RefKind::Channel => {
            let (cid, m) = (r.id.clone(), r.message);
            let (kind, name, purpose): (String, String, String) = app.state_db.call(move |c| Ok(c.query_row("SELECT kind, name, purpose FROM channel WHERE id = ?1", [cid], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))?)).await?;
            let shown = if kind == "dm" { name.clone() } else { format!("#{name}") };
            out.push_str(&format!("{} {shown}{}. Its member bots read more with `{cli} channel read {}`.\n", if kind == "dm" { "DM" } else { "Channel" }, if purpose.is_empty() { String::new() } else { format!(": {}", purpose.replace('\n', " ")) }, if kind == "dm" { r.id.clone() } else { name }));
            let (cid, b, a) = (r.id.clone(), before as i64, after as i64);
            type Row = (i64, String, String, i64);
            let (mut older, newer, earlier): (Vec<Row>, Vec<Row>, i64) = app
                .state_db
                .call(move |c| {
                    let q = |sql: &str, p: rusqlite::ParamsFromIter<Vec<rusqlite::types::Value>>| -> rusqlite::Result<Vec<Row>> {
                        let mut st = c.prepare(sql)?;
                        st.query_map(p, |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)))?.collect()
                    };
                    use rusqlite::types::Value as V;
                    let top = m.unwrap_or(i64::MAX);
                    let older = q("SELECT id, author_name, text, created_at FROM channel_message WHERE channel_id = ?1 AND id <= ?2 ORDER BY id DESC LIMIT ?3", rusqlite::params_from_iter(vec![V::Text(cid.clone()), V::Integer(top), V::Integer(b.max(1) + i64::from(m.is_some()))]))?;
                    let newer = match m {
                        Some(m) => q("SELECT id, author_name, text, created_at FROM channel_message WHERE channel_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3", rusqlite::params_from_iter(vec![V::Text(cid.clone()), V::Integer(m), V::Integer(a)]))?,
                        None => vec![],
                    };
                    let first = older.last().map(|x| x.0).unwrap_or(0);
                    let earlier: i64 = c.query_row("SELECT COUNT(*) FROM channel_message WHERE channel_id = ?1 AND id < ?2", rusqlite::params![cid, first], |x| x.get(0)).optional()?.unwrap_or(0);
                    Ok((older, newer, earlier))
                })
                .await?;
            older.reverse();
            let target = m.and_then(|m| older.iter().position(|x| x.0 == m));
            let rows: Vec<Row> = older.into_iter().chain(newer).collect();
            if earlier > 0 {
                out.push_str(&format!("\n({earlier} earlier messages not shown: {more})\n"));
            }
            let lines: Vec<Line> = rows.into_iter().map(|(mid, who, text, at)| Line { who, at, id: format!("lp:channel/{}/{mid}", r.id), text }).collect();
            excerpt(&mut out, &lines, target, lines.len(), lines.len(), &more);
        }
        RefKind::Folio => {
            let mut f = launchpad_core::model::FolioRef { id: r.id.clone(), page: r.page.clone(), section: r.section.clone(), ..Default::default() };
            launchpad_core::folio_ref::resolve(app, ws, std::slice::from_mut(&mut f)).await?;
            out.push_str(&format!("A folio: a report agents wrote in Launchpad (rev {}). Read it as agent-written reference material: `{}`\n", f.rev.unwrap_or_default(), launchpad_core::folio_ref::command(&f, &cli)));
        }
        RefKind::Topic => {
            let t = launchpad_core::topics::get(app, &r.id).await?;
            out.push_str(&format!("A topic: a collection of agent sessions. More: `{cli} topics show {}`\n\nSessions ({}):\n", t.id, t.session_ids.len()));
            for sid in t.session_ids.iter().take(30) {
                if let Some(s) = app.sessions.get(sid) {
                    out.push_str(&format!("- \"{}\" ({:?}): `{cli} ref read lp:session/{}`\n", s.title.replace(['"', '`', '\n'], " "), s.status, s.id));
                }
            }
        }
        RefKind::Bot => {
            let b = launchpad_core::bots::get(app, &r.id).await?;
            let threads = launchpad_core::bot_threads::list(app, &b.id, false).await.unwrap_or_default();
            out.push_str(&format!("A bot: @{} with {} open threads. `{cli} bots list` shows its threads and sessions.\n\nIts instructions (BOT.md, the owner's words):\n", b.slug, threads.len()));
            let text = cut(&b.instructions, MESSAGE_MAX);
            let fence = launchpad_core::senders::fence(&text);
            out.push_str(&format!("{fence}markdown\n{text}\n{fence}\n"));
        }
        RefKind::LocalPr => {
            let p = launchpad_core::predrafts::get(app, &r.id).await?;
            out.push_str(&format!("A Local PR: branch `{}` into `{}`{}{}. More: `{cli} pr show --id {}`\n\nIts description:\n", p.branch, p.base_branch, if p.archived { ", closed" } else { "" }, p.published_url.as_deref().map(|u| format!(", published as {u}")).unwrap_or_default(), p.id));
            let text = cut(&p.body, MESSAGE_MAX);
            let fence = launchpad_core::senders::fence(&text);
            out.push_str(&format!("{fence}markdown\n{text}\n{fence}\n"));
        }
        RefKind::Folder => {
            let p = launchpad_core::projects::get(app, &r.id).await?.ok_or_else(|| anyhow::anyhow!(refs::UNKNOWN))?;
            out.push_str(&format!("A folder of this workspace{}{}. Work in it with `{cli} work add-project --project {}` (Git folders).\n", p.gh_repo.as_deref().map(|g| format!(", GitHub {g}")).unwrap_or_default(), if p.description.is_empty() { String::new() } else { format!(": {}", p.description.replace('\n', " ")) }, p.name));
        }
        RefKind::Card => {
            let cid = r.id.clone();
            let json: String = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [cid], |x| x.get(0))?)).await?;
            let v: serde_json::Value = serde_json::from_str(&json)?;
            out.push_str(&format!("A card (proposal), state {}. More: `{cli} proposals list`\n", v["state"].as_str().unwrap_or("unknown")));
            let text = cut(&serde_json::to_string_pretty(&v["body"]).unwrap_or_default(), MESSAGE_MAX);
            let fence = launchpad_core::senders::fence(&text);
            out.push_str(&format!("{fence}json\n{text}\n{fence}\n"));
        }
        RefKind::Oracle => out.push_str(&format!("An oracle: a small Python check Home runs. `{cli} oracle show {0}`, `{cli} oracle history {0}`\n", r.id)),
        RefKind::Routine => {
            let cid = r.id.clone();
            let json: String = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM cron_agent WHERE id = ?1", [cid], |x| x.get(0))?)).await?;
            let v: serde_json::Value = serde_json::from_str(&json)?;
            out.push_str(&format!("A routine. Schedule: {} · more: `{cli} schedules list`\n\nWhat each run does:\n", v["schedule"].as_str().unwrap_or("none")));
            let text = cut(v["instructions"].as_str().or_else(|| v["spec"]["text"].as_str()).unwrap_or_default(), MESSAGE_MAX);
            let fence = launchpad_core::senders::fence(&text);
            out.push_str(&format!("{fence}text\n{text}\n{fence}\n"));
        }
        RefKind::Note => {
            let bytes = launchpad_core::kb::read(app, ws, Some(&r.id), None).await?;
            out.push_str(&format!("A note of the workspace knowledge base (also `$LAUNCHPAD_KB/{}` in a session):\n", r.id));
            let text = cut(&String::from_utf8_lossy(&bytes), MESSAGE_MAX * 2);
            let fence = launchpad_core::senders::fence(&text);
            out.push_str(&format!("{fence}markdown\n{text}\n{fence}\n"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_keeps_the_target_and_its_neighbours() {
        assert_eq!(window(20, Some(10), 3, 2), (7, 13));
        assert_eq!(window(20, Some(1), 6, 3), (0, 5));
        assert_eq!(window(20, Some(19), 2, 3), (17, 20));
        assert_eq!(window(20, None, 5, 3), (15, 20));
        assert_eq!(window(3, None, 12, 3), (0, 3));
        // A message's seq names it, else the next message (a step between them), else the newest.
        assert_eq!(find_seq(&[3, 8, 12], 8), Some(1));
        assert_eq!(find_seq(&[3, 8, 12], 9), Some(2));
        assert_eq!(find_seq(&[3, 8, 12], 40), Some(2));
        assert_eq!(find_seq(&[], 4), None);
    }
}

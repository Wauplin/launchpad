//! Multi-bot conversations on Home (`launchpad_core::bot_threads`): several bots share one conversation, each in its
//! own thread (its own session and context). Two things happen here:
//!
//! - Answers go back (`turn_ended`): when a bot's turn that answered another bot's message of the conversation ends
//!   Done, its final answer goes to the asking bot's thread of the conversation as a message from the answering bot
//!   (`MessageFrom.reply`, under its header), one hop after the message it answers. A turn started by the owner's
//!   message sends nothing; a reply is never answered in turn. Each answer is claimed once (`conversation_reply`)
//!   before it is sent and never sent again.
//! - Pull, not push (`timeline`): `launchpad conversation read` lists the conversation (the owner's messages with
//!   the bots they went to, each bot's final answers, the messages bots sent each other), so a bot reads the rest of
//!   its conversation on demand; it never receives it automatically.

use anyhow::{Context, Result};
use serde::Serialize;
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::bot_threads::{self, To};
use launchpad_core::model::{EventBody, MessageFrom, SenderKind, Session, SessionStatus};
use launchpad_core::senders;
use launchpad_core::util::now_ms;

/// Events of a session this module reads, oldest first: the sequence number, the time and the event.
pub type Stored = (i64, i64, EventBody);

/// The answer a turn sends back: the message it answers (the request of another bot of the conversation), the
/// sequence number of the turn's message (the claim) and the final answer.
#[derive(Debug, PartialEq)]
pub struct Answer {
    pub request: MessageFrom,
    pub claim: i64,
    pub text: String,
}

/// Pure: the answer a session's last turn owes another bot, from its events. The turn's message is its last
/// message: a bot's request (not itself an answer) is owed an answer; so is the newest request before it when the
/// turn's message is an answer that came back to this bot (it asked a third bot meanwhile) or the wake-up it asked
/// for, unless the owner wrote in between. The owner's message, a routine, a hook or a channel owes nothing.
pub fn owed(events: &[Stored]) -> Option<Answer> {
    let messages: Vec<(usize, i64, Option<&MessageFrom>)> = events.iter().enumerate().filter_map(|(i, (seq, _, e))| match e {
        EventBody::UserMessage { from, .. } => Some((i, *seq, from.as_ref())),
        _ => None,
    }).collect();
    let &(last_at, claim, last) = messages.last()?;
    let request = |f: &MessageFrom| f.kind == SenderKind::Bot && !f.reply && f.session.is_some();
    let asked = match last {
        Some(f) if request(f) => f,
        Some(f) if f.reply || f.kind == SenderKind::Wake => {
            let mut found = None;
            for &(_, _, from) in messages[..messages.len() - 1].iter().rev() {
                match from {
                    None => break,
                    Some(f) if request(f) => {
                        found = Some(f);
                        break;
                    }
                    Some(_) => {}
                }
            }
            found?
        }
        _ => return None,
    };
    let text = events[last_at + 1..].iter().rev().find_map(|(_, _, e)| match e {
        EventBody::AssistantMessage { text, .. } if !text.trim().is_empty() => Some(text.clone()),
        _ => None,
    })?;
    Some(Answer { request: asked.clone(), claim, text })
}

/// The stored events of a session that matter here (messages, answers, turn ends), oldest first.
async fn events(app: &App, session: &str) -> Result<Vec<Stored>> {
    let id = session.to_string();
    let rows: Vec<(i64, i64, String)> = app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND kind IN ('user_message', 'assistant_message', 'turn_end') ORDER BY seq")?;
        let rows = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    Ok(rows.into_iter().filter_map(|(seq, ts, j)| serde_json::from_str::<EventBody>(&j).ok().map(|e| (seq, ts, e))).collect())
}

/// Home: a bot session's turn ended. When it answered another bot of its conversation, its answer goes back.
pub fn turn_ended(app: &App, s: &Session) {
    if s.launch_spec.bot.is_none() || s.status != SessionStatus::Waiting || s.open_question || s.sleeping() {
        return;
    }
    let (app, s) = (app.clone(), s.clone());
    tokio::spawn(async move {
        if let Err(e) = answer(&app, &s).await {
            tracing::warn!("conversation answer of session {}: {e:#}", s.id);
        }
    });
}

async fn answer(app: &App, s: &Session) -> Result<()> {
    let Some(a) = owed(&events(app, &s.id).await?) else { return Ok(()) };
    let Some(mine) = bot_threads::of_session(app, s).await else { return Ok(()) };
    // The asking bot's thread must be of the same conversation (a message from outside it is a plain request).
    let asker = a.request.session.as_deref().and_then(|id| app.sessions.get(id));
    let Some(theirs) = (match &asker {
        Some(x) => bot_threads::of_session(app, x).await,
        None => None,
    }) else { return Ok(()) };
    if theirs.conversation != mine.conversation || theirs.bot_id == mine.bot_id {
        return Ok(());
    }
    let hops = a.request.hops + 1;
    let refused = senders::guard(hops).err().map(|e| format!("{e:#}"));
    if !claim(app, &s.id, a.claim, refused.as_deref().unwrap_or("sending")).await? {
        return Ok(());
    }
    if let Some(why) = refused {
        tracing::info!("conversation answer of session {} not sent: {why}", s.id);
        return Ok(());
    }
    let b = s.launch_spec.bot.as_ref().context("a bot session")?;
    let from = MessageFrom { kind: SenderKind::Bot, id: b.id.clone(), name: b.name.clone(), hops, session: Some(s.id.clone()), reply: true, ..Default::default() };
    let to = To::Conversation { id: mine.conversation.clone(), title: String::new() };
    let outcome = match crate::hooks::deliver_reply(app, &theirs.bot_id, to, a.text, from).await {
        Ok(crate::hooks::Outcome::Sent(_)) => "sent".to_string(),
        Ok(_) => "queued".to_string(),
        Err(e) => format!("failed: {e:#}"),
    };
    let (id, seq) = (s.id.clone(), a.claim);
    app.state_db.call(move |c| Ok(c.execute("UPDATE conversation_reply SET outcome = ?3 WHERE session_id = ?1 AND seq = ?2", rusqlite::params![id, seq, outcome])?)).await?;
    Ok(())
}

/// Claims the answer of the turn whose message is `seq` (true once per message).
async fn claim(app: &App, session: &str, seq: i64, outcome: &str) -> Result<bool> {
    let (id, outcome) = (session.to_string(), outcome.to_string());
    app.state_db.call(move |c| Ok(c.execute("INSERT OR IGNORE INTO conversation_reply (session_id, seq, at, outcome) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![id, seq, now_ms(), outcome])? > 0)).await
}

// ---------------------------------------------------------------- the timeline

/// One message of a conversation, as `launchpad conversation read` lists it.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct ConversationMessage {
    /// Its time (Unix ms), also the cursor of `--since`.
    #[ts(type = "number")]
    pub id: i64,
    /// "you" (the owner), a bot's name, or another sender (a routine, a hook, an agent).
    pub from: String,
    /// Whom it went to: bots' names, or "you" for a bot's answer to the owner.
    pub to: Vec<String>,
    pub text: String,
}

/// A participant's session with its stored events, for `timeline`.
pub struct Part<'a> {
    pub bot: &'a str,
    pub events: &'a [Stored],
}

/// The owner's same message to several bots counts once when they arrive this close together.
const SAME_MESSAGE_MS: i64 = 10_000;
/// The longest text of one message in the timeline.
const MAX_TEXT: usize = 8_000;

fn cut(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= MAX_TEXT { t.to_string() } else { format!("{}… (cut)", t.chars().take(MAX_TEXT).collect::<String>()) }
}

/// Pure: the timeline of a conversation, oldest first: the owner's messages with the bots they went to (the same
/// message to several bots once), messages bots sent each other (once, as received), and each bot's final answer
/// per turn (to whoever asked). Answers that came back to a bot (`reply`) and wake-ups are left out: the answer
/// already shows as its author's.
pub fn timeline(parts: &[Part]) -> Vec<ConversationMessage> {
    let mut out: Vec<ConversationMessage> = vec![];
    for p in parts {
        let mut asker: String = "you".into();
        let mut answer: Option<(i64, String)> = None;
        let flush = |out: &mut Vec<ConversationMessage>, answer: &mut Option<(i64, String)>, asker: &str| {
            if let Some((at, text)) = answer.take() {
                out.push(ConversationMessage { id: at, from: p.bot.to_string(), to: vec![asker.to_string()], text: cut(&text) });
            }
        };
        for (_, ts, e) in p.events {
            match e {
                EventBody::UserMessage { text, from, .. } => {
                    flush(&mut out, &mut answer, &asker);
                    match from {
                        None => {
                            asker = "you".into();
                            let text = cut(text);
                            match out.iter_mut().find(|m| m.from == "you" && m.text == text && (m.id - ts).abs() <= SAME_MESSAGE_MS) {
                                Some(m) if !m.to.iter().any(|t| t == p.bot) => m.to.push(p.bot.to_string()),
                                Some(_) => {}
                                None => out.push(ConversationMessage { id: *ts, from: "you".into(), to: vec![p.bot.to_string()], text }),
                            }
                        }
                        Some(f) if f.reply || f.kind == SenderKind::Wake => {}
                        Some(f) => {
                            asker = f.name.clone();
                            out.push(ConversationMessage { id: *ts, from: f.name.clone(), to: vec![p.bot.to_string()], text: cut(&senders::unwrap(text)) });
                        }
                    }
                }
                EventBody::AssistantMessage { text, .. } if !text.trim().is_empty() => answer = Some((*ts, text.clone())),
                EventBody::TurnEnd { .. } => flush(&mut out, &mut answer, &asker),
                _ => {}
            }
        }
        flush(&mut out, &mut answer, &asker);
    }
    out.sort_by_key(|m| m.id);
    out
}

/// What `launchpad conversation read` answers.
#[derive(Clone, Debug, Serialize, TS)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// Its bots' names, the main bot first.
    pub bots: Vec<String>,
    /// The messages after `since`, at most `limit` (the newest ones).
    pub messages: Vec<ConversationMessage>,
    /// Earlier messages left out by `limit`.
    #[ts(type = "number")]
    pub earlier: usize,
}

/// Home: the conversation of a bot session, its messages after `since` (the newest `limit`).
pub async fn read(app: &App, session: &str, since: Option<i64>, limit: usize) -> Result<Conversation> {
    let s = app.sessions.get(session).context("no such session")?;
    let mine = bot_threads::of_session(app, &s).await.context("this session is not a bot's thread: it has no conversation")?;
    let threads = bot_threads::conversation_threads(app, &mine.conversation).await?;
    let main = threads.iter().find(|t| t.bot_id == mine.main_bot()).cloned().unwrap_or_else(|| mine.clone());
    let mut names: Vec<(String, String)> = vec![];
    let mut loaded: Vec<(String, Vec<Stored>)> = vec![];
    for t in &threads {
        let name = match names.iter().find(|(id, _)| *id == t.bot_id) {
            Some((_, n)) => n.clone(),
            None => {
                let n = launchpad_core::bots::get(app, &t.bot_id).await.map(|b| b.settings.name).unwrap_or_else(|_| "A deleted bot".into());
                names.push((t.bot_id.clone(), n.clone()));
                n
            }
        };
        for x in bot_threads::sessions(app, &t.bot_id, &t.id) {
            loaded.push((name.clone(), events(app, &x.id).await?));
        }
    }
    let parts: Vec<Part> = loaded.iter().map(|(bot, events)| Part { bot, events }).collect();
    let all: Vec<ConversationMessage> = timeline(&parts).into_iter().filter(|m| since.is_none_or(|c| m.id > c)).collect();
    let earlier = all.len().saturating_sub(limit);
    let mut bots: Vec<String> = names.iter().filter(|(id, _)| *id == main.bot_id).chain(names.iter().filter(|(id, _)| *id != main.bot_id)).map(|(_, n)| n.clone()).collect();
    bots.dedup();
    Ok(Conversation { id: mine.conversation.clone(), title: main.title.clone(), bots, messages: all.into_iter().skip(earlier).collect(), earlier })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(seq: i64, text: &str, from: Option<MessageFrom>) -> Stored {
        (seq, seq * 1000, EventBody::UserMessage { text: text.into(), client_id: format!("c{seq}"), queued: false, images: vec![], from })
    }
    fn said(seq: i64, text: &str) -> Stored {
        (seq, seq * 1000, EventBody::AssistantMessage { item_id: format!("i{seq}"), text: text.into() })
    }
    fn end(seq: i64) -> Stored {
        (seq, seq * 1000, EventBody::TurnEnd { duration_ms: None, interrupted: false, error: None })
    }
    fn bot(name: &str, hops: u32) -> MessageFrom {
        MessageFrom { kind: SenderKind::Bot, id: format!("{name}-id"), name: name.into(), hops, session: Some(format!("{name}-s")), ..Default::default() }
    }
    fn reply(name: &str, hops: u32) -> MessageFrom {
        MessageFrom { reply: true, ..bot(name, hops) }
    }

    #[test]
    fn a_turn_that_answers_another_bot_sends_its_final_answer_back() {
        let ev = vec![user(1, "check CI", Some(bot("A", 1))), said(2, "Looking."), said(3, "CI is green."), end(4)];
        assert_eq!(owed(&ev), Some(Answer { request: bot("A", 1), claim: 1, text: "CI is green.".into() }));
    }

    #[test]
    fn the_owner_s_own_message_owes_nothing() {
        let ev = vec![user(1, "check CI", Some(bot("A", 1))), said(2, "green"), end(3), user(4, "and now?", None), said(5, "still green"), end(6)];
        assert_eq!(owed(&ev), None);
        // Nor does a turn without an answer, a routine or a hook.
        assert_eq!(owed(&[user(1, "x", Some(bot("A", 1))), end(2)]), None);
        let hook = MessageFrom { kind: SenderKind::Hook, ..Default::default() };
        assert_eq!(owed(&[user(1, "x", Some(hook)), said(2, "y"), end(3)]), None);
    }

    #[test]
    fn an_answer_that_came_back_finishes_the_request_it_served() {
        // A asked B; B asked C and said so; C's answer came back and B's next turn answers A again.
        let ev = vec![user(1, "compare", Some(bot("A", 1))), said(2, "Asked C."), end(3), user(4, "C's data", Some(reply("C", 3))), said(5, "B and C agree."), end(6)];
        let a = owed(&ev).unwrap();
        assert_eq!((a.request.name.as_str(), a.request.hops, a.claim, a.text.as_str()), ("A", 1, 4, "B and C agree."));
        // An answer is never answered in turn when no request waits (A's own thread gets B's answer).
        assert_eq!(owed(&[user(1, "hi", None), said(2, "asked B"), end(3), user(4, "B says", Some(reply("B", 2))), said(5, "ok"), end(6)]), None);
        // The owner wrote in between: the earlier request is settled.
        assert_eq!(owed(&[user(1, "x", Some(bot("A", 1))), said(2, "y"), end(3), user(4, "stop", None), said(5, "ok"), end(6), user(7, "z", Some(reply("C", 2))), said(8, "w"), end(9)]), None);
    }

    #[test]
    fn the_timeline_shows_each_message_once() {
        let a = vec![user(1, "compare the flows", None), said(3, "Asked B."), end(4), user(9, "B's answer", Some(reply("B", 2))), said(10, "Both match."), end(11)];
        let b = vec![user(2, "compare the flows", None), said(5, "On it"), end(6), user(7, &senders::wrap(&bot("A", 1), "What is yours?"), Some(bot("A", 1))), said(8, "Mine is X."), end(8)];
        let t = timeline(&[Part { bot: "A", events: &a }, Part { bot: "B", events: &b }]);
        let lines: Vec<String> = t.iter().map(|m| format!("{} > {}: {}", m.from, m.to.join(","), m.text)).collect();
        assert_eq!(lines, vec![
            "you > A,B: compare the flows",
            "A > you: Asked B.",
            "B > you: On it",
            "A > B: What is yours?",
            "B > A: Mine is X.",
            "A > you: Both match.",
        ]);
    }
}

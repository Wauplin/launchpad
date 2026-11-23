//! The spotlight's pure logic: Ask targets (the web's `askTarget` plus a device override; Tab cycles), the rows each
//! input shows with what Enter does, the text kept for 15 minutes after Esc, and the answer read from a thread's
//! session events. The GTK window (`spotlight.rs`) only draws these.

use serde_json::Value;

use crate::grammar::{self, Input};
use crate::search::{self, Entry};

/// A bot of the workspace, as the spotlight needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotInfo {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub admin: bool,
    pub created_at: i64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Bot(BotInfo),
    NewAgent,
}

impl Target {
    pub fn name(&self) -> &str {
        match self {
            Target::Bot(b) => &b.name,
            Target::NewAgent => "New agent",
        }
    }
}

/// The web's `askTarget`: the oldest bot with admin Launchpad CLI access, else the oldest bot.
pub fn ask_target(bots: &[BotInfo]) -> Option<&BotInfo> {
    let oldest = |admin: bool| bots.iter().filter(|b| !admin || b.admin).min_by_key(|b| (b.created_at, b.id.clone()));
    oldest(true).or_else(|| oldest(false))
}

/// The targets Tab cycles through: the default (the device's pick while that bot exists, else `ask_target`), the
/// other bots alphabetically, then New agent.
pub fn targets(bots: &[BotInfo], device_default: Option<&str>) -> Vec<Target> {
    let default = device_default.and_then(|id| bots.iter().find(|b| b.id == id)).or_else(|| ask_target(bots));
    let mut rest: Vec<&BotInfo> = bots.iter().filter(|b| Some(b.id.as_str()) != default.map(|d| d.id.as_str())).collect();
    rest.sort_by_key(|b| b.name.to_lowercase());
    default.into_iter().chain(rest).cloned().map(Target::Bot).chain(std::iter::once(Target::NewAgent)).collect()
}

/// The next (`step` 1) or previous (-1) target index.
pub fn cycle(index: usize, len: usize, step: i32) -> usize {
    if len == 0 {
        return 0;
    }
    ((index as i64 + step as i64).rem_euclid(len as i64)) as usize
}

/// The bot an `@name` names: its slug, or its name without spaces, ignoring case.
pub fn find_bot<'a>(bots: &'a [BotInfo], name: &str) -> Option<&'a BotInfo> {
    let n = name.to_lowercase();
    bots.iter().find(|b| b.slug.to_lowercase() == n || b.name.to_lowercase().replace(' ', "-") == n)
}

/// What a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Ask { bot: BotInfo, text: String },
    /// Opens the quick launch form with this prompt.
    Launch(String),
    /// Continue in the browser (Ctrl+D) with this text.
    Handoff(String),
    Open(String),
    /// Completes `@slug ` in the input.
    PickBot(String),
    Command { name: String, rest: String },
    Resolve(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// A group heading: never selected.
    pub header: bool,
    pub label: String,
    pub act: Option<Act>,
}

fn header(label: &str) -> Row {
    Row { header: true, label: label.into(), act: None }
}

fn item(label: String, act: Act) -> Row {
    Row { header: false, label, act: Some(act) }
}

/// Text longer than this goes to the browser composer instead of an ask (folio: States, spotlight edge cases).
pub const LONG_TEXT: usize = 4000;
const GOTO_WITH_TEXT: usize = 6;
const GOTO_ONLY: usize = 12;

pub struct Context<'a> {
    pub bots: &'a [BotInfo],
    pub target: Option<&'a Target>,
    pub entries: &'a [Entry],
    /// Today's needs-you rows (label, URL), then Recent (label, URL).
    pub needs: &'a [(String, String)],
    pub recent: &'a [(String, String)],
    /// Writes need a live Home (Ask, launch).
    pub live: bool,
}

fn goto_rows(entries: &[Entry], query: &str, limit: usize) -> Vec<Row> {
    search::search(entries, query, limit).into_iter().map(|e| item(format!("{} · {}", e.title, e.kind.word()), Act::Open(e.url.clone()))).collect()
}

/// The rows for the input, in order; the first row with an action is the default (what Enter does).
pub fn rows(input: &str, cx: &Context) -> Vec<Row> {
    let mut out = Vec::new();
    match grammar::parse(input) {
        Input::Empty => {
            if !cx.needs.is_empty() {
                out.push(header("Needs you"));
                out.extend(cx.needs.iter().map(|(l, u)| item(l.clone(), Act::Open(u.clone()))));
            }
            if !cx.recent.is_empty() {
                out.push(header("Recent"));
                out.extend(cx.recent.iter().map(|(l, u)| item(l.clone(), Act::Open(u.clone()))));
            }
        }
        Input::Mention(prefix) => {
            let p = prefix.to_lowercase();
            let mut bots: Vec<&BotInfo> = cx.bots.iter().filter(|b| b.slug.to_lowercase().starts_with(&p) || b.name.to_lowercase().starts_with(&p)).collect();
            bots.sort_by_key(|b| b.name.to_lowercase());
            if bots.is_empty() {
                out.push(Row { header: false, label: format!("No bot named {prefix}"), act: None });
            }
            out.extend(bots.into_iter().map(|b| item(format!("@{} · {}", b.slug, b.name), Act::PickBot(b.slug.clone()))));
        }
        Input::Text { bot, text } => {
            let named = bot.map(|n| (n, find_bot(cx.bots, n)));
            if let Some((n, None)) = named {
                out.push(Row { header: false, label: format!("No bot named {n}"), act: None });
                return out;
            }
            let target = match named {
                Some((_, Some(b))) => Some(Target::Bot(b.clone())),
                _ => cx.target.cloned(),
            };
            if !text.is_empty() {
                let long = text.chars().count() > LONG_TEXT;
                match (&target, long, cx.live) {
                    (_, true, _) => out.push(Row { header: false, label: "Too long for the spotlight: continue in the browser (Ctrl+D)".into(), act: None }),
                    (_, _, false) => out.push(Row { header: false, label: "Home is not connected: Ask and Launch wait for it".into(), act: None }),
                    (Some(Target::Bot(b)), _, _) if b.paused => out.push(Row { header: false, label: format!("{} is paused: your message still goes through", b.name), act: Some(Act::Ask { bot: b.clone(), text: text.into() }) }),
                    (Some(Target::Bot(b)), _, _) => out.push(item(format!("Ask {}: {}", b.name, crate::model::cut(text, 80)), Act::Ask { bot: b.clone(), text: text.into() })),
                    (Some(Target::NewAgent) | None, _, _) => out.push(item(format!("Launch a new agent: {}", crate::model::cut(text, 80)), Act::Launch(text.into()))),
                }
            }
            if bot.is_none() {
                let found = goto_rows(cx.entries, text, GOTO_WITH_TEXT);
                if !found.is_empty() {
                    out.push(header("Go to"));
                    out.extend(found);
                }
            }
            if !text.is_empty() {
                if !matches!(target, Some(Target::NewAgent) | None) && cx.live {
                    out.push(item("Launch a new agent with this prompt instead".into(), Act::Launch(text.into())));
                }
                out.push(item("Continue in the browser".into(), Act::Handoff(text.into())));
            }
        }
        Input::GoTo(q) => out.extend(goto_rows(cx.entries, q, GOTO_ONLY)),
        Input::Command { name, rest } => {
            for (n, help) in grammar::commands(name) {
                out.push(item(format!("/{n} · {help}"), Act::Command { name: n.to_string(), rest: rest.to_string() }));
            }
            if out.is_empty() {
                out.push(Row { header: false, label: format!("No command /{name}"), act: None });
            }
        }
        Input::Link(text) => out.push(item(format!("Open {}", crate::model::cut(text, 70)), Act::Resolve(text.into()))),
    }
    out
}

/// The index of the first row with an action.
pub fn first_action(rows: &[Row]) -> Option<usize> {
    rows.iter().position(|r| r.act.is_some())
}

/// What Enter does with this row, in words, for the footer.
pub fn footer(act: Option<&Act>) -> String {
    let enter = match act {
        Some(Act::Ask { bot, .. }) => format!("Enter: ask {} · Ctrl+Enter: ask and open the thread", bot.name),
        Some(Act::Launch(_)) => "Enter: quick launch (you confirm with Launch)".into(),
        Some(Act::Handoff(_)) => "Enter: continue in the browser".into(),
        Some(Act::Open(_)) | Some(Act::Resolve(_)) => "Enter: open in the browser".into(),
        Some(Act::PickBot(_)) => "Enter: pick this bot".into(),
        Some(Act::Command { .. }) => "Enter: run the command".into(),
        None => "Type to ask, > to go to, / for commands, @ to pick a bot".into(),
    };
    format!("{enter} · Tab: next target · Ctrl+D: browser · Esc: close")
}

/// Unsent text survives Esc for 15 minutes (Shift+Esc discards it).
pub const KEEP_MS: i64 = 15 * 60_000;

#[derive(Default)]
pub struct Kept {
    text: String,
    hidden_at: i64,
}

impl Kept {
    pub fn hide(&mut self, text: &str, now: i64) {
        self.text = text.to_string();
        self.hidden_at = now;
    }

    pub fn discard(&mut self) {
        self.text.clear();
    }

    /// The text to show at the next open.
    pub fn restore(&mut self, now: i64) -> String {
        if now - self.hidden_at > KEEP_MS {
            self.text.clear();
        }
        self.text.clone()
    }
}

/// The answer to one message in a thread's session, from its events (`SessionEvent.body`): after the
/// `user_message` whose `client_id` is the message's Idempotency-Key, the last `assistant_message` (or the live
/// deltas of the one being written), until `turn_end`; an `ask_user` is a question for the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// The message reached the session.
    pub received: bool,
    pub text: String,
    pub done: bool,
    pub question: Option<String>,
    pub error: Option<String>,
}

pub fn answer(bodies: &[Value], client_id: &str) -> Answer {
    let mut a = Answer::default();
    let mut live = String::new();
    for body in bodies {
        let kind = body.get("kind").and_then(Value::as_str).unwrap_or("");
        let text = || body.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        if kind == "user_message" {
            if body.get("client_id").and_then(Value::as_str) == Some(client_id) {
                a = Answer { received: true, ..Answer::default() };
                live.clear();
            } else if a.received && a.done {
                // A later message (a reply sent from elsewhere) starts another turn: this answer stays.
                break;
            }
            continue;
        }
        if !a.received {
            continue;
        }
        match kind {
            "assistant_delta" => live.push_str(&text()),
            "assistant_message" => {
                a.text = text();
                live.clear();
            }
            "ask_user" => {
                let q = body.get("questions").and_then(Value::as_array).and_then(|q| q.first()).and_then(|q| q.get("question")).and_then(Value::as_str).unwrap_or("The bot asks you something");
                a.question = Some(q.to_string());
            }
            "ask_answered" => a.question = None,
            "turn_end" => {
                a.done = true;
                a.error = body.get("error").and_then(Value::as_str).map(str::to_string);
            }
            _ => {}
        }
    }
    if !a.done && !live.is_empty() {
        a.text = if a.text.is_empty() { live } else { format!("{}\n\n{live}", a.text) };
    }
    a
}

/// A fresh Idempotency-Key in UUID form (a new thread's session takes it as its ID).
pub fn new_key() -> String {
    let b: [u8; 16] = rand::random();
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-4{}-{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[16..20], &h[20..32])
}

/// The inline answer is cut at this many characters; the rest is in the browser.
pub const ANSWER_CHARS: usize = 1200;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Kind;
    use serde_json::json;

    fn bot(id: &str, name: &str, admin: bool, created_at: i64) -> BotInfo {
        BotInfo { id: id.into(), name: name.into(), slug: name.to_lowercase().replace(' ', "-"), admin, created_at, paused: false }
    }

    #[test]
    fn targets_follow_the_web_rule_and_cycle() {
        let bots = vec![bot("b1", "Zed", false, 1), bot("b2", "Ask Launchpad", true, 5), bot("b3", "Hubert", true, 9)];
        assert_eq!(ask_target(&bots).unwrap().id, "b2");
        let names = |t: Vec<Target>| t.iter().map(|t| t.name().to_string()).collect::<Vec<_>>();
        assert_eq!(names(targets(&bots, None)), ["Ask Launchpad", "Hubert", "Zed", "New agent"]);
        assert_eq!(names(targets(&bots, Some("b1"))), ["Zed", "Ask Launchpad", "Hubert", "New agent"]);
        assert_eq!(names(targets(&bots, Some("gone"))), ["Ask Launchpad", "Hubert", "Zed", "New agent"]);
        assert_eq!(names(targets(&[], None)), ["New agent"]);
        assert_eq!((cycle(3, 4, 1), cycle(0, 4, -1), cycle(1, 4, 1)), (0, 3, 2));
        assert_eq!(find_bot(&bots, "ASK-launchpad").unwrap().id, "b2");
    }

    #[test]
    fn rows_for_each_input() {
        let bots = vec![bot("b2", "Ask Launchpad", true, 5), bot("b3", "Hubert", true, 9)];
        let entries = vec![Entry::new(Kind::Session, "Fix CI on main", "/agents?session=s1".into(), false, 1)];
        let target = Target::Bot(bots[0].clone());
        let needs = vec![("Hubert asks".to_string(), "/?card=x".to_string())];
        let cx = Context { bots: &bots, target: Some(&target), entries: &entries, needs: &needs, recent: &[], live: true };
        let labels = |input: &str| rows(input, &cx).into_iter().map(|r| r.label).collect::<Vec<_>>();
        assert_eq!(labels(""), ["Needs you", "Hubert asks"]);
        assert_eq!(labels("ci"), ["Ask Ask Launchpad: ci", "Go to", "Fix CI on main · agent", "Launch a new agent with this prompt instead", "Continue in the browser"]);
        assert_eq!(labels("@hu"), ["@hubert · Hubert"]);
        assert_eq!(labels("@nobody hi"), ["No bot named nobody"]);
        assert_eq!(labels("@hubert why"), ["Ask Hubert: why", "Launch a new agent with this prompt instead", "Continue in the browser"]);
        assert_eq!(labels(">fix"), ["Fix CI on main · agent"]);
        assert_eq!(labels("/la go")[0], "/launch · Quick launch a new agent (with the text after the command)");
        assert_eq!(rows("/la go", &cx)[0].act, Some(Act::Command { name: "launch".into(), rest: "go".into() }));
        assert_eq!(rows("lp:bot/b3", &cx)[0].act, Some(Act::Resolve("lp:bot/b3".into())));
        assert_eq!(rows(&"x".repeat(LONG_TEXT + 1), &cx)[0].act, None);
        let new_agent = Context { target: Some(&Target::NewAgent), ..cx };
        assert_eq!(rows("fix it", &new_agent)[0].act, Some(Act::Launch("fix it".into())));
        // Offline: no Ask or Launch, but Go to and the browser still work.
        let offline = Context { live: false, ..new_agent };
        let r = rows("fix it", &offline);
        assert!(r.iter().all(|r| !matches!(r.act, Some(Act::Ask { .. } | Act::Launch(_)))));
        assert_eq!(r[first_action(&r).unwrap()].act, Some(Act::Open("/agents?session=s1".into())));
        assert_eq!(r.last().unwrap().act, Some(Act::Handoff("fix it".into())));
    }

    #[test]
    fn keys_look_like_uuids() {
        let k = new_key();
        assert_eq!(k.len(), 36);
        assert_eq!(k.matches('-').count(), 4);
        assert_ne!(k, new_key());
    }

    #[test]
    fn kept_text_lasts_15_minutes() {
        let mut k = Kept::default();
        k.hide("draft", 0);
        assert_eq!(k.restore(KEEP_MS), "draft");
        assert_eq!(k.restore(KEEP_MS + 1), "");
        k.hide("again", 10);
        k.discard();
        assert_eq!(k.restore(11), "");
    }

    #[test]
    fn answer_from_events() {
        let ev = |v: Value| v;
        let bodies = vec![
            ev(json!({"kind": "user_message", "text": "older", "client_id": "k0", "queued": false})),
            ev(json!({"kind": "assistant_message", "item_id": "a", "text": "old answer"})),
            ev(json!({"kind": "turn_end", "duration_ms": 1, "interrupted": false, "error": null})),
            ev(json!({"kind": "user_message", "text": "why", "client_id": "k1", "queued": false})),
            ev(json!({"kind": "assistant_message", "item_id": "b", "text": "Looking."})),
            ev(json!({"kind": "assistant_delta", "item_id": "c", "text": "Because "})),
        ];
        let a = answer(&bodies, "k1");
        assert!(a.received && !a.done);
        assert_eq!(a.text, "Looking.\n\nBecause ");
        let mut more = bodies.clone();
        more.push(json!({"kind": "assistant_message", "item_id": "c", "text": "Because the cache expired."}));
        more.push(json!({"kind": "turn_end", "duration_ms": 1, "interrupted": false, "error": null}));
        more.push(json!({"kind": "user_message", "text": "next", "client_id": "k2", "queued": false}));
        assert_eq!(answer(&more, "k1"), Answer { received: true, text: "Because the cache expired.".into(), done: true, question: None, error: None });
        assert!(!answer(&bodies, "k9").received);
        let asking = vec![json!({"kind": "user_message", "client_id": "k1", "text": "x", "queued": false}), json!({"kind": "ask_user", "request_id": "r", "questions": [{"question": "Which branch?"}]})];
        assert_eq!(answer(&asking, "k1").question.as_deref(), Some("Which branch?"));
    }
}

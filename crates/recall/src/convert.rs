//! The converter: a session's normalized events (`SessionEvent`) become Funes turns (`.funes.jsonl`, the format of
//! funes `docs/funes-jsonl.md`). Pure and deterministic: the same events always give the same turns, which Funes
//! needs because its chunk IDs ignore the text (a turn emitted twice with different text keeps its first text).
//!
//! - A message to the agent is one `user` turn. Funes builds a hit's exchange frame from `user` and `assistant`
//!   text only, so every prompt has the role `user`; a prompt that is not the owner's starts with a line naming
//!   its sender (`[from routine "Daily check"]`), so another agent's words are never read as the owner's.
//! - What the agent did until the next message is one or more `assistant` turns: reasoning → `thinking`, messages
//!   → `text`, tool calls → `tool_use`, tool results → `tool_result`. A long run is cut into several turns
//!   (`MAX_TURN_BLOCKS`, `MAX_TURN_CHARS`) so a hit names a narrow place of the trace.
//! - A turn's ID is `lp<seq>`, the Home sequence of its first event: stable across moves, which leave a gap.
//! - Only finished turns are converted: everything through `through` (the last `turn_end`, see `complete_through`).
//!
//! A change of what this module emits for the same events needs a new `CONVERTER_VERSION`: a memory built by another
//! version is rebuilt.
use std::collections::HashMap;

use launchpad_core::model::{EventBody, SenderKind, SessionEvent};
use serde_json::{Value, json};

pub const CONVERTER_VERSION: u32 = 1;

/// A Funes turn holds at most this many blocks and about this many characters; a longer run continues in the next.
const MAX_TURN_BLOCKS: usize = 24;
const MAX_TURN_CHARS: usize = 48_000;
/// The largest block: a longer tool input or output keeps its start and its end around a line that says how much
/// was left out (the trace keeps all of it).
const MAX_TOOL_CHARS: usize = 16_000;
const MAX_TEXT_CHARS: usize = 32_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub block_type: &'static str,
    pub text: String,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    /// The Home sequence of the turn's first and last events.
    pub lp_seq: i64,
    pub lp_end: i64,
    /// Unix ms.
    pub ts: i64,
    pub role: &'static str,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Default)]
pub struct Converted {
    pub turns: Vec<Turn>,
    /// Blocks cut to the size limit.
    pub truncated: usize,
}

/// The sequence through which a session's events are final: its last top-level `turn_end`; all of them (`last_seq`)
/// once `settled` (the session does not run and has been quiet: a stopped or failed turn has no `turn_end`).
pub fn complete_through(events: &[SessionEvent], settled: bool) -> i64 {
    if settled {
        return events.last().map_or(0, |e| e.seq);
    }
    events.iter().rev().find(|e| matches!(e.body, EventBody::TurnEnd { .. })).map_or(0, |e| e.seq)
}

/// Keeps the start and the end of a text longer than `max` characters. Returns whether it cut.
fn clip(text: &str, max: usize) -> (String, bool) {
    let count = text.chars().count();
    if count <= max {
        return (text.to_string(), false);
    }
    let head = max * 3 / 4;
    let tail = max - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    (format!("{start}\n[… {} characters left out of recall; the session's trace has all of it …]\n{end}", count - max), true)
}

struct Builder {
    out: Converted,
    current: Option<Turn>,
    chars: usize,
    /// Tool call item → its block in the open turn, to apply a later upsert; and every call's name.
    calls: HashMap<String, usize>,
    names: HashMap<String, String>,
    last_ts: i64,
}

impl Builder {
    fn close(&mut self) {
        if let Some(t) = self.current.take()
            && !t.blocks.is_empty()
        {
            self.out.turns.push(t);
        }
        self.chars = 0;
        self.calls.clear();
    }

    fn push(&mut self, e: &SessionEvent, role: &'static str, mut block: Block) {
        let max = if matches!(block.block_type, "tool_use" | "tool_result") { MAX_TOOL_CHARS } else { MAX_TEXT_CHARS };
        let (text, cut) = clip(block.text.trim(), max);
        if text.is_empty() {
            return;
        }
        if cut {
            self.out.truncated += 1;
        }
        block.text = text;
        let full = self.current.as_ref().is_some_and(|t| t.role != role || t.blocks.len() >= MAX_TURN_BLOCKS || self.chars + block.text.len() > MAX_TURN_CHARS);
        if full {
            self.close();
        }
        let turn = self.current.get_or_insert_with(|| Turn { lp_seq: e.seq, lp_end: e.seq, ts: self.last_ts, role, blocks: vec![] });
        turn.lp_end = e.seq;
        self.chars += block.text.len();
        turn.blocks.push(block);
    }

    fn event(&mut self, e: &SessionEvent, body: &EventBody, sub: Option<&str>) {
        let text = |t: &str| Block { block_type: "text", text: t.to_string(), tool_name: None, tool_use_id: None };
        let item = |id: &str| match sub {
            Some(agent) => format!("{agent}/{id}"),
            None => id.to_string(),
        };
        match body {
            EventBody::SubagentEvent { agent_id, body } => self.event(e, body, Some(agent_id)),
            // A subagent's prompt is the agent's own text, not a message to it.
            EventBody::UserMessage { text: t, .. } if sub.is_some() => {
                self.push(e, "assistant", Block { block_type: "tool_use", text: t.clone(), tool_name: Some("subagent".into()), tool_use_id: None });
            }
            EventBody::UserMessage { text: t, from, .. } => {
                self.close();
                let label = from.as_ref().map(|f| {
                    let kind = match f.kind {
                        SenderKind::Bot => "bot",
                        SenderKind::Session => "agent",
                        SenderKind::Routine => "routine",
                        SenderKind::Hook => "hook",
                        SenderKind::Channel => "channel",
                        _ => "launchpad",
                    };
                    match &f.by {
                        Some(by) => format!("[from {kind} {:?}, written by {by}]\n", f.name),
                        None => format!("[from {kind} {:?}]\n", f.name),
                    }
                });
                self.push(e, "user", text(&format!("{}{t}", label.unwrap_or_default())));
                self.close();
            }
            EventBody::AskAnswered { answers, .. } if sub.is_none() => {
                self.close();
                self.push(e, "user", text(&format!("[answer] {}", answers.join("; "))));
                self.close();
            }
            EventBody::AssistantMessage { text: t, .. } => self.push(e, "assistant", text(t)),
            EventBody::Reasoning { text: t, .. } => self.push(e, "assistant", Block { block_type: "thinking", ..text(t) }),
            EventBody::ToolCall { item_id, name, summary, input } => {
                let id = item(item_id);
                let rendered = match input {
                    Value::Object(o) if !o.is_empty() => input.to_string(),
                    Value::Null | Value::Object(_) => summary.clone(),
                    other => other.to_string(),
                };
                let known = self.names.insert(id.clone(), name.clone()).is_some();
                // Some protocols send the call again with its input filled in: the block takes the last one.
                if let (Some(&at), Some(turn)) = (self.calls.get(&id), self.current.as_mut())
                    && let Some(b) = turn.blocks.get_mut(at)
                {
                    let (t, _) = clip(rendered.trim(), MAX_TOOL_CHARS);
                    if !t.is_empty() {
                        self.chars = self.chars + t.len() - b.text.len().min(self.chars);
                        b.text = t;
                        b.tool_name = Some(name.clone());
                    }
                    return;
                }
                // The call's block went out with an earlier turn of this run: it keeps what it had.
                if known {
                    return;
                }
                self.push(e, "assistant", Block { block_type: "tool_use", text: rendered, tool_name: Some(name.clone()), tool_use_id: Some(id.clone()) });
                if let Some(t) = &self.current
                    && t.blocks.last().is_some_and(|b| b.tool_use_id.as_deref() == Some(&id))
                {
                    self.calls.insert(id, t.blocks.len() - 1);
                }
            }
            EventBody::ToolResult { item_id, output, .. } => {
                let id = item(item_id);
                let name = self.names.get(&id).cloned();
                self.push(e, "assistant", Block { block_type: "tool_result", text: output.clone(), tool_name: name, tool_use_id: Some(id) });
            }
            EventBody::FileChange { item_id, path, change, diff } => {
                self.push(e, "assistant", Block { block_type: "tool_use", text: format!("{change} {path}\n{diff}"), tool_name: Some("file_change".into()), tool_use_id: Some(item(item_id)) });
            }
            EventBody::AskUser { questions, .. } => {
                let asked: Vec<String> = questions.iter().map(|q| q.question.clone()).collect();
                self.push(e, "assistant", text(&format!("[question] {}", asked.join("\n"))));
            }
            EventBody::TurnEnd { .. } if sub.is_none() => self.close(),
            _ => {}
        }
    }
}

/// The Funes turns of `events` (in order, one stored sequence each) through sequence `through`.
pub fn convert(events: &[SessionEvent], through: i64) -> Converted {
    let mut b = Builder { out: Converted::default(), current: None, chars: 0, calls: HashMap::new(), names: HashMap::new(), last_ts: 0 };
    for e in events.iter().filter(|e| e.seq <= through) {
        // Some of Home's own events carry no time: they take the time of the event before.
        if e.ts > 0 {
            b.last_ts = e.ts;
        }
        b.event(e, &e.body, None);
    }
    b.close();
    b.out
}

/// The harness facet: Funes accepts lowercase letters, digits, `-` and `_`.
pub fn harness_facet(id: &str) -> String {
    let s: String = id.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if s.is_empty() { "launchpad".into() } else { s }
}

/// One line of a turns file. `fseq`: the turn's position in its session's memory, a dense counter from 0 (the
/// ledger's). `fallback_ts`: for a turn whose events carry no time (Unix ms).
pub fn jsonl_line(session_id: &str, harness: &str, fseq: i64, turn: &Turn, fallback_ts: i64) -> String {
    let ms = if turn.ts > 0 { turn.ts } else { fallback_ts };
    let ts = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let blocks: Vec<Value> = turn
        .blocks
        .iter()
        .map(|b| {
            let mut v = json!({ "block_type": b.block_type, "text": b.text });
            if let Some(n) = &b.tool_name {
                v["tool_name"] = json!(n);
            }
            if let Some(i) = &b.tool_use_id {
                v["tool_use_id"] = json!(i);
            }
            v
        })
        .collect();
    json!({ "format": 1, "session_id": session_id, "turn_uuid": format!("lp{}", turn.lp_seq), "seq": fseq, "ts": ts, "role": turn.role, "harness": harness_facet(harness), "blocks": blocks }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchpad_core::model::MessageFrom;

    fn ev(seq: i64, body: EventBody) -> SessionEvent {
        SessionEvent { session_id: "s".into(), seq, ts: 1_780_000_000_000 + seq * 1000, body }
    }
    fn user(seq: i64, text: &str, from: Option<MessageFrom>) -> SessionEvent {
        ev(seq, EventBody::UserMessage { text: text.into(), client_id: String::new(), queued: false, images: vec![], from })
    }
    fn call(seq: i64, id: &str, input: Value) -> SessionEvent {
        ev(seq, EventBody::ToolCall { item_id: id.into(), name: "bash".into(), summary: "ls".into(), input })
    }
    fn result(seq: i64, id: &str, output: &str) -> SessionEvent {
        ev(seq, EventBody::ToolResult { item_id: id.into(), output: output.into(), is_error: false, exit_code: Some(0), duration_ms: None })
    }
    fn end(seq: i64) -> SessionEvent {
        ev(seq, EventBody::TurnEnd { duration_ms: None, interrupted: false, error: None })
    }
    fn said(seq: i64, text: &str) -> SessionEvent {
        ev(seq, EventBody::AssistantMessage { item_id: format!("m{seq}"), text: text.into() })
    }

    #[test]
    fn a_turn_becomes_a_user_turn_and_an_assistant_turn() {
        let events = vec![
            user(1, "why does the build fail?", None),
            ev(2, EventBody::Reasoning { item_id: "r".into(), text: "look at CI".into() }),
            call(3, "c1", Value::Null),
            call(4, "c1", json!({"command": "cat ci.yml"})),
            result(5, "c1", "runs-on: ubuntu"),
            said(6, "The target is wrong."),
            end(7),
            user(8, "not finished", None),
            said(9, "partial"),
        ];
        let through = complete_through(&events, false);
        assert_eq!(through, 7);
        let c = convert(&events, through);
        assert_eq!(c.turns.len(), 2);
        assert_eq!((c.turns[0].role, c.turns[0].lp_seq, c.turns[0].lp_end), ("user", 1, 1));
        let a = &c.turns[1];
        assert_eq!((a.role, a.lp_seq, a.lp_end), ("assistant", 2, 6));
        assert_eq!(a.blocks.iter().map(|b| b.block_type).collect::<Vec<_>>(), ["thinking", "tool_use", "tool_result", "text"]);
        // The call sent twice is one block, with the input of the last one.
        assert_eq!(a.blocks[1].text, r#"{"command":"cat ci.yml"}"#);
        assert_eq!(a.blocks[2].tool_name.as_deref(), Some("bash"));
        // A settled session gives its unfinished end too.
        assert_eq!(convert(&events, complete_through(&events, true)).turns.len(), 4);
    }

    #[test]
    fn converting_in_two_passes_gives_the_same_turns_as_in_one() {
        let events = vec![user(1, "a", None), said(2, "b"), end(3), user(4, "c", None), call(5, "x", json!({"q": 1})), result(6, "x", "out"), end(7)];
        let whole = convert(&events, 7).turns;
        let mut parts = convert(&events, 3).turns;
        parts.extend(convert(&events[3..], 7).turns);
        assert_eq!(whole, parts);
    }

    #[test]
    fn another_sender_is_named_and_a_subagent_never_ends_the_turn() {
        let from = MessageFrom { kind: SenderKind::Routine, name: "Daily check".into(), ..Default::default() };
        let events = vec![
            user(1, "run", Some(from)),
            ev(2, EventBody::SubagentEvent { agent_id: "a1".into(), body: Box::new(EventBody::UserMessage { text: "explore".into(), client_id: String::new(), queued: false, images: vec![], from: None }) }),
            ev(3, EventBody::SubagentEvent { agent_id: "a1".into(), body: Box::new(EventBody::TurnEnd { duration_ms: None, interrupted: false, error: None }) }),
            said(4, "done"),
            end(5),
        ];
        let c = convert(&events, 5);
        assert_eq!(c.turns.len(), 2);
        assert!(c.turns[0].blocks[0].text.starts_with("[from routine \"Daily check\"]\nrun"));
        assert_eq!(c.turns[1].blocks.len(), 2);
        assert_eq!(c.turns[1].blocks[0].tool_name.as_deref(), Some("subagent"));
    }

    #[test]
    fn long_runs_split_and_long_blocks_are_cut() {
        let mut events = vec![user(1, "go", None)];
        for i in 0..60 {
            events.push(said(2 + i, &format!("step {i}")));
        }
        events.push(result(100, "big", &"x".repeat(MAX_TOOL_CHARS * 2)));
        events.push(end(101));
        let c = convert(&events, 101);
        assert_eq!(c.truncated, 1);
        assert!(c.turns.iter().all(|t| t.blocks.len() <= MAX_TURN_BLOCKS));
        assert_eq!(c.turns.iter().map(|t| t.blocks.len()).sum::<usize>(), 62);
        // Every turn has its own ID, and they follow each other.
        let ids: Vec<i64> = c.turns.iter().map(|t| t.lp_seq).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        let big = c.turns.last().unwrap().blocks.last().unwrap();
        assert!(big.text.chars().count() < MAX_TOOL_CHARS + 200 && big.text.contains("characters left out"));
    }

    #[test]
    fn a_line_holds_only_the_fields_funes_accepts() {
        let c = convert(&[user(1, "hi", None), call(2, "c", json!({"a": 1})), end(3)], 3);
        let line: Value = serde_json::from_str(&jsonl_line("0b1e", "Claude Code", 4, &c.turns[1], 0)).unwrap();
        assert_eq!(line["turn_uuid"], "lp2");
        assert_eq!(line["seq"], 4);
        assert_eq!(line["harness"], "claude-code");
        assert_eq!(line["ts"], "2026-05-28T20:26:42Z");
        let keys: Vec<&String> = line.as_object().unwrap().keys().collect();
        assert!(keys.iter().all(|k| ["format", "session_id", "turn_uuid", "seq", "ts", "role", "harness", "blocks"].contains(&k.as_str())));
    }
}

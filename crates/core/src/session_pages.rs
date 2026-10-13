//! Paged reads of a session's event log for the UI: the newest events first, earlier pages on
//! request, and the few facts the Summary needs from events that are not loaded yet. Pasted images
//! are replaced by authenticated URLs so responses stay small; harnesses and `launchpad trace`
//! still read the stored events unchanged.

use anyhow::{Context, Result};
use rusqlite::params;
use serde::Serialize;
use ts_rs::TS;

use crate::App;
use crate::model::{EventBody, SessionEvent};

/// A contiguous slice of the stored events, oldest first. `facts` describes the events before
/// the slice and is `None` when the slice starts at the beginning of the log.
#[derive(Clone, Debug, Serialize, TS)]
pub struct SessionEventPage {
    pub events: Vec<SessionEvent>,
    pub facts: Option<EarlierFacts>,
}

/// Summary facts of the events before a page (`seq < before`).
#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct EarlierFacts {
    #[ts(type = "number")]
    pub before: i64,
    /// The first user message of the session (the initial task).
    pub first_user: Option<SessionEvent>,
    pub last_assistant: Option<SessionEvent>,
    /// The last question, with its answers when it was answered (anywhere in the log).
    pub last_ask: Option<SessionEvent>,
    pub last_ask_answers: Option<Vec<String>>,
    pub turns: u32,
    /// Sum of the turn durations; `None` when a turn has no recorded duration.
    #[ts(type = "number | null")]
    pub turns_duration_ms: Option<i64>,
    /// Native child agent metadata events (`subagent`).
    pub subagents: Vec<SessionEvent>,
}

const MAX_PAGE: i64 = 2000;

fn row(id: &str, seq: i64, ts: i64, json: &str) -> Option<SessionEvent> {
    serde_json::from_str::<EventBody>(json).ok().map(|body| SessionEvent { session_id: id.to_string(), seq, ts, body })
}

/// The `limit` events before `before` (exclusive), or the last `limit` events without `before`.
pub async fn page(app: &App, id: &str, before: Option<i64>, limit: i64) -> Result<SessionEventPage> {
    let id = id.to_string();
    let limit = limit.clamp(1, MAX_PAGE);
    let before = before.unwrap_or(i64::MAX);
    let mut page = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3")?;
            let mut events: Vec<SessionEvent> = st
                .query_map(params![id, before, limit], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?
                .filter_map(|r| r.ok())
                .filter_map(|(seq, ts, json)| row(&id, seq, ts, &json))
                .collect();
            events.reverse();
            let start = events.first().map(|e| e.seq).unwrap_or(before);
            let earlier: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM session_event WHERE session_id = ?1 AND seq < ?2)", params![id, start], |r| r.get(0))?;
            let facts = if earlier { Some(facts(c, &id, start)?) } else { None };
            Ok(SessionEventPage { events, facts })
        })
        .await?;
    for e in &mut page.events {
        link_images(e);
    }
    if let Some(f) = &mut page.facts {
        for e in f.first_user.iter_mut().chain(f.last_assistant.iter_mut()) {
            link_images(e);
        }
    }
    Ok(page)
}

fn facts(c: &rusqlite::Connection, id: &str, before: i64) -> Result<EarlierFacts> {
    let one = |kind: &str, order: &str| -> Result<Option<SessionEvent>> {
        let sql = format!("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq < ?2 AND kind = ?3 ORDER BY seq {order} LIMIT 1");
        let mut st = c.prepare(&sql)?;
        let mut rows = st.query(params![id, before, kind])?;
        Ok(match rows.next()? {
            Some(r) => row(id, r.get(0)?, r.get(1)?, &r.get::<_, String>(2)?),
            None => None,
        })
    };
    let mut f = EarlierFacts { before, first_user: one("user_message", "ASC")?, last_assistant: one("assistant_message", "DESC")?, last_ask: one("ask_user", "DESC")?, turns_duration_ms: Some(0), ..Default::default() };
    if let Some(SessionEvent { body: EventBody::AskUser { request_id, .. }, .. }) = &f.last_ask {
        let mut st = c.prepare("SELECT json FROM session_event WHERE session_id = ?1 AND kind = 'ask_answered' AND json_extract(json, '$.request_id') = ?2 ORDER BY seq DESC LIMIT 1")?;
        let mut rows = st.query(params![id, request_id])?;
        if let Some(r) = rows.next()?
            && let Ok(EventBody::AskAnswered { answers, .. }) = serde_json::from_str::<EventBody>(&r.get::<_, String>(0)?)
        {
            f.last_ask_answers = Some(answers);
        }
    }
    let mut st = c.prepare("SELECT seq, ts, kind, json FROM session_event WHERE session_id = ?1 AND seq < ?2 AND kind IN ('turn_end', 'subagent') ORDER BY seq")?;
    let mut rows = st.query(params![id, before])?;
    while let Some(r) = rows.next()? {
        let Some(e) = row(id, r.get(0)?, r.get(1)?, &r.get::<_, String>(3)?) else { continue };
        match &e.body {
            EventBody::TurnEnd { duration_ms, .. } => {
                f.turns += 1;
                f.turns_duration_ms = f.turns_duration_ms.zip(*duration_ms).map(|(a, b)| a + b);
            }
            EventBody::Subagent { .. } => f.subagents.push(e),
            _ => {}
        }
    }
    if f.turns == 0 {
        f.turns_duration_ms = None;
    }
    Ok(f)
}

/// The URL of the `index`-th image of the user message `seq`.
pub fn image_url(session_id: &str, seq: i64, index: usize) -> String {
    format!("/api/sessions/{session_id}/images/{seq}/{index}")
}

/// Replaces inline image data URLs with authenticated URLs (stored events keep the bytes).
pub fn link_images(e: &mut SessionEvent) {
    if let EventBody::UserMessage { images, .. } = &mut e.body {
        for (i, image) in images.iter_mut().enumerate() {
            if image.data_url.starts_with("data:") {
                image.data_url = image_url(&e.session_id, e.seq, i);
            }
        }
    }
}

/// The bytes and MIME type of an image pasted into a user message.
pub async fn image(app: &App, id: &str, seq: i64, index: usize) -> Result<(Vec<u8>, String)> {
    let id = id.to_string();
    let json: String = app
        .state_db
        .call(move |c| Ok(c.query_row("SELECT json FROM session_event WHERE session_id = ?1 AND seq = ?2", params![id, seq], |r| r.get(0))?))
        .await
        .context("no such event")?;
    let Ok(EventBody::UserMessage { images, .. }) = serde_json::from_str::<EventBody>(&json) else { anyhow::bail!("not a user message") };
    let image = images.get(index).context("no such image")?;
    crate::attachments::decode_image(&image.data_url)
}

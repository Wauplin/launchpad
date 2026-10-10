//! Explicit, read-only discovery of external conversations. Import stores an archived snapshot.
use crate::{App, model::*, util::now_ms};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Serialize, TS)]
#[ts(rename = "HistorySession")]
pub struct Candidate {
    pub id: String,
    pub harness: HarnessKind,
    pub cwd: String,
    pub title: String,
    #[ts(type = "number")]
    pub started_at: i64,
}
#[derive(Serialize, TS)]
#[ts(rename = "HistoryPage")]
pub struct Page {
    pub sessions: Vec<Candidate>,
    pub next: Option<String>,
}

pub(crate) fn timestamp(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or_else(now_ms)
}
pub type Snapshot = (
    Candidate,
    Vec<(i64, EventBody)>,
    Option<String>,
    Option<String>,
);

pub async fn list(
    app: &App,
    harness: HarnessKind,
    cursor: Option<String>,
    archived: bool,
) -> Result<Page> {
    let mut page = app
        .harnesses
        .get(harness)
        .history_list(app, cursor, archived)
        .await?;
    page.sessions.retain(|c| {
        !app.sessions
            .list()
            .iter()
            .any(|s| s.harness == c.harness && s.harness_session_id.as_deref() == Some(&c.id))
    });
    Ok(page)
}

pub async fn import(app: &App, harness: HarnessKind, id: &str) -> Result<Session> {
    if let Some(s) = app
        .sessions
        .list()
        .into_iter()
        .find(|s| s.harness == harness && s.harness_session_id.as_deref() == Some(id))
    {
        return Ok(s);
    }
    let (candidate, events, model, effort) =
        app.harnesses.get(harness).history_read(app, id).await?;
    let project = crate::projects::for_path(app, std::path::Path::new(&candidate.cwd)).await?;
    let now = now_ms();
    let prompt = events
        .iter()
        .find_map(|(_, b)| {
            if let EventBody::UserMessage { text, .. } = b {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        harness,
        harness_session_id: Some(candidate.id),
        project_id: project.map(|p| p.id),
        cwd: candidate.cwd,
        title: candidate.title,
        title_pending: false,
        status: SessionStatus::Closed,
        status_since: now,
        live: false,
        open_question: false,
        model,
        effort,
        prompt,
        usage: Usage::default(),
        started_at: candidate.started_at,
        ended_at: Some(now),
        last_seq: 0,
        launch_spec: LaunchSpec::default(),
        quota_pause: None,
    };
    app.sessions.import_history(app, s, events).await
}

pub(crate) fn content_text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

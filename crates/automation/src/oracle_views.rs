//! What the oracle pages and `launchpad oracle list|show` read (Home): each oracle with its last outcomes, its
//! failure streak and the routines gated on it.
use anyhow::Result;
use serde::Serialize;
use ts_rs::TS;

use crate::gates::{self, Fire};
use crate::oracle_eval::{self, Evaluation, Outcome, Streak};
use crate::oracles::{self, Oracle};
use launchpad_core::App;

/// A routine gated on an oracle.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleUser")]
pub struct UsedBy {
    pub routine_id: String,
    pub title: String,
    pub bot_id: Option<String>,
    pub run_on: Vec<Outcome>,
    pub fire: Option<Fire>,
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleRecent")]
pub struct Recent {
    pub outcome: Outcome,
    #[ts(type = "number")]
    pub at: i64,
}

/// One oracle of the list.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleListItem")]
pub struct ListItem {
    pub oracle: Oracle,
    /// The newest evaluation.
    pub last: Option<Evaluation>,
    /// The newest five outcomes, newest first.
    pub recent: Vec<Recent>,
    pub used_by: Vec<UsedBy>,
    /// Its failure streak (open while `count > 0` and not read).
    pub streak: Option<Streak>,
}

/// One oracle with its file (`content`: the key `oracle.save`'s Undo puts back).
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleDetail")]
pub struct Detail {
    pub oracle: Oracle,
    pub content: String,
    pub last: Option<Evaluation>,
    pub recent: Vec<Recent>,
    pub used_by: Vec<UsedBy>,
    pub streak: Option<Streak>,
}

async fn used_by(app: &App, ws: &str, slug: &str) -> Vec<UsedBy> {
    gates::users(app, ws, slug)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| UsedBy { routine_id: r.id.clone(), title: r.title.clone(), bot_id: r.bot_id.clone(), run_on: r.gate.as_ref().map(|g| g.run_on.clone()).unwrap_or_default(), fire: r.gate.as_ref().and_then(|g| g.fire), paused: !r.active() })
        .collect()
}

pub async fn list(app: &App, ws: &str) -> Result<Vec<ListItem>> {
    let recent = oracle_eval::recent(app, ws, 5).await?;
    let streaks = oracle_eval::streaks(app, Some(ws)).await?;
    let mut out = vec![];
    for o in oracles::list(app, ws).await? {
        let last = oracle_eval::history(app, ws, Some(&o.slug), false, None, 1).await?.into_iter().next();
        let r = recent.get(&o.slug).map(|v| v.iter().map(|(outcome, at)| Recent { outcome: *outcome, at: *at }).collect()).unwrap_or_default();
        let used = used_by(app, ws, &o.slug).await;
        let streak = streaks.iter().find(|s| s.oracle == o.slug).cloned();
        out.push(ListItem { oracle: o, last, recent: r, used_by: used, streak });
    }
    Ok(out)
}

pub async fn show(app: &App, ws: &str, slug: &str) -> Result<Detail> {
    let f = oracles::get(app, ws, slug).await?;
    let list = oracle_eval::history(app, ws, Some(slug), false, None, 5).await?;
    let recent = list.iter().map(|e| Recent { outcome: e.outcome, at: e.created_at }).collect();
    let streak = oracle_eval::streaks(app, Some(ws)).await?.into_iter().find(|s| s.oracle == slug);
    Ok(Detail { oracle: f.oracle, content: f.content, last: list.into_iter().next(), recent, used_by: used_by(app, ws, slug).await, streak })
}

/// The answer of `oracle.test`: the result, and the evaluation it recorded (a saved oracle; a draft is not recorded).
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleTestAnswer")]
pub struct TestAnswer {
    pub result: oracle_eval::RunResult,
    pub evaluation_id: Option<String>,
}

impl TestAnswer {
    pub fn of(e: &Evaluation) -> TestAnswer {
        let result = oracle_eval::RunResult { outcome: e.outcome, error: e.error.clone(), summary: e.summary.clone(), fingerprint: e.fingerprint.clone(), links: e.links.clone(), items: e.items.clone(), cursor: e.cursor_proposed.clone(), stderr_tail: e.stderr_tail.clone(), duration_ms: e.duration_ms };
        TestAnswer { result, evaluation_id: Some(e.id.clone()) }
    }
}

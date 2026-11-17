//! What the oracle pages and `launchpad oracle list|show` read (Home): each oracle with its last outcomes, its
//! failure streak, the routines gated on it, its tools with their grants, and the type warnings of its file.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use serde::Serialize;
use ts_rs::TS;

use crate::gates::{self, Fire};
use crate::oracle_eval::{self, Evaluation, Outcome, Streak};
use crate::oracles::{self, Oracle, Problem, Tool};
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
    /// Type warnings of the saved file.
    pub warnings: Vec<Problem>,
    /// Where the interface of `launchpad_oracle` is (its `.pyi`).
    pub interface: String,
}

/// One tool of the Tools tab: what it reaches, its registry tool and whether the workspace may use it.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleToolInfo")]
pub struct ToolInfo {
    pub tool: Tool,
    pub reaches: String,
    /// The registry tool whose token it uses (gh, hf, slack-read); None: it needs none.
    pub registry: Option<String>,
    /// Usable in this workspace: granted with a token (always for http and launchpad).
    pub ready: bool,
    /// Why it is not.
    pub problem: Option<String>,
}

fn reaches(t: Tool) -> &'static str {
    match t {
        Tool::Http => "Any public http(s) address, without a credential",
        Tool::Github => "GitHub's REST API (api.github.com), GET only",
        Tool::Huggingface => "The Hugging Face Hub (huggingface.co), GET only",
        Tool::Slack => "Slack through slackline's read commands, and slack.com/api, GET only",
        Tool::Launchpad => "Home's read API for this workspace, in process",
    }
}

/// The five tools and their state in workspace `ws`.
pub async fn tools(app: &App, ws: &str) -> Vec<ToolInfo> {
    let mut out = vec![];
    for t in Tool::ALL {
        let (ready, problem) = match t.registry() {
            None => (true, None),
            Some(id) => match launchpad_core::vault::oracle_token(app, ws, id).await {
                Ok(_) => (true, None),
                Err(why) => (false, Some(why)),
            },
        };
        out.push(ToolInfo { tool: t, reaches: reaches(t).into(), registry: t.registry().map(str::to_string), ready, problem });
    }
    out
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

fn warnings_cache() -> &'static Mutex<HashMap<String, Vec<Problem>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<Problem>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The type warnings of a saved file, by its hash (none for a broken file, or when the type check cannot run).
async fn warnings(app: &App, o: &Oracle, content: &str) -> Vec<Problem> {
    if o.broken() {
        return vec![];
    }
    if let Some(w) = warnings_cache().lock().unwrap_or_else(|p| p.into_inner()).get(&o.sha256) {
        return w.clone();
    }
    let w = match crate::oracle_monty::typecheck(app, content).await {
        Ok(Some(text)) => crate::oracle_check::typecheck_problems(&text),
        Ok(None) => vec![],
        Err(_) => return vec![],
    };
    let mut cache = warnings_cache().lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(o.sha256.clone(), w.clone());
    w
}

pub async fn show(app: &App, ws: &str, slug: &str) -> Result<Detail> {
    let f = oracles::get(app, ws, slug).await?;
    let list = oracle_eval::history(app, ws, Some(slug), false, None, 5).await?;
    let recent = list.iter().map(|e| Recent { outcome: e.outcome, at: e.created_at }).collect();
    let streak = oracle_eval::streaks(app, Some(ws)).await?.into_iter().find(|s| s.oracle == slug);
    let warnings = warnings(app, &f.oracle, &f.content).await;
    Ok(Detail { oracle: f.oracle, content: f.content, last: list.into_iter().next(), recent, used_by: used_by(app, ws, slug).await, streak, warnings, interface: format!("/api/oracles/{ws}/_interface") })
}

/// The answer of `oracle.test`: the result, the evaluation it recorded (a saved oracle; a draft is not recorded),
/// and a draft's type warnings.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleTestAnswer")]
pub struct TestAnswer {
    pub result: oracle_eval::RunResult,
    pub evaluation_id: Option<String>,
    pub warnings: Vec<Problem>,
}

impl TestAnswer {
    pub fn of(e: &Evaluation) -> TestAnswer {
        let result = oracle_eval::RunResult { outcome: e.outcome, error: e.error.clone(), summary: e.summary.clone(), fingerprint: e.fingerprint.clone(), links: e.links.clone(), items: e.items.clone(), cursor: e.cursor_proposed.clone(), log: e.log.clone(), calls: e.calls.clone(), duration_ms: e.duration_ms };
        TestAnswer { result, evaluation_id: Some(e.id.clone()), warnings: vec![] }
    }
}

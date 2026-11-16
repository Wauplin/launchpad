//! Oracles on Home (`launchpad_automation::oracles`, `oracle_eval`): list, show, evaluations, versions, and the
//! actions of the action list (`oracle.*`). Runners forward every `/api/oracles…` request to Home, which checks an
//! agent's workspace (`agent_auth::within_workspace`) and its access: every level reads and runs, admin saves code
//! but never adds access (a card waits for the owner), the rest is the owner's.
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use launchpad_automation::gates;
use launchpad_automation::oracle_eval::{self, Caller, CallerKind, Evaluation};
use launchpad_automation::oracle_views::{self, Detail, ListItem, TestAnswer};
use launchpad_automation::oracles::{self, Oracle};
use launchpad_core::kb::Author;
use launchpad_http::AgentSession;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/oracles/{workspace}", get(list))
        .route("/api/oracles/{workspace}/_tools", get(tools))
        .route("/api/oracles/{workspace}/{slug}", get(show).put(save).delete(remove))
        .route("/api/oracles/{workspace}/{slug}/evaluations", get(evaluations))
        .route("/api/oracles/{workspace}/{slug}/versions", get(versions))
        .route("/api/oracles/{workspace}/{slug}/versions/{change}", get(diff))
        .route("/api/oracles/{workspace}/{slug}/restore/{change}", post(restore))
        .route("/api/oracles/{workspace}/{slug}/pause", post(pause))
        .route("/api/oracles/{workspace}/{slug}/resume", post(resume))
        .route("/api/oracles/{workspace}/{slug}/read", post(mark_read))
        .route("/api/oracles/{workspace}/{slug}/test", post(test))
        .route("/api/oracles/{workspace}/{slug}/run", post(run))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

fn forbidden(why: String) -> ApiError {
    ApiError::new(axum::http::StatusCode::FORBIDDEN, why)
}

async fn author(s: &Ctx, agent: &Option<Extension<AgentSession>>) -> Result<Author, ApiError> {
    match agent {
        Some(a) => launchpad_core::kb::agent_author(&s.app, &a.0.0).await.map_err(bad),
        None => Ok(Author::owner(None)),
    }
}

async fn list(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Vec<ListItem>> {
    launchpad_core::workspace::get(&s.app, &ws).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(oracle_views::list(&s.app, &ws).await.map_err(bad)?))
}

/// The tools granted to the workspace, without secrets: what an oracle's `access.tools` may name, with the hosts
/// that receive each token (the Access tab, and `launchpad oracle test`, which allows the same hosts).
async fn tools(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Vec<Value>> {
    launchpad_core::workspace::get(&s.app, &ws).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    let tools = launchpad_core::vault::tools(&s.app).await.map_err(bad)?;
    Ok(Json(
        tools
            .iter()
            .filter(|t| t.granted(Some(&ws)) && t.id != launchpad_core::vault::LAUNCHPAD_TOOL)
            .map(|t| json!({"id": t.id, "name": t.name, "env": t.env, "hosts": t.hosts.iter().map(|h| h.host.to_ascii_lowercase()).collect::<Vec<_>>()}))
            .collect(),
    ))
}

async fn show(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Detail> {
    Ok(Json(oracle_views::show(&s.app, &ws, &slug).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}

#[derive(Deserialize)]
struct EvalQuery {
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    before: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn evaluations(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<EvalQuery>) -> ApiResult<Vec<Evaluation>> {
    Ok(Json(oracle_eval::history(&s.app, &ws, Some(&slug), q.failed, q.before, q.limit.unwrap_or(50).clamp(1, 500)).await.map_err(bad)?))
}

async fn versions(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Vec<launchpad_core::plain::Change>> {
    Ok(Json(oracles::history(&s.app, &ws, Some(&slug)).await.map_err(bad)?))
}

/// The change, checked to be one of this oracle's.
async fn change_of(s: &Ctx, ws: &str, slug: &str, change: i64) -> Result<(), ApiError> {
    let ok = oracles::history(&s.app, ws, Some(slug)).await.map_err(bad)?.iter().any(|c| c.id == change);
    if ok { Ok(()) } else { Err(ApiError::not_found(format!("no change {change} of the oracle {slug}"))) }
}

async fn diff(State(s): State<Ctx>, Path((ws, slug, change)): Path<(String, String, i64)>) -> ApiResult<Value> {
    change_of(&s, &ws, &slug, change).await?;
    Ok(Json(json!({"patch": oracles::diff(&s.app, &ws, change).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct Save {
    content: String,
}

/// An agent (admin) saves code and dependencies, never more access than the oracle has (`oracles::widens`).
async fn save(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(b): Json<Save>) -> ApiResult<Oracle> {
    if agent.is_some() {
        let meta = oracles::check(&s.app, &b.content).await.map_err(bad)?;
        let old = oracles::get(&s.app, &ws, &slug).await.ok().and_then(|f| f.oracle.meta).map(|m| m.access);
        let added = oracles::widens(old.as_ref(), &meta.access);
        if !added.is_empty() {
            return Err(forbidden(format!(
                "this version adds access ({}): only the owner gives an oracle more access. Propose it with `launchpad propose oracle --title \"…\" --spec-file o.json` ({{\"slug\": \"{slug}\", \"content\": \"<the whole file>\"}})",
                added.join(", ")
            )));
        }
    }
    let by = author(&s, &agent).await?;
    Ok(Json(oracles::save(&s.app, &ws, &slug, &b.content, by).await.map_err(bad)?))
}

#[derive(Deserialize, Default)]
struct Delete {
    #[serde(default)]
    force: bool,
}

async fn remove(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, body: axum::body::Bytes) -> ApiResult<Value> {
    let d: Delete = if body.is_empty() { Delete::default() } else { serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("{e}")))? };
    let users = gates::users(&s.app, &ws, &slug).await.map_err(bad)?;
    if !users.is_empty() && !d.force {
        let names = users.iter().map(|r| format!("“{}”", r.title)).collect::<Vec<_>>().join(", ");
        return Err(ApiError::new(axum::http::StatusCode::CONFLICT, format!("routines are gated on this oracle ({names}): confirm to delete it anyway (force)")));
    }
    oracles::delete(&s.app, &ws, &slug, Author::owner(None)).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn restore(State(s): State<Ctx>, Path((ws, slug, change)): Path<(String, String, i64)>) -> ApiResult<Value> {
    change_of(&s, &ws, &slug, change).await?;
    oracles::restore(&s.app, &ws, change, Author::owner(None)).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn pause(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Oracle> {
    Ok(Json(oracles::set_paused(&s.app, &ws, &slug, true).await.map_err(bad)?))
}

async fn resume(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Oracle> {
    Ok(Json(oracles::set_paused(&s.app, &ws, &slug, false).await.map_err(bad)?))
}

async fn mark_read(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Value> {
    oracles::get(&s.app, &ws, &slug).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    oracle_eval::mark_read(&s.app, &ws, &slug).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Test {
    #[serde(default)]
    params: Map<String, Value>,
    /// A draft to run instead of the saved file (not recorded).
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    cursor: Option<Value>,
}

/// The owner's Test now: a draft runs without records; the saved oracle is evaluated and recorded as a test (it
/// counts for the failure row: a good answer clears it), committing no cursor. A paused oracle runs like a draft.
async fn test(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Json(t): Json<Test>) -> ApiResult<TestAnswer> {
    let saved = match &t.content {
        Some(_) => None,
        None => Some(oracles::get(&s.app, &ws, &slug).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?),
    };
    if let Some(content) = t.content.as_deref().or(saved.as_ref().filter(|f| f.oracle.paused).map(|f| f.content.as_str())) {
        let result = oracle_eval::test_draft(&s.app, &ws, &slug, content, &t.params, t.cursor).await.map_err(bad)?;
        return Ok(Json(TestAnswer { result, evaluation_id: None }));
    }
    let req = oracle_eval::Request {
        workspace_id: ws,
        oracle: slug,
        params: t.params,
        caller: Caller { kind: CallerKind::Test, id: None, name: Some("Test now".into()) },
        event: json!({"kind": "owner"}),
        cursor_key: None,
        cursor: t.cursor,
        max_age_ms: None,
        force: true,
    };
    let e = oracle_eval::evaluate(&s.app, req).await.map_err(bad)?;
    Ok(Json(TestAnswer::of(&e)))
}

#[derive(Deserialize)]
struct Run {
    #[serde(default)]
    params: Map<String, Value>,
    /// A named cursor of the caller, committed when the answer is True or False.
    #[serde(default)]
    cursor: Option<String>,
    /// A stateless run from this cursor value.
    #[serde(default)]
    since: Option<Value>,
    /// Accept a result this fresh (`2m`).
    #[serde(default)]
    max_age: Option<String>,
}

/// Pure: a named cursor's key, `cli:<workspace>:<name>` (lowercase letters, digits, - and _).
fn cursor_key(ws: &str, name: &str) -> Option<String> {
    oracles::valid_slug(name).then(|| format!("cli:{ws}:{name}"))
}

/// `launchpad oracle run`: every access level of the workspace, and the owner. A named cursor is the caller's own
/// (shared by name within the workspace) and commits only on a True or False answer.
async fn run(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(r): Json<Run>) -> ApiResult<Evaluation> {
    if r.cursor.is_some() && r.since.is_some() {
        return Err(ApiError::bad_request("give either a named cursor or since, not both"));
    }
    let key = match r.cursor.as_deref() {
        Some(n) => Some(cursor_key(&ws, n).ok_or_else(|| ApiError::bad_request("a cursor name uses lowercase letters, digits, - and _"))?),
        None => None,
    };
    let max_age_ms = match r.max_age.as_deref() {
        Some(a) => Some(oracles::duration_text(a).map_err(|e| ApiError::bad_request(format!("max_age: {e:#}")))?),
        None => None,
    };
    let caller = match &agent {
        Some(a) => Caller { kind: CallerKind::Cli, id: Some(a.0.0.clone()), name: s.app.sessions.get(&a.0.0).map(|x| x.title) },
        None => Caller { kind: CallerKind::Owner, id: None, name: None },
    };
    let req = oracle_eval::Request { workspace_id: ws, oracle: slug, params: r.params, caller: caller.clone(), event: json!({"kind": "cli"}), cursor_key: key.clone(), cursor: r.since, max_age_ms, force: false };
    let e = oracle_eval::evaluate(&s.app, req).await.map_err(bad)?;
    let consequence = match &key {
        Some(k) if e.outcome != oracle_eval::Outcome::Failed => {
            let committed = oracle_eval::commit(&s.app, k, &e.id).await.map_err(bad)?;
            if committed { "returned, cursor committed" } else { "returned" }
        }
        _ => "returned",
    };
    let _ = oracle_eval::set_consequence(&s.app, &e.id, &caller, consequence, key).await;
    Ok(Json(e))
}

#[cfg(test)]
mod tests {
    #[test]
    fn named_cursors_are_per_workspace() {
        assert_eq!(super::cursor_key("w1", "triage").as_deref(), Some("cli:w1:triage"));
        assert_eq!(super::cursor_key("w1", "Triage"), None);
        assert_eq!(super::cursor_key("w1", "a:b"), None);
    }
}

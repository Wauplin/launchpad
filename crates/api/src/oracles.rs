//! Oracles on Home (`launchpad_automation::oracles`, `oracle_eval`): list, show, evaluations, versions, the tools
//! and interface, and the actions of the action list (`oracle.*`). Runners forward every `/api/oracles…` request to
//! Home, which checks an agent's workspace (`agent_auth::within_workspace`) and its access: every level reads and
//! runs, propose tests drafts (`launchpad oracle test`) and admin saves, both only with the credentialed tools
//! (github, huggingface, slack) the agent's own session holds; the rest is the owner's. A file the pre-check refuses
//! answers 422 with each problem (`problems`).
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use launchpad_automation::gates;
use launchpad_automation::oracle_check::Refused;
use launchpad_automation::oracle_eval::{self, Caller, CallerKind, Evaluation};
use launchpad_automation::oracle_views::{self, Detail, ListItem, TestAnswer, ToolInfo};
use launchpad_automation::oracles::{self, Oracle, Saved, Tool};
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
        .route("/api/oracles/{workspace}/_interface", get(interface))
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

/// A refused pre-check answers 422 with its problems; other errors 400.
fn bad(e: anyhow::Error) -> ApiError {
    if let Some(r) = e.downcast_ref::<Refused>() {
        let mut detail = Map::new();
        detail.insert("problems".into(), serde_json::to_value(&r.problems).unwrap_or_default());
        return ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, r.to_string()).with_detail(detail);
    }
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

/// The credentialed tools of a file (its header) that an agent's session does not hold: an agent saves, tests or
/// accepts an oracle only with the tools it already holds (`vault::session_tools`).
async fn not_held(s: &Ctx, session: &str, content: &str) -> Result<Vec<Tool>, ApiError> {
    let Ok(h) = oracles::header(content) else { return Ok(vec![]) };
    let need = oracles::credentialed(&h.tools);
    if need.is_empty() {
        return Ok(vec![]);
    }
    let held = launchpad_core::vault::session_tools(&s.app, session).await.map_err(bad)?;
    Ok(need.into_iter().filter(|t| !t.registry().is_some_and(|id| held.iter().any(|h| h == id))).collect())
}

fn names(tools: &[Tool]) -> String {
    tools.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
}

async fn list(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Vec<ListItem>> {
    launchpad_core::workspace::get(&s.app, &ws).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(oracle_views::list(&s.app, &ws).await.map_err(bad)?))
}

/// The five tools an oracle may list, with what each reaches, its registry tool and whether this workspace may use
/// it (granted, with a token), without secrets (the Tools tab).
async fn tools(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Vec<ToolInfo>> {
    launchpad_core::workspace::get(&s.app, &ws).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(oracle_views::tools(&s.app, &ws).await))
}

/// The whole interface of `launchpad_oracle` (its `.pyi`), the one Home's type checker reads.
async fn interface(Path(_ws): Path<String>) -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    ([(axum::http::header::CONTENT_TYPE, "text/x-python; charset=utf-8")], launchpad_automation::oracle_monty::STUB)
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

/// Saves an oracle: pre-checked, then stored formatted. An admin agent saves only a file whose credentialed tools
/// its own session holds; another file is a card for the owner.
async fn save(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(b): Json<Save>) -> ApiResult<Saved> {
    if let Some(a) = &agent {
        let refused = not_held(&s, &a.0.0, &b.content).await?;
        if !refused.is_empty() {
            return Err(forbidden(format!(
                "this oracle uses a tool this session does not hold ({}): only the owner gives it that token. Propose it with `launchpad propose oracle --title \"…\" --spec-file o.json` ({{\"slug\": \"{slug}\", \"content\": \"<the whole file>\"}})",
                names(&refused)
            )));
        }
    }
    let by = author(&s, &agent).await?;
    Ok(Json(Box::pin(oracles::save(&s.app, &ws, &slug, &b.content, by)).await.map_err(bad)?))
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
    /// None (not given): the saved oracle runs with the parameters of its last evaluation (Test now from Today or
    /// the list, which know no parameters).
    #[serde(default)]
    params: Option<Map<String, Value>>,
    /// A draft to run instead of the saved file (pre-checked, recorded nowhere).
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    cursor: Option<Value>,
}

/// Test: a draft is pre-checked, then runs exactly like the saved oracle would, recorded nowhere. The owner's Test
/// of the saved file is recorded as a test (it ends a failure streak on True or False) and commits no cursor, with
/// the parameters of its last evaluation when the request gives none; a paused one runs without records. An agent
/// (`launchpad oracle test`) only runs drafts, and only with the credentialed tools its own session holds: a draft
/// never inherits the tools of the saved oracle of the same slug.
async fn test(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(t): Json<Test>) -> ApiResult<TestAnswer> {
    if !oracles::valid_slug(&slug) {
        return Err(ApiError::bad_request(format!("{slug:?} is not a valid oracle slug")));
    }
    if let Some(a) = &agent {
        let Some(content) = &t.content else {
            return Err(forbidden("agents test drafts (`launchpad oracle test --file`); run a saved oracle with `launchpad oracle run`".into()));
        };
        let refused = not_held(&s, &a.0.0, content).await?;
        if !refused.is_empty() {
            return Err(forbidden(format!("this draft uses a tool this session does not hold ({}): test it without, or propose it with `launchpad propose oracle` for the owner", names(&refused))));
        }
    }
    if let Some(content) = &t.content {
        let params = t.params.unwrap_or_default();
        let (result, warnings) = Box::pin(oracle_eval::test_draft(&s.app, &ws, content, &params, t.cursor)).await.map_err(bad)?;
        return Ok(Json(TestAnswer { result, evaluation_id: None, warnings }));
    }
    let saved = oracles::get(&s.app, &ws, &slug).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    if let Some(why) = saved.oracle.broken_reason() {
        return Err(ApiError::bad_request(format!("the oracle {slug} is broken and does not run until it is saved again: {why}")));
    }
    let params = match t.params {
        Some(p) => p,
        None => match oracle_eval::history(&s.app, &ws, Some(&slug), false, None, 1).await.map_err(bad)?.into_iter().next() {
            Some(e) => e.params.as_object().cloned().unwrap_or_default(),
            None => Map::new(),
        },
    };
    if saved.oracle.paused {
        let meta = saved.oracle.meta.clone().ok_or_else(|| ApiError::bad_request("the oracle has no definition"))?;
        let params = oracles::validate_params(&meta.params, &params).map_err(bad)?;
        let seed = format!("test_{}", uuid::Uuid::new_v4().simple());
        let result = Box::pin(oracle_eval::run(&s.app, &ws, &saved.content, &meta, &params, &t.cursor, &seed)).await;
        return Ok(Json(TestAnswer { result, evaluation_id: None, warnings: vec![] }));
    }
    let req = oracle_eval::Request {
        workspace_id: ws,
        oracle: slug,
        params,
        caller: Caller { kind: CallerKind::Test, id: None, name: Some("Test now".into()) },
        event: json!({"kind": "owner"}),
        cursor_key: None,
        cursor: t.cursor,
        max_age_ms: None,
        force: true,
    };
    let e = Box::pin(oracle_eval::evaluate(&s.app, req)).await.map_err(bad)?;
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

/// Pure: a named cursor's key, `cli:<workspace>:<oracle>:<name>` (lowercase letters, digits, - and _): one cursor
/// per oracle, so a name used with two oracles never feeds one's cursor to the other.
fn cursor_key(ws: &str, slug: &str, name: &str) -> Option<String> {
    oracles::valid_slug(name).then(|| format!("cli:{ws}:{slug}:{name}"))
}

/// `launchpad oracle run`: every access level of the workspace, and the owner. A named cursor is the caller's own
/// (shared by name within the workspace) and commits only on a True or False answer.
async fn run(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(r): Json<Run>) -> ApiResult<Evaluation> {
    if r.cursor.is_some() && r.since.is_some() {
        return Err(ApiError::bad_request("give either a named cursor or since, not both"));
    }
    let key = match r.cursor.as_deref() {
        Some(n) => Some(cursor_key(&ws, &slug, n).ok_or_else(|| ApiError::bad_request("a cursor name uses lowercase letters, digits, - and _"))?),
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
    let e = Box::pin(oracle_eval::evaluate(&s.app, req)).await.map_err(bad)?;
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
        assert_eq!(super::cursor_key("w1", "ci-red", "triage").as_deref(), Some("cli:w1:ci-red:triage"));
        assert_eq!(super::cursor_key("w1", "ci-red", "Triage"), None);
        assert_eq!(super::cursor_key("w1", "ci-red", "a:b"), None);
    }
}

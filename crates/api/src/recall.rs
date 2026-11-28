//! Recall on Home (`launchpad_recall`): the search over a workspace's past sessions, its status, and the admin
//! actions of the action list (`recall.set`, `recall.backfill`, `recall.rebuild`). Runners forward every
//! `/api/recall…` request to Home, which checks an agent's workspace (`agent_auth::within_workspace`): every access
//! level reads, in its own workspace; a bot's session only when its bot.toml has `recall = true`.
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use launchpad_core::config::RecallConfig;
use launchpad_http::AgentSession;
use launchpad_recall::{index, query};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/recall/config", get(config).patch(set_config))
        .route("/api/recall/{workspace}/search", get(search))
        .route("/api/recall/{workspace}/get", get(read))
        .route("/api/recall/{workspace}/status", get(status))
        .route("/api/recall/{workspace}/backfill", post(backfill))
        .route("/api/recall/{workspace}/rebuild", post(rebuild))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn home(s: &Ctx, ws: &str) -> Result<(), ApiError> {
    if s.app.paths.role != launchpad_core::role::Role::Home {
        return Err(ApiError::bad_request("recall lives on Home"));
    }
    launchpad_core::workspace::get(&s.app, ws).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(())
}

/// A bot's session reads recall only when its bot has it on; `--mine` names that bot. Returns the caller's bot.
async fn caller_bot(s: &Ctx, agent: &Option<Extension<AgentSession>>) -> Result<Option<String>, ApiError> {
    let Some(session) = agent.as_ref().and_then(|a| s.app.sessions.get(&a.0.0)) else { return Ok(None) };
    let Some(bot) = session.launch_spec.bot else { return Ok(None) };
    let on = launchpad_core::bots::get(&s.app, &bot.id).await.is_ok_and(|b| b.settings.recall);
    if !on {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, format!("recall is not enabled for the bot {}: the owner, or an admin session, sets `recall = true` in its bot.toml (`launchpad bot update {} --body-file f.json` with {{\"set\": {{\"recall\": true}}}})", bot.name, bot.id)));
    }
    Ok(Some(bot.id))
}

/// The search's query string. It lists the fields of `query::Search` itself, without `#[serde(flatten)]`: query
/// strings only carry text, and flattened numbers and booleans (`k=2`, `oneline=true`) would not parse.
#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    k: Option<usize>,
    since: Option<String>,
    until: Option<String>,
    #[serde(rename = "type")]
    block_type: Option<String>,
    bot: Option<String>,
    #[serde(default)]
    oneline: bool,
    /// Only the calling bot's own sessions.
    #[serde(default)]
    mine: bool,
}

impl SearchQuery {
    /// The search, and whether it is narrowed to the calling bot (`mine`).
    fn split(self) -> (query::Search, bool) {
        let SearchQuery { q, k, since, until, block_type, bot, oneline, mine } = self;
        (query::Search { q, k, since, until, block_type, bot, oneline }, mine)
    }
}

async fn search(State(s): State<Ctx>, Path(ws): Path<String>, agent: Option<Extension<AgentSession>>, Query(q): Query<SearchQuery>) -> ApiResult<Value> {
    home(&s, &ws).await?;
    let bot = caller_bot(&s, &agent).await?;
    let (mut search, mine) = q.split();
    if mine {
        search.bot = Some(bot.ok_or_else(|| ApiError::bad_request("--mine is for a bot's session; name a bot with --bot <slug>"))?);
    }
    Ok(Json(json!({ "text": query::search(&s.app, &ws, &search).await.map_err(bad)? })))
}

async fn read(State(s): State<Ctx>, Path(ws): Path<String>, agent: Option<Extension<AgentSession>>, Query(q): Query<query::Get>) -> ApiResult<Value> {
    home(&s, &ws).await?;
    caller_bot(&s, &agent).await?;
    Ok(Json(json!({ "text": query::get(&s.app, &ws, &q).await.map_err(bad)? })))
}

async fn status(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Value> {
    home(&s, &ws).await?;
    Ok(Json(query::status(&s.app, &ws).await.map_err(bad)?))
}

async fn config(State(s): State<Ctx>) -> ApiResult<RecallConfig> {
    Ok(Json(s.app.config.get().home.recall))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetConfig {
    enabled: Option<bool>,
    include_archived: Option<bool>,
    hidden: Option<Vec<String>>,
}

async fn set_config(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>, Json(mut b): Json<SetConfig>) -> ApiResult<RecallConfig> {
    if s.app.paths.role != launchpad_core::role::Role::Home {
        return Err(ApiError::bad_request("recall lives on Home"));
    }
    // The hidden list is Home-wide. An agent changes only its own workspace's part of it: the entries of other
    // workspaces stay as they are, and it adds none of theirs.
    if let (Some(agent), Some(given)) = (&agent, b.hidden.take()) {
        let ws = launchpad_core::kb::workspace_of_session(&s.app, &agent.0.0).await.ok_or_else(|| ApiError::bad_request("this session has no workspace"))?;
        let own: std::collections::HashSet<String> = index::workspace_sessions(&s.app, &ws).await.map_err(bad)?.into_iter().map(|x| x.id).collect();
        let mut hidden: Vec<String> = s.app.config.get().home.recall.hidden.into_iter().filter(|h| !own.contains(h)).collect();
        hidden.extend(given.into_iter().map(|h| h.trim().to_string()).filter(|h| own.contains(h)));
        b.hidden = Some(hidden);
    }
    let saved = s
        .app
        .config
        .update(|c| {
            let r = &mut c.home.recall;
            if let Some(v) = b.enabled {
                r.enabled = v;
            }
            if let Some(v) = b.include_archived {
                r.include_archived = v;
            }
            if let Some(v) = b.hidden {
                r.hidden = v.into_iter().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).collect();
            }
            Ok(r.clone())
        })
        .map_err(bad)?;
    Ok(Json(saved))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct BackfillBody {
    #[serde(default)]
    dry_run: bool,
}

fn must_be_on(s: &Ctx) -> Result<(), ApiError> {
    if !index::enabled(&s.app) {
        return Err(ApiError::bad_request("recall is off: turn it on first (Settings → Recall, or `launchpad recall enable`)"));
    }
    launchpad_recall::funes::ready(&s.app, false).map(|_| ()).map_err(bad)
}

/// Starts the backfill in the background and answers with what it will do (`plan`): `launchpad recall status` shows
/// its progress. `dry_run`: only the plan.
async fn backfill(State(s): State<Ctx>, Path(ws): Path<String>, body: Option<Json<BackfillBody>>) -> ApiResult<Value> {
    home(&s, &ws).await?;
    let dry_run = body.is_some_and(|b| b.dry_run);
    let plan = index::plan(&s.app, &ws).await.map_err(bad)?;
    if dry_run {
        return Ok(Json(json!({ "dry_run": true, "started": false, "plan": plan })));
    }
    must_be_on(&s)?;
    index::keep_indexed(&s.app, &ws).map_err(bad)?;
    let started = index::start_backfill(&s.app, &ws, false);
    Ok(Json(json!({ "dry_run": false, "started": started, "already_running": !started, "plan": plan })))
}

async fn rebuild(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Value> {
    home(&s, &ws).await?;
    must_be_on(&s)?;
    if index::backfill_running(&s.app, &ws) {
        return Err(ApiError::bad_request("a backfill of this workspace runs now: wait for it (`launchpad recall status`), then rebuild"));
    }
    index::reset(&s.app, &ws).await.map_err(bad)?;
    index::keep_indexed(&s.app, &ws).map_err(bad)?;
    let plan = index::plan(&s.app, &ws).await.map_err(bad)?;
    let started = index::start_backfill(&s.app, &ws, false);
    Ok(Json(json!({ "started": started, "plan": plan })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(query: &str) -> T {
        let uri: axum::http::Uri = format!("/api/recall/ws/x?{query}").parse().unwrap();
        Query::<T>::try_from_uri(&uri).unwrap_or_else(|e| panic!("{query}: {e}")).0
    }

    /// The query strings the CLI sends parse through the handlers' own extractors, typed values included.
    #[test]
    fn query_strings_parse() {
        let (s, mine) = parse::<SearchQuery>("q=x&k=2&oneline=true&mine=true&type=text&since=2026-01-01&bot=scout").split();
        assert_eq!((s.q.as_str(), s.k, s.oneline, mine), ("x", Some(2), true, true));
        assert_eq!((s.block_type.as_deref(), s.since.as_deref(), s.until, s.bot.as_deref()), (Some("text"), Some("2026-01-01"), None, Some("scout")));
        let (s, mine) = parse::<SearchQuery>("q=two+words").split();
        assert_eq!((s.q.as_str(), s.k, s.oneline, mine), ("two words", None, false, false));
        let g = parse::<query::Get>("session=abc&seq=12");
        assert_eq!((g.session.as_str(), g.seq, g.from, g.to), ("abc", Some(12), None, None));
        let g = parse::<query::Get>("session=abc&from=3&to=9");
        assert_eq!((g.seq, g.from, g.to), (None, Some(3), Some(9)));
    }
}

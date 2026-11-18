//! Folios on Home (`launchpad_automation::folios`): the reader's views, the agents' reads and writes, marks,
//! feedback, revs and the owner's actions (`folio.*` in the action list). Runners forward every `/api/folios…`
//! request to Home, which checks an agent's workspace (`agent_auth::within_workspace`) and its access: every level
//! reads; propose creates, writes, fixes, replies and resolves, as the session's own writes (Home takes the author
//! from its own session record); marks, Send feedback, Restore, Archive and Delete are the owner's.
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::Engine;
use launchpad_automation::folios::{self, Activity, Ask, Folio, FolioDetail, Mark, MarkInput, Op, Rev, SendTo, Target, Written};
use launchpad_automation::folio_check::Problem;
use launchpad_core::kb::Author;
use launchpad_http::AgentSession;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/folios/{workspace}", get(list).post(create))
        .route("/api/folios/{workspace}/_activity", get(activity))
        .route("/api/folios/{workspace}/_session/{session}", get(of_session))
        .route("/api/folios/{workspace}/{slug}", get(show).delete(remove))
        .route("/api/folios/{workspace}/{slug}/file", get(file))
        .route("/api/folios/{workspace}/{slug}/raw/{*path}", get(raw))
        .route("/api/folios/{workspace}/{slug}/write", post(write).layer(axum::extract::DefaultBodyLimit::max(launchpad_http::agent_auth::FOLIO_BODY_MAX)))
        .route("/api/folios/{workspace}/{slug}/check", get(check))
        .route("/api/folios/{workspace}/{slug}/fix", post(fix))
        .route("/api/folios/{workspace}/{slug}/revs", get(revs))
        .route("/api/folios/{workspace}/{slug}/revs/{rev}", get(diff))
        .route("/api/folios/{workspace}/{slug}/revs/{rev}/restore", post(restore))
        .route("/api/folios/{workspace}/{slug}/archive", post(archive))
        .route("/api/folios/{workspace}/{slug}/marks", get(marks).post(mark))
        .route("/api/folios/{workspace}/{slug}/marks/{id}", axum::routing::delete(unmark))
        .route("/api/folios/{workspace}/{slug}/marks/{id}/reply", post(reply))
        .route("/api/folios/{workspace}/{slug}/marks/{id}/resolve", post(resolve))
        .route("/api/folios/{workspace}/{slug}/feedback", get(feedback))
        .route("/api/folios/{workspace}/{slug}/items", get(items))
        .route("/api/folios/{workspace}/{slug}/targets", get(targets))
        .route("/api/folios/{workspace}/{slug}/send", post(send))
        .route("/api/folios/{workspace}/{slug}/ask-fix", post(ask_fix))
        .route("/api/folios/{workspace}/{slug}/ask", post(ask))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

fn missing(e: anyhow::Error) -> ApiError {
    ApiError::not_found(format!("{e:#}"))
}

async fn author(s: &Ctx, agent: &Option<Extension<AgentSession>>) -> Result<Author, ApiError> {
    match agent {
        Some(a) => launchpad_core::kb::agent_author(&s.app, &a.0.0).await.map_err(bad),
        None => Ok(Author::owner(None)),
    }
}

/// The owner's actions refuse agents (they are not on the agents' list in `agent_auth`, so this only guards
/// mistakes).
fn owner_only(agent: &Option<Extension<AgentSession>>, what: &str) -> Result<(), ApiError> {
    match agent {
        Some(_) => Err(ApiError::new(StatusCode::FORBIDDEN, format!("{what} is the owner's: agents read feedback with `launchpad folio feedback` and answer with `launchpad folio reply|resolve`"))),
        None => Ok(()),
    }
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    archived: bool,
}

async fn list(State(s): State<Ctx>, Path(ws): Path<String>, Query(q): Query<ListQuery>) -> ApiResult<Vec<Folio>> {
    launchpad_core::workspace::get(&s.app, &ws).await.map_err(missing)?;
    Ok(Json(folios::list(&s.app, &ws, q.archived).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Create {
    title: String,
    #[serde(default = "report")]
    kind: String,
    #[serde(default)]
    slug: Option<String>,
}

fn report() -> String {
    "report".into()
}

async fn create(State(s): State<Ctx>, Path(ws): Path<String>, agent: Option<Extension<AgentSession>>, Json(c): Json<Create>) -> ApiResult<Written> {
    let by = author(&s, &agent).await?;
    Ok(Json(folios::create(&s.app, &ws, &c.title, &c.kind, c.slug.as_deref(), by).await.map_err(bad)?))
}

async fn activity(State(s): State<Ctx>, Path(ws): Path<String>) -> ApiResult<Vec<Activity>> {
    Ok(Json(folios::activity(&s.app, &ws).await.map_err(bad)?))
}

async fn of_session(State(s): State<Ctx>, Path((ws, session)): Path<(String, String)>) -> ApiResult<Vec<Folio>> {
    Ok(Json(folios::of_session(&s.app, &session).await.map_err(bad)?.into_iter().filter(|f| f.workspace_id == ws).collect()))
}

#[derive(Deserialize, Default)]
struct RevQuery {
    #[serde(default)]
    rev: Option<i64>,
    #[serde(default)]
    path: Option<String>,
}

async fn show(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<RevQuery>) -> ApiResult<FolioDetail> {
    Ok(Json(folios::detail(&s.app, &ws, &slug, q.rev).await.map_err(missing)?))
}

/// A file's raw content (pages as text), at the latest rev or `rev`.
async fn file(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<RevQuery>) -> Result<Response, ApiError> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    let path = q.path.ok_or_else(|| ApiError::bad_request("name a file (?path=)"))?;
    let data = folios::file_bytes(&s.app, &f.id, &path, q.rev).await.map_err(missing)?;
    Ok(served(&path, data, true))
}

/// A file for the reader's `<img>`: never run by the browser (sandboxed, no sniffing).
async fn raw(State(s): State<Ctx>, Path((ws, slug, path)): Path<(String, String, String)>, Query(q): Query<RevQuery>) -> Result<Response, ApiError> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    let data = folios::file_bytes(&s.app, &f.id, &path, q.rev).await.map_err(missing)?;
    Ok(served(&path, data, false))
}

fn served(path: &str, data: Vec<u8>, text: bool) -> Response {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "md" | "csv" | "txt" | "html" | "json" | "yaml" | "yml" if text => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    let mut r = (StatusCode::OK, data).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox; default-src 'none'; style-src 'unsafe-inline'"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=30"));
    r
}

/// One write as JSON: `content` for text, `base64` for media.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WireOp {
    Write {
        path: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        base64: Option<String>,
    },
    Append { path: String, text: String },
    Patch { path: String, section: String, text: String },
    Remove { path: String },
}

#[derive(Deserialize)]
struct WriteBody {
    ops: Vec<WireOp>,
    #[serde(default)]
    base_rev: Option<i64>,
    #[serde(default)]
    strict: bool,
}

/// `launchpad folio write|append|patch|add-media|rm`: the session's own write (propose access), or the owner's.
async fn write(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(b): Json<WriteBody>) -> ApiResult<Written> {
    let mut ops = vec![];
    for o in b.ops {
        ops.push(match o {
            WireOp::Write { path, content: Some(c), base64: None } => Op::Write { path, data: c.into_bytes() },
            WireOp::Write { path, content: None, base64: Some(b) } => Op::Write { path, data: base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|e| ApiError::bad_request(format!("base64: {e}")))? },
            WireOp::Write { .. } => return Err(ApiError::bad_request("a write gives either content or base64")),
            WireOp::Append { path, text } => Op::Append { path, text },
            WireOp::Patch { path, section, text } => Op::Patch { path, section, text },
            WireOp::Remove { path } => Op::Remove { path },
        });
    }
    if ops.is_empty() {
        return Err(ApiError::bad_request("nothing to write"));
    }
    let by = author(&s, &agent).await?;
    Ok(Json(folios::write(&s.app, &ws, &slug, ops, by, b.base_rev, b.strict, None).await.map_err(bad)?))
}

#[derive(Deserialize, Default)]
struct CheckQuery {
    #[serde(default)]
    rev: Option<i64>,
}

async fn check(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<CheckQuery>) -> ApiResult<Value> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    let problems: Vec<Problem> = folios::check_stored(&s.app, &f, q.rev).await.map_err(bad)?;
    Ok(Json(json!({"slug": f.slug, "rev": q.rev.unwrap_or(f.rev), "problems": problems})))
}

async fn fix(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>) -> ApiResult<Written> {
    let by = author(&s, &agent).await?;
    Ok(Json(folios::fix(&s.app, &ws, &slug, by).await.map_err(bad)?))
}

async fn revs(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Vec<Rev>> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    Ok(Json(folios::revs(&s.app, &f.id).await.map_err(bad)?))
}

async fn diff(State(s): State<Ctx>, Path((ws, slug, rev)): Path<(String, String, i64)>) -> ApiResult<Value> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    Ok(Json(json!({"rev": rev, "files": folios::rev_diff(&s.app, &f.id, rev).await.map_err(missing)?})))
}

async fn restore(State(s): State<Ctx>, Path((ws, slug, rev)): Path<(String, String, i64)>, agent: Option<Extension<AgentSession>>) -> ApiResult<Written> {
    owner_only(&agent, "Restore")?;
    Ok(Json(folios::restore(&s.app, &ws, &slug, rev, Author::owner(None)).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Archive {
    #[serde(default = "yes")]
    archived: bool,
}

fn yes() -> bool {
    true
}

async fn archive(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, body: Bytes) -> ApiResult<Folio> {
    owner_only(&agent, "Archive")?;
    let a: Archive = if body.is_empty() { Archive { archived: true } } else { serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("{e}")))? };
    Ok(Json(folios::set_archived(&s.app, &ws, &slug, a.archived).await.map_err(bad)?))
}

async fn remove(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>) -> ApiResult<Value> {
    owner_only(&agent, "Delete")?;
    folios::delete(&s.app, &ws, &slug).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
struct SinceQuery {
    #[serde(default)]
    since: Option<i64>,
}

async fn marks(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<SinceQuery>) -> ApiResult<Vec<Mark>> {
    let f = folios::get(&s.app, &ws, &slug).await.map_err(missing)?;
    Ok(Json(folios::marks(&s.app, &f.id, q.since).await.map_err(bad)?))
}

async fn mark(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(m): Json<MarkInput>) -> ApiResult<Option<Mark>> {
    owner_only(&agent, "Marking a folio")?;
    Ok(Json(folios::mark(&s.app, &ws, &slug, m).await.map_err(bad)?))
}

async fn unmark(State(s): State<Ctx>, Path((ws, slug, id)): Path<(String, String, i64)>, agent: Option<Extension<AgentSession>>) -> ApiResult<Value> {
    owner_only(&agent, "Deleting a mark")?;
    folios::unmark(&s.app, &ws, &slug, id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Reply {
    text: String,
}

async fn reply(State(s): State<Ctx>, Path((ws, slug, id)): Path<(String, String, i64)>, agent: Option<Extension<AgentSession>>, Json(r): Json<Reply>) -> ApiResult<Mark> {
    let by = author(&s, &agent).await?;
    Ok(Json(folios::reply(&s.app, &ws, &slug, id, &r.text, by).await.map_err(bad)?))
}

#[derive(Deserialize, Default)]
struct Resolve {
    #[serde(default)]
    rev: Option<i64>,
    /// Open it again.
    #[serde(default)]
    open: bool,
}

async fn resolve(State(s): State<Ctx>, Path((ws, slug, id)): Path<(String, String, i64)>, agent: Option<Extension<AgentSession>>, body: Bytes) -> ApiResult<Mark> {
    let r: Resolve = if body.is_empty() { Resolve::default() } else { serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("{e}")))? };
    let by = author(&s, &agent).await?;
    Ok(Json(folios::resolve(&s.app, &ws, &slug, id, r.rev, r.open, by).await.map_err(bad)?))
}

async fn feedback(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<SinceQuery>) -> ApiResult<Value> {
    Ok(Json(folios::feedback(&s.app, &ws, &slug, q.since).await.map_err(missing)?))
}

#[derive(Deserialize, Default)]
struct ItemsQuery {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    open: bool,
}

async fn items(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, Query(q): Query<ItemsQuery>) -> ApiResult<Vec<Value>> {
    Ok(Json(folios::items(&s.app, &ws, &slug, q.kind.as_deref(), q.verdict.as_deref(), q.open).await.map_err(missing)?))
}

async fn targets(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>) -> ApiResult<Vec<Target>> {
    Ok(Json(folios::targets(&s.app, &ws, &slug).await.map_err(missing)?))
}

async fn send(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(to): Json<SendTo>) -> ApiResult<Value> {
    owner_only(&agent, "Send feedback")?;
    Ok(Json(Box::pin(folios::send(&s.app, &ws, &slug, to)).await.map_err(bad)?))
}

async fn ask_fix(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(to): Json<SendTo>) -> ApiResult<Value> {
    owner_only(&agent, "Ask the author to fix")?;
    Ok(Json(Box::pin(folios::ask_fix(&s.app, &ws, &slug, to)).await.map_err(bad)?))
}

async fn ask(State(s): State<Ctx>, Path((ws, slug)): Path<(String, String)>, agent: Option<Extension<AgentSession>>, Json(a): Json<Ask>) -> ApiResult<Value> {
    owner_only(&agent, "Ask about a folio")?;
    Ok(Json(Box::pin(folios::ask(&s.app, &ws, &slug, a)).await.map_err(bad)?))
}

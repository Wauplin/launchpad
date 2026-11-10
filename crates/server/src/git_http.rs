//! Home Git over HTTP.
//!
//! - `/api/git/<folder>.git/…` (Home): Git's smart HTTP protocol for runners, through `git http-backend`.
//!   Authenticated with the runner credential; a request naming a session is refused when that session now
//!   runs elsewhere.
//! - `/git/<folder>.git/…` (runner): the local endpoint that runner clones use as `origin`. It accepts
//!   only this daemon's Git tokens (the daemon, or one session's worktrees) and forwards to Home with the
//!   runner credential.
//! - `/api/home-git/<folder>/…`: refresh, branches, Restore, merge and pull request branches, on Home (a
//!   runner forwards them).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use launchpad_core::home_git::{self, IMPORT_HEADER, SESSION_HEADER, TOKEN_HEADER};
use launchpad_core::role::Role;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/git/{*rest}", any(home))
        .route("/git/{*rest}", any(local))
        .route("/api/home-git/{id}/refresh", post(refresh))
        .route("/api/home-git/{id}/branches", get(branches))
        .route("/api/home-git/{id}/restore", post(restore))
        .route("/api/home-git/{id}/delete", post(delete))
        .route("/api/home-git/{id}/merge", post(merge))
        .route("/api/home-git/{id}/pull-request", post(pull_request))
        .route("/api/home-git/{id}/follow", get(follow_status))
}

/// Paths that skip the browser sign-in: their handlers check a runner credential or a Git token.
pub fn git_path(path: &str) -> bool {
    path.starts_with("/api/git/") || path.starts_with("/git/")
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// Home: a runner's Git request.
async fn home(State(s): State<Server>, Path(rest): Path<String>, method: Method, uri: axum::http::Uri, headers: HeaderMap, body: Body) -> Response {
    if s.app.paths.role != Role::Home {
        return ApiError::new(StatusCode::NOT_FOUND, "Home Git is served by Home").into_response();
    }
    let credential = header_str(&headers, crate::runners::CREDENTIAL_HEADER).unwrap_or("");
    let Some(runner) = launchpad_core::runners::authenticate(&s.app, credential).await.filter(|r| r.state == launchpad_core::runners::RunnerState::Approved) else {
        return ApiError::new(StatusCode::UNAUTHORIZED, "unknown runner").into_response();
    };
    let session = header_str(&headers, SESSION_HEADER).map(str::to_string);
    if let Some(sid) = &session
        && let Err(e) = home_git::check_session(&s.app, &runner.id, sid)
    {
        return ApiError::new(StatusCode::FORBIDDEN, format!("{e:#}")).into_response();
    }
    let import = header_str(&headers, IMPORT_HEADER) == Some("1") && session.is_none();
    // An import writes every ref: only into the Git folder Home just created for this runner's kept temporary
    // folder (`runner_trust::after`).
    let folder = rest.split('/').next().unwrap_or_default().trim_end_matches(".git");
    if import && !s.app.runners.granted(&runner.id, &format!("import:{folder}")) {
        return ApiError::new(StatusCode::FORBIDDEN, "an import goes only into the folder Home created for this runner's Keep as folder").into_response();
    }
    serve(&s, &rest, &method, uri.query().unwrap_or(""), &headers, body, import, session).await
}

/// Runner: a Git request of this machine's clones.
async fn local(State(s): State<Server>, Path(rest): Path<String>, method: Method, uri: axum::http::Uri, headers: HeaderMap, body: Body) -> Response {
    if !s.app.paths.role.runs_agents() {
        return ApiError::new(StatusCode::NOT_FOUND, "no local Git endpoint on Home").into_response();
    }
    let Some(scope) = header_str(&headers, TOKEN_HEADER).and_then(|t| home_git::verify(&s.app, t)) else {
        return ApiError::new(StatusCode::UNAUTHORIZED, "this clone has no Launchpad Git token").into_response();
    };
    let session = scope.strip_prefix("session:").map(str::to_string);
    if let Some(sid) = &session
        && s.app.sessions.get(sid).is_none()
    {
        return ApiError::new(StatusCode::FORBIDDEN, "this session no longer exists").into_response();
    }
    let import = header_str(&headers, IMPORT_HEADER) == Some("1") && session.is_none();
    match forward(&s, &rest, &method, uri.query(), &headers, body, import, session).await {
        Ok(r) => r,
        Err(e) => ApiError::new(StatusCode::BAD_GATEWAY, format!("Home Git: {e:#}")).into_response(),
    }
}

/// Runner: sends the request to Home with the runner credential, streaming both bodies.
#[allow(clippy::too_many_arguments)]
async fn forward(s: &Server, rest: &str, method: &Method, query: Option<&str>, headers: &HeaderMap, body: Body, import: bool, session: Option<String>) -> anyhow::Result<Response> {
    let link = launchpad_core::home_link::load(&s.app.paths).ok_or_else(|| anyhow::anyhow!("this runner is not connected to a Home"))?;
    let url = format!("{}/api/git/{rest}{}", link.url.trim_end_matches('/'), query.map(|q| format!("?{q}")).unwrap_or_default());
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30)).build()?;
    let mut req = client.request(reqwest::Method::from_bytes(method.as_str().as_bytes())?, url).header(crate::runners::CREDENTIAL_HEADER, link.credential());
    if let Some(token) = launchpad_core::home_link::space_token(&link.url) {
        req = req.bearer_auth(token);
    }
    for name in [header::CONTENT_TYPE.as_str(), header::CONTENT_ENCODING.as_str(), header::ACCEPT.as_str(), "git-protocol"] {
        if let Some(v) = header_str(headers, name) {
            req = req.header(name, v);
        }
    }
    if let Some(sid) = session {
        req = req.header(SESSION_HEADER, sid);
    }
    if import {
        req = req.header(IMPORT_HEADER, "1");
    }
    let stream = body.into_data_stream().map(|r| r.map_err(std::io::Error::other));
    let res = req.body(reqwest::Body::wrap_stream(stream)).send().await?;
    let mut out = Response::builder().status(res.status().as_u16());
    for (k, v) in res.headers() {
        if matches!(k.as_str(), "content-type" | "cache-control" | "expires" | "pragma") {
            out = out.header(k.as_str(), v.as_bytes());
        }
    }
    Ok(out.body(Body::from_stream(res.bytes_stream()))?)
}

/// Runs `git http-backend` for one request on a mirror of this daemon.
#[allow(clippy::too_many_arguments)]
async fn serve(s: &Server, rest: &str, method: &Method, query: &str, headers: &HeaderMap, body: Body, import: bool, session: Option<String>) -> Response {
    match serve_inner(s, rest, method, query, headers, body, import, session).await {
        Ok(r) => r,
        Err(e) => ApiError::bad_request(format!("{e:#}")).into_response(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve_inner(s: &Server, rest: &str, method: &Method, query: &str, headers: &HeaderMap, body: Body, import: bool, session: Option<String>) -> anyhow::Result<Response> {
    let app = s.app.clone();
    let (project, path_info) = home_git::resolve_path(&app, rest).await?;
    home_git::ensure(&app, &project).await?;
    let receive = path_info.ends_with("/git-receive-pack");
    let before = if receive { Some(home_git::launchpad_refs(&app, &project.id).await?) } else { None };
    // A session's push writes only its own branches and snapshots (the pre-receive hook checks each ref).
    let session_refs = match (&session, receive) {
        (Some(sid), true) => Some(home_git::session_refs(&app, &project.id, sid).await),
        _ => None,
    };
    let env = home_git::backend_env(&app, &path_info, method.as_str(), query, header_str(headers, "content-type"), header_str(headers, "content-encoding"), header_str(headers, "git-protocol"), import, session_refs.as_deref());
    let mut cmd = launchpad_core::procs::command(&app.env, "git", None);
    cmd.arg("http-backend").envs(env).stdin(launchpad_core::exec::Io::Piped).stdout(launchpad_core::exec::Io::Piped).stderr(launchpad_core::exec::Io::Piped);
    let mut child = cmd.spawn().await?;
    let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no stderr"))?;
    tokio::spawn(async move {
        let mut stream = body.into_data_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            if stdin.write_all(&chunk).await.is_err() {
                return;
            }
        }
        let _ = stdin.shutdown().await;
    });
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut res = Response::builder();
    let mut status = StatusCode::OK;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        if k.eq_ignore_ascii_case("status") {
            status = v.split_whitespace().next().and_then(|c| c.parse().ok()).and_then(|c| StatusCode::from_u16(c).ok()).unwrap_or(StatusCode::BAD_GATEWAY);
        } else {
            res = res.header(k, v);
        }
    }
    let project_id = project.id.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(8);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Ok(buf[..n].to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            }
        }
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err).await;
        let _ = child.wait().await;
        if !err.trim().is_empty() {
            tracing::debug!("git http-backend: {}", err.trim());
        }
        if let Some(before) = before
            && let Err(e) = home_git::after_push(&app, &project_id, &before, session.as_deref(), true).await
        {
            tracing::warn!("Home Git after push: {e:#}");
        }
    });
    let stream = tokio_stream_from(rx);
    Ok(res.status(status).body(Body::from_stream(stream))?)
}

fn tokio_stream_from(mut rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

/// Home: the folder of a Home Git request. A runner forwards these requests to Home.
async fn home_folder(s: &Server, id: &str) -> Result<launchpad_core::model::Project, ApiError> {
    if !home_git::here(&s.app) {
        return Err(ApiError::bad_request("Home Git runs on Home"));
    }
    let p = launchpad_core::projects::get(&s.app, id).await?.ok_or_else(|| ApiError::bad_request("no such folder"))?;
    if !p.home_git {
        return Err(ApiError::bad_request(format!("{} is not a Git folder of Home", p.name)));
    }
    Ok(p)
}

async fn refresh(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    launchpad_core::home_link::refresh_mirror(&s.app, &id).await.map_err(bad)?;
    let branch = home_git::default_branch(&s.app, &id).await.ok();
    Ok(Json(json!({"ok": true, "default_branch": branch})))
}

#[derive(Deserialize)]
struct BranchQuery {
    branch: String,
}

/// Whether GitHub has the commit of a followed branch (a pull request a session continues).
async fn follow_status(State(s): State<Server>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<BranchQuery>) -> ApiResult<Option<home_git::FollowStatus>> {
    home_folder(&s, &id).await?;
    Ok(Json(home_git::follow_status(&s.app, &id, &q.branch).await.map_err(bad)?))
}

async fn branches(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Vec<home_git::HomeBranch>> {
    home_folder(&s, &id).await?;
    Ok(Json(home_git::branches(&s.app, &id).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct BranchCommit {
    branch: String,
    #[serde(default)]
    commit: String,
}

async fn restore(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<BranchCommit>) -> ApiResult<Value> {
    home_folder(&s, &id).await?;
    home_git::restore(&s.app, &id, &b.branch, &b.commit).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct DeleteBranch {
    branch: String,
    /// The worktree whose snapshot goes too.
    #[serde(default)]
    worktree: Option<String>,
}

async fn delete(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<DeleteBranch>) -> ApiResult<Value> {
    home_folder(&s, &id).await?;
    home_git::delete(&s.app, &id, &b.branch, b.worktree.as_deref()).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Merge {
    branch: String,
    head: String,
    base: String,
    /// The session whose worktree asks (Home checks the branch, its When done and workspace in its own records).
    #[serde(default)]
    session_id: Option<String>,
    /// `launchpad work finish` rather than the owner's Merge; missing counts as the agent's.
    #[serde(default = "yes")]
    by_agent: bool,
}

fn yes() -> bool {
    true
}

async fn merge(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<Merge>) -> ApiResult<home_git::MergeOutcome> {
    let p = home_folder(&s, &id).await?;
    launchpad_core::worktrees::check_merge_request(&s.app, &p, b.session_id.as_deref(), &b.branch, &b.base, b.by_agent).await.map_err(|e| ApiError::new(StatusCode::FORBIDDEN, format!("{e:#}")))?;
    Ok(Json(launchpad_core::worktrees::merge_on_home(&s.app, &p, &b.branch, &b.head, &b.base).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct PullRequest {
    repo: String,
    number: u64,
    branch: String,
    session_id: Option<String>,
}

/// A session continues a GitHub pull request: Home reads it, fetches its head into the session's branch and
/// pushes the branch back to the pull request after each push. Runners hold no GitHub credential.
async fn pull_request(State(s): State<Server>, Path(id): Path<String>, Json(b): Json<PullRequest>) -> ApiResult<launchpad_core::worktrees::PullRequestBranch> {
    let p = home_folder(&s, &id).await?;
    let pr = launchpad_core::model::PullRequestRef { repo: b.repo, number: b.number };
    Ok(Json(launchpad_core::worktrees::continue_pull_request(&s.app, &p, &pr, &b.branch, b.session_id).await.map_err(bad)?))
}

//! Code viewer routes (ARCHITECTURE.md, section 8.1). Every request names its source (`root`, and `rev`
//! for a revision); `launchpad_core::code::open` checks it before anything is read.

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use launchpad_core::code::{self, CodeFile, CodeSources, CommitInfo, FileList, SearchQuery, SearchResult, Source};
use serde::Deserialize;

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/code/sources", get(sources))
        .route("/api/code/files", get(files))
        .route("/api/code/file", get(file))
        .route("/api/code/raw", get(raw))
        .route("/api/code/search", get(search))
        .route("/api/code/commit", get(commit))
        .route("/api/code/resolve-commits", get(resolve_commits))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

#[derive(Deserialize)]
struct ProjectQuery {
    project: String,
}

async fn sources(State(s): State<Server>, Query(q): Query<ProjectQuery>) -> ApiResult<CodeSources> {
    let p = launchpad_core::projects::get(&s.app, &q.project).await?.ok_or_else(|| ApiError::not_found("no such project"))?;
    Ok(Json(code::sources(&s.app, &p).await?))
}

#[derive(Deserialize)]
struct SourceQuery {
    root: String,
    rev: Option<String>,
}

async fn source(s: &Server, root: &str, rev: Option<&str>) -> Result<Source, ApiError> {
    code::open(&s.app, root, rev).await.map_err(bad)
}

async fn files(State(s): State<Server>, Query(q): Query<SourceQuery>) -> ApiResult<FileList> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    Ok(Json(code::files(&s.app, &src).await.map_err(bad)?))
}

async fn commit(State(s): State<Server>, Query(q): Query<SourceQuery>) -> ApiResult<CommitInfo> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    Ok(Json(code::commit(&s.app, &src).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct CommitQuery {
    root: String,
    oids: String,
}

async fn resolve_commits(State(s): State<Server>, Query(q): Query<CommitQuery>) -> ApiResult<std::collections::BTreeMap<String, String>> {
    let src = source(&s, &q.root, None).await?;
    let oids = q.oids.split(',').map(str::to_string).collect::<Vec<_>>();
    Ok(Json(code::resolve_commits(&s.app.env, &src, &oids).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct FileQuery {
    root: String,
    rev: Option<String>,
    path: String,
}

async fn file(State(s): State<Server>, Query(q): Query<FileQuery>) -> ApiResult<CodeFile> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    Ok(Json(code::file(&s.app, &src, &q.path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}

/// Image bytes. The sandbox policy keeps an SVG opened on its own from running scripts on this origin.
async fn raw(State(s): State<Server>, Query(q): Query<FileQuery>) -> Result<Response, ApiError> {
    if !code::is_image(&q.path) {
        return Err(ApiError::bad_request("only images"));
    }
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    let bytes = code::raw(&s.app, &src, &q.path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    let mime = mime_guess::from_path(&q.path).first_or_octet_stream();
    Ok((
        [
            (header::CONTENT_TYPE, mime.essence_str().to_string()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; sandbox".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        bytes,
    )
        .into_response())
}

// No `#[serde(flatten)]`: query strings only carry text, and flattened booleans would not parse.
#[derive(Deserialize)]
struct Search {
    root: String,
    rev: Option<String>,
    q: String,
    #[serde(default)]
    case: bool,
    #[serde(default)]
    word: bool,
    #[serde(default)]
    regex: bool,
}

async fn search(State(s): State<Server>, Query(q): Query<Search>) -> ApiResult<SearchResult> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    let query = SearchQuery { q: q.q, case: q.case, word: q.word, regex: q.regex };
    Ok(Json(code::search(&s.app, &src, &query).await.map_err(bad)?))
}

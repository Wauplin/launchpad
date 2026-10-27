//! Code viewer routes (ARCHITECTURE.md, section 8.1). Every request names its source (`root`, and `rev`
//! for a revision); `launchpad_core::code::open` checks it before anything is read.

use axum::Json;
use axum::body::Body;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use launchpad_core::code::{self, CodeFile, CodeSources, CommitInfo, FileList, SearchQuery, SearchResult, Source};
use serde::Deserialize;
use tokio::io::AsyncReadExt;

use crate::Server;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/code/sources", get(sources))
        .route("/api/code/files", get(files))
        .route("/api/code/file", get(file))
        .route("/api/code/raw", get(raw))
        .route("/api/code/download", get(download))
        .route("/api/code/search", get(search))
        .route("/api/code/commit", get(commit))
        .route("/api/code/commit-diff", get(commit_diff))
        .route("/api/code/commit-diff/file", get(commit_diff_file))
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

async fn commit_diff(State(s): State<Server>, Query(q): Query<SourceQuery>) -> ApiResult<serde_json::Value> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    Ok(Json(code::commit_diff(&s.app, &src).await.map_err(bad)?))
}

async fn commit_diff_file(State(s): State<Server>, Query(q): Query<FileQuery>) -> ApiResult<serde_json::Value> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    Ok(Json(code::commit_diff_file(&s.app, &src, &q.path).await.map_err(bad)?))
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

/// Downloads always use an attachment and an inert MIME type, including HTML/SVG outputs.
/// Reuse the authorized source and symlink confinement; local working-tree files stream without a size cap.
async fn download(State(s): State<Server>, Query(q): Query<FileQuery>) -> Result<Response, ApiError> {
    let src = source(&s, &q.root, q.rev.as_deref()).await?;
    let (body, len) = match code::download(&s.app, &src, &q.path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))? {
        code::Download::Bytes(bytes) => {
            let len = bytes.len() as u64;
            (Body::from(bytes), len)
        }
        code::Download::File(path, len) => {
            let file = tokio::fs::File::open(&path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
            let chunks = futures_util::stream::unfold(file, |mut file| async move {
                let mut buf = vec![0u8; 256 * 1024];
                match file.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        Some((Ok::<_, std::io::Error>(buf), file))
                    }
                    Err(e) => Some((Err(e), file)),
                }
            });
            (Body::from_stream(chunks), len)
        }
    };
    let name = q.path.rsplit('/').next().unwrap_or("download");
    let encoded: String = name.as_bytes().iter().map(|b| format!("%{b:02X}")).collect();
    Ok((
        [
            (header::CONTENT_LENGTH, len.to_string()),
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"download\"; filename*=UTF-8''{encoded}")),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
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

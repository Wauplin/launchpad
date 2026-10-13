use crate::{Server, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{get, post},
};
use launchpad_core::items;
use serde::Deserialize;
use serde_json::Value;
pub fn router() -> Router<Server> {
    Router::new()
        .route(
            "/api/items/{owner}/{repo}/{number}",
            get(detail).post(refresh),
        )
        .route("/api/items/{owner}/{repo}/{number}/{action}", post(action))
        .route("/api/items/{owner}/{repo}/{number}/diff/file", get(diff_file))
        .route(
            "/api/repo-items/{owner}/{repo}",
            get(list).post(list_refresh),
        )
        .route("/api/repo-meta/{owner}/{repo}", get(meta))
        .route("/img", get(image))
}
type ItemPath = (String, String, u64);
async fn detail(State(s): State<Server>, Path((o, r, n)): Path<ItemPath>) -> ApiResult<Value> {
    let repo = format!("{o}/{r}");
    if let Some(v) = items::cached(&s.app, format!("item:{repo}:{n}")).await? {
        return Ok(Json(v));
    }
    Ok(Json(items::detail(&s.app, &repo, n, false).await?))
}
#[derive(Deserialize)]
struct Refresh {
    #[serde(default = "default_force")]
    force: bool,
}
fn default_force() -> bool {
    true
}
async fn refresh(
    State(s): State<Server>,
    Path((o, r, n)): Path<ItemPath>,
    Json(q): Json<Refresh>,
) -> ApiResult<Value> {
    Ok(Json(
        items::detail(&s.app, &format!("{o}/{r}"), n, q.force).await?,
    ))
}
async fn action(
    State(s): State<Server>,
    Path((o, r, n, a)): Path<(String, String, u64, String)>,
    Json(body): Json<Value>,
) -> ApiResult<Value> {
    let repo = format!("{o}/{r}");
    if a == "diff" {
        return Ok(Json(items::diff_summary(&s.app, &repo, n).await?));
    }
    // Publication succeeds independently of subsequent cache refresh.
    let v = items::mutate(&s.app, &repo, n, &a, body).await?;

    Ok(Json(v))
}
async fn list(State(s): State<Server>, Path((o, r)): Path<(String, String)>) -> ApiResult<Value> {
    let repo = format!("{o}/{r}");
    if let Some(v) = items::cached(&s.app, format!("project-items:{repo}")).await?
        && v["prs"].is_array()
        && v["issues"].is_array()
    {
        return Ok(Json(v));
    }
    Ok(Json(items::project_items(&s.app, &repo, false).await?))
}
async fn meta(State(s): State<Server>, Path((o, r)): Path<(String, String)>) -> ApiResult<Value> {
    Ok(Json(items::repo_meta(&s.app, &format!("{o}/{r}")).await?))
}
async fn list_refresh(
    State(s): State<Server>,
    Path((o, r)): Path<(String, String)>,
) -> ApiResult<Value> {
    Ok(Json(
        items::project_items(&s.app, &format!("{o}/{r}"), true).await?,
    ))
}
#[derive(Deserialize)]
struct DiffFileQuery {
    base: String,
    head: String,
    project: Option<String>,
    path: String,
}
/// One file's patch of the cached PR diff (the `diff` action returns the file list only).
async fn diff_file(
    State(s): State<Server>,
    Path((o, r, n)): Path<ItemPath>,
    Query(q): Query<DiffFileQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        items::diff_file(&s.app, &format!("{o}/{r}"), n, &q.base, &q.head, q.project.as_deref(), &q.path).await?,
    ))
}
#[derive(Deserialize)]
struct ImageQuery {
    url: String,
}
async fn image(
    State(s): State<Server>,
    Query(q): Query<ImageQuery>,
) -> Result<impl IntoResponse, crate::ApiError> {
    let (bytes, mime) = items::image(&s.app, &q.url).await?;
    Ok((
        [
            ("content-type", mime),
            ("cache-control", "private, max-age=86400".into()),
            (
                "content-security-policy",
                "default-src 'none'; sandbox".into(),
            ),
            ("x-content-type-options", "nosniff".into()),
        ],
        bytes,
    ))
}

use crate::{Ctx, error::ApiResult};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use launchpad_github::ledger;
use launchpad_github::suggestions::{self, Create, Edit};
use serde_json::{Value, json};
pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/suggestions", get(list).post(create))
        .route(
            "/api/suggestions/{id}",
            axum::routing::patch(edit).delete(discard),
        )
        .route("/api/suggestions/{id}/post", post(publish))
        .route("/api/suggestions/ledger", get(ledger))
        .route("/api/suggestions/{id}/shown", post(shown))
        .route("/api/suggestions/{id}/inserted", post(inserted))
        .route("/api/suggestions/{id}/posted-by-hand", post(posted_by_hand))
        .route("/api/suggestions/{id}/dismiss-comment", post(dismiss_comment))
        .route("/api/notes", get(notes).post(write_note))
        .route("/api/notes/{id}", axum::routing::delete(delete_note))
}
#[derive(serde::Deserialize)]
struct NotesQuery {
    repo: String,
    number: u64,
}
/// The private notes about one GitHub item (`GET /api/notes?repo=&number=`, the owner's).
async fn notes(State(s): State<Ctx>, axum::extract::Query(q): axum::extract::Query<NotesQuery>) -> ApiResult<Value> {
    launchpad_github::items::validate(&q.repo)?;
    Ok(Json(json!(launchpad_github::notes::of_item(&s.app, &q.repo, q.number).await?)))
}
/// `launchpad note`: an agent's own write (`agent_auth` checks the body's `session_id`).
async fn write_note(State(s): State<Ctx>, Json(q): Json<launchpad_github::notes::Write>) -> ApiResult<Value> {
    Ok(Json(json!(launchpad_github::notes::write(&s.app, q).await?)))
}
/// `note.delete`: the owner's.
async fn delete_note(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    launchpad_github::notes::delete(&s.app, &id).await?;
    Ok(Json(json!({"ok": true})))
}
/// The part of a suggestion a ledger action is about: `body` (the default) or `line:<path>:<line>:<side>`.
#[derive(serde::Deserialize)]
struct Part {
    #[serde(default = "body")]
    part: String,
    #[serde(default)]
    text: Option<String>,
}
fn body() -> String {
    "body".into()
}
#[derive(serde::Deserialize)]
struct LedgerQuery {
    suggestion: Option<String>,
}
async fn ledger(State(s): State<Ctx>, axum::extract::Query(q): axum::extract::Query<LedgerQuery>) -> ApiResult<Value> {
    Ok(Json(json!(ledger::list(&s.app, q.suggestion.as_deref()).await?)))
}
async fn shown(State(s): State<Ctx>, Path(id): Path<String>, Json(q): Json<Part>) -> ApiResult<Value> {
    ledger::shown(&s.app, &id, &q.part).await?;
    Ok(Json(json!({"ok": true})))
}
async fn inserted(State(s): State<Ctx>, Path(id): Path<String>, Json(q): Json<Part>) -> ApiResult<Value> {
    Ok(Json(json!(ledger::inserted(&s.app, &id, &q.part, q.text).await?)))
}
async fn posted_by_hand(State(s): State<Ctx>, Path(id): Path<String>, Json(q): Json<Part>) -> ApiResult<Value> {
    let text = q.text.ok_or_else(|| crate::ApiError::bad_request("text: the final text that was posted"))?;
    ledger::posted_by_hand(&s.app, &id, &q.part, text).await?;
    Ok(Json(json!({"ok": true})))
}
async fn dismiss_comment(State(s): State<Ctx>, Path(id): Path<String>, Json(q): Json<Part>) -> ApiResult<Value> {
    ledger::dismiss_comment(&s.app, &id, &q.part).await?;
    Ok(Json(json!({"ok": true})))
}
async fn list(State(s): State<Ctx>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::list(&s.app).await?)))
}
async fn create(State(s): State<Ctx>, Json(q): Json<Create>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::create(&s.app, q).await?)))
}
async fn edit(
    State(s): State<Ctx>,
    Path(id): Path<String>,
    Json(q): Json<Edit>,
) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::edit(&s.app, &id, q).await?)))
}
async fn discard(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    suggestions::discard(&s.app, &id).await?;
    Ok(Json(json!({"ok":true})))
}
async fn publish(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!(suggestions::post(&s.app, &id).await?)))
}

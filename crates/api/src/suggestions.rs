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
        .route("/api/notes/owner", post(write_owner_note))
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
/// `note.write`: the owner's private note on an item's timeline, then a message to each agent that wrote about the
/// item in the workspace (its bot's thread of the item, else its session), else to the item's default bot. The owner's
/// own click: an owner message (no sender header, no hop). A delivery that fails is reported, never fails the note.
async fn write_owner_note(State(s): State<Ctx>, Json(q): Json<launchpad_github::notes::OwnerWrite>) -> ApiResult<Value> {
    use launchpad_core::{bot_threads, bots, model::{GitHubRef, MessageContext}};
    use launchpad_github::notes::{self, Recipient};
    let app = &s.app;
    let note = notes::write_owner(app, &q).await?;
    let cached = launchpad_github::items::cached(app, format!("item:{}:{}", q.repo, q.number)).await.ok().flatten();
    let pr = cached.as_ref().is_some_and(|v| v["headRefOid"].is_string());
    let url = format!("https://github.com/{}/{}/{}", q.repo, if pr { "pull" } else { "issues" }, q.number);
    let title = cached.as_ref().and_then(|v| v["title"].as_str()).map(str::to_string);
    let refs = vec![GitHubRef { url: url.clone(), kind: if pr { "pr" } else { "issue" }.into(), repo: q.repo.clone(), number: Some(q.number), title }];
    let text = notes::owner_message(app, &note, &url);
    let mut sent = vec![];
    for r in notes::recipients(app, &q.repo, q.number, &q.workspace).await.unwrap_or_default() {
        let context = MessageContext { github_refs: refs.clone(), ..Default::default() };
        let result = match &r {
            Recipient::Bot { id, .. } => {
                let to = bot_threads::route(bot_threads::Source::Items { refs: &refs });
                bots::message_with(app, id, bots::Message { text: text.clone(), attachments: vec![], context, new_session: None, automatic: false, key: None, to }).await.map(|_| ())
            }
            Recipient::Session { id, .. } => match app.sessions.get(id) {
                Some(x) if x.status != launchpad_core::model::SessionStatus::Closed => launchpad_core::runners::send_anywhere_with(app, id, &text, vec![], context, None).await,
                Some(_) => Err(anyhow::anyhow!("archived")),
                None => Err(anyhow::anyhow!("session not found")),
            },
        };
        let error = result.err().map(|e| format!("{e:#}"));
        if let Some(e) = &error {
            tracing::warn!("Owner note {}: message to {r:?}: {e}", note.id);
        }
        sent.push(json!({"recipient": r, "error": error}));
    }
    Ok(Json(json!({"note": note, "sent": sent})))
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

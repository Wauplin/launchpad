//! Bots on Home (`launchpad_core::bots`): files, settings and the thread. Runners forward these requests to Home.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use launchpad_core::ask_bot;
use launchpad_core::bot_threads::{self, BotThread, Source};
use launchpad_core::bots::{self, Bot, Create, Patch};
use launchpad_core::model::{FileAttachment, LaunchSpec, MessageContext, Target};
use launchpad_core::start::{self, StartPreview};
use launchpad_channels::sends;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/bots", get(list).post(create))
        .route("/api/bots/{id}", get(one).patch(update).delete(remove))
        .route("/api/bots/{id}/message", post(message))
        .route("/api/bots/{id}/sends", get(sends))
        .route("/api/bots/{id}/sends/{send}", axum::routing::delete(remove_send))
        .route("/api/bots/{id}/sends/{send}/retry", post(retry_send))
        .route("/api/bots/{id}/threads", get(threads))
        .route("/api/context/conversation", get(conversation))
        .route("/api/bots/{id}/threads/{thread}", axum::routing::patch(rename_thread))
        .route("/api/bots/{id}/threads/{thread}/auto-title", post(auto_title_thread))
        .route("/api/bots/{id}/threads/{thread}/archive", post(archive_thread))
        .route("/api/bots/{id}/history", get(history))
        .route("/api/bots/{id}/file", get(file))
        .route("/api/bots/{id}/history/{change}", get(diff))
        .route("/api/bots/{id}/restore/{change}", post(restore))
        .route("/api/bots/{id}/routines", post(create_routine))
        .route("/api/bots/{id}/pause", post(pause))
        .route("/api/bots/templates", get(templates))
        .route("/api/bots/{id}/start-preview", get(start_preview))
        .route("/api/launch/start-preview", post(launch_start_preview).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThreadQuery {
    /// The thread (None: a new thread).
    thread: Option<String>,
    /// Archived threads too.
    archived: bool,
}

/// "What it sees at start" of a new thread of the bot, or of the thread that would continue `?thread=` (its
/// session referenced): its instructions and first prompt, in labelled parts (its next message stands for the typed text).
async fn start_preview(State(s): State<Ctx>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<ThreadQuery>) -> ApiResult<StartPreview> {
    let app = &s.app;
    let bot = bots::get(app, &id).await.map_err(bad)?;
    let previous = q.thread.as_deref().and_then(|t| bot_threads::session(app, &id, t));
    let spec = bots::next_spec(app, &bot, "", previous.as_ref(), "(your next message)".into(), vec![], Default::default()).await.map_err(bad)?;
    Ok(Json(preview(app, &spec).await.map_err(bad)?))
}

/// "What it sees at start" of a launch spec, without starting anything.
async fn launch_start_preview(State(s): State<Ctx>, Json(spec): Json<LaunchSpec>) -> ApiResult<StartPreview> {
    Ok(Json(preview(&s.app, &spec).await.map_err(bad)?))
}

/// What a session of this launch would see at start, built from Home's records and stores with the functions of a
/// real start (`contain::apply` for the instructions, `Sessions::preview` for the prompt, `launchpad_core::start`).
async fn preview(app: &launchpad_core::App, spec: &LaunchSpec) -> anyhow::Result<StartPreview> {
    use anyhow::Context;
    use launchpad_core::{attribution, bot_memory, kb, launchpad_cli, plain, projects, vault};
    let harness = spec.harness.context("pick a profile")?;
    let mut prompt = app.sessions.preview(app, spec).await?.parts;
    let project_id = match &spec.target {
        Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.clone()),
        _ => None,
    };
    // The workspace of its lease (`contain::request`): its folder's, else the launch's.
    let folder_ws = match &project_id {
        Some(p) => projects::get(app, p).await?.and_then(|p| p.workspace_id),
        None => None,
    };
    let workspace_id = folder_ws.or_else(|| spec.workspace_id.clone());
    let cli = app.paths.cli_command();
    // What a launch adds after the prompt once the session has its id and folder (`Sessions::launch`), with
    // `$LAUNCHPAD_SESSION_ID` for its id (worktrees are named in the prompt above; attachments are left out).
    const SESSION: &str = "$LAUNCHPAD_SESSION_ID";
    if let (Target::Temp, Some(ws)) = (&spec.target, &spec.workspace_id) {
        prompt.push(start::part("temp", "Temporary folder", launchpad_core::sessions::temp_instruction(&cli, SESSION, ws, "<its temporary folder>")));
    }
    if let Some(text) = launchpad_core::ext::extensions().suggestion_instruction(app, SESSION)
    {
        prompt.push(start::part("publish", "GitHub suggestions and connections", text));
    }
    let kb = match &workspace_id {
        Some(ws) => kb::instructions_on_home(app, ws).await,
        None => start::NO_KB.into(),
    };
    let memory = match &spec.bot {
        Some(b) => {
            let tree = plain::tree(app, &bots::store(&b.id)).await?;
            bot_memory::compose(&bot_memory::store_notes(app, &tree.files).await, &cli)
        }
        None => Default::default(),
    };
    let req = vault::LeaseRequest {
        session_id: String::new(),
        generation: 0,
        harness,
        login: None,
        workspace_id: workspace_id.clone(),
        tools_off: spec.tools_off.clone(),
        tools_on: spec.tools_on.clone(),
        project_id,
        bot_id: spec.bot.as_ref().map(|b| b.id.clone()),
    };
    let cli_reference = vault::launchpad_access(app, &req).await?.map(|a| launchpad_cli::cli_reference(&cli, workspace_id.as_deref(), a));
    let attribution = attribution::instruction(&attribution::text(spec.bot.as_ref().map(|b| b.name.as_str()), harness.display()));
    Ok(StartPreview { instructions: start::instruction_parts(start::Inputs { kb, memory, cli_reference, attribution }), prompt })
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Ctx>) -> ApiResult<Vec<Bot>> {
    Ok(Json(bots::list(&s.app).await.map_err(bad)?))
}
/// An agent's new bot gets at most the default Launchpad CLI access (propose): only the owner gives more.
async fn create(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(c): Json<Create>) -> ApiResult<Bot> {
    if agent.is_some() && c.launchpad > launchpad_core::vault::LaunchpadAccess::default() {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "only the owner gives a bot more than propose access (`launchpad` in bot.toml); create it without `launchpad`"));
    }
    Ok(Json(bots::create(&s.app, c).await.map_err(bad)?))
}
async fn one(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Bot> {
    Ok(Json(bots::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}
/// An agent's change never raises the bot's access nor loosens its permission mode
/// (`bots::agent_refusal`).
async fn update(State(s): State<Ctx>, Path(id): Path<String>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(mut p): Json<Patch>) -> ApiResult<Bot> {
    p.by_agent = agent.is_some();
    // Routine files are checked before they are saved (a file with an error would keep its routine's old definition).
    if let Some(files) = &p.files {
        let bot = bots::get(&s.app, &id).await.map_err(bad)?;
        for (path, text) in files {
            if let Some(t) = text {
                launchpad_automation::crons::check_file(&s.app, &bot, t).await.map_err(|e| bad(e.context(path.clone())))?;
            }
        }
    }
    Ok(Json(bots::update(&s.app, &id, p).await.map_err(bad)?))
}
async fn remove(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    bots::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachments: Vec<FileAttachment>,
    /// Folders, GitHub links and agent traces, as in an agent's message box.
    #[serde(default)]
    context: MessageContext,
    /// The owner's thread it goes to (the one the bot page shows); None: a new thread. Agents always write in their own.
    #[serde(default)]
    thread: Option<String>,
    /// A new thread (New thread on the bot page, New conversation in the extension).
    #[serde(default)]
    new_thread: bool,
    /// The thread of the GitHub item the context names (the extension: the same item continues its conversation,
    /// other items run in parallel); several items start a new thread.
    #[serde(default)]
    item_thread: bool,
    /// A conversation (`<main bot>:<thread>`, `BotThread.conversation`): the owner's message goes to this bot's
    /// thread of it, and a bot that is not part of it yet joins it.
    #[serde(default)]
    conversation: Option<String>,
    /// The owner's message: answer with its receipt (`sends::BotSend`) as soon as Home stored it, instead of the
    /// session once it was sent (the bot chat; the extension waits for the session).
    #[serde(default)]
    accept: bool,
}

/// The owner's message, or an agent's (`launchpad bot message`): Home stamps the agent as its sender from its own
/// record (`senders::agent`), never from the body, and a paused bot refuses it like a routine's. A window's
/// Idempotency-Key (claimed by the server's retry protection) becomes the message's or the new session's id.
/// The owner's message is stored on Home first and sent in the background (`launchpad_channels::sends`): with
/// `accept` the answer is its receipt at once, else the session once it was sent.
/// The owner's message goes to the thread they name (`thread`, `new_thread`, `item_thread`, `conversation`); an
/// agent's to its own thread of the bot (one per sender session), or, from a bot's thread, to the bot's thread of the
/// sender's conversation (`bot_threads::joins`), queued while the bot works in as many threads as `max_threads`
/// allows (`{"queued": true, …}` instead of the session).
async fn message(State(s): State<Ctx>, Path(id): Path<String>, headers: axum::http::HeaderMap, agent: Option<axum::Extension<launchpad_http::AgentSession>>, Json(mut m): Json<Message>) -> ApiResult<Value> {
    let app = &s.app;
    m.context.from = launchpad_core::senders::of_request(app, agent.as_ref().map(|a| a.0.0.as_str())).await.map_err(bad)?;
    let key = headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let sender_thread = match &agent {
        Some(a) => match app.sessions.get(&a.0.0) {
            Some(sender) => bot_threads::of_session(app, &sender).await,
            None => None,
        },
        None => None,
    };
    let joined = bot_threads::joins(sender_thread.as_ref(), &id);
    let conversation = match (&agent, m.conversation.as_deref().filter(|c| !c.is_empty())) {
        (None, Some(c)) => Some(checked_conversation(app, &id, c).await.map_err(bad)?),
        _ => None,
    };
    let source = match &agent {
        Some(a) => Source::Agent { session: a.0.0.as_str(), name: m.context.from.as_ref().map_or("", |f| f.name.as_str()), conversation: joined.as_deref() },
        None if conversation.is_some() => Source::Conversation { id: conversation.as_deref().unwrap_or_default(), title: "" },
        None if m.new_thread => Source::NewThread,
        None if m.item_thread => Source::Items { refs: &m.context.github_refs },
        None => Source::Owner { thread: m.thread.as_deref() },
    };
    let to = bot_threads::route(source);
    if agent.is_none() {
        let sent = sends::accept(app, &id, to, m.text, m.attachments, m.context, key).await.map_err(bad)?;
        if m.accept {
            return Ok(Json(serde_json::to_value(sent).map_err(|e| bad(e.into()))?));
        }
        let session = sends::wait(app, &sent.id).await.map_err(bad)?;
        return Ok(Json(serde_json::to_value(session).map_err(|e| bad(e.into()))?));
    }
    // An agent's message: automatic, sent now (or queued for a free slot).
    // The loop guard refuses it now rather than once a slot is free.
    if let Some(f) = &m.context.from {
        launchpad_core::senders::guard_from(f).map_err(bad)?;
    }
    let bot = bots::get(app, &id).await.map_err(bad)?;
    if !bot.paused && bot_threads::at_cap(app, &bot, bot_threads::peek(app, &id, &to).await.map_err(bad)?.as_deref()).is_some() {
        let thread = bot_threads::resolve(app, &id, &to, m.text.lines().next().unwrap_or_default()).await.map_err(bad)?;
        launchpad_channels::hooks::queue_for_slot(app, &id, thread.clone(), m.text, m.attachments, m.context).await.map_err(bad)?;
        return Ok(Json(json!({"queued": true, "thread": thread, "status": launchpad_channels::hooks::QUEUED_SLOT})));
    }
    let msg = bots::Message { text: m.text, attachments: m.attachments, context: m.context, automatic: true, key, to };
    let session = bots::message_with(app, &id, msg).await.map_err(bad)?;
    Ok(Json(serde_json::to_value(session).map_err(|e| bad(e.into()))?))
}

/// A conversation the owner's message to bot `bot` may name: one of a bot of the same workspace, with an open
/// thread of its main bot.
async fn checked_conversation(app: &launchpad_core::App, bot: &str, conversation: &str) -> anyhow::Result<String> {
    use anyhow::{Context, ensure};
    let threads = bot_threads::conversation_threads(app, conversation).await?;
    let main = bot_threads::main_bot(conversation);
    ensure!(threads.iter().any(|t| t.bot_id == main && t.archived_at.is_none()), "this conversation is archived or gone");
    let ws = bots::workspace_of(app, bot).await?.context("no such bot")?;
    ensure!(bots::workspace_of(app, main).await?.as_deref() == Some(ws.as_str()), "this conversation belongs to another workspace");
    Ok(conversation.to_string())
}

/// The owner's messages Home accepted for the bot and has not shown in a session yet (waiting, sending, failed,
/// uncertain, or sent in the last half hour), oldest first.
async fn sends(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<sends::BotSend>> {
    Ok(Json(sends::list(&s.app, &id).await.map_err(bad)?))
}

/// Retry: a failed or uncertain message is sent again, with the same ID.
async fn retry_send(State(s): State<Ctx>, Path((id, send)): Path<(String, String)>) -> ApiResult<sends::BotSend> {
    Ok(Json(sends::retry(&s.app, &id, &send).await.map_err(bad)?))
}

/// Remove: a message not sent (waiting, failed or uncertain) is deleted and never sent.
async fn remove_send(State(s): State<Ctx>, Path((id, send)): Path<(String, String)>) -> ApiResult<Value> {
    sends::remove(&s.app, &id, &send).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// The bot's threads, newest first (archived ones with `?archived=true`).
async fn threads(State(s): State<Ctx>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<ThreadQuery>) -> ApiResult<Vec<BotThread>> {
    bots::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(bot_threads::list(&s.app, &id, q.archived).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct ConversationQuery {
    /// The owner's read: a session of the conversation (an agent reads its own).
    session: Option<String>,
    /// Only messages after this one (`ConversationMessage.id`).
    since: Option<i64>,
    limit: Option<usize>,
}

/// `launchpad conversation read`: the conversation of the caller's bot thread (the owner: of `session`), its newest
/// messages after `since`. An agent reads only its own session's conversation.
async fn conversation(State(s): State<Ctx>, agent: Option<axum::Extension<launchpad_http::AgentSession>>, axum::extract::Query(q): axum::extract::Query<ConversationQuery>) -> ApiResult<launchpad_channels::conversation::Conversation> {
    let session = match agent {
        Some(a) => a.0.0,
        None => q.session.ok_or_else(|| ApiError::bad_request("name the session (?session=)"))?,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(launchpad_channels::conversation::read(&s.app, &session, q.since, limit).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Rename {
    title: String,
}

async fn rename_thread(State(s): State<Ctx>, Path((id, thread)): Path<(String, String)>, Json(r): Json<Rename>) -> ApiResult<BotThread> {
    bots::get(&s.app, &id).await.map_err(bad)?;
    Ok(Json(bot_threads::rename(&s.app, &id, &thread, &r.title).await.map_err(bad)?))
}

async fn auto_title_thread(State(s): State<Ctx>, Path((id, thread)): Path<(String, String)>) -> ApiResult<BotThread> {
    bots::get(&s.app, &id).await.map_err(bad)?;
    Ok(Json(bot_threads::auto_title(&s.app, &id, &thread).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Archive {
    #[serde(default = "yes")]
    archived: bool,
}

fn yes() -> bool {
    true
}

async fn archive_thread(State(s): State<Ctx>, Path((id, thread)): Path<(String, String)>, Json(a): Json<Archive>) -> ApiResult<BotThread> {
    bots::get(&s.app, &id).await.map_err(bad)?;
    Ok(Json(bot_threads::archive(&s.app, &id, &thread, a.archived).await.map_err(bad)?))
}
#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

/// One of the bot's files as text (its memory notes, BOT.md, bot.toml), never rendered as a document.
async fn file(State(s): State<Ctx>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<FileQuery>) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    let data = bots::read_file(&s.app, &id, &q.path).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], data).into_response())
}

async fn history(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Vec<launchpad_core::plain::Change>> {
    Ok(Json(bots::history(&s.app, &id).await.map_err(bad)?))
}
async fn diff(State(s): State<Ctx>, Path((id, change)): Path<(String, i64)>) -> ApiResult<Value> {
    Ok(Json(json!({"patch": bots::diff(&s.app, &id, change).await.map_err(bad)?})))
}
async fn restore(State(s): State<Ctx>, Path((id, change)): Path<(String, i64)>, agent: Option<axum::Extension<launchpad_http::AgentSession>>) -> ApiResult<Bot> {
    Ok(Json(bots::restore(&s.app, &id, change, agent.is_some()).await.map_err(bad)?))
}

async fn create_routine(State(s): State<Ctx>, Path(id): Path<String>, Json(r): Json<launchpad_automation::crons::BotRoutine>) -> ApiResult<launchpad_automation::crons::CronAgent> {
    Ok(Json(launchpad_automation::crons::create_for_bot(&s.app, &id, r).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Pause {
    paused: bool,
}

async fn pause(State(s): State<Ctx>, Path(id): Path<String>, Json(p): Json<Pause>) -> ApiResult<Bot> {
    Ok(Json(bots::set_paused(&s.app, &id, p.paused).await.map_err(bad)?))
}

/// Ready-made bots the New bot form starts from (Ask Launchpad and the Librarian, to recreate or adapt them).
async fn templates() -> Json<Value> {
    use launchpad_core::librarian;
    Json(json!([
        { "id": "ask-launchpad", "name": ask_bot::NAME, "instructions": ask_bot::DEFAULT_INSTRUCTIONS, "starters": ask_bot::STARTERS },
        { "id": "librarian", "name": librarian::NAME, "instructions": librarian::DEFAULT_INSTRUCTIONS, "starters": librarian::STARTERS },
    ]))
}

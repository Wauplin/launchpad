//! Bot channels and DMs on Home (`launchpad_channels::{channel, delivery}`). The owner's window reads and posts here.
//! Agents use `/api/channels/agent…` (`launchpad channel list|read|post`, `launchpad dm`, `launchpad_channels::agent`):
//! Home takes the caller from its own record of the CLI token's session (`AgentSession`), never from the body, and
//! its bot posts; runners forward every channel call to Home (`home_client::home_owned`).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use launchpad_channels::agent;
use launchpad_channels::channel::{self, Channel, ChannelDelivery, ChannelPage, ChannelPosted, Edit, Mode, NewChannel, Post, Poster};
use launchpad_http::AgentSession;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/channels", get(list).post(create))
        .route("/api/channels/dm", post(dm))
        .route("/api/channels/agent", get(agent_list))
        .route("/api/channels/agent/read", get(agent_read))
        .route("/api/channels/agent/post", post(agent_post))
        .route("/api/channels/{id}", get(one).patch(edit).delete(remove))
        .route("/api/channels/{id}/messages", get(messages).post(post_message))
        .route("/api/channels/{id}/messages/{message}/deliveries", get(deliveries))
        .route("/api/channels/{id}/read", post(read))
        .route("/api/channels/{id}/pause", post(pause))
        .route("/api/channels/{id}/block", post(block))
        .route("/api/channels/{id}/archive", post(archive))
        .route("/api/channels/{id}/allow-more", post(allow_more))
        .route("/api/channels/{id}/digest", post(digest))
        .route("/api/channels/{id}/members/{bot}", get(member).put(set_member))
        .route("/api/channels/{id}/members/{bot}/remove", post(remove_member))
        .route("/api/channels/{id}/members/{bot}/mode", axum::routing::put(set_mode))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

/// Who created a record: the owner, or the agent session.
fn by(agent: &Option<Extension<AgentSession>>) -> String {
    agent.as_ref().map_or("owner".into(), |a| format!("session:{}", a.0.0))
}

#[derive(Deserialize)]
struct ListQuery {
    workspace: Option<String>,
}

async fn list(State(s): State<Ctx>, Query(q): Query<ListQuery>) -> ApiResult<Vec<Channel>> {
    Ok(Json(channel::list(&s.app, q.workspace.as_deref()).await.map_err(bad)?))
}

async fn create(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>, Json(n): Json<NewChannel>) -> ApiResult<Channel> {
    Ok(Json(channel::create(&s.app, n, &by(&agent)).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Dm {
    bots: [String; 2],
}

/// The DM of two bots of one workspace, created on first use.
async fn dm(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>, Json(d): Json<Dm>) -> ApiResult<Channel> {
    Ok(Json(channel::dm(&s.app, &d.bots[0], &d.bots[1], &by(&agent)).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct PageQuery {
    before: Option<i64>,
    since: Option<i64>,
    limit: Option<u32>,
}

/// The channel with its newest messages (oldest first; `before` pages back) and their deliveries.
async fn one(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<PageQuery>) -> ApiResult<ChannelPage> {
    Ok(Json(channel::page(&s.app, &id, q.before, q.limit).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?))
}

/// Messages after `since` (oldest first), or the newest before `before`.
async fn messages(State(s): State<Ctx>, Path(id): Path<String>, Query(q): Query<PageQuery>) -> ApiResult<Value> {
    channel::get(&s.app, &id).await.map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    let (list, more) = channel::messages(&s.app, &id, q.since, q.before, q.limit).await.map_err(bad)?;
    Ok(Json(json!({"messages": list, "more": more})))
}

/// The owner's message (agents post with `launchpad channel post`, `/api/channels/agent/post`).
async fn post_message(State(s): State<Ctx>, Path(id): Path<String>, agent: Option<Extension<AgentSession>>, Json(p): Json<Post>) -> ApiResult<ChannelPosted> {
    if agent.is_some() {
        return Err(ApiError::bad_request("bots post with `launchpad channel post <name> --body-file <file>`"));
    }
    Ok(Json(channel::post(&s.app, &id, Poster::Owner, p).await.map_err(bad)?))
}

/// The session of an agent's CLI call (Home's record), or a refusal for the owner's window.
fn agent_of(agent: Option<Extension<AgentSession>>) -> Result<String, ApiError> {
    agent.map(|a| a.0.0).ok_or_else(|| ApiError::bad_request("these routes are an agent's (`launchpad channel …`); the owner's window uses /api/channels/{id}"))
}

async fn agent_list(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>) -> ApiResult<Value> {
    let session = agent_of(agent)?;
    Ok(Json(agent::list(&s.app, &session).await.map_err(forbidden)?))
}

#[derive(Deserialize)]
struct AgentRead {
    channel: String,
    since: Option<i64>,
    limit: Option<u32>,
}

async fn agent_read(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>, Query(q): Query<AgentRead>) -> ApiResult<Value> {
    let session = agent_of(agent)?;
    Ok(Json(agent::read(&s.app, &session, &q.channel, q.since, q.limit).await.map_err(forbidden)?))
}

#[derive(Deserialize)]
struct AgentPost {
    channel: String,
    #[serde(flatten)]
    post: Post,
}

async fn agent_post(State(s): State<Ctx>, agent: Option<Extension<AgentSession>>, Json(p): Json<AgentPost>) -> ApiResult<Value> {
    let session = agent_of(agent)?;
    Ok(Json(agent::post(&s.app, &session, &p.channel, p.post).await.map_err(forbidden)?))
}

/// An agent's refusal (not a bot, not a member, another workspace, blocked) or a bad request.
fn forbidden(e: anyhow::Error) -> ApiError {
    ApiError::new(axum::http::StatusCode::FORBIDDEN, format!("{e:#}"))
}

async fn deliveries(State(s): State<Ctx>, Path((id, message)): Path<(String, i64)>) -> ApiResult<Vec<ChannelDelivery>> {
    // Only a message of this channel (the agent boundary checked the channel, not the message).
    if !channel::has_message(&s.app, &id, message).await.map_err(bad)? {
        return Err(ApiError::not_found(format!("message {message} is not in this channel")));
    }
    let list = channel::deliveries(&s.app, vec![message]).await.map_err(bad)?;
    Ok(Json(list))
}

#[derive(Deserialize, Default)]
struct Read {
    #[serde(default)]
    through: Option<i64>,
}

/// The owner read the channel (up to `through`, else its newest message).
async fn read(State(s): State<Ctx>, Path(id): Path<String>, body: Option<Json<Read>>) -> ApiResult<Channel> {
    let through = body.and_then(|b| b.0.through);
    Ok(Json(channel::mark_read(&s.app, &id, through).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Paused {
    paused: bool,
}
async fn pause(State(s): State<Ctx>, Path(id): Path<String>, Json(p): Json<Paused>) -> ApiResult<Channel> {
    Ok(Json(channel::pause(&s.app, &id, p.paused).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Blocked {
    blocked: bool,
}
async fn block(State(s): State<Ctx>, Path(id): Path<String>, Json(b): Json<Blocked>) -> ApiResult<Channel> {
    Ok(Json(channel::block(&s.app, &id, b.blocked).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct Archived {
    archived: bool,
}
async fn archive(State(s): State<Ctx>, Path(id): Path<String>, Json(a): Json<Archived>) -> ApiResult<Channel> {
    Ok(Json(channel::archive(&s.app, &id, a.archived).await.map_err(bad)?))
}

async fn allow_more(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<ChannelPage> {
    Ok(Json(channel::allow_more(&s.app, &id).await.map_err(bad)?))
}

async fn digest(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<ChannelPage> {
    Ok(Json(launchpad_channels::digest::now(&s.app, &id).await.map_err(bad)?))
}

async fn edit(State(s): State<Ctx>, Path(id): Path<String>, Json(e): Json<Edit>) -> ApiResult<Channel> {
    Ok(Json(channel::edit(&s.app, &id, e).await.map_err(bad)?))
}

async fn remove(State(s): State<Ctx>, Path(id): Path<String>) -> ApiResult<Value> {
    channel::delete(&s.app, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// A bot's membership (`{"mode": null}` when it is not a member): what an admin change's Undo puts back.
async fn member(State(s): State<Ctx>, Path((id, bot)): Path<(String, String)>) -> ApiResult<Value> {
    Ok(Json(json!({"mode": channel::member(&s.app, &id, &bot).await.map_err(bad)?})))
}

#[derive(Deserialize)]
struct SetMember {
    /// Required; null removes the bot (the Undo of an add).
    mode: Value,
}

async fn set_member(State(s): State<Ctx>, Path((id, bot)): Path<(String, String)>, Json(m): Json<SetMember>) -> ApiResult<Channel> {
    let mode: Option<Mode> = serde_json::from_value(m.mode).map_err(|_| ApiError::bad_request("mode is all, mentions, digest, muted or null"))?;
    Ok(Json(channel::set_member(&s.app, &id, &bot, mode).await.map_err(bad)?))
}

async fn remove_member(State(s): State<Ctx>, Path((id, bot)): Path<(String, String)>) -> ApiResult<Channel> {
    Ok(Json(channel::set_member(&s.app, &id, &bot, None).await.map_err(bad)?))
}

#[derive(Deserialize)]
struct SetMode {
    mode: Mode,
}

/// A member bot's new mode (it must be a member already).
async fn set_mode(State(s): State<Ctx>, Path((id, bot)): Path<(String, String)>, Json(m): Json<SetMode>) -> ApiResult<Channel> {
    Ok(Json(channel::set_mode(&s.app, &id, &bot, m.mode).await.map_err(bad)?))
}

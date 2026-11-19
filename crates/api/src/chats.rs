//! The Chats list's routes, Home-owned and owner-only (agents never reach them):
//! - `GET /api/chat-threads`: the threads of a workspace's bots the list shows (open ones, and a page of archived
//!   conversations, `bot_threads::for_chats`), in one request instead of one full list per bot.
//! - `GET /api/chat-search`: the channels and DMs of a workspace whose messages contain a text (bot chats search their
//!   messages through `/api/sessions/search`).
//! - Pinned chats (`chat.pin`): the owner's shortcuts to a conversation of bots (`conversation:<id>`, which a follow-up
//!   thread keeps) or a bot channel (`channel:<id>`), shown in the laptop sidebar and first on the phone's Chats list.
//!   A pin names a chat of its workspace; deleting the bot that started the conversation, the channel or the workspace
//!   deletes its pins.
use crate::{
    Ctx,
    error::{ApiError, ApiResult},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use launchpad_core::util::now_ms;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn router() -> Router<Ctx> {
    Router::new()
        .route("/api/chat-pins", get(list).post(set))
        .route("/api/chat-threads", get(threads))
        .route("/api/chat-search", get(search))
}

#[derive(Deserialize)]
struct ThreadsQuery {
    workspace: String,
    /// Archived conversations to include, newest first (30 by default).
    archived: Option<usize>,
}

#[derive(Serialize)]
struct ChatThreads {
    threads: Vec<launchpad_core::bot_threads::BotThread>,
    /// Archived conversations in all (the list offers Show more while it holds fewer).
    archived_total: usize,
}

async fn threads(State(s): State<Ctx>, Query(q): Query<ThreadsQuery>) -> ApiResult<ChatThreads> {
    let bots: Vec<String> = launchpad_core::bots::list(&s.app).await?.into_iter().filter(|b| b.workspace_id == q.workspace).map(|b| b.id).collect();
    let (threads, archived_total) = launchpad_core::bot_threads::for_chats(&s.app, &bots, q.archived.unwrap_or(30).min(5000)).await?;
    Ok(Json(ChatThreads { threads, archived_total }))
}

#[derive(Deserialize)]
struct SearchQuery {
    workspace: String,
    q: String,
}

/// Channel and DM IDs of the workspace with a message containing `q` (case-insensitive for ASCII), at most 200.
async fn search(State(s): State<Ctx>, Query(q): Query<SearchQuery>) -> ApiResult<Vec<String>> {
    let needle = q.q.trim().to_string();
    if needle.is_empty() {
        return Ok(Json(vec![]));
    }
    let ws = q.workspace;
    let ids = s
        .app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT DISTINCT m.channel_id FROM channel_message m JOIN channel ch ON ch.id = m.channel_id WHERE ch.workspace_id = ?1 AND instr(lower(m.text), lower(?2)) > 0 LIMIT 200")?;
            let rows = st.query_map(params![ws, needle], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(ApiError::from)?;
    Ok(Json(ids))
}

/// The workspace a pin target belongs to: the conversation's main bot's, or the channel's.
async fn target_workspace(s: &Ctx, target: &str) -> Option<String> {
    if let Some(rest) = target.strip_prefix("conversation:") {
        let bot = rest.split(':').next()?;
        return launchpad_core::bots::get(&s.app, bot).await.ok().map(|b| b.workspace_id);
    }
    let id = target.strip_prefix("channel:")?;
    launchpad_channels::channel::get(&s.app, id).await.ok().map(|c| c.workspace_id)
}

#[derive(Serialize)]
struct Pin {
    target: String,
    workspace_id: String,
    at: i64,
}

#[derive(Deserialize)]
struct Workspace {
    workspace: String,
}

#[derive(Deserialize)]
struct Set {
    workspace_id: String,
    target: String,
    pinned: bool,
}

/// `conversation:<bot>:<thread>` or `channel:<id>`, of IDs made of the characters Launchpad's IDs use.
pub(crate) fn valid_target(target: &str) -> bool {
    let id_ok = |s: &str| !s.is_empty() && s.len() <= 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if let Some(rest) = target.strip_prefix("conversation:") {
        let mut parts = rest.splitn(2, ':');
        return matches!((parts.next(), parts.next()), (Some(a), Some(b)) if id_ok(a) && id_ok(b));
    }
    target.strip_prefix("channel:").is_some_and(id_ok)
}

async fn list(State(s): State<Ctx>, Query(q): Query<Workspace>) -> ApiResult<Vec<Pin>> {
    let ws = q.workspace;
    let pins = s
        .app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT target, workspace_id, at FROM chat_pin WHERE workspace_id = ?1 ORDER BY at")?;
            let rows = st.query_map(params![ws], |r| Ok(Pin { target: r.get(0)?, workspace_id: r.get(1)?, at: r.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(ApiError::from)?;
    Ok(Json(pins))
}

async fn set(State(s): State<Ctx>, Json(b): Json<Set>) -> ApiResult<Value> {
    if !valid_target(&b.target) {
        return Err(ApiError::bad_request("a chat pin names `conversation:<bot>:<thread>` or `channel:<id>`"));
    }
    if launchpad_core::workspace::get(&s.app, &b.workspace_id).await.is_err() {
        return Err(ApiError::bad_request("no such workspace"));
    }
    // Unpinning always works (a pin whose chat is gone); a new pin names a chat of its workspace.
    if b.pinned && target_workspace(&s, &b.target).await.as_deref() != Some(b.workspace_id.as_str()) {
        return Err(ApiError::bad_request("this chat is not in that workspace (or no longer exists)"));
    }
    let (target, ws, pinned) = (b.target.clone(), b.workspace_id.clone(), b.pinned);
    s.app
        .state_db
        .call(move |c| {
            if pinned {
                c.execute("INSERT INTO chat_pin (target, workspace_id, at) VALUES (?1, ?2, ?3) ON CONFLICT(target) DO UPDATE SET workspace_id = ?2", params![target, ws, now_ms()])?;
            } else {
                c.execute("DELETE FROM chat_pin WHERE target = ?1", params![target])?;
            }
            Ok(())
        })
        .await
        .map_err(ApiError::from)?;
    s.app.bus.publish("chats", "pins", json!({"workspace": b.workspace_id}));
    Ok(Json(json!({"ok": true, "pinned": b.pinned})))
}

#[cfg(test)]
mod tests {
    use super::valid_target;

    #[test]
    fn targets() {
        assert!(valid_target("conversation:bot-1:thread_2"));
        assert!(valid_target("channel:abc-123"));
        assert!(!valid_target("conversation:bot-1"));
        assert!(!valid_target("conversation::x"));
        assert!(!valid_target("channel:"));
        assert!(!valid_target("channel:a/b"));
        assert!(!valid_target("session:abc"));
    }
}

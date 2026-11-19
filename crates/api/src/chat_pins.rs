//! Pinned chats (`chat.pin`): the owner's shortcuts to a conversation of bots (`conversation:<id>`, which a follow-up
//! thread keeps) or a bot channel (`channel:<id>`), shown in the laptop sidebar and first on the phone's Chats list.
//! Home-owned and owner-only (agents never reach these routes); a pin whose chat is gone is left out by the windows.
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
    Router::new().route("/api/chat-pins", get(list).post(set))
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

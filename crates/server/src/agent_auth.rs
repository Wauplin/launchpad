//! The CLI inside a jail may read context and prepare local work, but cannot operate the owner's machine.
//! Launchpad state, proposals and admin changes follow the session's Launchpad CLI access (`LaunchpadAccess`,
//! from its lease): read, propose or admin.
use axum::{body::{Body, to_bytes}, extract::Request, http::Method};
use launchpad_core::App;
use launchpad_core::vault::LaunchpadAccess;
use serde_json::Value;

use crate::error::ApiError;

/// The access a Launchpad CLI request needs: Read for state, Propose for writing proposals.
fn tool_need(method: &Method, path: &str) -> Option<LaunchpadAccess> {
    if method == Method::GET && (path == "/api/status" || path.starts_with("/api/context/") || path == "/api/proposals") {
        Some(LaunchpadAccess::Read)
    } else if method == Method::POST && path == "/api/proposals" {
        Some(LaunchpadAccess::Propose)
    } else {
        None
    }
}

/// The session's access, from its lease (None: the tool is not granted). A session without a lease on this
/// machine (not contained) proposes, never more.
fn access_of(app: &App, id: &str) -> Option<LaunchpadAccess> {
    match app.proxy.for_session(id) {
        Some(a) => a.lease.read().unwrap().launchpad,
        None => Some(LaunchpadAccess::Propose),
    }
}

/// Admin access: Launchpad's own records. GitHub, the Outbox, channels, credentials, devices, terminals, hosts,
/// runners, setup fixes, backups and Burst stay owner-only, whatever the access.
fn admin_allows(method: &Method, path: &str) -> bool {
    let p: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if *method == Method::GET {
        const CLOSED: &[&str] = &["devices", "terminals", "hosts", "vault", "git", "handoff", "runners", "runner", "setup", "burst", "home", "channels", "outbox", "review-state", "plain", "kb", "knowledge", "code", "browser"];
        return p.first() == Some(&"api") && p.get(1).is_some_and(|a| !CLOSED.contains(a));
    }
    match p.as_slice() {
        ["api", "bots"] | ["api", "bots", ..] => true,
        ["api", "launch", "crons"] | ["api", "launch", "crons", ..] | ["api", "launch", "schedules"] | ["api", "launch", "schedules", ..] => true,
        ["api", "launch", "preview"] => *method == Method::POST,
        ["api", "proposals", _, "accept" | "dismiss" | "recheck"] => *method == Method::POST,
        ["api", "workspaces"] | ["api", "workspaces", _] => true,
        ["api", "sessions"] => *method == Method::POST,
        ["api", "sessions", _, "input" | "answer" | "interrupt" | "close" | "resume" | "resume-subscription" | "rename" | "model" | "permissions" | "follow-up" | "seen" | "unread"] => *method == Method::POST,
        ["api", "mcp", _, "tools", _] => *method == Method::PUT,
        _ => false,
    }
}

/// Paid usage is the owner's choice only.
fn asks_paid_usage(v: &Value) -> bool {
    v["allow_paid_usage"] == Value::Bool(true) || v["spec"]["allow_paid_usage"] == Value::Bool(true)
}

fn read_allowed(path: &str) -> bool {
    path == "/api/status" || path.starts_with("/api/context/") || path == "/api/proposals" || path == "/api/predrafts"
        || (path.starts_with("/api/predrafts/") && !path.ends_with("/resolve"))
        || (path.starts_with("/api/sessions/") && path.ends_with("/trace"))
}

/// `launchpad kb` sends contents (media included) in its request.
pub const KB_BODY_MAX: usize = 128 << 20;

fn own_write(path: &str) -> bool {
    matches!(path, "/api/worktrees/finish" | "/api/worktrees/add-project" | "/api/assets" | "/api/suggestions" | "/api/proposals" | "/api/kb" | "/api/channels/send")
}

async fn owns_cwd(app: &App, id: &str, cwd: &str) -> bool {
    launchpad_core::worktrees::find_by_path(app, cwd).await.ok().flatten().is_some_and(|w| w.sessions.iter().any(|s| s == id))
}

pub async fn authorize(app: &App, id: &str, req: Request) -> Result<Request, ApiError> {
    let denied = || ApiError::new(axum::http::StatusCode::FORBIDDEN, "this session CLI token can only read context and prepare local work; use an owner window for this action");
    let path = req.uri().path().to_string();
    let access = access_of(app, id);
    if let Some(need) = tool_need(req.method(), &path)
        && access.is_none_or(|a| a < need)
    {
        let why = match access {
            None => "the Launchpad CLI tool is not granted to this session (Settings → Credentials and tools)",
            Some(_) => "this session's Launchpad CLI access is read only (`launchpad` in its bot's bot.toml)",
        };
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, why));
    }
    if access == Some(LaunchpadAccess::Admin) && tool_need(req.method(), &path).is_none() && !read_allowed(&path) && !own_write(&path) && admin_allows(req.method(), &path) {
        if req.method() == Method::GET || req.method() == Method::DELETE {
            return Ok(req);
        }
        let (parts, body) = req.into_parts();
        let bytes = to_bytes(body, 1 << 20).await.map_err(|_| denied())?;
        if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| asks_paid_usage(&v)) {
            return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "paid usage is the owner's choice: leave allow_paid_usage off"));
        }
        return Ok(Request::from_parts(parts, Body::from(bytes)));
    }
    if req.method() == Method::GET {
        if path == "/api/predrafts/resolve" {
            let url = reqwest::Url::parse(&format!("http://local{}", req.uri())).map_err(|_| denied())?;
            let cwd = url.query_pairs().find(|(k, _)| k == "cwd").map(|(_, v)| v.into_owned()).ok_or_else(denied)?;
            if owns_cwd(app, id, &cwd).await { return Ok(req); }
        } else if read_allowed(&path) { return Ok(req); }
        return Err(denied());
    }
    // PATCH of inert Local PR descriptions is allowed; publication and every other action stay denied.
    let patch = req.method() == Method::PATCH && path.strip_prefix("/api/predrafts/").is_some_and(|p| !p.is_empty() && !p.contains('/'));
    if patch { return Ok(req); }
    if req.method() != Method::POST || !(own_write(&path) || path == "/api/predrafts") { return Err(denied()); }
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, if path == "/api/kb" { KB_BODY_MAX } else { 1 << 20 }).await.map_err(|_| denied())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| denied())?;
    if path == "/api/predrafts" {
        if !owns_cwd(app, id, value["cwd"].as_str().unwrap_or_default()).await
            || value["session_id"].as_str().is_some_and(|s| s != id) { return Err(denied()); }
    } else if value["session_id"].as_str() != Some(id) { return Err(denied()); }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_and_publication_routes_are_not_cli_routes() {
        for path in ["/api/config", "/api/terminals", "/api/setup/fix", "/api/hosts", "/api/vault", "/api/code/file", "/api/devices/pair"] {
            assert!(!read_allowed(path));
            assert!(!own_write(path));
        }
        for path in ["/api/predrafts/p/publish", "/api/predrafts/p/merge-local", "/api/suggestions/s/post", "/api/outbox/o/send", "/api/channels", "/api/channels/telegram/token"] {
            assert!(!own_write(path));
        }
        assert!(read_allowed("/api/context/folders"));
        assert!(own_write("/api/worktrees/finish"));
    }

    #[test]
    fn admin_reaches_launchpad_records_only() {
        for (m, path) in [(Method::PATCH, "/api/bots/b"), (Method::POST, "/api/bots/b/message"), (Method::POST, "/api/launch/crons"), (Method::DELETE, "/api/launch/crons/c"), (Method::POST, "/api/proposals/p/accept"), (Method::POST, "/api/sessions"), (Method::POST, "/api/sessions/s/input"), (Method::GET, "/api/inbox")] {
            assert!(admin_allows(&m, path), "{m} {path}");
        }
        for (m, path) in [(Method::POST, "/api/suggestions/s/post"), (Method::POST, "/api/predrafts/p/publish"), (Method::POST, "/api/outbox/o/send"), (Method::POST, "/api/items/o/r/1/merge"), (Method::PUT, "/api/vault/entries/github"), (Method::PUT, "/api/vault/tools/x"), (Method::PUT, "/api/config"), (Method::POST, "/api/terminals"), (Method::POST, "/api/sessions/s/resume-paid"), (Method::POST, "/api/sessions/s/paid-usage"), (Method::POST, "/api/devices/pair"), (Method::POST, "/api/runners/r/approve"), (Method::GET, "/api/vault"), (Method::GET, "/api/devices"), (Method::POST, "/api/home-git/f/merge"), (Method::POST, "/api/inbox/t/reply")] {
            assert!(!admin_allows(&m, path), "{m} {path}");
        }
        assert!(asks_paid_usage(&serde_json::json!({"spec": {"allow_paid_usage": true}})));
        assert!(!asks_paid_usage(&serde_json::json!({"spec": {"allow_paid_usage": false}})));
    }
}

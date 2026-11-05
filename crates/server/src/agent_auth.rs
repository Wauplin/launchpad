//! The CLI inside a jail may read context and prepare local work, but cannot operate the owner's machine.
use axum::{body::{Body, to_bytes}, extract::Request, http::Method};
use launchpad_core::App;
use serde_json::Value;

use crate::error::ApiError;

/// Launchpad state and proposals: what the Launchpad CLI tool grants (`vault::LAUNCHPAD_TOOL`).
fn launchpad_tool_path(method: &Method, path: &str) -> bool {
    (method == Method::GET && (path == "/api/status" || path.starts_with("/api/context/") || path == "/api/proposals")) || (method == Method::POST && path == "/api/proposals")
}

/// A contained session whose lease does not grant the Launchpad CLI tool. Sessions without a lease on this
/// machine (not contained, or before their lease) keep the CLI as before.
fn tool_off(app: &App, id: &str) -> bool {
    app.proxy.for_session(id).is_some_and(|a| !a.lease.read().unwrap().tools.iter().any(|t| t.id == launchpad_core::vault::LAUNCHPAD_TOOL))
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
    if launchpad_tool_path(req.method(), &path) && tool_off(app, id) {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "the Launchpad CLI tool is not granted to this session (Settings → Credentials and tools)"));
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
}

//! The CLI inside a jail may read context and prepare local work, but cannot operate the owner's machine.
//! Launchpad state, proposals and admin changes follow the session's Launchpad CLI access (`LaunchpadAccess`,
//! from its lease): read, propose or admin.
use axum::{body::{Body, to_bytes}, extract::Request, http::Method};
use launchpad_core::App;
use launchpad_core::actions::{self, ActionAccess};
use launchpad_core::vault::LaunchpadAccess;
use serde_json::Value;

use crate::error::ApiError;

/// Diagnostic reads (`launchpad_api::diagnostics`): the recent log of Home and runners, runners, Burst runs and
/// their HF Jobs, Home's starts, the failure feed, tools, channels and health. They hold no secret, and need admin
/// access.
pub fn diagnostic(path: &str) -> bool {
    path == "/api/logs"
        || ["runners", "burst", "home", "failures", "tools", "channels", "health"].iter().any(|p| {
            path.strip_prefix("/api/context/").and_then(|r| r.strip_prefix(p)).is_some_and(|rest| rest.is_empty() || rest.strip_prefix('/').is_some_and(|r| !r.split('/').any(str::is_empty)))
        })
}

/// The access a Launchpad CLI request needs: Read for state, Propose for writing proposals, Admin for diagnostics.
fn tool_need(method: &Method, path: &str) -> Option<LaunchpadAccess> {
    if method == Method::GET && diagnostic(path) {
        Some(LaunchpadAccess::Admin)
    } else if method == Method::GET && (path == "/api/status" || path.starts_with("/api/context/") || path == "/api/proposals" || path == "/api/today" || path == "/api/activity") {
        Some(LaunchpadAccess::Read)
    } else if method == Method::POST && (path == "/api/proposals" || path == "/api/inbox/agent-done") {
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

/// Admin access: reads (except the closed list) and the `Admin` actions of the action list
/// (`launchpad_core::actions`). GitHub, the Outbox, channels, credentials, devices, terminals, hosts, runners,
/// setup fixes, backups, Burst and config.toml (except the shared settings list, `settings.set`) are never `Admin`
/// actions, whatever the access.
fn admin_allows(method: &Method, path: &str) -> bool {
    if *method == Method::GET {
        let p: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        const CLOSED: &[&str] = &["config", "devices", "terminals", "vault", "git", "handoff", "runners", "runner", "setup", "burst", "home", "channels", "outbox", "review-state", "plain", "kb", "knowledge", "code", "browser"];
        return p.first() == Some(&"api") && p.get(1).is_some_and(|a| !CLOSED.contains(a));
    }
    actions::find(method.as_str(), path).is_some_and(|a| a.access == ActionAccess::Admin)
}

/// Whether accepting a card of this body runs an action admin may run directly: accepting a card is running its
/// action (`actions::for_card`). Tool cards and ready cards are owner actions and wait for the owner. A status
/// card only lists sessions.
fn admin_may_accept(body: &Value) -> bool {
    let id = |k: &str| body[k].as_str().is_some_and(|s| !s.is_empty() && !s.contains('/'));
    let Some(kind) = body["kind"].as_str() else { return false };
    let ids = match kind { "message" => id("session_id"), "routine" | "bot_instructions" | "bot_files" => id("bot_id"), _ => true };
    kind == "status" || (ids && actions::for_card(kind).is_some_and(|a| a.access == ActionAccess::Admin))
}

/// A proposal's body: on a runner from Home (which holds proposals), else from this daemon's state.
async fn proposal_body(app: &App, id: &str) -> Option<Value> {
    if app.paths.role == launchpad_core::role::Role::Runner {
        let list = launchpad_core::home_link::home_request("GET", "/api/proposals", None).await.ok()?;
        return list.as_array()?.iter().find(|p| p["id"] == id).map(|p| p["body"].clone());
    }
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [&id], |r| r.get(0)).ok())).await.ok()?;
    serde_json::from_str::<Value>(&json?).ok().map(|p| p["body"].clone())
}

/// A body field an action with a fixed input list does not take (folder.set never sets a setup command).
fn unknown_field(a: &actions::Action, body: Option<&Value>) -> Option<String> {
    if a.body_file.is_some() || (a.input.is_empty() && a.fixed.is_none()) {
        return None;
    }
    let fixed: Vec<String> = a.fixed.and_then(|f| serde_json::from_str::<serde_json::Map<String, Value>>(f).ok()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    body?.as_object()?.keys().find(|k| !a.input.iter().any(|f| f.name == k.as_str()) && !fixed.contains(k)).cloned()
}

/// Paid usage is the owner's choice only.
fn asks_paid_usage(v: &Value) -> bool {
    v["allow_paid_usage"] == Value::Bool(true) || v["spec"]["allow_paid_usage"] == Value::Bool(true)
}

fn read_allowed(path: &str) -> bool {
    path == "/api/status" || (path.starts_with("/api/context/") && !diagnostic(path)) || path == "/api/proposals" || path == "/api/today" || path == "/api/activity" || path == "/api/predrafts"
        || (path.starts_with("/api/predrafts/") && !path.ends_with("/resolve"))
        || (path.starts_with("/api/sessions/") && path.ends_with("/trace"))
}

/// `launchpad kb` and `launchpad bot memory` send contents (media included) in their request.
pub const KB_BODY_MAX: usize = 128 << 20;

fn own_write(path: &str) -> bool {
    matches!(path, "/api/worktrees/finish" | "/api/worktrees/add-project" | "/api/assets" | "/api/suggestions" | "/api/proposals" | "/api/kb" | "/api/bot-memory" | "/api/channels/send" | "/api/inbox/agent-done")
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
            Some(_) if need == LaunchpadAccess::Admin => "diagnostics (logs, runners, Burst, Home, health) need admin Launchpad CLI access (`launchpad = \"admin\"` in the bot's bot.toml)",
            Some(_) => "this session's Launchpad CLI access is read only (`launchpad` in its bot's bot.toml)",
        };
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, why));
    }
    if tool_need(req.method(), &path) == Some(LaunchpadAccess::Admin) {
        return Ok(req);
    }
    if access == Some(LaunchpadAccess::Admin) && tool_need(req.method(), &path).is_none() && !read_allowed(&path) && !own_write(&path) && admin_allows(req.method(), &path) {
        if req.method() == Method::GET || req.method() == Method::DELETE {
            return Ok(req);
        }
        if let Some(pid) = path.strip_prefix("/api/proposals/").and_then(|r| r.strip_suffix("/accept")) {
            let direct = proposal_body(app, pid).await.as_ref().is_some_and(admin_may_accept);
            if !direct {
                return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "admin access cannot run this card's action directly (tools and ready cards): it waits for the owner's click"));
            }
        }
        let (parts, body) = req.into_parts();
        let bytes = to_bytes(body, 1 << 20).await.map_err(|_| denied())?;
        let value = serde_json::from_slice::<Value>(&bytes).ok();
        if value.as_ref().is_some_and(asks_paid_usage) {
            return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "paid usage is the owner's choice: leave allow_paid_usage off"));
        }
        if let Some(a) = actions::find(parts.method.as_str(), &path)
            && let Some(key) = unknown_field(a, value.as_ref())
        {
            return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, format!("{} takes no field {key:?}: {}", a.name, actions::usage("launchpad", a))));
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
    let bytes = to_bytes(body, if path == "/api/kb" || path == "/api/bot-memory" { KB_BODY_MAX } else { 1 << 20 }).await.map_err(|_| denied())?;
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
        for path in ["/api/config", "/api/terminals", "/api/vault", "/api/code/file", "/api/devices/pair"] {
            assert!(!read_allowed(path));
            assert!(!own_write(path));
        }
        for path in ["/api/predrafts/p/publish", "/api/predrafts/p/merge-local", "/api/suggestions/s/post", "/api/outbox/o/send", "/api/channels", "/api/channels/telegram/w1/token", "/api/channels/telegram/w1"] {
            assert!(!own_write(path));
        }
        assert!(read_allowed("/api/context/folders"));
        assert!(own_write("/api/worktrees/finish"));
        // Bots allowed by bot.toml mark Inbox threads done (Home checks the bot); read access never does.
        assert!(own_write("/api/inbox/agent-done"));
        assert_eq!(tool_need(&Method::POST, "/api/inbox/agent-done"), Some(LaunchpadAccess::Propose));
        assert!(!own_write("/api/inbox/t/action"));
        // A bot writes its own memory like the knowledge base, at every access level (Home checks the bot).
        assert!(own_write("/api/bot-memory"));
        assert_eq!(tool_need(&Method::POST, "/api/bot-memory"), None);
    }

    #[test]
    fn diagnostics_need_admin_and_stay_reads() {
        for path in ["/api/logs", "/api/context/health", "/api/context/runners", "/api/context/burst", "/api/context/burst/r1/job", "/api/context/home", "/api/context/failures", "/api/context/tools", "/api/context/channels"] {
            assert!(diagnostic(path), "{path}");
            assert_eq!(tool_need(&Method::GET, path), Some(LaunchpadAccess::Admin), "{path}");
            assert!(!read_allowed(path), "{path}");
            assert_eq!(tool_need(&Method::POST, path), None, "{path}");
        }
        for path in ["/api/context/sessions", "/api/context/sessions/s1", "/api/context/burst/", "/api/context/burst//job", "/api/context/burstx", "/api/context/homes", "/api/logs/x"] {
            assert!(!diagnostic(path), "{path}");
        }
        assert_eq!(tool_need(&Method::GET, "/api/context/sessions/s1"), Some(LaunchpadAccess::Read));
        // The owner's routes with secrets or machine state stay closed, whatever the access.
        for path in ["/api/burst", "/api/runners", "/api/vault/leases", "/api/channels", "/api/channels/telegram/w1", "/api/home/backup", "/api/config"] {
            assert!(!admin_allows(&Method::GET, path), "{path}");
        }
    }

    #[test]
    fn admin_reaches_launchpad_records_only() {
        for (m, path) in [(Method::PATCH, "/api/bots/b"), (Method::POST, "/api/bots/b/message"), (Method::POST, "/api/launch/crons"), (Method::DELETE, "/api/launch/crons/c"), (Method::POST, "/api/proposals/p/accept"), (Method::POST, "/api/sessions"), (Method::POST, "/api/sessions/s/input"), (Method::GET, "/api/inbox")] {
            assert!(admin_allows(&m, path), "{m} {path}");
        }
        for (m, path) in [(Method::POST, "/api/suggestions/s/post"), (Method::POST, "/api/predrafts/p/publish"), (Method::POST, "/api/outbox/o/send"), (Method::POST, "/api/items/o/r/1/merge"), (Method::PUT, "/api/vault/entries/github"), (Method::POST, "/api/vault/entries/codex/refresh"), (Method::PUT, "/api/vault/tools/x"), (Method::PUT, "/api/config"), (Method::POST, "/api/terminals"), (Method::POST, "/api/sessions/s/resume-paid"), (Method::POST, "/api/sessions/s/paid-usage"), (Method::POST, "/api/devices/pair"), (Method::POST, "/api/runners/r/approve"), (Method::GET, "/api/vault"), (Method::GET, "/api/devices"), (Method::POST, "/api/home-git/f/merge"), (Method::POST, "/api/inbox/t/reply"), (Method::POST, "/api/proposals/p/accept/"), (Method::POST, "/api/sessions//input")] {
            assert!(!admin_allows(&m, path), "{m} {path}");
        }
        assert!(!admin_allows(&Method::GET, "/api/config"));
        let direct = |b: Value| admin_may_accept(&b);
        assert!(direct(serde_json::json!({"kind": "routine", "bot_id": "b"})) && direct(serde_json::json!({"kind": "bot_instructions", "bot_id": "b"})) && direct(serde_json::json!({"kind": "bot_files", "bot_id": "b"})));
        assert!(direct(serde_json::json!({"kind": "preset"})));
        assert!(!direct(serde_json::json!({"kind": "tool"})) && !direct(serde_json::json!({"kind": "ready", "action": "outbox.send"})));
        let folder = actions::get("folder.set").unwrap();
        assert_eq!(unknown_field(folder, Some(&serde_json::json!({"worktree_setup": "x"}))).as_deref(), Some("worktree_setup"));
        assert!(unknown_field(folder, Some(&serde_json::json!({"when_done": "keep"}))).is_none());
        assert!(unknown_field(actions::get("bot.update").unwrap(), Some(&serde_json::json!({"set": {"inbox_done": true}}))).is_none());
        assert!(asks_paid_usage(&serde_json::json!({"spec": {"allow_paid_usage": true}})));
        assert!(!asks_paid_usage(&serde_json::json!({"spec": {"allow_paid_usage": false}})));
    }

    /// Admin checks for and installs a newer Launchpad on Home's Space (release or main); updating the runners
    /// afterwards stays the owner's (the button's `update_runners` is not an input), and the owner's routes of
    /// Home and runners stay closed.
    #[test]
    fn admin_updates_home_but_not_the_runners() {
        assert!(admin_allows(&Method::POST, "/api/home/update/check"));
        assert!(admin_allows(&Method::POST, "/api/home/update"));
        assert!(!admin_allows(&Method::GET, "/api/home/update"));
        for (m, path) in [(Method::POST, "/api/home/update/"), (Method::POST, "/api/home/backup"), (Method::POST, "/api/runner/update"), (Method::POST, "/api/runners/r/update")] {
            assert!(!admin_allows(&m, path), "{m} {path}");
        }
        let update = actions::get("home.update").unwrap();
        assert!(update.ready && update.confirm.is_some());
        assert!(unknown_field(update, Some(&serde_json::json!({"channel": "main"}))).is_none());
        assert!(unknown_field(update, Some(&serde_json::json!({"version": "v2026.11.09.1", "channel": "release"}))).is_none());
        assert_eq!(unknown_field(update, Some(&serde_json::json!({"channel": "main", "update_runners": true}))).as_deref(), Some("update_runners"));
        let reference = launchpad_core::launchpad_cli::cli_reference("launchpad", None, LaunchpadAccess::Admin);
        assert!(reference.contains("`launchpad home update [--version <text>] [--channel <text>]`") && reference.contains("`launchpad home check-update`"));
        let owner_only = reference.split("Only the owner's click runs these").nth(1).unwrap().split("Propose a ready card").next().unwrap();
        assert!(!owner_only.contains("Home's Space"), "{owner_only}");
    }

    /// Every action has a route that resolves to it, a CLI reference line exactly when admin may run it, and a
    /// confirmation (or an undo) when it reaches GitHub, other people, money or the machine, or deletes something.
    #[test]
    fn action_list_drives_routes_reference_and_confirmations() {
        let reference = launchpad_core::launchpad_cli::cli_reference("launchpad", None, LaunchpadAccess::Admin);
        let propose = launchpad_core::launchpad_cli::cli_reference("launchpad", None, LaunchpadAccess::Propose);
        for a in actions::ACTIONS {
            let args: Vec<String> = actions::params(a).iter().map(|p| format!("x-{p}")).collect();
            let path = actions::path(a, &args.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
            assert_eq!(actions::find(a.method, &path).map(|f| f.name), Some(a.name), "{} {path}", a.name);
            let method = Method::from_bytes(a.method.as_bytes()).unwrap();
            assert_eq!(admin_allows(&method, &path), a.access == ActionAccess::Admin, "{}", a.name);
            let line = format!("`{}`", actions::usage("launchpad", a));
            assert_eq!(reference.contains(&line), a.access == ActionAccess::Admin, "{line}");
            assert!(!propose.contains(&line), "{line}");
            if matches!(a.effect, actions::ActionEffect::Outside | actions::ActionEffect::Machine) {
                assert_eq!(a.access, ActionAccess::Owner, "{}", a.name);
            }
            if (matches!(a.effect, actions::ActionEffect::Outside | actions::ActionEffect::Machine) || a.method == "DELETE") && a.undo.is_none() {
                assert!(a.confirm.is_some(), "{} needs a confirmation or an undo", a.name);
            }
            assert!(a.undo.is_none_or(|u| actions::get(u).is_some()), "{}", a.name);
            // An admin action's undo is an admin action: it fits the route parameters, or the new record's id.
            if a.access == ActionAccess::Admin && let Some(u) = a.undo.and_then(actions::get) {
                assert_eq!(u.access, ActionAccess::Admin, "{}", a.name);
                match a.before {
                    Some(b) => assert!(actions::params(u) == actions::params(a) && b.split('/').filter(|s| s.starts_with('{')).count() == actions::params(a).len(), "{}", a.name),
                    None => assert_eq!(actions::params(u).len(), 1, "{}", a.name),
                }
            }
            // A ready card prepares an owner click; only a Home update is also an admin action.
            assert!(!a.ready || a.access == ActionAccess::Owner || a.effect == actions::ActionEffect::Installs, "{}", a.name);
            if a.effect == actions::ActionEffect::Installs {
                assert!(a.confirm.is_some(), "{}", a.name);
            }
        }
    }
}

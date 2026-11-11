//! Home: what a runner may ask on its own credential. A runner is a low-trust worker: through its tunnel it gets
//! Home Git and plain folders, handoffs, leases, quota readings, replication and uploads for the sessions Home
//! placed on it (each checked by its handler), plus the closed list below of API requests its own work needs
//! (`FromRunner::Req` without a session), each scoped to its sessions here. An agent's CLI call the runner
//! forwards names its session and is checked on Home with that session's own access
//! (`launchpad_http::agent_auth`, through `runners::serve`). Nothing a runner sends runs as the owner: it never
//! reaches the vault, MCP settings, channels, the Outbox, Suggestions, Local PR publication, Home updates or the
//! owner's actions.

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use serde_json::Value;

/// Pure: the API requests a runner may send for itself. Each is also scoped by `check`.
pub(crate) fn may(method: &str, path: &str) -> bool {
    let p: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if p.iter().any(|s| s.is_empty()) {
        return false;
    }
    matches!(
        (method, p.as_slice()),
        ("GET", ["api", "projects", _] | ["api", "projects", _, "branches"])
            | ("POST", ["api", "projects", "remote"])
            | ("GET", ["api", "predrafts"] | ["api", "predrafts", _, "record"])
            | ("POST", ["api", "predrafts"])
            | ("PATCH", ["api", "predrafts", _])
            | ("POST", ["api", "home-git", _, "refresh" | "delete" | "merge" | "pull-request"])
            | ("GET", ["api", "home-git", _, "branches" | "follow"])
            | ("POST", ["api", "plain", "assets", _])
            | ("GET", ["api", "sessions", _] | ["api", "sessions", _, "trace"])
            | ("GET", ["api", "context", "presets"] | ["api", "channels", "names"])
            | ("POST", ["api", "channels", "telegram", "notify"])
    )
}

/// Whether Home placed session `id` on runner `runner` (it runs there now).
fn placed(app: &App, runner: &str, id: &str) -> bool {
    app.sessions.get(id).is_some_and(|s| s.runner_id.as_deref() == Some(runner))
}

/// Pure: a branch named for the session (`worktrees::slug_for`), before its worktree record reached Home.
fn named_for(session_id: &str, branch: &str) -> bool {
    let short: String = session_id.chars().take(8).collect();
    !short.is_empty() && branch.strip_prefix("lp/").is_some_and(|n| n.ends_with(&format!("-{short}")) && !n.contains(".."))
}

/// Whether `branch` of folder `project` is the session's: its name, its worktree records on Home, or the Local PR
/// its launch continues.
async fn owns_branch(app: &App, session_id: &str, project: &str, branch: &str) -> bool {
    if named_for(session_id, branch) || launchpad_core::away::session_branch(app, session_id, project, branch).await.ok().flatten().is_some() {
        return true;
    }
    let Some(launchpad_core::model::Target::ExistingPredraft { predraft_id }) = app.sessions.get(session_id).map(|s| s.launch_spec.target) else { return false };
    launchpad_core::predrafts::get(app, &predraft_id).await.is_ok_and(|p| p.project_id == project && p.branch == branch)
}

fn str_of<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().filter(|s| !s.is_empty()).with_context(|| format!("{key} is required"))
}

/// Home: checks a runner's own request (`may`) against Home's records, scoped to the sessions and folders Home
/// placed on that runner, and to their workspaces (`scope::of_runner`) for Local PRs. Returns the query to add
/// (the Local PR list narrowed to those workspaces) and the body to run, which Home may correct (a merge without
/// the owner's grant is the agent's; a kept folder's workspace is its own).
pub(crate) async fn check(app: &App, runner: &str, method: &str, path: &str, body: Option<String>) -> Result<(Option<String>, Option<String>)> {
    ensure!(may(method, path), "runners may only send requests about their own sessions and folders");
    let p: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut v: Value = body.as_deref().map(serde_json::from_str).transpose().context("the body is not JSON")?.unwrap_or(Value::Null);
    let session = |v: &Value| -> Result<String> {
        let sid = str_of(v, "session_id")?;
        ensure!(placed(app, runner, sid), "Home did not place this session on this runner");
        Ok(sid.to_string())
    };
    match (method, p.as_slice()) {
        // Keep as folder of a temporary folder that holds a Git repository: the owner's click, for this runner's folder.
        ("POST", ["api", "projects", "remote"]) => {
            let temp = str_of(&v, "temp_id")?.to_string();
            let t = launchpad_core::projects::get(app, &temp).await?.context("no such temporary folder")?;
            ensure!(t.is_temp && t.runner_id.as_deref() == Some(runner), "this temporary folder is not on this runner");
            ensure!(app.runners.granted(runner, &format!("keep:{temp}")), "Keep as folder comes from the owner's window");
            v["workspace_id"] = Value::from(t.workspace_id.context("this temporary folder has no workspace")?);
        }
        ("POST", ["api", "predrafts"]) => {
            let sid = session(&v)?;
            ensure!(v["cwd"].is_null(), "Home has no worktrees");
            let (project, branch) = (str_of(&v, "project_id")?, str_of(&v, "branch")?);
            ensure!(owns_branch(app, &sid, project, branch).await, "{branch} is not a branch of this session");
        }
        ("GET", ["api", "predrafts"]) => {
            let mut mine: Vec<String> = launchpad_core::scope::of_runner(app, runner).await.into_iter().collect();
            mine.sort();
            // "-" names no workspace: a runner without sessions reads no Local PR.
            let list = if mine.is_empty() { "-".to_string() } else { mine.join(",") };
            return Ok((Some(format!("workspace={}", list)), None));
        }
        ("GET", ["api", "predrafts", id, "record"]) => {
            let ws = launchpad_core::scope::of_predraft(app, id).await;
            ensure!(launchpad_core::scope::runner_reads(app, runner, ws).await, "this Local PR belongs to a workspace with no session on this runner");
        }
        ("PATCH", ["api", "predrafts", id]) => {
            let pr = launchpad_core::predrafts::get(app, id).await?;
            ensure!(pr.session_id.as_deref().is_some_and(|s| placed(app, runner, s)), "this Local PR belongs to a session of another runner");
        }
        ("POST", ["api", "home-git", project, "delete"]) => {
            let branch = str_of(&v, "branch")?;
            let mine: Vec<String> = app.sessions.list().into_iter().filter(|s| s.runner_id.as_deref() == Some(runner)).map(|s| s.id).collect();
            let mut owned = false;
            for sid in &mine {
                if owns_branch(app, sid, project, branch).await {
                    owned = true;
                    break;
                }
            }
            ensure!(owned, "{branch} is not a branch of a session of this runner");
        }
        ("POST", ["api", "home-git", _, "merge"]) => {
            session(&v)?;
            // The owner's merge reaches Home from the owner's window (`runners::relay` grants its reviewed head);
            // any other merge a runner asks for is its agent's (`worktrees::check_merge_request`).
            let owners = v["by_agent"] == Value::Bool(false) && v["head"].as_str().is_some_and(|h| app.runners.granted(runner, &format!("merge:{h}")));
            v["by_agent"] = Value::Bool(!owners);
        }
        ("POST", ["api", "home-git", project, "pull-request"]) => {
            let sid = session(&v)?;
            let branch = str_of(&v, "branch")?;
            ensure!(owns_branch(app, &sid, project, branch).await, "{branch} is not a branch of this session");
        }
        // Records and traces: the runner's own sessions, and the context agents Home sent it.
        ("GET", ["api", "sessions", id] | ["api", "sessions", id, "trace"]) => {
            let context = app.runners.granted(runner, &format!("trace:{id}"))
                || app.sessions.list().iter().any(|s| s.runner_id.as_deref() == Some(runner) && s.launch_spec.agent_traces.iter().any(|t| t.session_id == *id));
            ensure!(placed(app, runner, id) || context, "Home did not place this session on this runner, nor send it as context");
        }
        ("POST", ["api", "channels", "telegram", "notify"]) => {
            session(&v)?;
        }
        // Asset folders: of a Git folder of a workspace the runner works in.
        ("POST", ["api", "plain", "assets", project]) => {
            let ws = launchpad_core::scope::of_project(app, project).await;
            ensure!(launchpad_core::scope::runner_reads(app, runner, ws).await, "this folder belongs to a workspace with no session on this runner");
        }
        _ => {}
    }
    if body.is_none() {
        return Ok((None, None));
    }
    Ok((None, Some(v.to_string())))
}

/// After a runner's request succeeded: the Git folder Home created for its kept temporary folder may receive the
/// import of its repository (`git_http`).
pub(crate) fn after(app: &App, runner: &str, method: &str, path: &str, status: u16, answer: &str) {
    if method == "POST" && path == "/api/projects/remote" && status == 200
        && let Some(id) = serde_json::from_str::<Value>(answer).ok().and_then(|v| v["id"].as_str().map(str::to_string))
    {
        app.runners.grant(runner, &format!("import:{id}"));
    }
}

/// Home: an agent call a runner forwards: only for a session Home placed on that runner.
pub(crate) fn agent(app: &App, runner: &str, session: &str) -> Result<()> {
    if !placed(app, runner, session) {
        bail!("Home did not place this session on this runner");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_sends_only_what_its_own_work_needs() {
        for (m, p) in [
            ("GET", "/api/projects/f1"),
            ("POST", "/api/projects/remote"),
            ("GET", "/api/predrafts"),
            ("POST", "/api/predrafts"),
            ("PATCH", "/api/predrafts/p1"),
            ("GET", "/api/predrafts/p1/record"),
            ("POST", "/api/home-git/f1/merge"),
            ("POST", "/api/home-git/f1/refresh"),
            ("GET", "/api/sessions/s1/trace"),
            ("GET", "/api/context/presets"),
        ] {
            assert!(may(m, p), "{m} {p}");
        }
        for (m, p) in [
            ("GET", "/api/vault"),
            ("PUT", "/api/vault/tools/x"),
            ("PUT", "/api/mcp/m/tools/t"),
            ("GET", "/api/channels"),
            ("POST", "/api/channels/send"),
            ("POST", "/api/outbox/o/send"),
            ("POST", "/api/suggestions"),
            ("POST", "/api/suggestions/s/post"),
            ("POST", "/api/predrafts/p1/publish"),
            ("POST", "/api/predrafts/p1/merge-local"),
            ("POST", "/api/home/update"),
            ("PATCH", "/api/settings/shared"),
            ("PATCH", "/api/bots/b"),
            ("POST", "/api/proposals"),
            ("POST", "/api/proposals/p/accept"),
            ("POST", "/api/home-git/f1/restore"),
            ("PATCH", "/api/projects/f1"),
            ("POST", "/api/projects/plain"),
            ("POST", "/api/sessions"),
            ("POST", "/api/sessions/s1/input"),
            ("GET", "/api/context/sessions"),
            ("GET", "/api/logs"),
            ("POST", "/api/workspaces"),
            ("POST", "/api/activity"),
            ("GET", "/api/sessions//trace"),
            ("GET", "/api/projects/"),
        ] {
            assert!(!may(m, p), "{m} {p}");
        }
    }

    #[test]
    fn a_branch_named_for_a_session_is_its_own() {
        assert!(named_for("0123456789ab", "lp/fix-it-01234567"));
        assert!(!named_for("0123456789ab", "lp/fix-it-01234568"));
        assert!(!named_for("0123456789ab", "main-01234567"));
        assert!(!named_for("", "lp/x-"));
    }
}

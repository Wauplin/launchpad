//! Workspaces are a boundary for agents: a session acts on and reads only the records of its own workspace
//! (folders, sessions and their traces, bots, routines, scheduled launches, proposals, Local PRs, knowledge base),
//! and a runner reads only the records of the workspaces of the sessions Home placed on it. The owner is never
//! limited. Home answers "which workspace" from its own records with the functions below; the checks live with
//! each route family (`launchpad_http::agent_auth::within_workspace`, `proposals::create`, `runner_trust`).

use std::collections::HashSet;

use anyhow::{Result, bail, ensure};
use serde_json::Value;

use crate::App;
use crate::model::{LaunchSpec, Target};

/// The workspace of a session (its folder's, else its launch's).
pub async fn of_session(app: &App, id: &str) -> Option<String> {
    crate::kb::workspace_of_session(app, id).await
}

pub async fn of_project(app: &App, id: &str) -> Option<String> {
    crate::projects::get(app, id).await.ok().flatten().and_then(|p| p.workspace_id)
}

pub async fn of_bot(app: &App, id: &str) -> Option<String> {
    crate::bots::get(app, id).await.ok().map(|b| b.workspace_id)
}

/// A Local PR's workspace: its folder's.
pub async fn of_predraft(app: &App, id: &str) -> Option<String> {
    let p = crate::predrafts::get(app, id).await.ok()?;
    of_project(app, &p.project_id).await
}

/// A plain store's workspace: a knowledge base (`kb-<workspace>`), a bot's files (`bot-<id>`), a temporary
/// folder (`tmp-<folder>`), an asset folder (its Git folder's) or a plain folder.
pub async fn of_store(app: &App, store: &str) -> Option<String> {
    if let Some(ws) = crate::kb::workspace_of(store) {
        return Some(ws.to_string());
    }
    if let Some(bot) = crate::bots::id_of(store) {
        return of_bot(app, bot).await;
    }
    let folder = store.strip_prefix("tmp-").unwrap_or(store);
    if let Some(ws) = of_project(app, folder).await {
        return Some(ws);
    }
    let asset = crate::plain::assets(app, None).await.ok()?.into_iter().find(|a| a.id == store)?;
    of_project(app, &asset.project_id).await
}

async fn worktree_record(app: &App, id: &str) -> Option<Value> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM runner_worktree WHERE id = ?1", [&id], |r| r.get(0)).ok())).await.ok()?;
    serde_json::from_str(&json?).ok()
}

/// Home: a worktree's workspace, from the records runners send (its folder's).
pub async fn of_worktree(app: &App, id: &str) -> Option<String> {
    let v = worktree_record(app, id).await?;
    of_project(app, v["project_id"].as_str()?).await
}

/// Home: the last session of a worktree, whose runner holds its folder.
pub async fn worktree_session(app: &App, id: &str) -> Option<String> {
    worktree_record(app, id).await?["sessions"].as_array()?.last()?.as_str().map(str::to_string)
}

/// The workspace a launch spec runs in: its target's record, else the workspace it names.
pub async fn of_spec(app: &App, spec: &LaunchSpec) -> Option<String> {
    let named = match &spec.target {
        Target::Project { project_id } | Target::Predraft { project_id } | Target::Browser { project_id: Some(project_id), .. } => of_project(app, project_id).await,
        Target::ExistingPredraft { predraft_id } => of_predraft(app, predraft_id).await,
        Target::SessionFolder { session_id } => of_session(app, session_id).await,
        Target::Browser { source_session_id: Some(s), .. } => of_session(app, s).await,
        _ => None,
    };
    named.or_else(|| spec.workspace_id.clone())
}

/// Home: a routine's or a scheduled launch's workspace (`cron_agent`, `scheduled_launch`: their bot's, else their
/// spec's, else the session a follow-up resumes).
pub async fn of_launch_record(app: &App, table: &'static str, id: &str) -> Option<String> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row(&format!("SELECT json FROM {table} WHERE id = ?1"), [&id], |r| r.get(0)).ok())).await.ok()?;
    let v: Value = serde_json::from_str(&json?).ok()?;
    if let Some(bot) = v["bot_id"].as_str() {
        return of_bot(app, bot).await;
    }
    if let Some(s) = v["resume_session_id"].as_str() {
        return of_session(app, s).await;
    }
    let spec: LaunchSpec = serde_json::from_value(v["spec"].clone()).ok()?;
    of_spec(app, &spec).await
}

/// Home: a proposal's workspace.
pub async fn of_proposal(app: &App, id: &str) -> Option<String> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [&id], |r| r.get(0)).ok())).await.ok()?;
    serde_json::from_str::<Value>(&json?).ok()?["workspace_id"].as_str().map(str::to_string)
}

/// The GitHub repositories (`owner/name`, lowercase) of a workspace's folders.
pub async fn repos(app: &App, ws: &str) -> Vec<String> {
    let ids = crate::workspace::project_ids(app, ws).await.unwrap_or_default();
    crate::projects::list(app).await.unwrap_or_default().into_iter().filter(|p| ids.contains(&p.id)).flat_map(|p| [p.gh_repo, p.gh_upstream]).flatten().map(|r| r.to_lowercase()).collect()
}

/// A repository is outside `ws` when a folder of another workspace holds it and none of `ws` does. Repositories
/// without a folder belong to no workspace.
pub async fn repo_outside(app: &App, ws: &str, repo: &str) -> bool {
    let repo = repo.to_lowercase();
    let mut held = false;
    for p in crate::projects::list(app).await.unwrap_or_default() {
        if [&p.gh_repo, &p.gh_upstream].into_iter().flatten().any(|r| r.to_lowercase() == repo) {
            if p.workspace_id.as_deref() == Some(ws) {
                return false;
            }
            held = true;
        }
    }
    held
}

/// Pure: whether a record of workspace `record` is inside the session's workspace `own`. A record without a
/// workspace is outside every workspace.
pub fn inside(own: &str, record: Option<&str>) -> bool {
    record == Some(own)
}

pub fn ensure_inside(own: &str, record: Option<&str>, what: &str) -> Result<()> {
    ensure!(inside(own, record), "{what} belongs to another workspace: agents act only in their own workspace");
    Ok(())
}

/// Every folder, session, Local PR and bot an agent's launch spec names is in workspace `ws`, and the spec runs
/// there (a spec without a workspace gets `ws`). Browser launches stay the owner's.
pub async fn check_spec(app: &App, ws: &str, spec: &mut LaunchSpec) -> Result<()> {
    if spec.target.is_browser() {
        bail!("browser sessions are the owner's: launch them from Launchpad's window");
    }
    match &spec.workspace_id {
        Some(w) => ensure_inside(ws, Some(w), "this launch's workspace")?,
        None => spec.workspace_id = Some(ws.to_string()),
    }
    ensure_inside(ws, of_spec(app, spec).await.as_deref(), "this launch's target")?;
    if let Some(b) = &spec.bot {
        ensure_inside(ws, of_bot(app, &b.id).await.as_deref(), &format!("bot {}", b.id))?;
    }
    let projects: Vec<String> = spec.context_projects.iter().map(|c| c.project_id.clone()).collect();
    let traces: Vec<String> = spec.agent_traces.iter().map(|t| t.session_id.clone()).collect();
    check_context(app, ws, &projects, &traces).await
}

/// Read-only folders and agent traces given as context are of workspace `ws`.
pub async fn check_context(app: &App, ws: &str, projects: &[String], sessions: &[String]) -> Result<()> {
    for p in projects {
        ensure_inside(ws, of_project(app, p).await.as_deref(), &format!("folder {p}"))?;
    }
    for s in sessions {
        ensure_inside(ws, of_session(app, s).await.as_deref(), &format!("session {s}"))?;
    }
    Ok(())
}

/// Home: the workspaces of the sessions Home placed on a runner (running, or archived in the last day, so their
/// last saves and Archive still reach their folders and stores).
pub async fn of_runner(app: &App, runner: &str) -> HashSet<String> {
    let since = crate::util::now_ms() - 24 * 3_600_000;
    let folders: std::collections::HashMap<String, Option<String>> = crate::projects::list(app).await.unwrap_or_default().into_iter().map(|p| (p.id, p.workspace_id)).collect();
    app.sessions
        .list()
        .into_iter()
        .filter(|s| s.runner_id.as_deref() == Some(runner) && (s.status != crate::model::SessionStatus::Closed || s.status_since >= since))
        .filter_map(|s| s.project_id.as_ref().and_then(|p| folders.get(p).cloned().flatten()).or(s.launch_spec.workspace_id))
        .collect()
}

/// Home: a runner reads a record of workspace `ws` only when Home placed a session of it there.
pub async fn runner_reads(app: &App, runner: &str, ws: Option<String>) -> bool {
    match ws {
        Some(w) => of_runner(app, runner).await.contains(&w),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_without_a_workspace_are_outside_every_workspace() {
        assert!(inside("w1", Some("w1")));
        assert!(!inside("w1", Some("w2")));
        assert!(!inside("w1", None));
        assert!(ensure_inside("w1", Some("w2"), "folder f").unwrap_err().to_string().contains("another workspace"));
    }
}

//! Home's answers for a session whose runner is away (an offline laptop, a stopped Burst job).
//!
//! The session's worktrees are gone with its runner, but Home still has their branches (Home Git) and their
//! records: runners send each record whenever it changes (`replica::Item::Worktree`, with the branch tip in the
//! folder), and every handoff holds them too. Changes, the worktree list, Merge and Create Local PR use those, so
//! finished work stays reviewable. Uncommitted files and commits the runner did not push stay on that machine;
//! a record whose tip differs from Home's branch shows them as not on Home.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::App;
use crate::model::Session;
use crate::session_git::{GitChanges, GitDiff, GitFile, GitHistory, GitStatus};
use crate::worktrees::{Merged, TaskWorktree, WorktreeInfo, WorktreeState};

/// Home: the session runs on a runner that is not connected now.
pub fn is_away(app: &App, s: &Session) -> bool {
    app.paths.role == crate::role::Role::Home && s.runner_id.as_ref().is_some_and(|r| !app.runners.connected(r))
}

/// Why Home answers: "Its Burst runner has stopped" or "Its runner (name) is offline".
pub async fn reason(app: &App, s: &Session) -> String {
    if crate::burst::needs_new_job(app, s).await {
        return "Its Burst runner has stopped".into();
    }
    let runner = s.runner_id.clone().unwrap_or_default();
    match crate::runners::list(app).await.ok().and_then(|l| l.into_iter().find(|r| r.runner.id == runner)) {
        Some(r) => format!("Its runner ({}) is offline", r.runner.name),
        None => "Its runner is offline".into(),
    }
}

/// The session's worktree records, primary first: the ones its runners sent, then those of its last handoff
/// that Home did not receive on their own.
async fn records(app: &App, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let key = session_id.to_string();
    let rows: Vec<String> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT json FROM runner_worktree WHERE instr(json, ?1) > 0")?;
            let rows = st.query_map([key], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let mut list: Vec<TaskWorktree> = rows.iter().filter_map(|j| serde_json::from_str::<TaskWorktree>(j).ok()).filter(|w| w.sessions.iter().any(|s| s == session_id)).collect();
    for w in crate::handoff::latest(app, session_id).await?.map(|h| h.worktrees).unwrap_or_default() {
        if !list.iter().any(|x| x.id == w.id) {
            list.push(w);
        }
    }
    list.sort_by_key(|w| (!w.primary, w.created_at));
    Ok(list)
}

/// Home: stores a worktree record a runner sent, unless its session runs elsewhere now (it moved on).
pub(crate) async fn apply_replica(app: &App, runner_id: &str, n: u64, w: TaskWorktree) -> Result<()> {
    let current = w.sessions.iter().rev().find_map(|id| app.sessions.get(id));
    let fresh = current.is_none_or(|s| s.runner_id.as_deref() == Some(runner_id));
    let (runner, id, json) = (runner_id.to_string(), w.id.clone(), serde_json::to_string(&w)?);
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            if fresh {
                tx.execute(
                    "INSERT INTO runner_worktree (id, runner_id, json) VALUES (?1, ?2, ?3) ON CONFLICT (id) DO UPDATE SET runner_id = ?2, json = ?3",
                    rusqlite::params![id, runner, json],
                )?;
            }
            crate::replica::set_acked(&tx, &runner, n)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    if fresh {
        app.bus.publish("worktrees", "updated", serde_json::json!(w));
    }
    Ok(())
}

/// The worktree of the session's last handoff whose folder is `root` or contains it.
pub async fn worktree_at(app: &App, session_id: &str, root: &str) -> Option<TaskWorktree> {
    records(app, session_id).await.ok()?.into_iter().find(|w| root == w.root || root.starts_with(&format!("{}/", w.root)))
}

async fn primary(app: &App, session_id: &str) -> Result<TaskWorktree> {
    records(app, session_id).await?.into_iter().find(|w| w.primary).context("its runner is offline and Home has no record of this session's worktree yet")
}

async fn git(app: &App, dir: &Path, args: &[&str]) -> Result<String> {
    crate::home_git::git_in(app, dir, args, 60).await
}

/// The folder's mirror on Home, with the task branch and its base as local branches.
fn mirror(app: &App, w: &TaskWorktree) -> Result<PathBuf> {
    let dir = crate::home_git::mirror(app, &w.project_id);
    ensure!(dir.join("HEAD").exists(), "Home has no copy of this folder's branches");
    Ok(dir)
}

/// The branch tip on Home.
async fn head(app: &App, w: &TaskWorktree) -> Option<String> {
    crate::home_git::tip(app, &w.project_id, &format!("refs/heads/{}", w.branch)).await.ok().flatten()
}

async fn count(app: &App, dir: &Path, range: &str) -> u32 {
    git(app, dir, &["rev-list", "--count", range, "--"]).await.ok().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// A record as the worktree card shows it, from the branch on Home.
async fn info(app: &App, mut w: TaskWorktree, away: &str) -> WorktreeInfo {
    let project_name = crate::projects::get(app, &w.project_id).await.ok().flatten().map(|p| p.name).unwrap_or_else(|| "Removed folder".into());
    let (mut ahead, mut behind) = (0, 0);
    if let (Some(base), Some(head), Ok(dir)) = (w.base_branch.clone(), head(app, &w).await, mirror(app, &w)) {
        ahead = count(app, &dir, &format!("refs/heads/{base}..{head}")).await;
        behind = count(app, &dir, &format!("{head}..refs/heads/{base}")).await;
        // Merged on Home while the runner was away: the record does not know yet.
        if ahead == 0 && w.merged.is_none() && !w.base_commit.is_empty() && count(app, &dir, &format!("{}..{head}", w.base_commit)).await > 0 {
            w.merged = Some(Merged { base: Some(w.base_commit.clone()), into: base, head, note: String::new(), at: crate::util::now_ms() });
        }
    }
    if w.predraft_id.is_none() {
        w.predraft_id = crate::predrafts::list(app).await.ok().and_then(|l| l.into_iter().find(|p| p.project_id == w.project_id && p.branch == w.branch && p.is_open())).map(|p| p.id);
    }
    let home = head(app, &w).await;
    let unpushed = is_unpushed(w.head.as_deref(), home.as_deref(), &w.base_commit);
    let path = crate::worktrees::folder(&w);
    WorktreeInfo { worktree: w, project_name, path, exists: false, dirty_files: 0, ahead, behind, unpushed, head: home, away: Some(away.to_string()) }
}

/// The runner's last known tip (`local`) is not on Home: Home's branch differs, or Home has none while the tip
/// moved past where the branch started.
fn is_unpushed(local: Option<&str>, home: Option<&str>, base_commit: &str) -> bool {
    match (local, home) {
        (Some(l), Some(h)) => l != h,
        (Some(l), None) => l != base_commit,
        (None, _) => false,
    }
}

/// The session's worktrees, for the session view.
pub async fn worktrees(app: &App, s: &Session) -> Result<Vec<WorktreeInfo>> {
    let away = reason(app, s).await;
    let mut out = vec![];
    for w in records(app, &s.id).await? {
        out.push(info(app, w, &away).await);
    }
    Ok(out)
}

/// Every worktree of away sessions, for Review → Agent branches.
pub async fn all_worktrees(app: &App) -> Vec<WorktreeInfo> {
    let mut out = vec![];
    for s in app.sessions.list().into_iter().filter(|s| is_away(app, s)) {
        let Ok(list) = records(app, &s.id).await else { continue };
        if list.is_empty() {
            continue;
        }
        let away = reason(app, &s).await;
        for w in list.into_iter().filter(|w| w.state != WorktreeState::Discarded && w.sessions.last() == Some(&s.id)) {
            out.push(info(app, w, &away).await);
        }
    }
    out
}

/// The commit the branch is compared with: where it meets its base branch on Home.
async fn fork_point(app: &App, dir: &Path, w: &TaskWorktree, head: &str) -> Result<String> {
    match &w.base_branch {
        Some(base) => git(app, dir, &["merge-base", &format!("refs/heads/{base}"), head]).await,
        None => git(app, dir, &["merge-base", &w.base_commit, head]).await,
    }
}

/// Changes: the committed branch on Home compared with its base. Uncommitted files are not on Home.
pub async fn status(app: &App, s: &Session) -> Result<GitStatus> {
    let w = primary(app, &s.id).await?;
    let dir = mirror(app, &w)?;
    let head = head(app, &w).await.with_context(|| format!("{} is not on Home", w.branch))?;
    let base = fork_point(app, &dir, &w, &head).await?;
    let stat = git(app, &dir, &["diff", "--no-ext-diff", "--no-textconv", "--numstat", &base, &head, "--"]).await?;
    let mut changes = GitChanges { files: 0, additions: Some(0), deletions: Some(0) };
    for line in stat.lines() {
        let mut parts = line.split('\t');
        changes.files += 1;
        changes.additions = changes.additions.zip(parts.next().and_then(|n| n.parse::<u32>().ok())).map(|(a, b)| a + b);
        changes.deletions = changes.deletions.zip(parts.next().and_then(|n| n.parse::<u32>().ok())).map(|(a, b)| a + b);
    }
    let names = crate::procs::command(&app.env, "git", Some(&dir)).args(["diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", &base, &head, "--"]).output().await?;
    ensure!(names.status.success(), "could not read the changed files");
    let main_files = String::from_utf8_lossy(&names.stdout).split('\0').filter(|p| !p.is_empty()).map(|p| GitFile { path: p.into(), status: "Δ".into(), previous: None }).collect();
    Ok(GitStatus { branch: w.branch, files: vec![], unpushed_commits: None, main_changes: Some(changes), main_files })
}

/// The commits of the branch on Home.
pub async fn history(app: &App, s: &Session, commit: Option<&str>) -> Result<Option<GitHistory>> {
    let Some(w) = records(app, &s.id).await?.into_iter().find(|w| w.primary) else { return Ok(None) };
    let dir = mirror(app, &w)?;
    crate::session_git::history_in(app, &w, &dir, commit).await.map(Some)
}

/// One file of Changes, from the branch on Home.
pub async fn diff(app: &App, s: &Session, path: &str, main: bool, committed: bool, commit: Option<&str>) -> Result<GitDiff> {
    let w = primary(app, &s.id).await?;
    let dir = mirror(app, &w)?;
    if committed {
        return crate::session_git::committed_diff_in(app, &w, &dir, path, commit).await;
    }
    if !main {
        bail!("{}: its uncommitted files are not on Home", reason(app, s).await);
    }
    let head = head(app, &w).await.with_context(|| format!("{} is not on Home", w.branch))?;
    let base = fork_point(app, &dir, &w, &head).await?;
    let patch = git(app, &dir, &["--literal-pathspecs", "diff", "--no-ext-diff", "--no-textconv", "--no-color", &base, &head, "--", path]).await?;
    Ok(GitDiff { staged: String::new(), truncated: patch.len() > 200_000, unstaged: crate::sessions::truncate_output(&patch, 200_000) })
}

/// A worktree action while the runner is away: Merge runs on Home (only for the `head` the owner reviewed,
/// which must be Home's tip); the others need the folder.
pub async fn worktree_action(app: &App, s: &Session, worktree_id: &str, action: &str, reviewed: Option<&str>) -> Result<WorktreeInfo> {
    let away = reason(app, s).await;
    let w = records(app, &s.id).await?.into_iter().find(|w| w.id == worktree_id).with_context(|| format!("{away}, and Home has no record of this worktree"))?;
    match action {
        "merge" => {
            ensure!(!s.status.is_running() && s.quota_pause.is_none() && !app.sessions.is_busy(s), "Wait for the agent to stop before merging");
            ensure!(w.pull_request.is_none(), "this branch continues a pull request: merge it on GitHub");
            let base = w.base_branch.clone().context("this worktree has no base branch to merge into")?;
            let p = crate::projects::get(app, &w.project_id).await?.context("the folder no longer exists")?;
            let head = head(app, &w).await.with_context(|| format!("{} is not on Home", w.branch))?;
            let reviewed = reviewed.context("Merge needs the reviewed commit (head); reload the page and review again")?;
            ensure!(head == reviewed, "{} changed on Home since review; review again", w.branch);
            if let crate::home_git::MergeOutcome::NeedsRebase { reason, .. } = crate::worktrees::merge_on_home(app, &p, &w.branch, &head, &base).await? {
                bail!("{reason}. {away}: send the agent a message to rebase it, then merge again.");
            }
            Ok(info(app, w, &away).await)
        }
        "restore" => bail!("{away}: its folder comes back from the branch on Home when the session continues (send a message, or move it in Summary → Runs on)."),
        _ => bail!("{away}, so this action cannot run now. Merge and Create Local PR work from its branch on Home."),
    }
}

/// Create Local PR while the runner is away: a Local PR of the branch Home holds.
pub async fn local_pr(app: &App, s: &Session, cwd: &str, title: Option<String>) -> Result<crate::predrafts::Predraft> {
    let away = reason(app, s).await;
    let w = records(app, &s.id).await?.into_iter().find(|w| crate::terminals::inside(cwd, &w.root)).with_context(|| format!("{away}, and Home has no record of this worktree"))?;
    ensure!(w.pull_request.is_none(), "this worktree continues a pull request; push your commits to it instead of creating a Local PR");
    ensure!(head(app, &w).await.is_some(), "{away}, and {} is not on Home", w.branch);
    crate::predrafts::open_for_branch(app, &w.project_id, &w.branch, w.base_branch.as_deref(), Some(&s.id), title).await
}

#[cfg(test)]
mod tests {
    use super::is_unpushed;

    #[test]
    fn unpushed_compares_the_runner_tip_with_home() {
        assert!(!is_unpushed(None, Some("a"), "b"));
        assert!(!is_unpushed(Some("a"), Some("a"), "b"));
        assert!(is_unpushed(Some("c"), Some("a"), "b"));
        // A new branch Home has not received: unsaved only when it has commits.
        assert!(!is_unpushed(Some("b"), None, "b"));
        assert!(is_unpushed(Some("c"), None, "b"));
    }
}

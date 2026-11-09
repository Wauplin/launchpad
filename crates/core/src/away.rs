//! Home's answers for a session whose runner is away (an offline laptop, a stopped Burst job), or whose worktree
//! folder is gone from its runner (retention removed it, or the session moved and its new runner clones it again
//! at the next message).
//!
//! Home still has the branches (Home Git) and the records: runners send each record whenever it changes
//! (`replica::Item::Worktree`, with the branch tip in the folder), and every handoff holds them too. Changes, the
//! worktree list, Merge and Create Local PR use those, so finished work stays reviewable. Changes and Files also
//! show the uncommitted files of the worktree's last snapshot on Home (`worktrees::snapshot`, as of when it was
//! taken); newer work stays on that machine, and a record whose tip differs from Home's branch shows it as not
//! on Home. Home is also the source of truth for what it did itself: a branch's merge (`branch_merge`)
//! and its Local PR, looked up by folder and branch (`from_home`), so they survive a runner's reconnection.

use std::path::{Path, PathBuf};

use serde_json::Value;

use anyhow::{Context, Result, bail, ensure};

use crate::App;
use crate::model::Session;
use crate::predrafts::Predraft;
use crate::session_git::{GitChanges, GitDiff, GitFile, GitHistory, GitStatus};
use crate::worktrees::{Merged, TaskWorktree, WorktreeInfo, WorktreeState};

/// Home: the session runs on a runner that is not connected now.
pub fn is_away(app: &App, s: &Session) -> bool {
    app.paths.role == crate::role::Role::Home && s.runner_id.as_ref().is_some_and(|r| !app.runners.connected(r))
}

/// Why Home answers: "Its Burst runner has stopped", "Its runner (name) is offline", or (runner connected) "Its
/// folder is no longer on its runner".
pub async fn reason(app: &App, s: &Session) -> String {
    if !is_away(app, s) {
        return "Its folder is no longer on its runner".into();
    }
    if crate::burst::needs_new_job(app, s).await {
        return "Its Burst runner has stopped".into();
    }
    let runner = s.runner_id.clone().unwrap_or_default();
    match crate::runners::list(app).await.ok().and_then(|l| l.into_iter().find(|r| r.runner.id == runner)) {
        Some(r) => format!("Its runner ({}) is offline", r.runner.name),
        None => "Its runner is offline".into(),
    }
}

/// The reason shown on worktree cards: only for an away runner (a connected runner still restores and discards).
async fn away_reason(app: &App, s: &Session) -> Option<String> {
    if is_away(app, s) { Some(reason(app, s).await) } else { None }
}

/// The session's worktree records, primary first: the ones its runners sent, then those of its last handoff
/// that Home did not receive on their own, with Home's merge and Local PR.
async fn records(app: &App, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let mut list = records_sent(app, session_id).await?;
    for w in crate::handoff::latest(app, session_id).await?.map(|h| h.worktrees).unwrap_or_default() {
        if !list.iter().any(|x| x.id == w.id) {
            list.push(w);
        }
    }
    list.sort_by_key(|w| (!w.primary, w.created_at));
    let predrafts = crate::predrafts::list(app).await.ok();
    for w in &mut list {
        from_home(app, w, predrafts.as_deref(), None).await;
    }
    Ok(list)
}

/// Home's records win for what Home did: the branch's last merge (`worktrees::merged_on_home`) and its Local PR
/// (the newest one not discarded), looked up by folder and branch. A runner's record does not know about a merge
/// or a Local PR made while it was away. `predrafts`: None when the list could not be read (the record's stays).
/// A merged Local PR stops counting once the branch has commits past what it merged (on Home, or `runner_tip`),
/// so Merge (or a new Local PR) is offered for them.
async fn from_home(app: &App, w: &mut TaskWorktree, predrafts: Option<&[Predraft]>, runner_tip: Option<&str>) {
    if let Some(m) = crate::worktrees::merged_on_home(app, &w.project_id, &w.branch).await {
        w.merged = Some(m);
    }
    if let Some(list) = predrafts {
        let found = list.iter().find(|p| p.project_id == w.project_id && p.branch == w.branch && !p.archived);
        w.predraft_id = match found.and_then(|p| p.merged.as_ref().map(|m| (p, m))) {
            Some((p, m)) => {
                let home = crate::home_git::tip(app, &w.project_id, &format!("refs/heads/{}", w.branch)).await.ok().flatten();
                let past = |t: Option<&str>| t.is_some_and(|t| t != m.head);
                (!past(home.as_deref()) && !past(runner_tip)).then(|| p.id.clone())
            }
            None => found.map(|p| p.id.clone()),
        };
    }
}

/// The worktree's folder is gone from its runner: retention removed it after Home confirmed the branch, or the
/// session moved and its new runner clones it again at the next message. Home answers for it from Home Git,
/// whatever the runner's state.
pub fn gone(w: &TaskWorktree) -> bool {
    w.state == WorktreeState::Removed
}

/// Home: the folder of the session's primary worktree is gone (Changes are then read on Home).
pub async fn primary_gone(app: &App, session_id: &str) -> bool {
    records_sent(app, session_id).await.is_ok_and(|l| l.iter().any(|w| w.primary && gone(w)))
}

/// Home: the folder of this worktree of the session is gone (Merge then runs on Home).
pub async fn worktree_gone(app: &App, session_id: &str, worktree_id: &str) -> bool {
    records_sent(app, session_id).await.is_ok_and(|l| l.iter().any(|w| w.id == worktree_id && gone(w)))
}

/// Home: the folder of the session's worktree that holds `cwd` is gone (Create Local PR then runs on Home).
pub async fn folder_gone(app: &App, session_id: &str, cwd: &str) -> bool {
    records_sent(app, session_id).await.is_ok_and(|l| l.iter().any(|w| crate::terminals::inside(cwd, &w.root) && gone(w)))
}

/// Home: the worktree records runners sent whose folder is `root` or contains it (Files, `code::open`).
pub async fn records_at(app: &App, root: &str) -> Vec<TaskWorktree> {
    let rows: Vec<String> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM runner_worktree")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .unwrap_or_default();
    rows.iter().filter_map(|j| serde_json::from_str::<TaskWorktree>(j).ok()).filter(|w| root == w.root || root.starts_with(&format!("{}/", w.root))).collect()
}

/// Whether every session of a worktree is archived on Home (a session Home does not know yet keeps it).
fn archived(app: &App, w: &TaskWorktree) -> bool {
    w.sessions.iter().all(|id| app.sessions.get(id).is_some_and(|s| s.status == crate::model::SessionStatus::Closed))
}

/// The session's worktree records its runners sent (`replica::Item::Worktree`).
async fn records_sent(app: &App, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let key = session_id.to_string();
    let rows: Vec<String> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT json FROM runner_worktree WHERE instr(json, ?1) > 0")?;
            let rows = st.query_map([key], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(rows.iter().filter_map(|j| serde_json::from_str::<TaskWorktree>(j).ok()).filter(|w| w.sessions.iter().any(|s| s == session_id)).collect())
}

/// Home: stores a worktree record a runner sent, only when its session is one Home placed there and runs there
/// now (not moved on). The record of a worktree whose sessions are all archived goes: their away views use the
/// last handoff.
pub(crate) async fn apply_replica(app: &App, runner_id: &str, n: u64, w: TaskWorktree) -> Result<()> {
    let current = w.sessions.iter().rev().find_map(|id| app.sessions.get(id));
    let fresh = current.is_some_and(|s| s.runner_id.as_deref() == Some(runner_id));
    let gone = fresh && archived(app, &w);
    let (runner, id, json) = (runner_id.to_string(), w.id.clone(), serde_json::to_string(&w)?);
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            if gone {
                tx.execute("DELETE FROM runner_worktree WHERE id = ?1", [&id])?;
            } else if fresh {
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

/// Home, when a session is archived: drops the records of its worktrees whose sessions are all archived.
pub(crate) async fn forget_archived(app: &App, session_id: &str) -> Result<()> {
    let ids: Vec<String> = records_sent(app, session_id).await?.into_iter().filter(|w| archived(app, w)).map(|w| w.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    app.state_db
        .call(move |c| {
            for id in &ids {
                c.execute("DELETE FROM runner_worktree WHERE id = ?1", [id])?;
            }
            Ok(())
        })
        .await?;
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

/// A record as the worktree card shows it, from the branch on Home. `away`: why, when the runner is away.
async fn info(app: &App, mut w: TaskWorktree, away: Option<&str>) -> WorktreeInfo {
    let project_name = crate::projects::get(app, &w.project_id).await.ok().flatten().map(|p| p.name).unwrap_or_else(|| "Removed folder".into());
    let (mut ahead, mut behind) = (0, 0);
    if let (Some(base), Some(head), Ok(dir)) = (w.base_branch.clone(), head(app, &w).await, mirror(app, &w)) {
        ahead = count(app, &dir, &format!("refs/heads/{base}..{head}")).await;
        behind = count(app, &dir, &format!("{head}..refs/heads/{base}")).await;
        // Merged outside Launchpad's merge records (an older merge, a merge on the remote).
        if ahead == 0 && w.merged.is_none() && !w.base_commit.is_empty() && count(app, &dir, &format!("{}..{head}", w.base_commit)).await > 0 {
            w.merged = Some(Merged { base: Some(w.base_commit.clone()), into: base, head, note: String::new(), at: crate::util::now_ms() });
        }
    }
    let home = head(app, &w).await;
    let unpushed = is_unpushed(w.head.as_deref(), home.as_deref(), &w.base_commit);
    let path = crate::worktrees::folder(&w);
    WorktreeInfo { worktree: w, project_name, path, exists: false, dirty_files: 0, ahead, behind, unpushed, head: home, away: away.map(str::to_string), on_home: true }
}

/// Home: a connected runner's worktree list (`/api/sessions/{id}/worktrees`, `/api/worktrees`), completed from
/// Home: the merge and Local PR of each branch from Home's records, and a worktree whose folder is gone described
/// from its branch on Home (its Merge and Create Local PR run on Home).
pub async fn complete(app: &App, list: Vec<Value>) -> Vec<Value> {
    let predrafts = crate::predrafts::list(app).await.ok();
    let mut out = Vec::with_capacity(list.len());
    for mut item in list {
        let Ok(mut w) = serde_json::from_value::<TaskWorktree>(item["worktree"].clone()) else {
            out.push(item);
            continue;
        };
        from_home(app, &mut w, predrafts.as_deref(), item["head"].as_str()).await;
        if gone(&w) {
            out.push(serde_json::to_value(info(app, w, None).await).unwrap_or(item));
            continue;
        }
        // Merged on Home (by the worktree or its Local PR) at the tip the runner shows: nothing ahead, whatever the
        // runner's copy of the base branch says.
        let local_pr = predrafts.iter().flatten().find(|p| w.predraft_id.as_deref() == Some(p.id.as_str())).and_then(|p| p.merged.as_ref());
        let tip = item["head"].as_str();
        if tip.is_some() && (w.merged.as_ref().map(|m| m.head.as_str()) == tip || local_pr.map(|m| m.head.as_str()) == tip) {
            item["ahead"] = Value::from(0);
        }
        if let Ok(v) = serde_json::to_value(&w) {
            item["worktree"] = v;
        }
        out.push(item);
    }
    out
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
    let away = away_reason(app, s).await;
    let mut out = vec![];
    for w in records(app, &s.id).await? {
        out.push(info(app, w, away.as_deref()).await);
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
            out.push(info(app, w, Some(&away)).await);
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

/// Home's snapshot of the worktree's files (`worktrees::snapshot`) when it holds work the branch on Home lacks:
/// the snapshot commit and when it was taken.
async fn snapshot(app: &App, dir: &Path, w: &TaskWorktree, head: &str) -> Option<(String, i64)> {
    let snap = crate::home_git::tip(app, &w.project_id, &crate::worktrees::snapshot_ref(&w.id)).await.ok().flatten()?;
    crate::worktrees::newer_snapshot(app, dir, &snap, head).await?;
    let at = git(app, dir, &["show", "-s", "--format=%ct", &snap]).await.ok()?.parse::<i64>().ok()? * 1000;
    Some((snap, at))
}

/// The files that differ between two commits, with a working-tree status (` M`, ` D`, `??` for new files).
async fn changed_files(app: &App, dir: &Path, from: &str, to: &str) -> Result<Vec<GitFile>> {
    let out = crate::procs::command(&app.env, "git", Some(dir)).args(["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--name-status", "-z", from, to, "--"]).output().await?;
    ensure!(out.status.success(), "could not read the uncommitted files");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split('\0').filter(|p| !p.is_empty());
    let mut files = vec![];
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        let status = if status.starts_with('A') { "??".to_string() } else { format!(" {}", &status[..1]) };
        files.push(GitFile { path: path.into(), status, previous: None });
    }
    Ok(files)
}

/// Changes: the branch on Home compared with its base, and the uncommitted files of Home's last snapshot (as of
/// when it was taken). Newer files stay on the runner.
pub async fn status(app: &App, s: &Session) -> Result<GitStatus> {
    let w = primary(app, &s.id).await?;
    let dir = mirror(app, &w)?;
    let head = head(app, &w).await.with_context(|| format!("{} is not on Home", w.branch))?;
    let base = fork_point(app, &dir, &w, &head).await?;
    let snap = snapshot(app, &dir, &w, &head).await;
    let tip = snap.as_ref().map(|(c, _)| c.clone()).unwrap_or_else(|| head.clone());
    let stat = git(app, &dir, &["diff", "--no-ext-diff", "--no-textconv", "--numstat", &base, &tip, "--"]).await?;
    let mut changes = GitChanges { files: 0, additions: Some(0), deletions: Some(0) };
    for line in stat.lines() {
        let mut parts = line.split('\t');
        changes.files += 1;
        changes.additions = changes.additions.zip(parts.next().and_then(|n| n.parse::<u32>().ok())).map(|(a, b)| a + b);
        changes.deletions = changes.deletions.zip(parts.next().and_then(|n| n.parse::<u32>().ok())).map(|(a, b)| a + b);
    }
    let names = crate::procs::command(&app.env, "git", Some(&dir)).args(["diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", &base, &tip, "--"]).output().await?;
    ensure!(names.status.success(), "could not read the changed files");
    let main_files = String::from_utf8_lossy(&names.stdout).split('\0').filter(|p| !p.is_empty()).map(|p| GitFile { path: p.into(), status: "Δ".into(), previous: None }).collect();
    let files = match &snap {
        Some((c, _)) => changed_files(app, &dir, &head, c).await?,
        None => vec![],
    };
    Ok(GitStatus { branch: w.branch, files, unpushed_commits: None, main_changes: Some(changes), main_files, uncommitted_at: snap.map(|(_, at)| at) })
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
    let head = head(app, &w).await.with_context(|| format!("{} is not on Home", w.branch))?;
    let snap = snapshot(app, &dir, &w, &head).await.map(|(c, _)| c);
    let (from, to) = match (main, snap) {
        (true, snap) => (fork_point(app, &dir, &w, &head).await?, snap.unwrap_or(head)),
        (false, Some(snap)) => (head, snap),
        (false, None) => bail!("{}: Home has no uncommitted files of it", reason(app, s).await),
    };
    let patch = git(app, &dir, &["--literal-pathspecs", "diff", "--no-ext-diff", "--no-textconv", "--no-color", &from, &to, "--", path]).await?;
    Ok(GitDiff { staged: String::new(), truncated: patch.len() > 200_000, unstaged: crate::sessions::truncate_output(&patch, 200_000) })
}

/// A worktree action while the runner is away or the folder is gone: Merge runs on Home (only for the `head` the
/// owner reviewed, which must be Home's tip); the others need the folder.
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
            let mut w = w;
            from_home(app, &mut w, None, None).await;
            Ok(info(app, w, away_reason(app, s).await.as_deref()).await)
        }
        "restore" => bail!("{away}: its folder comes back from the branch on Home when the session continues (send a message, or move it in Summary → Runs on)."),
        _ => bail!("{away}, so this action cannot run now. Merge and Create Local PR work from its branch on Home."),
    }
}

/// Create Local PR while the runner is away or the folder is gone: a Local PR of the branch Home holds.
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

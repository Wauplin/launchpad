//! Best-effort synchronization of task base branches. Never force or retry a push.
use std::{path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use crate::{App, exec::Io};

async fn git(app: &App, repo: &str, args: &[&str]) -> Result<String> {
    git_timeout(app, repo, args, 10).await
}

/// A noninteractive Git command bounded by `secs`.
pub(crate) async fn git_timeout(app: &App, repo: &str, args: &[&str], secs: u64) -> Result<String> {
    let mut cmd = crate::procs::command(&app.env, "git", Some(Path::new(repo)));
    cmd.process_tree().args(args).stdin(Io::Null)
        .env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "true")
        .env("GCM_INTERACTIVE", "Never").env("SSH_ASKPASS", "true").env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    let out = tokio::time::timeout(Duration::from_secs(secs), cmd.output()).await
        .with_context(|| format!("git {} timed out after {secs} seconds; check the remote before retrying", args.first().copied().unwrap_or_default()))??;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn upstream(app: &App, repo: &str, branch: &str) -> Result<Option<(String, String)>> {
    // Task branches stay local. Other base branches use their upstream, or origin with the same name.
    if branch.starts_with("lp/") { return Ok(None); }
    let info = git(app, repo, &["for-each-ref", "--format=%(upstream:remotename)%00%(upstream:remoteref)", &format!("refs/heads/{branch}")]).await?;
    if let Some((remote, reference)) = info.split_once('\0')
        && !remote.is_empty() && !reference.is_empty()
    {
        if remote == "." { return Ok(None); }
        ensure!(!remote.starts_with('-') && reference.starts_with("refs/heads/"), "invalid upstream for {branch}");
        return Ok(Some((remote.to_string(), reference.to_string())));
    }
    if git(app, repo, &["remote"]).await?.lines().any(|r| r == "origin") {
        return Ok(Some(("origin".into(), format!("refs/heads/{branch}"))));
    }
    Ok(None)
}

async fn idle_checkout(app: &App, target: &str, branch: &str) -> Result<()> {
    ensure!(git(app, target, &["branch", "--show-current"]).await? == branch, "the checkout changed branches");
    ensure!(!crate::worktrees::operation_in_progress(app, target).await, "a Git operation is in progress in {target}");
    ensure!(crate::worktrees::dirty_files(app, target).await?.is_empty(), "{target} has uncommitted changes");
    ensure!(!app.sessions.list().iter().any(|s| crate::hosts::loc::within(&s.cwd, target) && app.sessions.is_busy(s)), "an agent is running in {target}");
    Ok(())
}

/// A bounded fast-forward pull before resolving the new worktree's base commit.
/// Dirty or busy checkouts keep their current contents and base.
pub async fn pull(app: &App, repo: &str, branch: &str) -> Option<String> {
    let result = async {
        let Some((remote, reference)) = upstream(app, repo, branch).await? else { return Ok(None) };
        if let Some(target) = crate::worktrees::checkout_of(app, repo, branch).await? {
            idle_checkout(app, &target, branch).await?;
        }
        let local_ref = format!("refs/heads/{branch}");
        let old = git(app, repo, &["rev-parse", "--verify", &local_ref]).await?;
        git(app, repo, &["fetch", "--no-tags", "--no-recurse-submodules", &remote, &reference]).await?;
        let fetched = git(app, repo, &["rev-parse", "FETCH_HEAD"]).await?;
        // Local commits ahead of the remote are fine; diverged branches are left alone.
        if git(app, repo, &["merge-base", "--is-ancestor", &fetched, &old]).await.is_err() {
            git(app, repo, &["merge-base", "--is-ancestor", &old, &fetched]).await.context("local and remote branches have diverged")?;
            // Fetch may take several seconds: recheck the checkout and local head before editing.
            if let Some(target) = crate::worktrees::checkout_of(app, repo, branch).await? {
                idle_checkout(app, &target, branch).await?;
                ensure!(git(app, &target, &["rev-parse", "HEAD"]).await? == old, "{branch} changed during the pull");
                git(app, &target, &["-c", "merge.autoStash=false", "merge", "--ff-only", &fetched]).await?;
            } else {
                git(app, repo, &["update-ref", &local_ref, &fetched, &old]).await?;
            }
        }
        Ok::<_, anyhow::Error>(Some(format!("Remote sync: checked {branch} against {remote}; starting from the local base.")))
    }.await;
    match result {
        Ok(note) => note,
        Err(e) => Some(format!("Remote sync: could not pull {branch}: {e:#}. Launch continues from the local base.")),
    }
}

/// Push exactly the merged commit, with an explicit destination and no force.
pub async fn push(app: &App, repo: &str, branch: &str, head: &str) -> String {
    let result = async {
        let Some((remote, reference)) = upstream(app, repo, branch).await? else { return Ok(None) };
        git(app, repo, &["push", "--no-force", "--no-mirror", "--no-follow-tags", "--recurse-submodules=no", &remote, &format!("{head}:{reference}")]).await?;
        Ok::<_, anyhow::Error>(Some(remote))
    }.await;
    match result {
        Ok(Some(remote)) => format!("Remote sync: pushed {branch} ({}) to {remote}.", &head[..12.min(head.len())]),
        Ok(None) => format!("Remote sync: {branch} was merged locally; no remote destination configured (task branches stay local)."),
        Err(e) => format!("Remote sync: {branch} was merged locally, but pushing failed: {e:#}. Check the remote before retrying; the local merge is kept."),
    }
}

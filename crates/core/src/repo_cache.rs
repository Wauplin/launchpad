//! A runner's repository cache: one clone per Git folder of Home, made on first use (`<data_dir>/repos/<id>`).
//!
//! `origin` is Home Git, reached through this daemon's local Git endpoint (`/git/<id>.git`), which adds the
//! runner's credential: nothing in the clone holds a credential. `upstream` is the folder's remote, fetched
//! first so most objects come from it directly. Task worktrees are linked worktrees of the cache; the
//! cache's own checkout is a read-only view of the default branch (Files, terminals, read-only context).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::App;
use crate::model::Project;

pub fn dir(app: &App, project_id: &str) -> PathBuf {
    app.paths.data_dir.join("repos").join(project_id)
}

/// Home Git as seen from this daemon.
pub fn home_url(app: &App, project_id: &str) -> String {
    crate::git_safe::home_url(app.paths.port, project_id)
}

/// Who a daemon Git request to Home speaks for.
#[derive(Clone, Copy)]
pub enum Auth<'a> {
    /// No credential (local commands, the folder's own remote).
    None,
    /// The daemon, for folder `project_id`.
    Daemon(&'a str),
    /// The session `session_id`, for folder `project_id` (pushes of its worktrees).
    Session(&'a str, &'a str),
}

/// `http.<Home URL>.extraHeader=…`: the credential goes only to Home's exact URL, whatever a repository says.
pub fn auth_config(app: &App, auth: Auth) -> Result<Option<String>> {
    let (project, who) = match auth {
        Auth::None => return Ok(None),
        Auth::Daemon(project) => (project, "daemon".to_string()),
        Auth::Session(project, session) => (project, format!("session:{session}")),
    };
    Ok(Some(format!("http.{}.extraHeader={}: {}", home_url(app, project), crate::home_git::TOKEN_HEADER, crate::home_git::token(app, &who)?)))
}

/// The header a session's worktrees send (their per-worktree Git config).
pub fn session_header(app: &App, session_id: &str) -> Result<String> {
    Ok(format!("{}: {}", crate::home_git::TOKEN_HEADER, crate::home_git::token(app, &format!("session:{session_id}"))?))
}

fn lock(app: &App, project_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    app.suggestion_locks.get(&format!("repo-cache:{project_id}"))
}

/// Git in `dir` with the credential `auth`, bounded by `secs`.
pub async fn git(app: &App, dir: &Path, args: &[&str], auth: Auth<'_>, secs: u64) -> Result<String> {
    let header = auth_config(app, auth)?;
    let mut all: Vec<&str> = vec![];
    if let Some(h) = &header {
        all.extend(["-c", h.as_str()]);
    }
    all.extend_from_slice(args);
    crate::home_git::git_in(app, dir, &all, secs).await
}

/// Makes sure the cache exists and is current: Home refreshes its mirror from the remote, the cache fetches
/// the remote directly (best effort), then Home. Returns the cache folder.
pub async fn sync(app: &App, p: &Project) -> Result<PathBuf> {
    ensure!(p.home_git, "{} is not a Git folder of Home", p.name);
    let lock = lock(app, &p.id);
    let _lock = lock.lock().await;
    let dir = dir(app, &p.id);
    if !dir.join(".git").exists() {
        create(app, p, &dir).await?;
    }
    crate::home_link::refresh_mirror(app, &p.id).await.with_context(|| format!("Home could not refresh {}", p.name))?;
    // The folder's remote as Home knows it, never the clone's own config (agents can write it).
    if let Some(url) = upstream(p)
        && let Err(e) = git(app, &dir, &["fetch", "--quiet", "--no-tags", url, "+refs/heads/*:refs/remotes/upstream/*"], Auth::None, 600).await
    {
        tracing::debug!("{}: direct fetch from the remote failed, Home sends everything: {e:#}", p.name);
    }
    // `origin` is Home's URL in the daemon's own config (`git_safe`).
    let auth = Auth::Daemon(&p.id);
    git(app, &dir, &["fetch", "--quiet", "--prune", "origin"], auth, 900).await.context("fetch from Home Git")?;
    let _ = git(app, &dir, &["remote", "set-head", "origin", "--auto"], auth, 60).await;
    // The view follows the default branch while nobody changed it.
    if git(app, &dir, &["status", "--porcelain"], Auth::None, 60).await.is_ok_and(|s| s.is_empty())
        && app.terminals.alive_in(&dir.to_string_lossy()).is_empty()
    {
        let _ = git(app, &dir, &["checkout", "--quiet", "--detach", "refs/remotes/origin/HEAD"], Auth::None, 120).await;
    }
    Ok(dir)
}

/// The folder's remote, from Home's record.
fn upstream(p: &Project) -> Option<&str> {
    p.remote_url.as_deref().filter(|u| !u.trim().is_empty() && !u.starts_with('-'))
}

async fn create(app: &App, p: &Project, dir: &Path) -> Result<()> {
    let parent = dir.parent().context("cache folder")?;
    tokio::fs::create_dir_all(parent).await?;
    let partial = parent.join(format!(".{}.creating", p.id));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial).await?;
    git(app, &partial, &["init", "--quiet"], Auth::None, 30).await?;
    // The clone's own config is for the agents' Git (daemon Git reads its own, `git_safe`): written as a file.
    let config = partial.join(".git/config");
    let config = config.to_string_lossy();
    let origin = home_url(app, &p.id);
    let mut values = vec![
        ("extensions.worktreeConfig", "true"),
        ("push.default", "upstream"),
        ("advice.detachedHead", "false"),
        ("remote.origin.url", origin.as_str()),
        ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
    ];
    if let Some(url) = upstream(p) {
        values.extend([("remote.upstream.url", url), ("remote.upstream.fetch", "+refs/heads/*:refs/remotes/upstream/*"), ("remote.upstream.pushurl", "no-push")]);
    }
    for (k, v) in values {
        git(app, &partial, &["config", "--file", &config, k, v], Auth::None, 10).await?;
    }
    let _ = tokio::fs::remove_dir_all(dir).await;
    tokio::fs::rename(&partial, dir).await?;
    Ok(())
}

/// Commits every change of a worktree (tracked, untracked, not ignored) into a commit on top of `HEAD`,
/// without touching its index or files. Returns the commit, or `None` when nothing changed (`always`: the commit
/// even then, with `HEAD`'s files).
pub async fn capture(app: &App, worktree: &Path, message: &str, always: bool) -> Result<Option<String>> {
    let index = git(app, worktree, &["rev-parse", "--path-format=absolute", "--git-path", "index"], Auth::None, 30).await?;
    // Where daemon Git can read it (its sandbox has a private `/tmp`).
    let scratch = app.env.git_guard().map(|g| g.scratch_dir()).unwrap_or_else(std::env::temp_dir);
    tokio::fs::create_dir_all(&scratch).await?;
    let tmp = scratch.join(format!("lp-index-{}", uuid::Uuid::new_v4()));
    if Path::new(&index).exists() {
        let (from, to) = (PathBuf::from(&index), tmp.clone());
        tokio::task::spawn_blocking(move || crate::git_safe::copy_regular(&from, &to)).await??;
    }
    let run = |args: Vec<String>| {
        let tmp = tmp.clone();
        async move {
            let mut cmd = crate::procs::command(&app.env, "git", Some(worktree));
            cmd.args(&args).env("GIT_INDEX_FILE", &tmp).stdin(crate::exec::Io::Null);
            // Launchpad's own commits (snapshots, leftovers) never depend on a Git identity.
            for (k, v) in [("GIT_AUTHOR_NAME", "Launchpad"), ("GIT_AUTHOR_EMAIL", "launchpad@localhost"), ("GIT_COMMITTER_NAME", "Launchpad"), ("GIT_COMMITTER_EMAIL", "launchpad@localhost")] {
                cmd.env(k, v);
            }
            let out = tokio::time::timeout(Duration::from_secs(300), cmd.output()).await.context("git timed out")??;
            ensure!(out.status.success(), "git {}: {}", args.first().map(String::as_str).unwrap_or_default(), String::from_utf8_lossy(&out.stderr).trim());
            Ok::<_, anyhow::Error>(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
    };
    let result = async {
        run(vec!["add".into(), "--all".into()]).await?;
        let tree = run(vec!["write-tree".into()]).await?;
        let head_tree = run(vec!["rev-parse".into(), "HEAD^{tree}".into()]).await?;
        if tree == head_tree && !always {
            return Ok(None);
        }
        let commit = run(vec!["commit-tree".into(), tree, "-p".into(), "HEAD".into(), "-m".into(), message.into()]).await?;
        Ok(Some(commit))
    }.await;
    let _ = tokio::fs::remove_file(&tmp).await;
    result
}

/// Removes the caches of folders that left Home.
pub async fn forget(app: &App, project_id: &str) {
    let _ = tokio::fs::remove_dir_all(dir(app, project_id)).await;
}

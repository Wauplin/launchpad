//! Home Git: a bare mirror of every Git folder in use, served to runners as their `origin`.
//!
//! A mirror (`<data_dir>/git/<folder-id>.git`) follows the folder's remote for every branch except Launchpad
//! branches (`lp/…`, `lp-scratch/…`), which only runners push. Only Home writes the mirrored base branches,
//! through Merge or Publish. Every previous tip of a Launchpad branch is kept as a real ref
//! (`refs/kept/<time>/<branch>`) for 30 days, also after force pushes and deletes, with Restore.
//!
//! Runners reach a mirror with Git's smart HTTP protocol (`git http-backend`) through their tunnel's
//! credential; agents push through their runner, never with a credential of their own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::exec::Io;
use crate::model::Project;
use crate::util::now_ms;

/// Previous branch tips and leftover branches are kept this long.
pub const KEEP_MS: i64 = 30 * 24 * 3600 * 1000;

/// Refuses pushes to anything but Launchpad branches and snapshots, except imports by a runner daemon. A
/// session's push (`LP_SESSION=1`) may write only the refs of `LP_SESSION_REFS` (`session_ref_patterns`).
const PRE_RECEIVE: &str = r#"#!/bin/sh
[ "$LP_IMPORT" = 1 ] && exit 0
set -f
while read old new ref; do
  case "$ref" in
    refs/heads/lp/*|refs/heads/lp-scratch/*|refs/lp-snapshots/*) ;;
    *) echo "Home Git: only Launchpad branches (lp/...) can be pushed here; $ref changes through Merge or Publish in Launchpad"; exit 1 ;;
  esac
  if [ "$LP_SESSION" = 1 ]; then
    ok=
    for p in $LP_SESSION_REFS; do
      case "$ref" in $p) ok=1; break ;; esac
    done
    [ -n "$ok" ] || { echo "Home Git: a session may push only its own branches and snapshots; $ref is not one of them"; exit 1; }
  fi
done
"#;

/// Home: what a session's Git token may write in a folder, as shell `case` patterns for the pre-receive hook: the
/// branches named for it (`lp/<slug>-<session8>`) and the branches and snapshots of its worktree records
/// (`branches`, `worktree_ids`), with their `lp-scratch/` leftovers (`<name>`, `<name>-snapshot`,
/// `<name>-gen<n>`). Names with characters Git refuses in refs (or pattern characters) are left out.
pub fn session_ref_patterns(session_id: &str, branches: &[String], worktree_ids: &[String]) -> Vec<String> {
    let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    let short: String = session_id.chars().take(8).collect();
    let mut names = vec![];
    if safe(&short) {
        names.push(format!("*-{short}"));
    }
    names.extend(branches.iter().filter_map(|b| b.strip_prefix("lp/")).filter(|n| safe(n)).map(str::to_string));
    let mut out = vec![];
    for n in names {
        out.push(format!("refs/heads/lp/{n}"));
        out.push(format!("refs/heads/lp-scratch/{n}"));
        out.push(format!("refs/heads/lp-scratch/{n}-snapshot"));
        out.push(format!("refs/heads/lp-scratch/{n}-gen[0-9]*"));
    }
    out.extend(worktree_ids.iter().filter(|id| safe(id) && !id.contains('/')).map(|id| crate::worktrees::snapshot_ref(id)));
    out.sort();
    out.dedup();
    out
}

/// Home: `session_ref_patterns` from Home's records of the session's worktrees in the folder.
pub async fn session_refs(app: &App, project_id: &str, session_id: &str) -> Vec<String> {
    let records: Vec<_> = crate::away::session_records(app, session_id).await.unwrap_or_default().into_iter().filter(|w| w.project_id == project_id).collect();
    let branches: Vec<String> = records.iter().map(|w| w.branch.clone()).collect();
    let ids: Vec<String> = records.iter().map(|w| w.id.clone()).collect();
    session_ref_patterns(session_id, &branches, &ids)
}

pub fn root(app: &App) -> PathBuf {
    app.paths.data_dir.join("git")
}

pub fn mirror(app: &App, project_id: &str) -> PathBuf {
    root(app).join(format!("{project_id}.git"))
}

/// Home Git lives in this daemon (Home).
pub fn here(app: &App) -> bool {
    app.paths.role == crate::role::Role::Home
}

fn lock(app: &App, project_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    app.suggestion_locks.get(&format!("home-git:{project_id}"))
}

/// A noninteractive Git command in `dir`, bounded by `secs`.
pub(crate) async fn git_in(app: &App, dir: &Path, args: &[&str], secs: u64) -> Result<String> {
    git_with_input(app, dir, args, None, secs).await
}

async fn git_with_input(app: &App, dir: &Path, args: &[&str], input: Option<Vec<u8>>, secs: u64) -> Result<String> {
    let mut cmd = crate::procs::command(&app.env, "git", Some(dir));
    if let Some(input) = input {
        cmd.stdin_bytes(input);
    }
    cmd.process_tree().args(args).stdin(Io::Null)
        .env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "true")
        .env("GCM_INTERACTIVE", "Never").env("SSH_ASKPASS", "true").env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    let out = tokio::time::timeout(Duration::from_secs(secs), cmd.output()).await
        .with_context(|| format!("git {} timed out after {secs} seconds", args.first().copied().unwrap_or_default()))??;
    ensure!(out.status.success(), "git {}: {}", args.first().copied().unwrap_or_default(), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn git(app: &App, dir: &Path, args: &[&str]) -> Result<String> {
    git_in(app, dir, args, 60).await
}

/// Where the mirror fetches from: the folder's remote, if it has one.
fn upstream(p: &Project) -> Option<String> {
    p.remote_url.clone().filter(|u| !u.trim().is_empty())
}

/// Creates the mirror of a Git folder on first use and fetches it. Returns its path.
pub async fn ensure(app: &App, p: &Project) -> Result<PathBuf> {
    ensure!(here(app), "Home Git runs on Home");
    let dir = mirror(app, &p.id);
    if dir.join("HEAD").exists() && dir.join("hooks/pre-receive").exists() {
        // Mirrors made by an older Home get the current hook.
        if tokio::fs::read_to_string(dir.join("hooks/pre-receive")).await.ok().as_deref() != Some(PRE_RECEIVE) {
            write_hook(&dir).await?;
        }
        restore_pending(app, p).await?;
        return Ok(dir);
    }
    let lock = lock(app, &p.id);
    let _lock = lock.lock().await;
    if dir.join("hooks/pre-receive").exists() {
        drop(_lock);
        restore_pending(app, p).await?;
        return Ok(dir);
    }
    let partial = root(app).join(format!(".{}.creating", p.id));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial).await?;
    git(app, &partial, &["init", "--quiet", "--bare", "--initial-branch=main"]).await?;
    for (k, v) in [("gc.auto", "0"), ("receive.denyDeleteCurrent", "true"), ("uploadpack.allowAnySHA1InWant", "true"), ("core.logAllRefUpdates", "false")] {
        git(app, &partial, &["config", k, v]).await?;
    }
    if let Some(url) = upstream(p) {
        git(app, &partial, &["remote", "add", "origin", &url]).await?;
        git(app, &partial, &["config", "--replace-all", "remote.origin.fetch", "+refs/heads/*:refs/heads/*"]).await?;
        for negative in ["^refs/heads/lp/*", "^refs/heads/lp-scratch/*"] {
            git(app, &partial, &["config", "--add", "remote.origin.fetch", negative]).await?;
        }
        git(app, &partial, &["config", "--add", "remote.origin.fetch", "+refs/tags/*:refs/tags/*"]).await?;
    }
    write_hook(&partial).await?;
    tokio::fs::create_dir_all(root(app)).await?;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::rename(&partial, &dir).await?;
    drop(_lock);
    if upstream(p).is_some() {
        refresh(app, p).await.with_context(|| format!("first fetch of {}", p.name))?;
    }
    // A Home restored from a backup gets its Launchpad branches back.
    crate::home_backup::restore_bundle(app, &p.id).await?;
    Ok(dir)
}

/// A restored mirror whose bundle is not applied yet (an earlier fetch or download failed): fetch the remote the
/// bundle builds on, then apply it. Every use of the mirror tries again until it worked.
async fn restore_pending(app: &App, p: &Project) -> Result<()> {
    if !crate::home_backup::bundle_pending(app, &p.id) {
        return Ok(());
    }
    if upstream(p).is_some() {
        fetch_origin(app, p).await.with_context(|| format!("fetch {} before applying its Home Git bundle", p.name))?;
    }
    crate::home_backup::restore_bundle(app, &p.id).await
}

/// Where a bundle is fetched first, so it never overwrites a ref pushed since the restore.
const RESTORE_NS: &str = "refs/lp-restore/";

/// Applies a backup bundle to a mirror: fetched under `RESTORE_NS`, then only the refs the mirror does not have
/// are created.
pub(crate) async fn apply_bundle(app: &App, project_id: &str, bundle: &Path) -> Result<()> {
    let dir = mirror(app, project_id);
    let lock = lock(app, project_id);
    let _lock = lock.lock().await;
    let refs = "--format=%(objectname) %(refname)";
    let clear = |listed: String| listed.lines().filter_map(|l| l.split_once(' ')).map(|(_, r)| format!("delete {r}\n")).collect::<String>();
    let leftover = clear(git(app, &dir, &["for-each-ref", refs, RESTORE_NS]).await?);
    if !leftover.is_empty() {
        git_with_input(app, &dir, &["update-ref", "--stdin"], Some(leftover.into_bytes()), 60).await?;
    }
    let source = bundle.to_string_lossy();
    git_in(app, &dir, &["fetch", "--quiet", "--no-tags", &source, &format!("+refs/*:{RESTORE_NS}*")], 600).await?;
    let restored = git(app, &dir, &["for-each-ref", refs, RESTORE_NS]).await?;
    let existing = git(app, &dir, &["for-each-ref", refs]).await?;
    let updates = restore_updates(&restored, &existing) + &clear(restored);
    if !updates.is_empty() {
        git_with_input(app, &dir, &["update-ref", "--stdin"], Some(updates.into_bytes()), 60).await?;
    }
    browse_stale(project_id);
    Ok(())
}

/// `update-ref --stdin` lines that create each restored ref (`<sha> refs/lp-restore/<ref>` lines) the mirror does
/// not have (`<sha> <ref>` lines). `create` fails if a push created the ref meanwhile: the next try skips it.
fn restore_updates(restored: &str, existing: &str) -> String {
    let have: std::collections::HashSet<&str> = existing.lines().filter_map(|l| l.split_once(' ')).map(|(_, r)| r).collect();
    restored
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter_map(|(sha, r)| Some((sha, format!("refs/{}", r.strip_prefix(RESTORE_NS)?))))
        // A bundle taken while another was being applied may hold leftovers of that one.
        .filter(|(_, r)| !r.starts_with(RESTORE_NS) && !have.contains(r.as_str()))
        .map(|(sha, r)| format!("create {r} {sha}\n"))
        .collect()
}

/// Writes the hook beside it and renames it into place, so a push never runs a half-written (empty, accepting) hook.
async fn write_hook(dir: &Path) -> Result<()> {
    let hook = dir.join("hooks/pre-receive");
    tokio::fs::create_dir_all(hook.parent().unwrap()).await?;
    let tmp = dir.join(format!("hooks/.pre-receive.{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, PRE_RECEIVE).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
    }
    tokio::fs::rename(&tmp, &hook).await?;
    Ok(())
}

/// Fetches the folder's remote into the mirror (every branch but Launchpad branches) and follows its
/// default branch. Launchpad branches never change here.
pub async fn refresh(app: &App, p: &Project) -> Result<()> {
    fetch_origin(app, p).await?;
    // Followed branches that did not reach GitHub yet get another push, in the background: a refresh (a runner's
    // clone, a Local PR's Refresh) never waits for GitHub.
    let (app, id) = (app.clone(), p.id.clone());
    tokio::spawn(async move { retry_follows(&app, &id).await });
    Ok(())
}

/// When each mirror last fetched its remote (a merge skips the fetch of a mirror fetched seconds ago).
static FETCHED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::LazyLock::new(Default::default);

/// Whether the mirror fetched its remote within `within`.
fn fetched_within(project_id: &str, within: Duration) -> bool {
    FETCHED.lock().unwrap().get(project_id).is_some_and(|t| t.elapsed() < within)
}

async fn fetch_origin(app: &App, p: &Project) -> Result<()> {
    let dir = mirror(app, &p.id);
    ensure!(dir.join("HEAD").exists(), "{} has no mirror on Home yet", p.name);
    let lock = lock(app, &p.id);
    let _lock = lock.lock().await;
    if git(app, &dir, &["remote"]).await?.lines().all(|r| r != "origin") {
        return Ok(());
    }
    git_in(app, &dir, &["fetch", "--quiet", "--prune", "--no-recurse-submodules", "origin"], 15 * 60).await?;
    FETCHED.lock().unwrap().insert(p.id.clone(), std::time::Instant::now());
    if let Ok(out) = git_in(app, &dir, &["ls-remote", "--symref", "origin", "HEAD"], 60).await
        && let Some(branch) = out.lines().find_map(|l| l.strip_prefix("ref: refs/heads/").and_then(|s| s.split_once('\t').map(|(b, _)| b.to_string())))
        && git(app, &dir, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok()
    {
        git(app, &dir, &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")]).await?;
    }
    Ok(())
}

/// Home: where Files reads a Git folder. The mirror is bare, so Home keeps a read-only checkout of the
/// default branch that borrows the mirror's objects, with every branch as `origin/<branch>`.
pub fn browse_dir(app: &App, project_id: &str) -> PathBuf {
    app.paths.data_dir.join("browse").join(project_id)
}

/// When each browse copy was last updated.
static BROWSE_FRESH: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::LazyLock::new(Default::default);

/// A push or merge changed the folder's branches: the next read updates its browse copy at once.
fn browse_stale(project_id: &str) {
    BROWSE_FRESH.lock().unwrap().remove(project_id);
}

/// Home: creates or updates the browse copy of a Git folder (at most every 30 seconds, and at once after a push
/// or merge). Returns its path.
pub async fn browse_copy(app: &App, p: &Project) -> Result<PathBuf> {
    let mirror = ensure(app, p).await?;
    let dir = browse_dir(app, &p.id);
    let lock = app.suggestion_locks.get(&format!("home-browse:{}", p.id));
    let _lock = lock.lock().await;
    if dir.join(".git").exists() && BROWSE_FRESH.lock().unwrap().get(&p.id).is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
        return Ok(dir);
    }
    let branch = default_branch(app, &p.id).await.unwrap_or_else(|_| "main".into());
    if !dir.join(".git").exists() {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.parent().context("browse folder")?).await?;
        let (m, d) = (mirror.to_string_lossy().into_owned(), dir.to_string_lossy().into_owned());
        git_in(app, dir.parent().unwrap(), &["clone", "--quiet", "--shared", "--no-checkout", &m, &d], 600).await?;
    } else {
        git_in(app, &dir, &["fetch", "--quiet", "--prune", "origin"], 600).await?;
    }
    // An empty mirror (no commit yet) has nothing to show.
    if git(app, &dir, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).await.is_ok() {
        git(app, &dir, &["checkout", "--quiet", "--force", "--detach", &format!("origin/{branch}")]).await?;
    }
    BROWSE_FRESH.lock().unwrap().insert(p.id.clone(), std::time::Instant::now());
    Ok(dir)
}

/// The folder's default branch on Home.
pub async fn default_branch(app: &App, project_id: &str) -> Result<String> {
    let dir = mirror(app, project_id);
    let head = git(app, &dir, &["symbolic-ref", "--short", "HEAD"]).await?;
    Ok(head)
}

/// Base branches (everything but Launchpad branches), most recent first, with the default branch first.
pub async fn base_branches(app: &App, project_id: &str) -> Result<(Option<String>, Vec<String>)> {
    let dir = mirror(app, project_id);
    let current = default_branch(app, project_id).await.ok();
    let list = git(app, &dir, &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"]).await?;
    let mut names: Vec<String> = list.lines().filter(|b| !b.is_empty() && !b.starts_with("lp/") && !b.starts_with("lp-scratch/")).map(str::to_string).collect();
    if let Some(c) = &current {
        names.retain(|b| b != c);
        names.insert(0, c.clone());
    }
    Ok((current, names))
}

/// Commit of a ref in the mirror, `None` when it does not exist.
pub async fn tip(app: &App, project_id: &str, refname: &str) -> Result<Option<String>> {
    let dir = mirror(app, project_id);
    Ok(git(app, &dir, &["rev-parse", "--verify", "--quiet", &format!("{refname}^{{commit}}")]).await.ok().filter(|s| !s.is_empty()))
}

/// Launchpad branch tips: `refs/heads/…` → commit.
pub async fn launchpad_refs(app: &App, project_id: &str) -> Result<BTreeMap<String, String>> {
    let dir = mirror(app, project_id);
    let out = git(app, &dir, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads/lp/", "refs/heads/lp-scratch/"]).await?;
    Ok(out.lines().filter_map(|l| l.split_once(' ').map(|(r, c)| (r.to_string(), c.to_string()))).collect())
}

/// After a push: keeps the previous tip of every changed Launchpad branch, forwards followed branches to
/// GitHub and tells windows. `session`: the pushing session, if any. `runner`: a runner pushed (else Home itself):
/// a followed branch goes to GitHub only when the pushing session is the follow's or works on that branch by
/// Home's records, never for a push with the runner daemon's own token.
pub async fn after_push(app: &App, project_id: &str, before: &BTreeMap<String, String>, session: Option<&str>, runner: bool) -> Result<()> {
    let after = launchpad_refs(app, project_id).await?;
    let dir = mirror(app, project_id);
    let at = now_ms();
    let mut changed = vec![];
    for (refname, old) in before {
        if after.get(refname) != Some(old) {
            let branch = refname.trim_start_matches("refs/heads/");
            git(app, &dir, &["update-ref", &format!("refs/kept/{at}/{branch}"), old]).await?;
            changed.push(branch.to_string());
        }
    }
    changed.extend(after.keys().filter(|r| !before.contains_key(*r)).map(|r| r.trim_start_matches("refs/heads/").to_string()));
    // Leftover branches age from their last push, not from their commits (`prune`).
    let scratch: Vec<(String, bool)> = changed.iter().filter(|b| b.starts_with("lp-scratch/")).map(|b| (b.clone(), after.contains_key(&format!("refs/heads/{b}")))).collect();
    if !scratch.is_empty() {
        let project = project_id.to_string();
        let recorded = app
            .state_db
            .call(move |c| {
                for (branch, present) in &scratch {
                    if *present {
                        c.execute("INSERT OR REPLACE INTO scratch_push (project_id, branch, at) VALUES (?1, ?2, ?3)", rusqlite::params![project, branch, at])?;
                    } else {
                        c.execute("DELETE FROM scratch_push WHERE project_id = ?1 AND branch = ?2", [&project, branch])?;
                    }
                }
                Ok(())
            })
            .await;
        if let Err(e) = recorded {
            tracing::warn!("push times of leftover branches: {e:#}");
        }
    }
    // Also for a push of snapshots only (`refs/lp-snapshots/…`): backups bundle them.
    crate::home_backup::changed();
    if changed.is_empty() {
        return Ok(());
    }
    browse_stale(project_id);
    // A deleted branch (Discard, cleanup) takes its merge record with it (`worktrees::merge_on_home`).
    let deleted: Vec<String> = changed.iter().filter(|b| !after.contains_key(&format!("refs/heads/{b}"))).cloned().collect();
    if !deleted.is_empty() {
        let project = project_id.to_string();
        let pruned = app
            .state_db
            .call(move |c| {
                for branch in &deleted {
                    c.execute("DELETE FROM branch_merge WHERE project_id = ?1 AND branch = ?2", [&project, branch])?;
                }
                Ok(())
            })
            .await;
        if let Err(e) = pruned {
            tracing::warn!("merge records of deleted branches: {e:#}");
        }
    }
    app.bus.publish("home_git", "pushed", serde_json::json!({"project_id": project_id, "branches": changed, "session_id": session}));
    for branch in &changed {
        if after.contains_key(&format!("refs/heads/{branch}")) && let Some(f) = follow(app, project_id, branch).await? {
            let owner = match session {
                Some(sid) => f.session_id.as_deref() == Some(sid) || crate::away::session_branch(app, sid, project_id, branch).await.ok().flatten().is_some(),
                None => !runner,
            };
            if !owner {
                tracing::warn!("{branch} follows {}: a push by {} does not go to GitHub", f.url, session.map(|s| format!("session {s}, which does not work on it")).unwrap_or_else(|| "a runner daemon".into()));
                continue;
            }
            let (app, project_id, branch) = (app.clone(), project_id.to_string(), branch.clone());
            tokio::spawn(async move { forward(&app, &project_id, &branch).await });
        }
    }
    Ok(())
}

/// A Launchpad branch whose commits Home pushes to GitHub after each push from a runner: a published Local
/// PR or a session that continues a GitHub pull request. Never forced.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Follow {
    /// Remote name in the mirror or URL.
    pub remote: String,
    pub branch: String,
    /// The pull request, for notices.
    pub url: String,
    /// The session told about each push, when there is one.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The last commit GitHub accepted from Home (or had when the follow started).
    #[serde(default)]
    pub forwarded: Option<String>,
    /// Why the last push to GitHub failed; None after a successful one.
    #[serde(default)]
    pub error: Option<String>,
}

/// Whether GitHub has a followed branch's commit on Home, for the Local PR and pull request views.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FollowStatus {
    pub url: String,
    /// The branch's commit on Home.
    pub head: Option<String>,
    pub forwarded: Option<String>,
    /// Home has commits that are not on GitHub yet.
    pub behind: bool,
    pub error: Option<String>,
}

pub async fn set_follow(app: &App, project_id: &str, branch: &str, follow: &Follow) -> Result<()> {
    let (p, b, json) = (project_id.to_string(), branch.to_string(), serde_json::to_string(follow)?);
    app.state_db.call(move |c| {
        c.execute("INSERT INTO git_follow (project_id, branch, json) VALUES (?1, ?2, ?3) ON CONFLICT (project_id, branch) DO UPDATE SET json = ?3", [p, b, json])?;
        Ok(())
    }).await
}

pub async fn follow(app: &App, project_id: &str, branch: &str) -> Result<Option<Follow>> {
    let (p, b) = (project_id.to_string(), branch.to_string());
    let json: Option<String> = app.state_db.call(move |c| {
        use rusqlite::OptionalExtension;
        Ok(c.query_row("SELECT json FROM git_follow WHERE project_id = ?1 AND branch = ?2", [p, b], |r| r.get(0)).optional()?)
    }).await?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

pub async fn follow_status(app: &App, project_id: &str, branch: &str) -> Result<Option<FollowStatus>> {
    let Some(f) = follow(app, project_id, branch).await? else { return Ok(None) };
    let head = tip(app, project_id, &format!("refs/heads/{branch}")).await?;
    let behind = head.is_some() && head != f.forwarded;
    Ok(Some(FollowStatus { url: f.url, head, forwarded: f.forwarded, behind, error: f.error }))
}

/// Pushes the current commit of a followed branch to GitHub, without force; one push at a time per branch,
/// so pushes never arrive out of order. The record keeps the last commit GitHub accepted and the last error,
/// so a refresh retries (`retry_follows`). The session hears of each push, and of each new failure once.
async fn forward(app: &App, project_id: &str, branch: &str) {
    let dir = mirror(app, project_id);
    let lock = app.suggestion_locks.get(&format!("home-git-forward:{project_id}:{branch}"));
    let _lock = lock.lock().await;
    let (Ok(Some(mut follow)), Ok(Some(head))) = (follow(app, project_id, branch).await, tip(app, project_id, &format!("refs/heads/{branch}")).await) else { return };
    if follow.forwarded.as_deref() == Some(head.as_str()) {
        return;
    }
    let text = match git_in(app, &dir, &["push", "--quiet", &follow.remote, &format!("{head}:refs/heads/{}", follow.branch)], 300).await {
        Ok(_) => {
            follow.forwarded = Some(head.clone());
            follow.error = None;
            Some(format!("Home pushed {} to {} ({}).", &head[..12.min(head.len())], follow.url, follow.branch))
        }
        Err(e) => {
            let error = format!("{e:#}");
            let new = follow.error.as_deref() != Some(error.as_str());
            follow.error = Some(error.clone());
            new.then(|| format!("Home could not push {branch} to {}: {error}. Rebase onto the pull request branch (`git pull --rebase` from it) and push to Home again; Home never force-pushes.", follow.url))
        }
    };
    if let Err(e) = set_follow(app, project_id, branch, &follow).await {
        tracing::warn!("follow record of {branch}: {e:#}");
    }
    if let (Some(session), Some(text)) = (&follow.session_id, text) {
        crate::sessions::notice_anywhere(app, session, &text).await;
    }
}

/// Pushes again every followed branch of a folder whose commit GitHub does not have yet (pushes are never
/// forced, so retrying is safe). Called after each refresh of the mirror.
pub async fn retry_follows(app: &App, project_id: &str) {
    let p = project_id.to_string();
    let rows: Result<Vec<(String, String)>> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT branch, json FROM git_follow WHERE project_id = ?1")?;
            let rows = st.query_map([p], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await;
    for (branch, json) in rows.unwrap_or_default() {
        let Ok(f) = serde_json::from_str::<Follow>(&json) else { continue };
        let head = tip(app, project_id, &format!("refs/heads/{branch}")).await.ok().flatten();
        if head.is_some() && head != f.forwarded {
            forward(app, project_id, &branch).await;
        }
    }
}

/// A Launchpad branch on Home with its previous tips.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct HomeBranch {
    pub name: String,
    /// None: deleted; only previous tips remain.
    pub head: Option<String>,
    pub subject: String,
    #[ts(type = "number")]
    pub committed_at: i64,
    pub kept: Vec<KeptTip>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct KeptTip {
    pub commit: String,
    pub subject: String,
    /// When the branch moved away from this commit.
    #[ts(type = "number")]
    pub replaced_at: i64,
}

/// Launchpad branches and their kept tips, newest first.
pub async fn branches(app: &App, project_id: &str) -> Result<Vec<HomeBranch>> {
    let dir = mirror(app, project_id);
    if !dir.join("HEAD").exists() {
        return Ok(vec![]);
    }
    let fmt = "--format=%(refname)%00%(objectname)%00%(subject)%00%(committerdate:unix)";
    let heads = git(app, &dir, &["for-each-ref", fmt, "refs/heads/lp/", "refs/heads/lp-scratch/"]).await?;
    let kept = git(app, &dir, &["for-each-ref", fmt, "refs/kept/"]).await?;
    let mut out: BTreeMap<String, HomeBranch> = BTreeMap::new();
    for line in heads.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 4 { continue }
        let name = f[0].trim_start_matches("refs/heads/").to_string();
        out.insert(name.clone(), HomeBranch { name, head: Some(f[1].into()), subject: f[2].into(), committed_at: f[3].parse::<i64>().unwrap_or(0) * 1000, kept: vec![] });
    }
    for line in kept.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 4 { continue }
        let Some((at, name)) = f[0].trim_start_matches("refs/kept/").split_once('/') else { continue };
        let b = out.entry(name.to_string()).or_insert_with(|| HomeBranch { name: name.to_string(), head: None, subject: String::new(), committed_at: 0, kept: vec![] });
        b.kept.push(KeptTip { commit: f[1].into(), subject: f[2].into(), replaced_at: at.parse().unwrap_or(0) });
    }
    let mut list: Vec<HomeBranch> = out.into_values().collect();
    for b in &mut list {
        b.kept.sort_by_key(|k| std::cmp::Reverse(k.replaced_at));
        b.kept.dedup_by(|a, b| a.commit == b.commit);
        if b.committed_at == 0 {
            b.committed_at = b.kept.first().map(|k| k.replaced_at).unwrap_or(0);
        }
    }
    list.sort_by_key(|b| std::cmp::Reverse(b.committed_at));
    Ok(list)
}

/// Restore: points a Launchpad branch back at one of its kept tips. The current tip is kept in turn.
pub async fn restore(app: &App, project_id: &str, branch: &str, commit: &str) -> Result<()> {
    ensure!(branch.starts_with("lp/") || branch.starts_with("lp-scratch/"), "only Launchpad branches can be restored");
    let dir = mirror(app, project_id);
    let kept = git(app, &dir, &["for-each-ref", "--format=%(objectname)", &format!("--points-at={commit}"), "refs/kept/"]).await?;
    ensure!(!kept.is_empty(), "this commit is not a kept tip of {branch}");
    let before = launchpad_refs(app, project_id).await?;
    git(app, &dir, &["update-ref", &format!("refs/heads/{branch}"), commit]).await?;
    after_push(app, project_id, &before, None, false).await
}

/// Deletes a Launchpad branch on Home; its tip stays restorable. `worktree`: the snapshot of that worktree
/// (`refs/lp-snapshots/<id>`) goes too.
pub async fn delete(app: &App, project_id: &str, branch: &str, worktree: Option<&str>) -> Result<()> {
    ensure!(branch.starts_with("lp/") || branch.starts_with("lp-scratch/"), "only Launchpad branches can be deleted");
    if let Some(id) = worktree {
        ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "invalid worktree");
        let r = crate::worktrees::snapshot_ref(id);
        if tip(app, project_id, &r).await?.is_some() {
            git(app, &mirror(app, project_id), &["update-ref", "-d", &r]).await?;
            crate::home_backup::changed();
        }
    }
    let before = launchpad_refs(app, project_id).await?;
    if before.contains_key(&format!("refs/heads/{branch}")) {
        git(app, &mirror(app, project_id), &["update-ref", "-d", &format!("refs/heads/{branch}")]).await?;
        after_push(app, project_id, &before, None, false).await?;
    }
    Ok(())
}

/// The outcome of a merge on Home.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// The base branch now points at `head`; `note` tells how the push to the remote went.
    Merged { head: String, note: String },
    /// The base branch moved: the branch must be rebased onto it first (by its agent).
    NeedsRebase { base: String, reason: String },
}

/// Merge on Home: pushes the reviewed `head` of `branch` to `base` on the folder's remote (never forced), then
/// fast-forwards `base` on Home. The merge counts only once the remote accepted it: the mirror follows the
/// remote, so a base moved only on Home would go back at the next refresh. A refused push fails with its
/// reason and changes nothing. A branch behind `base` must be rebased by its agent first.
pub async fn merge(app: &App, p: &Project, branch: &str, head: &str, base: &str) -> Result<MergeOutcome> {
    ensure!(!base.starts_with('-') && !base.starts_with("lp/"), "invalid base branch {base}");
    // A mirror fetched seconds ago (the runner's push, a refresh) is fresh enough: the push to the remote is never
    // forced, so a base that moved since is refused there.
    if !fetched_within(&p.id, Duration::from_secs(30)) {
        let _ = refresh(app, p).await;
    }
    let lock = lock(app, &p.id);
    let _lock = lock.lock().await;
    let dir = mirror(app, &p.id);
    let current = tip(app, &p.id, &format!("refs/heads/{branch}")).await?.with_context(|| format!("{branch} is not on Home; push it first"))?;
    ensure!(current == head, "{branch} changed since review; review again");
    let base_tip = tip(app, &p.id, &format!("refs/heads/{base}")).await?.with_context(|| format!("{base} does not exist on Home"))?;
    if base_tip == head || git(app, &dir, &["merge-base", "--is-ancestor", head, &base_tip]).await.is_ok() {
        return Ok(MergeOutcome::Merged { head: base_tip, note: format!("{branch} is already part of {base}.") });
    }
    if git(app, &dir, &["merge-base", "--is-ancestor", &base_tip, head]).await.is_err() {
        return Ok(MergeOutcome::NeedsRebase { base: base.into(), reason: format!("{base} moved on Home ({}); {branch} must be rebased onto it", &base_tip[..12.min(base_tip.len())]) });
    }
    let remote = git(app, &dir, &["remote"]).await?.lines().any(|r| r == "origin");
    if remote {
        git_in(app, &dir, &["push", "--quiet", "origin", &format!("{head}:refs/heads/{base}")], 300)
            .await
            .with_context(|| format!("the folder's remote refused {base}, so nothing was merged (a protected branch needs a pull request: publish a Local PR instead)"))?;
    }
    git(app, &dir, &["update-ref", &format!("refs/heads/{base}"), head, &base_tip]).await?;
    crate::home_backup::changed();
    let note = if remote { format!("Merged into {base} and pushed to the remote.") } else { format!("Merged into {base} on Home (this folder has no remote).") };
    browse_stale(&p.id);
    app.bus.publish("home_git", "merged", serde_json::json!({"project_id": p.id, "branch": branch, "base": base}));
    Ok(MergeOutcome::Merged { head: head.into(), note })
}

/// Fetches an open pull request's head into a new Launchpad branch, for a session that continues it.
pub async fn pull_request(app: &App, p: &Project, number: u64, branch: &str) -> Result<String> {
    ensure!(branch.starts_with("lp/"), "invalid branch");
    let dir = ensure(app, p).await?;
    let lock = lock(app, &p.id);
    let _lock = lock.lock().await;
    let before = launchpad_refs(app, &p.id).await?;
    git_in(app, &dir, &["fetch", "--quiet", "--no-tags", "origin", &format!("+refs/pull/{number}/head:refs/heads/{branch}")], 300).await
        .with_context(|| format!("could not fetch pull request #{number}"))?;
    drop(_lock);
    after_push(app, &p.id, &before, None, false).await?;
    tip(app, &p.id, &format!("refs/heads/{branch}")).await?.context("pull request head missing")
}

/// Pushes a commit from the mirror to a remote branch (Publish). Never forced.
pub async fn push(app: &App, project_id: &str, remote: &str, commit: &str, branch: &str) -> Result<()> {
    ensure!(!remote.starts_with('-') && !branch.starts_with('-'), "invalid remote");
    git_in(app, &mirror(app, project_id), &["push", "--quiet", remote, &format!("{commit}:refs/heads/{branch}")], 300).await.map(|_| ())
}

/// Whether `prune` deletes a leftover branch or snapshot: its last push (when Home recorded one) and its commit
/// are both older than `cutoff` (milliseconds).
fn scratch_expired(committed_at: i64, pushed_at: Option<i64>, cutoff: i64) -> bool {
    committed_at.max(pushed_at.unwrap_or(0)) < cutoff
}

/// Deletes kept tips older than 30 days, and leftover branches and snapshots 30 days after their last push (or
/// commit); a leftover branch's tip stays restorable under `refs/kept/` for 30 more days.
pub async fn prune(app: &App) -> Result<()> {
    let Ok(mut entries) = tokio::fs::read_dir(root(app)).await else { return Ok(()) };
    let now = now_ms();
    let cutoff = now - KEEP_MS;
    while let Some(e) = entries.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(project_id) = name.strip_suffix(".git").map(str::to_string) else { continue };
        let dir = e.path();
        let project = project_id.clone();
        let pushed: BTreeMap<String, i64> = app
            .state_db
            .call(move |c| {
                let mut st = c.prepare("SELECT branch, at FROM scratch_push WHERE project_id = ?1")?;
                let rows = st.query_map([&project], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
                Ok(rows)
            })
            .await?;
        let kept = git(app, &dir, &["for-each-ref", "--format=%(refname)", "refs/kept/"]).await.unwrap_or_default();
        for r in kept.lines() {
            if r.trim_start_matches("refs/kept/").split('/').next().and_then(|t| t.parse::<i64>().ok()).is_some_and(|t| t < cutoff) {
                git(app, &dir, &["update-ref", "-d", r]).await?;
            }
        }
        // Leftover branches and snapshots of uncommitted files (`worktrees::snapshot`, re-taken at every change).
        let scratch = git(app, &dir, &["for-each-ref", "--format=%(refname) %(objectname) %(committerdate:unix)", "refs/heads/lp-scratch/", "refs/lp-snapshots/"]).await.unwrap_or_default();
        for l in scratch.lines() {
            let [r, commit, t] = l.split(' ').collect::<Vec<_>>()[..] else { continue };
            let Ok(t) = t.parse::<i64>() else { continue };
            let branch = r.strip_prefix("refs/heads/");
            if !scratch_expired(t * 1000, branch.and_then(|b| pushed.get(b).copied()), cutoff) {
                continue;
            }
            if let Some(b) = branch {
                git(app, &dir, &["update-ref", &format!("refs/kept/{now}/{b}"), commit]).await?;
            }
            // Only if it did not move meanwhile.
            if git(app, &dir, &["update-ref", "-d", r, commit]).await.is_err() {
                continue;
            }
            if let Some(b) = branch.map(str::to_string) {
                let project = project_id.clone();
                app.state_db.call(move |c| Ok(c.execute("DELETE FROM scratch_push WHERE project_id = ?1 AND branch = ?2", [&project, &b])?)).await?;
                browse_stale(&project_id);
            }
            crate::home_backup::changed();
        }
    }
    Ok(())
}

/// Removes the mirror of a folder that left Home.
pub async fn forget(app: &App, project_id: &str) {
    let _ = tokio::fs::remove_dir_all(mirror(app, project_id)).await;
}

pub fn spawn(app: &App) {
    if !here(app) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            tick.tick().await;
            if let Err(e) = crate::predrafts::close_contained(&app).await {
                tracing::warn!("Local PRs merged outside Launchpad: {e:#}");
            }
            if let Err(e) = prune(&app).await {
                tracing::warn!("Home Git prune: {e:#}");
            }
        }
    });
}

/// Environment of `git http-backend` for one request on a mirror.
/// `session_refs`: a session's push, which may write only these refs (`session_ref_patterns`); None for the daemon.
#[allow(clippy::too_many_arguments)]
pub fn backend_env(app: &App, path_info: &str, method: &str, query: &str, content_type: Option<&str>, encoding: Option<&str>, protocol: Option<&str>, import: bool, session_refs: Option<&[String]>) -> Vec<(String, String)> {
    let mut env = vec![
        ("GIT_PROJECT_ROOT".to_string(), root(app).to_string_lossy().into_owned()),
        ("GIT_HTTP_EXPORT_ALL".into(), "1".into()),
        ("PATH_INFO".into(), path_info.into()),
        ("REQUEST_METHOD".into(), method.into()),
        ("QUERY_STRING".into(), query.into()),
        ("REMOTE_USER".into(), "launchpad".into()),
        ("REMOTE_ADDR".into(), "127.0.0.1".into()),
        ("LP_IMPORT".into(), if import { "1" } else { "0" }.into()),
        ("LP_SESSION".into(), if session_refs.is_some() { "1" } else { "0" }.into()),
        ("LP_SESSION_REFS".into(), session_refs.map(|r| r.join(" ")).unwrap_or_default()),
    ];
    if let Some(t) = content_type { env.push(("CONTENT_TYPE".into(), t.into())); }
    if let Some(e) = encoding { env.push(("HTTP_CONTENT_ENCODING".into(), e.into())); }
    if let Some(p) = protocol { env.push(("GIT_PROTOCOL".into(), p.into())); }
    env
}

/// Splits `<folder-id>.git/<rest>` and checks the folder is a Home Git folder of this daemon.
pub async fn resolve_path(app: &App, rest: &str) -> Result<(Project, String)> {
    let (repo, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let id = repo.strip_suffix(".git").context("not a Home Git path")?;
    ensure!(matches!(tail, "info/refs" | "git-upload-pack" | "git-receive-pack" | "HEAD"), "unsupported Git path");
    let p = crate::projects::get(app, id).await?.context("no such folder on Home")?;
    ensure!(p.home_git, "{} is not a Git folder of Home", p.name);
    Ok((p, format!("/{id}.git/{tail}")))
}

/// Per-request secrets for the runner-local Git endpoint: the daemon's own fetches and pushes, and each
/// session's worktrees (so Home knows which session pushed and refuses a session that moved away).
pub fn token(app: &App, scope: &str) -> Result<String> {
    use hmac::{KeyInit, Mac};
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(app.paths.token()?.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    mac.update(format!("launchpad-git:{scope}").as_bytes());
    Ok(format!("{scope}:{}", hex::encode(mac.finalize().into_bytes())))
}

/// The scope of a valid token: `daemon` or `session:<id>`.
pub fn verify(app: &App, header: &str) -> Option<String> {
    let (scope, _) = header.rsplit_once(':')?;
    let expected = token(app, scope).ok()?;
    (expected.len() == header.len() && expected.bytes().zip(header.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0).then(|| scope.to_string())
}

/// Header carrying `token` on requests to the runner-local Git endpoint.
pub const TOKEN_HEADER: &str = "X-Launchpad-Git";
/// Header naming the pushing session, from a runner to Home.
pub const SESSION_HEADER: &str = "X-Launchpad-Session";
/// A runner daemon's import of a folder without a remote: every ref may be written.
pub const IMPORT_HEADER: &str = "X-Launchpad-Import";

/// Home: refuses requests a runner makes for a session Home did not place on it, or that now runs elsewhere
/// (another runner or generation).
pub fn check_session(app: &App, runner_id: &str, session: &str) -> Result<()> {
    let s = app.sessions.get(session).context("Home did not place this session on this runner")?;
    if s.runner_id.as_deref() != Some(runner_id) {
        bail!("this session now runs on another runner; this runner can no longer push its branches");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftover_branches_age_from_their_last_push() {
        let cutoff = 100 * KEEP_MS;
        assert!(scratch_expired(cutoff - 1, None, cutoff));
        assert!(!scratch_expired(cutoff - KEEP_MS, Some(cutoff), cutoff));
        assert!(scratch_expired(cutoff - KEEP_MS, Some(cutoff - 1), cutoff));
        assert!(!scratch_expired(cutoff, Some(0), cutoff));
    }

    /// Runs the pre-receive hook with `sh`, as a session (`refs`) or the daemon (None), on one pushed ref.
    fn hook_accepts(refs: Option<&[String]>, pushed: &str) -> bool {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .args(["-c", PRE_RECEIVE])
            .env("LP_IMPORT", "0")
            .env("LP_SESSION", if refs.is_some() { "1" } else { "0" })
            .env("LP_SESSION_REFS", refs.map(|r| r.join(" ")).unwrap_or_default())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "0000 1111 {pushed}").unwrap();
        child.wait().unwrap().success()
    }

    #[test]
    fn a_session_pushes_only_its_own_refs() {
        let refs = session_ref_patterns("abcd1234-5678", &["lp/fix-login-ffff0000".into(), "lp/bad*name".into()], &["wt-1".into()]);
        assert!(!refs.iter().any(|r| r.contains("bad")));
        let session = Some(refs.as_slice());
        for ok in ["refs/heads/lp/fix-abcd1234", "refs/heads/lp/fix-login-ffff0000", "refs/heads/lp-scratch/fix-abcd1234", "refs/heads/lp-scratch/fix-login-ffff0000-snapshot", "refs/heads/lp-scratch/fix-login-ffff0000-gen3", "refs/lp-snapshots/wt-1"] {
            assert!(hook_accepts(session, ok), "{ok}");
        }
        for refused in ["refs/heads/lp/other-99990000", "refs/heads/lp-scratch/other-99990000", "refs/lp-snapshots/wt-2", "refs/heads/main", "refs/heads/lp-scratch/fix-login-ffff0000-genx"] {
            assert!(!hook_accepts(session, refused), "{refused}");
        }
        // The daemon keeps its rights on Launchpad refs, never on base branches.
        assert!(hook_accepts(None, "refs/heads/lp/other-99990000"));
        assert!(hook_accepts(None, "refs/lp-snapshots/wt-2"));
        assert!(!hook_accepts(None, "refs/heads/main"));
    }

    #[test]
    fn restore_creates_only_missing_refs() {
        let restored = "a1 refs/lp-restore/heads/lp/old\nb2 refs/lp-restore/heads/lp/pushed-since\nc3 refs/lp-restore/kept/1/lp/x\ne5 refs/lp-restore/lp-restore/heads/y";
        let existing = "f9 refs/heads/main\nd4 refs/heads/lp/pushed-since\nb2 refs/lp-restore/heads/lp/pushed-since";
        assert_eq!(restore_updates(restored, existing), "create refs/heads/lp/old a1\ncreate refs/kept/1/lp/x c3\n");
        assert_eq!(restore_updates("", existing), "");
    }
}

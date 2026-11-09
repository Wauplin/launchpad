//! Task worktrees owned by Launchpad, on the machine that runs the agent (a runner, or `all`).
//!
//! Each session gets fresh worktrees of its folders, cloned from Home Git (`repo_cache`). A worktree is
//! disposable: the runner pushes its branch to Home at the end of every turn and records snapshots during
//! long turns, and removes the folder only after Home confirmed the push. Uncommitted files left at removal
//! go to a scratch branch on Home. Merges happen on Home (`home_git::merge`); a branch that needs a rebase
//! goes back to its agent.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::App;
use crate::model::{EventBody, Project, PullRequestRef, Session, SessionStatus, WorktreeOptions};
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TaskWorktree {
    pub id: String,
    /// Sessions working in it: the launching session first, then continuations with another agent.
    pub sessions: Vec<String>,
    pub project_id: String,
    /// The runner's clone of the folder (`repo_cache`), which owns the worktree.
    pub repo: String,
    pub root: String,
    pub branch: String,
    /// The branch it started from, where merges go. None: a pull request worktree.
    pub base_branch: Option<String>,
    /// Empty until the worktree is prepared.
    pub base_commit: String,
    /// The session's working directory, rather than an editable context folder.
    pub primary: bool,
    pub state: WorktreeState,
    pub merge: Option<MergeRequest>,
    pub merged: Option<Merged>,
    pub predraft_id: Option<String>,
    /// Why the last automatic cleanup kept the folder.
    pub cleanup_note: Option<String>,
    /// The GitHub pull request this worktree continues: Home pushes its commits there.
    #[serde(default)]
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestBranch>,
    /// The last commit Home confirmed for the branch.
    #[serde(default)]
    pub pushed: Option<String>,
    /// Why the last push to Home failed; cleared by the next successful push.
    #[serde(default)]
    pub push_note: Option<String>,
    /// The branch tip in the folder at the last turn end or push. Home receives the record with it, so it can
    /// tell when its copy of the branch is older (`away::info`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    /// Uncommitted files saved to a scratch branch on Home when the folder was removed.
    #[serde(default)]
    pub leftovers: Option<Leftovers>,
    /// What the worktree starts from: an existing Launchpad branch on Home (a Local PR, a restored branch).
    #[serde(default)]
    #[ts(skip)]
    pub from_branch: bool,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// Where the commits of a pull request worktree go: Home pushes them after each push from the runner.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct PullRequestBranch {
    /// `owner/repo` of the base repository.
    pub repo: String,
    pub number: u64,
    pub url: String,
    /// `owner/repo` holding the PR branch: a fork for outside contributions.
    pub head_repo: String,
    pub head_branch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Leftovers {
    /// `lp-scratch/…` on Home.
    pub branch: String,
    pub files: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// Being cloned from Home before the agent starts.
    Preparing,
    /// The folder exists.
    Active,
    /// The folder was removed after Home confirmed its branch; Resume clones it again.
    Removed,
    /// Folder and branch deleted by an explicit Discard (the branch tip stays restorable on Home).
    Discarded,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct MergeRequest {
    pub phase: MergePhase,
    /// Why the merge waits or stopped.
    pub reason: String,
    /// Requested by the agent with `launchpad work finish`: conflicts go back to it.
    pub by_agent: bool,
    /// The commit the owner reviewed: the merge stops if the branch moved past it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    /// Conflict messages already sent to the agent for this request.
    pub agent_retries: u32,
    /// A failed merge was already sent back to the agent once (automatic Fix merge): a second failure waits for
    /// the owner.
    #[serde(default)]
    pub auto_fixed: bool,
    #[ts(type = "number")]
    pub requested_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergePhase {
    /// Retried automatically: the agent is running, or the branch is not on Home yet.
    Waiting,
    /// The base branch moved: the agent (or the owner) must rebase.
    Conflict,
    /// Something unexpected; the owner decides.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Merged {
    /// Review baseline: the start of the branch.
    #[serde(default)]
    pub base: Option<String>,
    pub into: String,
    pub head: String,
    /// How the push of the base branch to the folder's remote went.
    #[serde(default)]
    pub note: String,
    #[ts(type = "number")]
    pub at: i64,
}

/// A worktree with its current Git state, for the session view and settings.
#[derive(Clone, Debug, Serialize, TS)]
pub struct WorktreeInfo {
    pub worktree: TaskWorktree,
    pub project_name: String,
    /// The folder agents work in.
    pub path: String,
    pub exists: bool,
    pub dirty_files: u32,
    /// Commits of the branch missing from the base branch on Home, and the reverse.
    pub ahead: u32,
    pub behind: u32,
    /// Commits not on Home yet.
    pub unpushed: bool,
    /// The branch tip this view describes (the folder's, or Home's when it answers for an away runner): Merge
    /// sends it back, and refuses a branch that moved since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    /// Set when Home answers for an away runner (offline, or a stopped Burst job): why, for the window.
    pub away: Option<String>,
}

const MAX_AGENT_RETRIES: u32 = 3;
/// Snapshots of running sessions' worktrees go to Home this often.
const SNAPSHOT_EVERY: Duration = Duration::from_secs(5 * 60);

async fn git(app: &App, dir: &str, args: &[&str]) -> Result<String> {
    crate::home_git::git_in(app, Path::new(dir), args, 120).await
}

pub fn folder(w: &TaskWorktree) -> String {
    w.root.clone()
}

pub async fn list(app: &App) -> Result<Vec<TaskWorktree>> {
    let mut rows: Vec<TaskWorktree> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM task_worktree")?;
            let texts = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(texts.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await?;
    rows.sort_by_key(|w| std::cmp::Reverse(w.created_at));
    Ok(rows)
}

pub async fn get(app: &App, id: &str) -> Result<TaskWorktree> {
    list(app).await?.into_iter().find(|w| w.id == id).context("worktree record not found")
}

pub async fn for_session(app: &App, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let mut rows: Vec<_> = list(app).await?.into_iter().filter(|w| w.sessions.iter().any(|s| s == session_id)).collect();
    rows.sort_by_key(|w| (!w.primary, w.created_at));
    Ok(rows)
}

/// The active record whose folder contains `path`.
pub async fn find_by_path(app: &App, path: &str) -> Result<Option<TaskWorktree>> {
    Ok(list(app).await?.into_iter().find(|w| w.state == WorktreeState::Active && crate::terminals::inside(path, &w.root)))
}

/// Stores a record. On a runner it also goes to Home (`replica`), so Home knows the session's worktrees and
/// their heads before any handoff and while the runner is away.
pub async fn save(app: &App, w: &TaskWorktree) -> Result<()> {
    let (id, text, item) = (w.id.clone(), serde_json::to_string(w)?, crate::replica::Item::Worktree { worktree: Box::new(w.clone()) });
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("INSERT INTO task_worktree (id, json) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET json = excluded.json", rusqlite::params![id, text])?;
            crate::replica::queue(&tx, &item)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("worktrees", "updated", json!(w));
    Ok(())
}

async fn notice(app: &App, w: &TaskWorktree, text: &str) {
    for id in &w.sessions {
        if app.sessions.get(id).is_some() {
            app.sessions.sink(app, id).emit(EventBody::Notice { text: text.to_string() }).await;
        }
    }
}

/// The editable worktree of a context folder.
pub(crate) async fn context_folder(_app: &App, records: &[TaskWorktree], project: &Project) -> Result<String> {
    let w = records.iter().find(|w| w.project_id == project.id && w.state == WorktreeState::Active)
        .with_context(|| format!("editable context folder {} has no active worktree; restore it or remove it from the task before resuming", project.name))?;
    Ok(w.root.clone())
}

fn slug_for(session_id: &str, project: &Project, title: &str, primary: bool) -> String {
    let name = if primary { crate::compose::slug_from(title) } else { format!("{}-{}", crate::compose::slug_from(&project.name), crate::compose::slug_from(title)) };
    let name: String = name.chars().take(48).collect::<String>().trim_matches('-').to_string();
    format!("{}-{}", if name.is_empty() { "task" } else { &name }, &session_id[..8.min(session_id.len())])
}

/// Records the worktree of `project` for a session (or returns the one it already has for that folder).
/// Nothing is cloned here, so a launch never waits for the network: `prepare` clones it before the agent
/// starts. `branch`: continue an existing Launchpad branch of Home (a Local PR) instead of a new one.
pub async fn plan(app: &App, session_id: &str, project: &Project, opts: &WorktreeOptions, primary: bool, title: &str, branch: Option<&str>) -> Result<(TaskWorktree, String)> {
    let _lock = app.worktree_lock.lock().await;
    ensure!(project.home_git, "{} is not a Git folder of Home", project.name);
    if let Some(w) = for_session(app, session_id).await?.into_iter().find(|w| w.project_id == project.id && matches!(w.state, WorktreeState::Active | WorktreeState::Preparing)) {
        return Ok((w.clone(), w.root.clone()));
    }
    let pr = opts.pull_request.as_ref().filter(|_| primary);
    let base_branch = if pr.is_some() { None } else { opts.base.clone().filter(|b| !b.is_empty()) };
    if let Some(b) = &base_branch {
        ensure!(!b.starts_with('-') && !b.starts_with("lp/"), "invalid base branch {b}");
    }
    let slug = slug_for(session_id, project, title, primary);
    let branch_name = match branch {
        Some(b) => {
            ensure!(b.starts_with("lp/"), "only Launchpad branches can be continued");
            b.to_string()
        }
        None => format!("lp/{slug}"),
    };
    let root = app.paths.data_dir.join("worktrees").join(&project.id).join(&slug);
    ensure!(!root.exists(), "{} already exists", root.display());
    let w = TaskWorktree {
        id: uuid::Uuid::new_v4().to_string(),
        sessions: vec![session_id.to_string()],
        project_id: project.id.clone(),
        repo: crate::repo_cache::dir(app, &project.id).to_string_lossy().into_owned(),
        root: root.to_string_lossy().into_owned(),
        branch: branch_name,
        base_branch,
        base_commit: String::new(),
        primary,
        state: WorktreeState::Preparing,
        merge: None,
        merged: None,
        predraft_id: None,
        cleanup_note: None,
        pull_request: pr.map(|pr| PullRequestBranch { repo: pr.repo.clone(), number: pr.number, url: String::new(), head_repo: String::new(), head_branch: String::new() }),
        pushed: None,
        push_note: None,
        head: None,
        leftovers: None,
        from_branch: branch.is_some(),
        created_at: now_ms(),
    };
    save(app, &w).await?;
    Ok((w.clone(), w.root))
}

/// Clones a planned worktree from Home Git: Home refreshes its mirror, the runner's clone fetches, then a
/// fresh worktree of the branch (or of the base branch) appears. Runs before the agent starts.
pub async fn prepare(app: &App, id: &str) -> Result<TaskWorktree> {
    let mut w = get(app, id).await?;
    if w.state != WorktreeState::Preparing {
        return Ok(w);
    }
    let project = crate::projects::get(app, &w.project_id).await?.context("the folder no longer exists")?;
    let cache = crate::repo_cache::sync(app, &project).await?;
    let cache_s = cache.to_string_lossy().into_owned();
    if let Some(pr) = w.pull_request.clone() {
        w.pull_request = Some(pull_request_branch(app, &project, &w, &PullRequestRef { repo: pr.repo, number: pr.number }).await?);
        crate::repo_cache::git(app, &cache, &["fetch", "--quiet", "origin"], true, 600).await?;
    }
    let base = match &w.base_branch {
        Some(b) => b.clone(),
        None if w.pull_request.is_none() => {
            let head = git(app, &cache_s, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).await.context("Home has no default branch for this folder")?;
            head.trim_start_matches("origin/").to_string()
        }
        None => String::new(),
    };
    let existing = git(app, &cache_s, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{}^{{commit}}", w.branch)]).await.ok().filter(|s| !s.is_empty());
    let start = match (&existing, w.from_branch || w.pull_request.is_some()) {
        (Some(head), _) => head.clone(),
        (None, true) => bail!("{} is not on Home", w.branch),
        (None, false) => git(app, &cache_s, &["rev-parse", "--verify", &format!("refs/remotes/origin/{base}^{{commit}}")]).await.with_context(|| format!("branch {base} not found on Home"))?,
    };
    let _lock = app.worktree_lock.lock().await;
    if let Some(parent) = Path::new(&w.root).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let _ = git(app, &cache_s, &["worktree", "prune"]).await;
    git(app, &cache_s, &["worktree", "add", "--quiet", "--no-track", "-B", &w.branch, &w.root, &start]).await?;
    configure(app, &w, w.sessions.last().map(String::as_str).unwrap_or_default()).await?;
    w.base_commit = if w.pull_request.is_some() || w.from_branch {
        git(app, &cache_s, &["merge-base", &start, &format!("refs/remotes/origin/{base}")]).await.unwrap_or_else(|_| start.clone())
    } else {
        start.clone()
    };
    if w.base_branch.is_none() && w.pull_request.is_none() {
        w.base_branch = Some(base);
    }
    w.pushed = existing;
    w.head = Some(start);
    w.state = WorktreeState::Active;
    save(app, &w).await?;
    Ok(w)
}

/// Per-worktree Git settings: pushes name the session, and `git push` goes to the same branch on Home.
async fn configure(app: &App, w: &TaskWorktree, session_id: &str) -> Result<()> {
    git(app, &w.root, &["config", "--worktree", "http.extraHeader", &crate::repo_cache::session_header(app, session_id)?]).await?;
    git(app, &w.root, &["config", &format!("branch.{}.remote", w.branch), "origin"]).await?;
    git(app, &w.root, &["config", &format!("branch.{}.merge", w.branch), &format!("refs/heads/{}", w.branch)]).await?;
    Ok(())
}

/// Continues a GitHub pull request: Home reads it with its own GitHub login, fetches its head into the session's
/// branch and follows it. A runner asks Home (`/api/home-git/{id}/pull-request`): it holds no GitHub credential.
async fn pull_request_branch(app: &App, project: &Project, w: &TaskWorktree, pr: &PullRequestRef) -> Result<PullRequestBranch> {
    let session_id = w.sessions.first().cloned();
    if crate::home_git::here(app) {
        return continue_pull_request(app, project, pr, &w.branch, session_id).await;
    }
    let body = json!({"repo": pr.repo, "number": pr.number, "branch": w.branch, "session_id": session_id});
    let v = crate::home_link::home_request("POST", &format!("/api/home-git/{}/pull-request", project.id), Some(body)).await?;
    Ok(serde_json::from_value(v)?)
}

/// Home: reads an open pull request, fetches its head into `branch` and follows it (`home_git::Follow`).
pub async fn continue_pull_request(app: &App, project: &Project, pr: &PullRequestRef, branch: &str, session_id: Option<String>) -> Result<PullRequestBranch> {
    let label = format!("{}#{}", pr.repo, pr.number);
    let v = app.github.get(&format!("/repos/{}/pulls/{}", pr.repo, pr.number)).await.with_context(|| format!("could not read pull request {label}"))?;
    ensure!(v["state"] == "open", "pull request {label} is not open");
    let head_repo = v["head"]["repo"]["full_name"].as_str().with_context(|| format!("the branch repository of {label} was deleted"))?.to_string();
    let head_branch = v["head"]["ref"].as_str().context("pull request branch missing")?.to_string();
    ensure!(!head_branch.starts_with('-'), "invalid pull request branch");
    let url = v["html_url"].as_str().context("pull request URL missing")?.to_string();
    let remote = if project.gh_repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(&head_repo)) {
        "origin".to_string()
    } else {
        let origin = project.remote_url.clone().unwrap_or_default();
        let repo = project.gh_repo.clone().unwrap_or_default();
        crate::predrafts::fork_url(&origin, &repo, &head_repo).with_context(|| format!("cannot derive the URL of {head_repo}"))?
    };
    let follow = crate::home_git::Follow { remote, branch: head_branch.clone(), url: url.clone(), session_id };
    crate::home_git::pull_request(app, project, pr.number, branch).await?;
    crate::home_git::set_follow(app, &project.id, branch, &follow).await?;
    Ok(PullRequestBranch { repo: pr.repo.clone(), number: pr.number, url, head_repo, head_branch })
}

/// The pull request a worktree folder continues, if any.
pub async fn pull_request_at(app: &App, root: &str) -> Option<PullRequestBranch> {
    list(app).await.ok()?.into_iter().find(|w| w.state == WorktreeState::Active && std::path::Path::new(root).starts_with(&w.root))?.pull_request
}

/// Pushes the branch to Home unless Home already has it. Never overwrites a tip Home got elsewhere (a
/// Restore, another runner): the push expects the tip this clone last saw.
/// The caller holds `worktree_lock` (the record is saved as it is); `after_turn` pushes without it.
pub async fn push(app: &App, w: &mut TaskWorktree) -> Result<bool> {
    let before = push_fields(w);
    let result = push_branch(app, w).await;
    if push_fields(w) != before {
        save(app, w).await?;
    }
    result
}

/// What a push changes in a record: the tip seen, the tip on Home and the failure note.
fn push_fields(w: &TaskWorktree) -> (Option<String>, Option<String>, Option<String>) {
    (w.head.clone(), w.pushed.clone(), w.push_note.clone())
}

/// `push` without saving the record.
async fn push_branch(app: &App, w: &mut TaskWorktree) -> Result<bool> {
    if w.state != WorktreeState::Active || !Path::new(&w.root).is_dir() {
        return Ok(false);
    }
    let head = git(app, &w.root, &["rev-parse", "--verify", &format!("refs/heads/{}^{{commit}}", w.branch)]).await?;
    w.head = Some(head.clone());
    if w.pushed.as_deref() == Some(head.as_str()) && w.push_note.is_none() {
        return Ok(false);
    }
    let lease = format!("--force-with-lease=refs/heads/{}", w.branch);
    let result = crate::home_git::git_in(app, Path::new(&w.root), &["push", "--quiet", &lease, "origin", &format!("refs/heads/{0}:refs/heads/{0}", w.branch)], 900).await;
    match result {
        Ok(_) => {
            w.pushed = Some(head);
            w.push_note = None;
            Ok(true)
        }
        Err(e) => {
            let note = format!("Push of {} to Home failed: {e:#}", w.branch);
            if w.push_note.as_deref() != Some(note.as_str()) {
                w.push_note = Some(note.clone());
                notice(app, w, &format!("{note}. Launchpad retries every few minutes and when this runner reconnects to Home.")).await;
            }
            Err(e)
        }
    }
}

/// When a turn ends, before its Done is stored: each worktree's branch tip goes into its record (and so to
/// Home, ahead of the session's new status). Local Git only, no network.
pub async fn record_heads(app: &App, session_id: &str) {
    let Ok(records) = for_session(app, session_id).await else { return };
    for w in records.into_iter().filter(|w| w.state == WorktreeState::Active && Path::new(&w.root).is_dir()) {
        let Ok(head) = git(app, &w.root, &["rev-parse", "--verify", &format!("refs/heads/{}^{{commit}}", w.branch)]).await else { continue };
        if w.head.as_deref() == Some(head.as_str()) {
            continue;
        }
        // Read again: the record may have changed while Git ran.
        if let Ok(mut fresh) = get(app, &w.id).await {
            fresh.head = Some(head);
            if let Err(e) = save(app, &fresh).await {
                tracing::warn!("worktree head of {}: {e:#}", w.branch);
            }
        }
    }
}

/// After every turn (`turn_sync`): the session's branches go to Home. Errors name each branch that failed.
pub async fn after_turn(app: &App, session_id: &str) -> Result<()> {
    let mut failed = vec![];
    for mut w in for_session(app, session_id).await?.into_iter().filter(|w| w.state == WorktreeState::Active) {
        // The push runs without the lock (it may take minutes); its result goes into the record as it is now,
        // so a merge request or cleanup saved meanwhile stays.
        let before = push_fields(&w);
        let result = push_branch(app, &mut w).await;
        if push_fields(&w) != before {
            let _lock = app.worktree_lock.lock().await;
            if let Ok(mut fresh) = get(app, &w.id).await {
                (fresh.head, fresh.pushed, fresh.push_note) = push_fields(&w);
                save(app, &fresh).await?;
            }
        }
        if let Err(e) = result {
            failed.push(format!("{}: {e:#}", w.branch));
        }
    }
    ensure!(failed.is_empty(), "push to Home failed ({})", failed.join("; "));
    Ok(())
}

/// During long turns: the state of every file (committed or not) goes to `refs/lp-snapshots/<worktree>` on
/// Home, so nothing is lost if the machine disappears.
async fn snapshots(app: &App, last: &mut HashMap<String, String>) {
    let Ok(records) = list(app).await else { return };
    for w in records.iter().filter(|w| w.state == WorktreeState::Active) {
        let running = w.sessions.iter().filter_map(|id| app.sessions.get(id)).any(|s| app.sessions.is_busy(&s));
        if !running || !Path::new(&w.root).is_dir() {
            continue;
        }
        let tree = match crate::repo_cache::capture(app, Path::new(&w.root), "Launchpad snapshot").await {
            Ok(Some(commit)) => commit,
            Ok(None) => git(app, &w.root, &["rev-parse", "HEAD"]).await.unwrap_or_default(),
            Err(e) => {
                tracing::debug!("snapshot {}: {e:#}", w.root);
                continue;
            }
        };
        let content = git(app, &w.root, &["rev-parse", &format!("{tree}^{{tree}}")]).await.unwrap_or_default();
        if last.get(&w.id) == Some(&content) {
            continue;
        }
        if crate::home_git::git_in(app, Path::new(&w.root), &["push", "--quiet", "origin", &format!("+{tree}:refs/lp-snapshots/{}", w.id)], 900).await.is_ok() {
            last.insert(w.id.clone(), content);
        }
    }
}

/// Removes worktrees created for a launch that failed before its agent started.
pub async fn abandon(app: &App, created: &[TaskWorktree]) {
    let _lock = app.worktree_lock.lock().await;
    for w in created {
        let result: Result<()> = async {
            if Path::new(&w.root).exists() {
                git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
            }
            let id = w.id.clone();
            app.state_db.call(move |c| Ok(c.execute("DELETE FROM task_worktree WHERE id = ?1", [id])?)).await?;
            app.bus.publish("worktrees", "removed", json!({"id": w.id}));
            Ok(())
        }.await;
        if let Err(e) = result { tracing::warn!("could not clean up failed launch worktree {}: {e:#}", w.root); }
    }
}

/// Remove a failed launch's membership from worktrees shared with an older session.
pub(crate) async fn leave(app: &App, session_id: &str) -> Result<()> {
    let _lock = app.worktree_lock.lock().await;
    for mut w in for_session(app, session_id).await?.into_iter().filter(|w| w.sessions.iter().any(|id| id != session_id)) {
        w.sessions.retain(|id| id != session_id);
        save(app, &w).await?;
    }
    Ok(())
}

/// A continuation with another agent works in the same worktrees; its pushes name it.
pub async fn join(app: &App, source_session: &str, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let _lock = app.worktree_lock.lock().await;
    let worktrees = for_session(app, source_session).await?;
    for w in &worktrees {
        if let Some(reason) = idle_agent_reason(app, w, false) {
            bail!("cannot continue in this worktree: {reason}");
        }
    }
    let mut joined = vec![];
    for mut w in worktrees {
        if !w.sessions.iter().any(|s| s == session_id) {
            w.sessions.push(session_id.to_string());
            save(app, &w).await?;
        }
        if w.state == WorktreeState::Active {
            configure(app, &w, session_id).await?;
        }
        joined.push(w);
    }
    Ok(joined)
}

/// Launch instructions: where to work, and how the task ends. Overrides checkout workflow rules only.
pub fn instruction(app: &App, session_id: &str, worktrees: &[(TaskWorktree, String)], projects: &[Project], when_done: crate::model::WhenDone) -> String {
    let cli = app.paths.cli_command();
    let name = |id: &str| projects.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| "folder".into());
    let read_only = when_done == crate::model::WhenDone::ReadOnly && !worktrees.iter().any(|(w, _)| w.pull_request.is_some());
    let mut text = String::from(if read_only {
        "Launchpad worktree (read only): Launchpad created fresh Git worktrees for this task so you can read, build and run the code. This task must not change it.\n"
    } else {
        "Launchpad worktree: Launchpad created fresh Git worktrees for this task and manages them. This launch setting overrides repository or preset instructions to work on the main branch, in the main checkout, or to create, merge or delete worktrees yourself; follow all other repository rules (checks, documentation, commit conventions)."
    });
    for (w, path) in worktrees {
        text.push_str(&format!(
            "\n- {}{}: work in `{path}` on branch `{}`, {}.",
            name(&w.project_id),
            if w.primary { " (your working directory)" } else { " (editable)" },
            w.branch,
            match (&w.pull_request, w.base_branch.as_deref(), w.from_branch) {
                (Some(pr), _, _) => format!("which continues pull request #{} of {}", pr.number, pr.repo),
                (None, _, true) => "which continues earlier work".into(),
                (None, Some(b), false) => format!("created from `{b}`"),
                (None, None, false) => "created from the default branch".into(),
            },
        ));
    }
    if read_only {
        text.push_str("\nRead only: do not edit, create or delete tracked files, do not commit, push, switch branches or run `launchpad work finish`. Build outputs, installed dependencies and other Git-ignored files are fine. If the task needs a change, describe it (or show a patch) in your answer instead of applying it. Give your findings in your final message. When the agent is archived, Launchpad deletes these worktrees and their branches if nothing changed.");
        return text;
    }
    text.push_str("\n`origin` is Launchpad's Git server (Home Git), with `origin/<branch>` for every branch. Commit your work on these branches: Launchpad pushes them to `origin` at the end of every turn (you may also `git push` them yourself). Do not switch branches, push other branches, or remove these worktrees; never push to the base branch. Uncommitted files are saved when the folder is removed, but commit what matters.");
    if let Some(pr) = worktrees.iter().find_map(|(w, _)| w.pull_request.clone()) {
        text.push_str(&format!("\nThis work continues pull request #{} on GitHub: after each push, Launchpad pushes your commits to its branch. Never force-push or rewrite commits that are already pushed. If Launchpad reports that the pull request branch moved, run `git fetch upstream refs/pull/{}/head` and rebase onto `FETCH_HEAD`, re-run your checks and commit. Do not open another pull request. The pull request on GitHub is the review: Launchpad does not merge this branch.", pr.number, pr.number));
    }
    text.push_str(&format!(
        "\nTo modify another registered folder, first run `{cli} work add-project --session-id {session_id} --project <name>`. It prints a dedicated worktree folder: edit that folder."
    ));
    let bases: Vec<String> = worktrees.iter().filter_map(|(w, _)| w.base_branch.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().map(|b| format!("`{b}`")).collect();
    let base = if bases.is_empty() { "its base branch".to_string() } else { bases.join(", ") };
    if worktrees.iter().any(|(w, _)| w.pull_request.is_some()) {
    } else if when_done == crate::model::WhenDone::LocalPr {
        text.push_str(&format!(
            "\nWhen the task is done and checked, commit everything, then run `{cli} work finish --session-id {session_id}`. Launchpad opens a Local PR of your branch on Home for the owner to review; then set its title and description as the command output explains. Do not merge into {base} or open a pull request on GitHub yourself."
        ));
    } else if when_done == crate::model::WhenDone::Merge {
        text.push_str(&format!(
            "\nWhen the task is done and checked, commit everything, then run `{cli} work finish --session-id {session_id}`. Launchpad merges your branches into {base} on Home after your turn ends. Do not merge yourself, and report the merge as requested, not as done. If Launchpad later reports that {base} moved, run `git fetch origin`, rebase onto `origin/<base>`, resolve conflicts, re-run your checks, commit, and run the finish command again."
        ));
    } else {
        text.push_str(&format!("\nDo not merge into {base} yourself: the owner merges, publishes or discards the branch from Launchpad."));
    }
    text
}

/// Runs the owner-configured setup command of a project in a new worktree folder.
pub async fn run_setup(app: &App, project: &Project, folder: &str, sink: &crate::sessions::Sink, cancelled: &mut tokio::sync::watch::Receiver<bool>) -> Result<()> {
    anyhow::ensure!(!*cancelled.borrow(), "setup cancelled");
    let command = project.worktree_setup.trim();
    if command.is_empty() {
        return Ok(());
    }
    sink.emit(EventBody::Notice { text: format!("Running the worktree setup of {}: {command}", project.name) }).await;
    let started = std::time::Instant::now();
    let mut cmd = if cfg!(windows) {
        let mut c = crate::procs::child_command(&app.env, "powershell", Some(Path::new(folder)));
        c.args(["-NoProfile", "-Command", command]);
        c
    } else {
        let mut c = crate::procs::child_command(&app.env, "sh", Some(Path::new(folder)));
        c.args(["-c", command]);
        c
    };
    cmd.process_tree().stdin(crate::exec::Io::Null).stdout(crate::exec::Io::Piped).stderr(crate::exec::Io::Piped).env("LAUNCHPAD_WORKTREE", folder).env("LAUNCHPAD_SOURCE_CHECKOUT", &project.path);
    let out = setup_output(&mut cmd, cancelled, Duration::from_secs(30 * 60), Some((app.sessions.clone(), sink.id.clone()))).await?;
    finish_setup(sink, command, started, out).await
}

async fn setup_output(cmd: &mut crate::exec::Cmd, cancelled: &mut tokio::sync::watch::Receiver<bool>, timeout: Duration, owner: Option<(crate::sessions::Sessions, String)>) -> Result<crate::exec::Output> {
    anyhow::ensure!(!*cancelled.borrow(), "setup cancelled");
    let child = cmd.spawn().await?;
    let killer = child.killer();
    let mut output = tokio::spawn(child.wait_with_output());
    let reason = tokio::select! {
        out = &mut output => return Ok(out??),
        _ = cancelled.wait_for(|cancel| *cancel) => "setup cancelled",
        _ = tokio::time::sleep(timeout) => "setup did not finish within 30 minutes",
    };
    // Keep ownership until exit is confirmed.
    killer.kill().await;
    if let Some((sessions, id)) = owner {
        sessions.wait_for_shutdown(&id, async move { output.await??; Ok(()) }).await.with_context(|| reason)?;
    } else {
        let _ = tokio::time::timeout(Duration::from_secs(5), output).await;
    }
    bail!("{reason}")
}

async fn finish_setup(sink: &crate::sessions::Sink, command: &str, started: std::time::Instant, out: crate::exec::Output) -> Result<()> {
    let mut log = String::from_utf8_lossy(&out.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    let tail = crate::sessions::truncate_output(log.trim(), 4000);
    if !out.status.success() {
        bail!("{} exited with {}\n{tail}", command, out.status);
    }
    sink.emit(EventBody::Notice { text: format!("Worktree setup finished in {} s", started.elapsed().as_secs()) }).await;
    Ok(())
}


/// Sessions of a record are idle enough for Git operations on its folder.
fn idle_agent_reason(app: &App, w: &TaskWorktree, strict: bool) -> Option<String> {
    for id in &w.sessions {
        let Some(s) = app.sessions.get(id) else { continue };
        if app.sessions.is_busy(&s) || app.sessions.setup_running(id) {
            return Some("an agent is running in this worktree".into());
        }
        if s.status == SessionStatus::InTerminal || s.quota_pause.is_some() {
            return Some("an agent of this worktree is quota paused or open in a terminal".into());
        }
        if strict && s.open_question {
            return Some("an agent of this worktree waits for an answer".into());
        }
    }
    None
}

fn idle_reason(app: &App, w: &TaskWorktree, strict: bool) -> Option<String> {
    if let Some(reason) = idle_agent_reason(app, w, strict) {
        return Some(reason);
    }
    if !app.terminals.alive_in(&w.root).is_empty() {
        return Some("a terminal is open in this worktree".into());
    }
    None
}

pub(crate) async fn dirty_files(app: &App, dir: &str) -> Result<Vec<String>> {
    Ok(git(app, dir, &["status", "--porcelain", "--untracked-files=all"]).await?.lines().map(|l| l.get(2..).unwrap_or(l).trim_start().to_string()).collect())
}

/// A rebase, merge or cherry-pick stopped in this worktree.
pub(crate) async fn operation_in_progress(app: &App, dir: &str) -> bool {
    for marker in ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD"] {
        if let Ok(p) = git(app, dir, &["rev-parse", "--path-format=absolute", "--git-path", marker]).await
            && Path::new(&p).exists()
        {
            return true;
        }
    }
    false
}

async fn count(app: &App, dir: &str, range: &str) -> u32 {
    git(app, dir, &["rev-list", "--count", range, "--"]).await.ok().and_then(|n| n.parse().ok()).unwrap_or(0)
}

pub async fn info(app: &App, w: TaskWorktree) -> WorktreeInfo {
    let project_name = crate::projects::get(app, &w.project_id).await.ok().flatten().map(|p| p.name).unwrap_or_else(|| "Removed folder".into());
    let path = folder(&w);
    let exists = w.state == WorktreeState::Active && Path::new(&w.root).is_dir();
    let dirty_files = if exists { dirty_files(app, &w.root).await.map(|f| f.len() as u32).unwrap_or(0) } else { 0 };
    let (mut ahead, mut behind, mut unpushed) = (0, 0, false);
    let mut tip = w.pushed.clone();
    if exists {
        let head = git(app, &w.root, &["rev-parse", &format!("refs/heads/{}", w.branch)]).await.unwrap_or_default();
        unpushed = !head.is_empty() && w.pushed.as_deref() != Some(head.as_str());
        tip = Some(head).filter(|h| !h.is_empty());
        if let Some(base) = &w.base_branch {
            ahead = count(app, &w.root, &format!("refs/remotes/origin/{base}..refs/heads/{}", w.branch)).await;
            behind = count(app, &w.root, &format!("refs/heads/{}..refs/remotes/origin/{base}", w.branch)).await;
        }
    } else if let (Some(base), Some(pushed)) = (&w.base_branch, &w.pushed)
        && Path::new(&w.repo).is_dir()
    {
        ahead = count(app, &w.repo, &format!("refs/remotes/origin/{base}..{pushed}")).await;
    }
    WorktreeInfo { worktree: w, project_name, path, exists, dirty_files, ahead, behind, unpushed, head: tip, away: None }
}

pub async fn session_info(app: &App, session_id: &str) -> Result<Vec<WorktreeInfo>> {
    let mut out = vec![];
    for w in for_session(app, session_id).await? {
        out.push(info(app, w).await);
    }
    Ok(out)
}

/// `launchpad work add-project`: a worktree of another Git folder, during the task.
pub async fn add_project(app: &App, session_id: &str, project: &str) -> Result<String> {
    let s = app.sessions.get(session_id).context("no such session")?;
    let projects = crate::projects::list(app).await?;
    let wanted = project.trim();
    let matches: Vec<&Project> = projects.iter().filter(|p| p.id == wanted || p.name.eq_ignore_ascii_case(wanted)).collect();
    let p = match matches.as_slice() {
        [p] => (*p).clone(),
        [] => bail!("no registered folder named {wanted:?}; registered folders: {}", projects.iter().filter(|p| p.home_git).map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")),
        _ => bail!("several folders are named {wanted:?}; use the folder ID"),
    };
    ensure!(p.home_git, "{} is not a Git folder", p.name);
    let base = s.launch_spec.context_projects.iter().find(|c| c.project_id == p.id).and_then(|c| c.branch.clone());
    let (w, path) = plan(app, session_id, &p, &WorktreeOptions { base, ..Default::default() }, false, &s.title, None).await?;
    let w = prepare(app, &w.id).await?;
    let sink = app.sessions.sink(app, session_id);
    sink.emit(EventBody::Notice { text: format!("Added a worktree for {} on branch {}: {path}", p.name, w.branch) }).await;
    let (_cancel, mut cancelled) = tokio::sync::watch::channel(false);
    if let Err(e) = run_setup(app, &p, &path, &sink, &mut cancelled).await {
        return Ok(format!("Worktree for {} on branch `{}`: {path}\nIts setup command failed; fix the environment yourself if needed:\n{e:#}", p.name, w.branch));
    }
    Ok(format!("Worktree for {} on branch `{}`: {path}\nCommit your work on this branch; Launchpad pushes it to Home at the end of each turn.", p.name, w.branch))
}

/// Restores the removed worktrees of a session before it resumes. Returns whether a folder was restored.
pub async fn restore_for_session(app: &App, session_id: &str) -> Result<bool> {
    let mut restored = false;
    for w in for_session(app, session_id).await? {
        if matches!(w.state, WorktreeState::Removed | WorktreeState::Preparing) {
            restore(app, &w.id).await?;
            restored = true;
        }
    }
    Ok(restored)
}

/// Clones a removed folder again from its branch on Home (or its base, when the branch is gone).
pub async fn restore(app: &App, id: &str) -> Result<TaskWorktree> {
    let w = get(app, id).await?;
    if w.state == WorktreeState::Active && Path::new(&w.root).is_dir() {
        return Ok(w);
    }
    if w.state == WorktreeState::Preparing {
        return prepare(app, id).await;
    }
    ensure!(w.state == WorktreeState::Removed, "this worktree cannot be restored");
    let project = crate::projects::get(app, &w.project_id).await?.context("the folder no longer exists")?;
    let cache = crate::repo_cache::sync(app, &project).await?;
    let cache_s = cache.to_string_lossy().into_owned();
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    ensure!(!Path::new(&w.root).exists(), "{} already exists", w.root);
    let start = match git(app, &cache_s, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{}^{{commit}}", w.branch)]).await {
        Ok(c) if !c.is_empty() => c,
        _ => {
            let base = w.base_branch.clone().context("the branch is gone from Home and there is no base branch to restart from")?;
            git(app, &cache_s, &["rev-parse", "--verify", &format!("refs/remotes/origin/{base}^{{commit}}")]).await?
        }
    };
    let _ = git(app, &cache_s, &["worktree", "prune"]).await;
    git(app, &cache_s, &["worktree", "add", "--quiet", "--no-track", "-B", &w.branch, &w.root, &start]).await?;
    configure(app, &w, w.sessions.last().map(String::as_str).unwrap_or_default()).await?;
    w.state = WorktreeState::Active;
    w.cleanup_note = None;
    w.pushed = Some(start);
    w.push_note = None;
    save(app, &w).await?;
    let mut text = format!("Worktree cloned again from branch {} on Home. Ignored files such as installed dependencies were not restored; run the setup again if needed.", w.branch);
    if let Some(l) = &w.leftovers {
        text.push_str(&format!(" {} uncommitted files from before are on the branch {} (`git checkout origin/{} -- .` brings them back).", l.files, l.branch, l.branch));
    }
    notice(app, &w, &text).await;
    Ok(w)
}

/// Removes a folder once Home has everything: the branch is pushed, and uncommitted files go to a scratch
/// branch first. Fails with the reason when an agent or a terminal still uses it.
async fn remove_clean(app: &App, w: &mut TaskWorktree) -> Result<()> {
    ensure!(w.state == WorktreeState::Active, "this worktree is not active");
    if let Some(reason) = idle_reason(app, w, true) {
        bail!("{reason}");
    }
    ensure!(w.merge.is_none(), "a merge is pending");
    if Path::new(&w.root).exists() {
        ensure!(!operation_in_progress(app, &w.root).await, "a Git operation is in progress");
        let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
        ensure!(branch == w.branch, "the worktree is on another branch ({branch})");
        push(app, w).await.context("Home did not confirm the branch")?;
        let dirty = dirty_files(app, &w.root).await?;
        if !dirty.is_empty() {
            let commit = crate::repo_cache::capture(app, Path::new(&w.root), &format!("Leftover files of {}", w.branch)).await?.context("leftover files could not be saved")?;
            let scratch = format!("lp-scratch/{}", w.branch.trim_start_matches("lp/"));
            crate::home_git::git_in(app, Path::new(&w.root), &["push", "--quiet", "origin", &format!("+{commit}:refs/heads/{scratch}")], 900).await.context("Home did not confirm the leftover files")?;
            w.leftovers = Some(Leftovers { branch: scratch, files: dirty.len() as u32 });
        }
        for id in w.sessions.clone() {
            app.sessions.release(app, &id).await?;
        }
        git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
    }
    w.state = WorktreeState::Removed;
    w.cleanup_note = None;
    save(app, w).await
}

/// Runner: a session Home moved on while this runner was away (`handoff::release`). Its worktrees' commits and
/// uncommitted files go to `lp-scratch/<branch>-gen<generation>` on Home (the branch itself belongs to the newer
/// generation now), then the folders and records go. A worktree another open session here still uses only
/// loses this session. The caller holds the session action lock.
pub(crate) async fn release_moved(app: &App, session_id: &str, generation: u32) -> Result<()> {
    let _lock = app.worktree_lock.lock().await;
    for mut w in for_session(app, session_id).await? {
        if w.sessions.iter().any(|id| id != session_id && app.sessions.get(id).is_some_and(|s| s.status != SessionStatus::Closed)) {
            w.sessions.retain(|id| id != session_id);
            save(app, &w).await?;
            continue;
        }
        if w.state == WorktreeState::Active && Path::new(&w.root).is_dir() {
            // A shell there would write after the capture, into a folder that is about to go.
            app.terminals.close_in(&w.root).await?;
            let head = git(app, &w.root, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
            let tip = crate::repo_cache::capture(app, Path::new(&w.root), &format!("Leftover files of {}", w.branch)).await?.unwrap_or(head);
            if w.pushed.as_deref() != Some(tip.as_str()) {
                let scratch = format!("lp-scratch/{}-gen{generation}", w.branch.trim_start_matches("lp/"));
                // As the daemon, not the session: Home refuses the Git requests of a session that now runs
                // elsewhere (`home_git::check_session`). The empty value drops the worktree's session header.
                let header = crate::repo_cache::daemon_header(app)?;
                let url = crate::repo_cache::home_url(app, &w.project_id);
                let args = ["-c", "http.extraHeader=", "-c", header.as_str(), "push", "--quiet", url.as_str(), &format!("+{tip}:refs/heads/{scratch}")];
                crate::home_git::git_in(app, Path::new(&w.root), &args, 900)
                    .await
                    .with_context(|| format!("Home did not confirm the leftovers of {}", w.branch))?;
                tracing::info!("released worktree {}: leftovers on {scratch}", w.root);
            }
            git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
        }
        let id = w.id.clone();
        app.state_db.call(move |c| Ok(c.execute("DELETE FROM task_worktree WHERE id = ?1", [id])?)).await?;
        app.bus.publish("worktrees", "removed", json!({"id": w.id}));
    }
    Ok(())
}

/// Settings → Worktrees "Remove folder": removal after Home confirmed the branch.
pub async fn remove(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    remove_clean(app, &mut w).await?;
    Ok(w)
}

/// Deletes the folder and the branch (here and on Home, where its tip stays restorable for 30 days) and
/// archives the sessions. Owner confirmation only.
pub async fn discard(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    ensure!(matches!(w.state, WorktreeState::Active | WorktreeState::Removed | WorktreeState::Preparing), "this worktree was already discarded");
    for sid in &w.sessions {
        if let Some(s) = app.sessions.get(sid)
            && (app.sessions.is_busy(&s) || s.quota_pause.is_some() || s.status == SessionStatus::InTerminal || app.sessions.setup_running(sid))
        {
            bail!("stop the agents of this worktree first");
        }
    }
    app.terminals.close_in(&w.root).await?;
    // Without its working directory an agent cannot continue: archive it. A discarded editable context
    // folder only detaches idle processes.
    for sid in w.sessions.clone() {
        if w.primary {
            if app.sessions.get(&sid).is_some_and(|s| s.status != SessionStatus::Closed) {
                app.sessions.close_locked(app, &sid).await?;
            }
        } else {
            app.sessions.release(app, &sid).await?;
        }
    }
    discard_locked(app, &mut w).await?;
    Ok(w)
}

/// Deletes the folder and the branch, here and on Home. The caller holds the action and worktree locks.
async fn discard_locked(app: &App, w: &mut TaskWorktree) -> Result<()> {
    if Path::new(&w.root).exists() {
        git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
    }
    let _ = git(app, &w.repo, &["branch", "-D", &w.branch]).await;
    if w.pushed.is_some() || w.from_branch {
        let body = json!({"branch": w.branch});
        let deleted = if crate::home_git::here(app) {
            crate::home_git::delete(app, &w.project_id, &w.branch).await
        } else {
            crate::home_link::home_request("POST", &format!("/api/home-git/{}/delete", w.project_id), Some(body)).await.map(|_| ())
        };
        if let Err(e) = deleted {
            tracing::warn!("discard {}: Home kept the branch: {e:#}", w.branch);
        }
    }
    w.state = WorktreeState::Discarded;
    w.merge = None;
    save(app, w).await
}

/// Merge into base: the owner's button (with the `head` it showed), or `by_agent` from `launchpad work finish`.
/// Returns at once; the merge runs on Home as soon as the agents are idle and the branch is pushed, and stops if
/// the branch moved past `head`.
pub async fn request_merge(app: &App, id: &str, by_agent: bool, head: Option<String>) -> Result<TaskWorktree> {
    let mut w = {
        let _lock = app.worktree_lock.lock().await;
        let mut w = get(app, id).await?;
        ensure!(w.state == WorktreeState::Active, "only an active worktree can be merged");
        ensure!(w.pull_request.is_none(), "this branch continues a pull request: merge it on GitHub");
        let base = w.base_branch.clone().context("this worktree has no base branch to merge into")?;
        if let Some(head) = &head {
            let tip = git(app, &w.root, &["rev-parse", "--verify", &format!("refs/heads/{}^{{commit}}", w.branch)]).await?;
            ensure!(&tip == head, "{} changed since review; review again", w.branch);
        }
        let before = w.merge.as_ref().filter(|m| m.by_agent && by_agent);
        let (retries, auto_fixed) = before.map(|m| (m.agent_retries, m.auto_fixed)).unwrap_or_default();
        w.merge = Some(MergeRequest { phase: MergePhase::Waiting, reason: format!("Waiting to merge into {base}"), by_agent, head, agent_retries: retries, auto_fixed, requested_at: now_ms() });
        save(app, &w).await?;
        w
    };
    if !by_agent {
        attempt(app, &w.id).await;
        w = get(app, id).await?;
    } else {
        let (app, id) = (app.clone(), w.id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            attempt(&app, &id).await;
        });
    }
    Ok(w)
}

/// `launchpad work finish`: requests the merge of every worktree of the session, or opens a Local PR of the
/// primary worktree's branch when the launch chose it.
pub async fn finish(app: &App, session_id: &str) -> Result<String> {
    let when_done = app.sessions.get(session_id).and_then(|s| s.launch_spec.worktree).map(|o| o.when_done).unwrap_or_default();
    ensure!(when_done != crate::model::WhenDone::ReadOnly, "this session is read only: there is nothing to finish. Undo your changes and give your answer in your final message");
    if when_done == crate::model::WhenDone::LocalPr {
        return finish_local_pr(app, session_id).await;
    }
    let all = for_session(app, session_id).await?;
    let records: Vec<_> = all.into_iter().filter(|w| w.state == WorktreeState::Active && w.base_branch.is_some() && w.pull_request.is_none()).collect();
    ensure!(!records.is_empty(), "this session has no Launchpad worktree to merge");
    for w in &records {
        ready_to_finish(app, w).await?;
    }
    let mut lines = vec![];
    for w in &records {
        request_merge(app, &w.id, true, None).await?;
        lines.push(format!("- {} into {}", w.branch, w.base_branch.as_deref().unwrap_or_default()));
    }
    Ok(format!(
        "Merge requested:\n{}\nLaunchpad pushes your branches to Home and merges them there after your turn ends. End your turn now and report the merge as requested, not as done. Do not merge yourself.",
        lines.join("\n")
    ))
}

async fn ready_to_finish(app: &App, w: &TaskWorktree) -> Result<()> {
    let dirty = dirty_files(app, &w.root).await?;
    ensure!(dirty.is_empty(), "{} has uncommitted changes ({}); commit or remove them, then run finish again", w.root, dirty.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
    ensure!(!operation_in_progress(app, &w.root).await, "{}: finish the Git operation in progress first", w.root);
    let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
    ensure!(branch == w.branch, "{} must stay on branch {} (it is on {branch:?})", w.root, w.branch);
    Ok(())
}

/// Opens a Local PR of the session's primary branch on Home. Idempotent: a second call returns the same
/// Local PR. The session's other branches join the same change set when they become Local PRs.
async fn finish_local_pr(app: &App, session_id: &str) -> Result<String> {
    let all = for_session(app, session_id).await?;
    let mut w = all.iter().find(|w| w.primary && w.state == WorktreeState::Active).cloned().context("this session has no Launchpad worktree to turn into a Local PR")?;
    ensure!(w.pull_request.is_none(), "this branch continues a pull request: push your commits instead of opening a Local PR");
    ready_to_finish(app, &w).await?;
    push(app, &mut w).await.context("push the branch to Home")?;
    let title = app.sessions.get(session_id).map(|s| s.title).filter(|t| !t.trim().is_empty());
    let p = crate::predrafts::open_for_branch(app, &w.project_id, &w.branch, w.base_branch.as_deref(), Some(session_id), title).await?;
    mark_predraft(app, &w.id, &p.id).await?;
    let others: Vec<String> = all.iter().filter(|o| !o.primary && o.state == WorktreeState::Active).map(|o| format!("`{}`", o.branch)).collect();
    let mut text = format!(
        "Local PR opened on Home from branch `{}` (id {}), based on `{}`. Now set its title and description: write the description to a file, then run `{} pr update --title \"…\" --body-file <file>` in `{}`. Then end your turn and report the Local PR as opened for review. Do not open a pull request on GitHub.",
        p.branch, p.id, p.base_branch, app.paths.cli_command(), w.root
    );
    if !others.is_empty() {
        text.push_str(&format!(" The branches of your other worktrees ({}) can join the same change set: run `{} pr create` in each of them.", others.join(", "), app.paths.cli_command()));
    }
    Ok(text)
}

pub async fn cancel_merge(app: &App, id: &str) -> Result<TaskWorktree> {
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    w.merge = None;
    save(app, &w).await?;
    notice(app, &w, &format!("Merge of {} cancelled.", w.branch)).await;
    Ok(w)
}

fn conflict_message(app: &App, w: &TaskWorktree, reason: &str, session_id: &str) -> String {
    let base = w.base_branch.as_deref().unwrap_or_default();
    format!(
        "Launchpad could not merge branch `{}` into `{base}`: {reason}\nIn your worktree `{}`, run `git fetch origin`, rebase onto `origin/{base}`, resolve the conflicts, re-run the relevant checks, and commit. Then run `{} work finish --session-id {session_id}` again. Do not merge into `{base}` yourself.\nWhen you finish this recovery turn, make your final message a concise, self-contained summary of the original task and this recovery: lead with the overall outcome, preserve the substantive results from your previous task summary, and update the checks actually performed and their results, remaining work or limitations, and merge status. Report the merge as requested, not completed. The user sees this as the run's final message, so include the task results even though this is a follow-up turn.",
        w.branch,
        w.root,
        app.paths.cli_command(),
    )
}

/// Fix with agent: sends the conflict to the worktree's latest session, under normal quota protection.
pub async fn fix_with_agent(app: &App, id: &str) -> Result<()> {
    let w = get(app, id).await?;
    let m = w.merge.clone().context("no merge problem to fix")?;
    ensure!(m.phase != MergePhase::Waiting, "the merge is still waiting");
    let session = w.sessions.last().context("this worktree has no session")?.clone();
    app.sessions.send(app, &session, &conflict_message(app, &w, &m.reason, &session), vec![]).await
}

/// One merge attempt: push the branch, then Home fast-forwards the base branch.
async fn attempt(app: &App, id: &str) {
    let outcome = {
        let _action = app.sessions.action_lock().await;
        let _lock = app.worktree_lock.lock().await;
        let Ok(mut w) = get(app, id).await else { return };
        let Some(m) = w.merge.clone() else { return };
        if m.phase != MergePhase::Waiting {
            return;
        }
        let result = match try_merge(app, &mut w).await {
            Ok(Step::Merged(head, note)) => {
                let into = w.base_branch.clone().unwrap_or_default();
                w.merge = None;
                w.merged = Some(Merged { base: Some(w.base_commit.clone()), into: into.clone(), head: head.clone(), note: note.clone(), at: now_ms() });
                notice(app, &w, &format!("Launchpad merged {} into {into} ({}). {note}", w.branch, &head[..12.min(head.len())])).await;
                None
            }
            Ok(Step::Wait(reason)) => {
                w.merge = Some(MergeRequest { reason, ..m });
                None
            }
            Ok(Step::Conflict(reason)) => {
                let send = m.by_agent && m.agent_retries < MAX_AGENT_RETRIES;
                w.merge = Some(MergeRequest { phase: MergePhase::Conflict, reason: reason.clone(), agent_retries: m.agent_retries + send as u32, ..m });
                notice(app, &w, &format!("Merge of {} stopped: {reason}", w.branch)).await;
                send.then(|| (w.sessions.last().cloned(), reason))
            }
            Err(e) => {
                // Usually a race on the base branch: the agent gets it once (automatic Fix merge), never twice.
                let reason = format!("{e:#}");
                let send = m.by_agent && !m.auto_fixed;
                w.merge = Some(MergeRequest { phase: MergePhase::Failed, reason: reason.clone(), auto_fixed: m.auto_fixed || send, ..m });
                let then = if send { " Sent back to the agent once." } else { "" };
                notice(app, &w, &format!("Merge of {} failed: {reason}{then}", w.branch)).await;
                send.then(|| (w.sessions.last().cloned(), reason))
            }
        };
        if let Err(e) = save(app, &w).await {
            tracing::warn!("worktree merge {id}: {e:#}");
        }
        result.map(|r| (w, r))
    };
    // The agent rebases itself, after the locks are released.
    if let Some((w, (Some(session), reason))) = outcome
        && let Err(e) = app.sessions.send_follow_up(app, &session, &conflict_message(app, &w, &reason, &session), false).await
    {
        tracing::warn!("worktree conflict message {session}: {e:#}");
    }
}

enum Step {
    Merged(String, String),
    Wait(String),
    Conflict(String),
}

async fn try_merge(app: &App, w: &mut TaskWorktree) -> Result<Step> {
    let base = w.base_branch.clone().context("no base branch")?;
    if let Some(reason) = idle_agent_reason(app, w, false) {
        return Ok(Step::Wait(format!("Waiting: {reason}")));
    }
    ensure!(Path::new(&w.root).is_dir(), "the worktree folder is missing");
    let dirty = dirty_files(app, &w.root).await?;
    if !dirty.is_empty() {
        return Ok(Step::Conflict(format!("the worktree has {} uncommitted files ({}); commit them first", dirty.len(), dirty.iter().take(5).cloned().collect::<Vec<_>>().join(", "))));
    }
    ensure!(!operation_in_progress(app, &w.root).await, "a Git operation is in progress in the worktree");
    let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
    ensure!(branch == w.branch, "the worktree is on {branch:?} instead of {}", w.branch);
    if let Err(e) = push(app, w).await {
        return Ok(Step::Wait(format!("Waiting: the branch is not on Home yet ({e:#})")));
    }
    let head = w.pushed.clone().context("the branch is not on Home")?;
    if let Some(reviewed) = w.merge.as_ref().and_then(|m| m.head.as_deref()) {
        ensure!(head == reviewed, "{} changed since review ({} now); review again", w.branch, &head[..12.min(head.len())]);
    }
    let outcome = if crate::home_git::here(app) {
        let project = crate::projects::get(app, &w.project_id).await?.context("the folder no longer exists")?;
        merge_on_home(app, &project, &w.branch, &head, &base).await?
    } else {
        let v = crate::home_link::home_request("POST", &format!("/api/home-git/{}/merge", w.project_id), Some(json!({"branch": w.branch, "head": head, "base": base}))).await?;
        serde_json::from_value(v)?
    };
    Ok(match outcome {
        crate::home_git::MergeOutcome::Merged { head, note } => Step::Merged(head, note),
        crate::home_git::MergeOutcome::NeedsRebase { reason, .. } => Step::Conflict(reason),
    })
}

/// Home: a worktree merge (`home_git::merge`) that also closes the open Local PRs of the branch, so they
/// leave the review lists as merged. Every worktree merge goes through here (Home, a runner's request, away).
pub async fn merge_on_home(app: &App, project: &Project, branch: &str, head: &str, base: &str) -> Result<crate::home_git::MergeOutcome> {
    let outcome = crate::home_git::merge(app, project, branch, head, base).await?;
    if let crate::home_git::MergeOutcome::Merged { head, .. } = &outcome
        && let Err(e) = crate::predrafts::close_merged(app, &project.id, branch, head).await
    {
        tracing::warn!("worktree merge of {branch}: Local PR not marked merged: {e:#}");
    }
    Ok(outcome)
}

/// Links the worktree to the Local PR of its branch.
pub async fn mark_predraft(app: &App, id: &str, predraft_id: &str) -> Result<()> {
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    w.predraft_id = Some(predraft_id.to_string());
    w.merge = None;
    save(app, &w).await
}

/// After Archive: remove the worktrees whose sessions are all archived.
pub async fn after_archive(app: &App, session_id: &str) {
    let Ok(records) = for_session(app, session_id).await else { return };
    for w in records.into_iter().filter(|w| w.state == WorktreeState::Active) {
        let archived = w.sessions.iter().all(|id| app.sessions.get(id).is_none_or(|s| s.status == SessionStatus::Closed));
        if archived && read_only(app, &w) && unchanged(app, &w).await {
            let _action = app.sessions.action_lock().await;
            let _lock = app.worktree_lock.lock().await;
            let Ok(mut w) = get(app, &w.id).await else { continue };
            if w.state != WorktreeState::Active || w.merge.is_some() || idle_reason(app, &w, true).is_some() {
                continue;
            }
            let result: Result<()> = async {
                for id in w.sessions.clone() {
                    app.sessions.release(app, &id).await?;
                }
                discard_locked(app, &mut w).await
            }.await;
            if let Err(e) = result {
                tracing::warn!("read-only worktree {}: {e:#}", w.root);
            }
        } else if archived {
            cleanup(app, &w.id).await;
        }
    }
}

/// Every session of the worktree was launched read only.
fn read_only(app: &App, w: &TaskWorktree) -> bool {
    w.pull_request.is_none() && !w.from_branch && w.predraft_id.is_none() && w.merged.is_none()
        && w.sessions.iter().all(|id| app.sessions.get(id).and_then(|s| s.launch_spec.worktree).is_some_and(|o| o.when_done == crate::model::WhenDone::ReadOnly))
}

/// No commit beyond the base branch and no changed or new file (Git-ignored files do not count).
async fn unchanged(app: &App, w: &TaskWorktree) -> bool {
    let Some(base) = &w.base_branch else { return false };
    if !Path::new(&w.root).is_dir() {
        return false;
    }
    dirty_files(app, &w.root).await.is_ok_and(|d| d.is_empty())
        && git(app, &w.root, &["rev-list", "--count", &format!("refs/remotes/origin/{base}..refs/heads/{}", w.branch), "--"]).await.is_ok_and(|n| n == "0")
}

async fn cleanup(app: &App, id: &str) {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let Ok(mut w) = get(app, id).await else { return };
    if let Err(e) = remove_clean(app, &mut w).await {
        let note = format!("Kept: {e:#}");
        if w.cleanup_note.as_deref() != Some(note.as_str()) {
            w.cleanup_note = Some(note);
            let _ = save(app, &w).await;
        }
    }
}

fn last_activity(app: &App, w: &TaskWorktree) -> i64 {
    w.sessions.iter().filter_map(|id| app.sessions.get(id)).map(|s: Session| s.status_since).max().unwrap_or(w.created_at)
}

/// Retention: keeps the newest `keep` worktrees of idle agents; older ones are removed once Home has their
/// branch (Resume clones them again). Busy and pending-merge worktrees always stay.
async fn retention(app: &App) {
    let keep = app.config.get().worktrees.keep as usize;
    let Ok(records) = list(app).await else { return };
    let mut active: Vec<_> = records.into_iter().filter(|w| w.state == WorktreeState::Active).collect();
    active.sort_by_key(|w| std::cmp::Reverse(last_activity(app, w)));
    let archived = |w: &TaskWorktree| w.sessions.iter().all(|id| app.sessions.get(id).is_none_or(|s| s.status == SessionStatus::Closed));
    for (i, w) in active.iter().enumerate() {
        if archived(w) || (keep > 0 && i >= keep) {
            cleanup(app, &w.id).await;
        }
    }
}

/// Retries waiting merges every 15 s, snapshots running worktrees every five minutes, and applies retention
/// every hour (and at startup).
pub fn spawn(app: &App) {
    if !app.paths.role.runs_agents() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        let mut last = HashMap::new();
        loop {
            if tick.is_multiple_of(240) {
                retention(&app).await;
            }
            if tick > 0 && tick.is_multiple_of(SNAPSHOT_EVERY.as_secs() / 15) {
                snapshots(&app, &mut last).await;
            }
            if let Ok(records) = list(&app).await {
                for w in records.iter().filter(|w| w.merge.as_ref().is_some_and(|m| m.phase == MergePhase::Waiting)) {
                    attempt(&app, &w.id).await;
                }
            }
            tick += 1;
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
}

/// Disk usage of the worktree folders, in bytes (Settings → Worktrees).
pub async fn sizes(app: &App) -> Result<HashMap<String, u64>> {
    let records: Vec<TaskWorktree> = list(app).await?.into_iter().filter(|w| w.state == WorktreeState::Active).collect();
    let roots: Vec<String> = records.iter().map(|w| w.root.clone()).collect();
    let sizes = crate::files::disk_usage(&roots).await;
    Ok(records.into_iter().zip(sizes).map(|(w, s)| (w.id, s)).collect())
}

//! Local pull requests, on Home: a Launchpad branch of Home Git under review. Diff and review read Home's
//! mirror, so they work while the runner that wrote the branch sleeps. Only `publish` writes to GitHub, and
//! it pushes exactly the reviewed commit. Local PRs opened by one session form a change set.
//!
//! On a runner, every request goes to Home (the server forwards them); `open_for_branch` and `get` do it for
//! core callers.
use crate::{App, model::SessionStatus};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Note {
    pub path: Option<String>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Predraft {
    pub id: String,
    pub project_id: String,
    /// The Launchpad branch on Home.
    pub branch: String,
    pub base_branch: String,
    pub initial_base: String,
    pub title: String,
    pub body: String,
    /// The session that works on it last.
    pub session_id: Option<String>,
    pub notes: Vec<Note>,
    pub archived: bool,
    pub published_url: Option<String>,
    pub published_number: Option<u64>,
    pub default_remote_branch: String,
    pub remote_branch: Option<String>,
    pub pushed_head: Option<String>,
    /// GitHub repository the branch is pushed to when it is not `origin` (the owner's fork).
    #[serde(default)]
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_repo: Option<String>,
    pub created_at: i64,
    /// How the last merge or publication reached the remote.
    #[serde(default)]
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_sync_note: Option<String>,
    /// Merged into the base branch on Home; the push to the remote is best effort.
    #[serde(default)]
    pub merged: Option<LocalMerge>,
    /// Local PRs opened by the same session: reviewed and published together, linked to each other.
    #[serde(default)]
    pub change_set: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct LocalMerge {
    /// The merged commit, now the tip of the base branch.
    pub head: String,
    pub at: i64,
}
impl Predraft {
    /// Still a Local PR under review: not discarded, published or merged.
    pub fn is_open(&self) -> bool {
        !self.archived && self.published_url.is_none() && self.merged.is_none()
    }
    /// Agents may work on it: an open Local PR, or a published one (Home pushes their commits to the PR).
    pub fn agents_can_work(&self) -> bool {
        !self.archived && self.merged.is_none()
    }
}
#[derive(Default, Deserialize, Serialize)]
pub struct Create {
    pub project_id: Option<String>,
    /// An agent's worktree (`launchpad pr create`): its branch is pushed to Home first.
    pub cwd: Option<String>,
    /// A Launchpad branch on Home.
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
}
#[derive(Default, Deserialize)]
pub struct Patch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub notes: Option<Vec<Note>>,
}
#[derive(Deserialize)]
pub struct Publish {
    pub remote_branch: String,
    pub draft: bool,
    pub head: String,
    /// Push to the signed-in user's fork of `origin` and open the PR from there.
    #[serde(default)]
    pub fork: bool,
}
/// Where a publication pushes: a remote name or URL, and the GitHub repository behind it.
pub(crate) struct PushTarget {
    pub(crate) remote: String,
    pub(crate) repo: String,
}
/// The URL of `fork` built from the origin URL, keeping its scheme (SSH or HTTPS).
pub(crate) fn fork_url(origin_url: &str, origin_repo: &str, fork: &str) -> Option<String> {
    let trimmed = origin_url.trim_end_matches('/');
    let suffix = if trimmed.ends_with(".git") { ".git" } else { "" };
    let base = trimmed.strip_suffix(suffix)?.strip_suffix(origin_repo)?;
    Some(format!("{base}{fork}{suffix}"))
}
/// The mirror's remote (`origin`, the folder's remote) or the owner's fork.
pub(crate) async fn push_target(mirror: &Path, fork: Option<&str>) -> Result<PushTarget> {
    let remotes = crate::git::remotes(mirror).await;
    let (origin_url, origin_repo) = remotes
        .iter()
        .find(|(n, _)| n == "origin")
        .and_then(|(_, u)| Some((u.clone(), crate::git::github_repo(u)?)))
        .context("the folder's remote must be a GitHub repository")?;
    let Some(fork) = fork.filter(|f| *f != origin_repo) else {
        return Ok(PushTarget { remote: "origin".into(), repo: origin_repo });
    };
    let url = fork_url(&origin_url, &origin_repo, fork).context("cannot derive the fork URL from origin")?;
    Ok(PushTarget { remote: url, repo: fork.to_string() })
}
/// The signed-in user's fork of `origin`, checked on GitHub.
async fn user_fork(app: &App, mirror: &Path) -> Result<String> {
    let origin = push_target(mirror, None).await?.repo;
    let login = app.github.get("/user").await?["login"].as_str().context("GitHub login missing")?.to_string();
    let name = origin.split('/').nth(1).context("repository name")?;
    let fork = format!("{login}/{name}");
    ensure!(fork != origin, "you own {origin}; publish without the fork option");
    let v = app.github.get(&format!("/repos/{fork}")).await.with_context(|| format!("fork {fork} not found; fork {origin} on GitHub first"))?;
    ensure!(v["fork"].as_bool() == Some(true), "{fork} is not a fork of {origin}");
    Ok(fork)
}
#[derive(Deserialize)]
pub struct MergeLocal {
    /// The reviewed head; the merge refuses a branch that changed since.
    pub head: String,
}

/// The repository skill, installed as a managed copy for agents on SSH hosts.
pub const SKILL: &str = include_str!("../../../skills/launchpad/SKILL.md");

pub fn link_skill() -> Result<()> {
    let installed = std::env::current_exe()?
        .parent()
        .context("binary directory")?
        .join("skills/launchpad");
    let source = if installed.exists() {
        installed
    } else {
        Path::new(crate::SOURCE).join("skills/launchpad")
    };
    if !source.exists() {
        return Ok(());
    }
    let dest = dirs::home_dir()
        .context("home directory")?
        .join(".agents/skills/launchpad");
    std::fs::create_dir_all(dest.parent().unwrap())?;
    if let Ok(meta) = std::fs::symlink_metadata(&dest) {
        if !meta.file_type().is_symlink() {
            #[cfg(windows)]
            if dest.join(".launchpad-managed").exists() {
                std::fs::copy(source.join("SKILL.md"), dest.join("SKILL.md"))?;
                return Ok(());
            }
            tracing::warn!("keeping existing launchpad skill directory");
            return Ok(());
        }
        if std::fs::read_link(&dest)? == source {
            return Ok(());
        }
        #[cfg(unix)]
        std::fs::remove_file(&dest)?;
        #[cfg(windows)]
        std::fs::remove_dir(&dest)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, dest)?;
    #[cfg(windows)]
    {
        std::fs::create_dir_all(&dest)?;
        std::fs::copy(source.join("SKILL.md"), dest.join("SKILL.md"))?;
        std::fs::write(dest.join(".launchpad-managed"), "")?;
    }
    Ok(())
}
async fn git(app: &App, dir: &Path, args: &[&str]) -> Result<String> {
    crate::home_git::git_in(app, dir, args, 120).await
}
fn ensure_home(app: &App) -> Result<()> {
    ensure!(crate::home_git::here(app), "Local PRs live on Home");
    Ok(())
}
pub async fn list(app: &App) -> Result<Vec<Predraft>> {
    if !crate::home_git::here(app) {
        return Ok(serde_json::from_value(crate::home_link::home_request("GET", "/api/predrafts", None).await?)?);
    }
    let mut rows: Vec<Predraft> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM predraft")?;
            let texts = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(texts.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await?;
    rows.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    Ok(rows)
}
pub async fn get(app: &App, id: &str) -> Result<Predraft> {
    if !crate::home_git::here(app) {
        return Ok(serde_json::from_value(crate::home_link::home_request("GET", &format!("/api/predrafts/{id}/record"), None).await?)?);
    }
    list(app).await?.into_iter().find(|p| p.id == id).context("Local PR not found")
}
pub async fn save(app: &App, p: &Predraft) -> Result<()> {
    ensure_home(app)?;
    let id = p.id.clone();
    let text = serde_json::to_string(p)?;
    app.state_db.call(move |c| { c.execute("INSERT INTO predraft (id,json) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![id,text])?; Ok(()) }).await?;
    app.bus.publish("predrafts", "updated", json!(p));
    Ok(())
}
/// The open Local PR of a branch.
pub async fn for_branch(app: &App, project_id: &str, branch: &str) -> Result<Option<Predraft>> {
    Ok(list(app).await?.into_iter().find(|p| p.project_id == project_id && p.branch == branch && p.is_open()))
}
/// `launchpad pr create|show|update` in a worktree: the Local PR of its branch.
pub async fn resolve(app: &App, cwd: &str) -> Result<Predraft> {
    let w = crate::worktrees::find_by_path(app, cwd).await?.context("this folder is not a Launchpad worktree")?;
    for_branch(app, &w.project_id, &w.branch).await?.context("no Local PR for this branch yet; run launchpad pr create")
}
/// A Local PR of a Launchpad branch (Home, or forwarded there). An open Local PR of the same branch is
/// returned, now linked to `session_id`.
pub async fn open_for_branch(app: &App, project_id: &str, branch: &str, base: Option<&str>, session_id: Option<&str>, title: Option<String>) -> Result<Predraft> {
    let q = Create { project_id: Some(project_id.into()), branch: Some(branch.into()), base_branch: base.map(str::to_string), session_id: session_id.map(str::to_string), title, ..Default::default() };
    if !crate::home_git::here(app) {
        return Ok(serde_json::from_value(crate::home_link::home_request("POST", "/api/predrafts", Some(serde_json::to_value(&q)?)).await?)?);
    }
    create_record(app, q).await
}
pub async fn create(app: &App, q: Create) -> Result<Predraft> {
    if let Some(cwd) = q.cwd.clone() {
        // An agent's worktree: Home gets the branch first, then opens its Local PR.
        let mut w = crate::worktrees::find_by_path(app, &cwd).await?.context("run this in a Launchpad worktree")?;
        ensure!(w.pull_request.is_none(), "this worktree continues a pull request; push your commits to it instead of creating a Local PR");
        crate::worktrees::push(app, &mut w).await.context("push the branch to Home")?;
        let session = q.session_id.clone().or_else(|| w.sessions.last().cloned());
        let p = open_for_branch(app, &w.project_id, &w.branch, w.base_branch.as_deref(), session.as_deref(), q.title.clone()).await?;
        crate::worktrees::mark_predraft(app, &w.id, &p.id).await?;
        if q.body.is_some() || q.title.is_some() {
            return update_anywhere(app, &p.id, Patch { title: q.title, body: q.body, notes: None }).await;
        }
        return Ok(p);
    }
    create_record(app, q).await
}
/// Home: the Local PR of a branch, created or found.
async fn create_record(app: &App, q: Create) -> Result<Predraft> {
    ensure_home(app)?;
    let _lock = app.predraft_lock.lock().await;
    let project_id = q.project_id.clone().context("project_id required")?;
    let project = crate::projects::get(app, &project_id).await?.context("folder not found")?;
    ensure!(project.home_git, "{} is not a Git folder of Home", project.name);
    let branch = q.branch.clone().context("branch required")?;
    ensure!(branch.starts_with("lp/"), "a Local PR needs a Launchpad branch (lp/…)");
    if let Some(mut p) = list(app).await?.into_iter().find(|p| p.project_id == project_id && p.branch == branch && p.is_open()) {
        if q.session_id.is_some() && p.session_id != q.session_id {
            p.session_id = q.session_id.clone();
            save(app, &p).await?;
        }
        return Ok(p);
    }
    let dir = crate::home_git::ensure(app, &project).await?;
    let base_branch = match q.base_branch.clone().filter(|b| !b.is_empty()) {
        Some(b) => b,
        None => crate::home_git::default_branch(app, &project_id).await?,
    };
    let initial_base = git(app, &dir, &["rev-parse", "--verify", &format!("refs/heads/{base_branch}^{{commit}}")]).await.with_context(|| format!("{base_branch} not found on Home"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let slug = branch.trim_start_matches("lp/").to_string();
    let user = app.github.get("/user").await.ok().and_then(|u| u["login"].as_str().map(str::to_string));
    let p = Predraft {
        default_remote_branch: format!("{}/{}", user.unwrap_or_else(|| "local".into()), slug),
        id,
        project_id,
        branch,
        base_branch,
        initial_base,
        title: q.title.unwrap_or_else(|| "Local pull request".into()),
        body: q.body.unwrap_or_default(),
        change_set: q.session_id.clone(),
        session_id: q.session_id,
        notes: vec![],
        archived: false,
        published_url: None,
        published_number: None,
        remote_branch: None,
        pushed_head: None,
        push_repo: None,
        created_at: crate::util::now_ms(),
        remote_sync_note: None,
        merged: None,
    };
    save(app, &p).await?;
    Ok(p)
}
/// Launch instructions for an agent that works on a Local PR's branch.
pub fn instruction(app: &App, p: &Predraft, cwd: &str) -> String {
    match &p.published_url {
        Some(url) => format!(
            "You are continuing Local PR {} (branch `{}`), published as {url}. Commit your work on this branch; Launchpad pushes it to Home and Home pushes it to the pull request. Never force-push or rewrite commits that are already pushed. Do not open another pull request. Comments and reviews still go through `launchpad suggest`; never post them yourself.",
            p.id, p.branch
        ),
        None => format!(
            "You are working on Local PR {} (branch `{}`). Commit your work on this branch; Launchpad pushes it to Home. Do not open a pull request on GitHub. When done, set the title and description with `{} pr update --title \"…\" --body-file <file>`.",
            p.id, p.branch, app.cli_command_in(cwd)
        ),
    }
}

fn editable(p: &Predraft) -> Result<()> {
    ensure!(p.is_open(), "this Local PR is archived, published or merged");
    Ok(())
}
/// The agent of the Local PR is not running.
fn agents_idle(app: &App, p: &Predraft) -> Result<()> {
    ensure!(
        !p.session_id.as_ref().and_then(|id| app.sessions.get(id)).is_some_and(|s| app.sessions.is_busy(&s) || s.status == SessionStatus::InTerminal),
        "the agent of this Local PR is running; wait for it to finish"
    );
    Ok(())
}
pub async fn update(app: &App, id: &str, q: Patch) -> Result<Predraft> {
    ensure_home(app)?;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    if let Some(t) = q.title {
        ensure!(!t.trim().is_empty(), "title is empty");
        p.title = t;
    }
    if let Some(b) = q.body {
        p.body = b;
    }
    if let Some(n) = q.notes {
        p.notes = n;
    }
    save(app, &p).await?;
    Ok(p)
}
/// `update` here or on Home.
pub async fn update_anywhere(app: &App, id: &str, q: Patch) -> Result<Predraft> {
    if crate::home_git::here(app) {
        return update(app, id, q).await;
    }
    let body = json!({"title": q.title, "body": q.body, "notes": q.notes});
    Ok(serde_json::from_value(crate::home_link::home_request("PATCH", &format!("/api/predrafts/{id}"), Some(body)).await?)?)
}
pub async fn detail(app: &App, id: &str, fetch: bool) -> Result<Value> {
    ensure_home(app)?;
    let p = get(app, id).await?;
    let status = p.session_id.as_ref().and_then(|id| app.sessions.get(id)).map(|s| s.status.as_str().to_string());
    // A removed folder leaves the record readable so it can be discarded.
    let Some(project) = crate::projects::get(app, &p.project_id).await? else {
        return Ok(json!({"predraft":p,"status":status,"files":[],"commits":[],"base_ahead":0,"project_missing":true}));
    };
    let set = change_set(app, &p).await?;
    let dir = crate::home_git::ensure(app, &project).await?;
    if fetch {
        crate::home_git::refresh(app, &project).await?;
    }
    let head = crate::home_git::tip(app, &p.project_id, &format!("refs/heads/{}", p.branch)).await?;
    let base = crate::home_git::tip(app, &p.project_id, &format!("refs/heads/{}", p.base_branch)).await?;
    let (Some(head), Some(base)) = (head, base) else {
        return Ok(json!({"predraft":p,"status":status,"files":[],"commits":[],"base_ahead":0,"project_id":project.id,"change_set":set,"default_remote_branch":p.remote_branch.clone().unwrap_or_else(|| p.default_remote_branch.clone())}));
    };
    if p.archived {
        return Ok(json!({"predraft":p,"status":status,"files":[],"commits":[],"base_ahead":0,"change_set":set}));
    }
    let merge_base = git(app, &dir, &["merge-base", &base, &head]).await?;
    let patch = git(app, &dir, &["-c", "core.quotepath=false", "diff", "--no-ext-diff", "--no-textconv", "--no-color", "--find-renames", &format!("{base}...{head}"), "--"]).await?;
    let commits = git(app, &dir, &["log", "--format=%h %s", &format!("{base}..{head}"), "--"]).await?;
    let ahead = git(app, &dir, &["rev-list", "--count", &format!("{head}..{base}")]).await?.parse::<u64>()?;
    let remote = p.remote_branch.clone().unwrap_or_else(|| p.default_remote_branch.clone());
    Ok(json!({"predraft":p,"status":status,"head":head,"base":merge_base,"base_ahead":ahead,"commits":commits.lines().collect::<Vec<_>>(),"files":crate::items::parse_diff(&patch),"project_id":project.id,"fallback":false,"default_remote_branch":remote,"change_set":set}))
}
/// The other Local PRs of the same change set, with their folders.
async fn change_set(app: &App, p: &Predraft) -> Result<Vec<Value>> {
    let Some(set) = &p.change_set else { return Ok(vec![]) };
    let projects = crate::projects::list(app).await?;
    Ok(list(app).await?.into_iter().filter(|o| o.change_set.as_ref() == Some(set) && o.id != p.id && !o.archived).map(|o| {
        let folder = projects.iter().find(|f| f.id == o.project_id).map(|f| f.name.clone()).unwrap_or_default();
        json!({"id": o.id, "title": o.title, "folder": folder, "branch": o.branch, "published_url": o.published_url, "merged": o.merged.is_some()})
    }).collect())
}
pub async fn feedback(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    agents_idle(app, &p)?;
    let session = p.session_id.clone().context("this Local PR has no linked session")?;
    ensure!(!p.notes.is_empty(), "write review notes first");
    let message = format!(
        "Please address these Local PR review notes on branch `{}`, commit your changes, and update the title and description with launchpad pr update. Do not open a pull request on GitHub.\n\n{}",
        p.branch,
        p.notes.iter().map(|n| format!("{}:\n{}", n.path.as_deref().unwrap_or("General"), n.text)).collect::<Vec<_>>().join("\n\n")
    );
    let sent_notes = p.notes.clone();
    drop(_lock);
    crate::sessions::send_anywhere(app, &session, &message).await?;
    let _lock = app.predraft_lock.lock().await;
    p = get(app, id).await?;
    if p.notes == sent_notes {
        p.notes.clear();
        save(app, &p).await?;
    }
    Ok(())
}
/// Discard: archives the Local PR and deletes its branch on Home (the tip stays restorable for 30 days).
pub async fn discard(app: &App, id: &str) -> Result<Predraft> {
    ensure_home(app)?;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    agents_idle(app, &p)?;
    ensure!(p.pushed_head.is_none(), "branch was pushed during publication; retry Publish before discarding");
    if crate::projects::get(app, &p.project_id).await?.is_some() {
        crate::home_git::delete(app, &p.project_id, &p.branch).await?;
    }
    p.archived = true;
    save(app, &p).await?;
    Ok(p)
}
/// Merge on Home: pushes the reviewed commit to the base branch on the folder's remote, then fast-forwards it on
/// Home (`home_git::merge`); a refused push merges nothing. A branch behind its base goes back to its agent.
pub async fn merge_local(app: &App, id: &str, q: MergeLocal) -> Result<Predraft> {
    ensure_home(app)?;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    ensure!(p.pushed_head.is_none(), "publication has started; finish it with Publish instead");
    agents_idle(app, &p)?;
    let project = crate::projects::get(app, &p.project_id).await?.context("folder not found")?;
    match crate::home_git::merge(app, &project, &p.branch, &q.head, &p.base_branch).await? {
        crate::home_git::MergeOutcome::Merged { head, note } => {
            p.remote_sync_note = Some(note);
            p.merged = Some(LocalMerge { head, at: crate::util::now_ms() });
            save(app, &p).await?;
            Ok(p)
        }
        crate::home_git::MergeOutcome::NeedsRebase { reason, .. } => {
            anyhow::bail!("{reason}; nothing was merged. Ask the agent to rebase onto {} (review note and Send to agent), then merge again", p.base_branch)
        }
    }
}
pub async fn publish(app: &App, id: &str, q: Publish) -> Result<Predraft> {
    ensure_home(app)?;
    // Serialize retries of this publication without holding any global gate during network I/O.
    let publication = app.suggestion_locks.get(&format!("predraft-publish:{id}"));
    let _publication = publication.lock().await;
    let first = get(app, id).await?;
    let mirror = crate::home_git::mirror(app, &first.project_id);
    // A retry keeps the target of the first attempt.
    let fork = match (&first.pushed_head, q.fork, first.published_url.is_some()) {
        (Some(_), _, _) | (_, _, true) => first.push_repo.clone(),
        (None, true, false) => Some(user_fork(app, &mirror).await?),
        (None, false, false) => None,
    };
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    if p.published_url.is_some() {
        return Ok(p);
    }
    editable(&p)?;
    ensure!(!p.title.trim().is_empty(), "title is empty");
    git(app, &mirror, &["check-ref-format", "--branch", &q.remote_branch]).await?;
    ensure!(!q.remote_branch.starts_with('-'), "invalid remote branch");
    let origin = push_target(&mirror, None).await?.repo;
    let target = push_target(&mirror, fork.as_deref()).await?;
    // Home pushes exactly the reviewed commit; a newer push asks for another review.
    let head = crate::home_git::tip(app, &p.project_id, &format!("refs/heads/{}", p.branch)).await?.context("the branch is not on Home")?;
    ensure!(head == q.head, "the branch changed since review; review again");
    if let Some(remote) = &p.remote_branch {
        ensure!(remote == &q.remote_branch, "retry with the same remote branch");
    }
    if let Some(pushed) = &p.pushed_head {
        ensure!(pushed == &head, "publication already started for a different head");
    }
    p.pushed_head = Some(head.clone());
    p.push_repo = fork.clone();
    save(app, &p).await?;
    drop(_lock);
    if p.remote_branch.is_none() {
        let available: Result<()> = async {
            let exists = crate::home_git::git_in(app, &mirror, &["ls-remote", "--heads", &target.remote, &format!("refs/heads/{}", q.remote_branch)], 60).await?;
            ensure!(exists.is_empty(), "remote branch already exists; choose a new name");
            Ok(())
        }.await;
        let _lock = app.predraft_lock.lock().await;
        // Nothing was pushed if this read-only check failed: release the claim.
        p = get(app, id).await?;
        if let Err(e) = available {
            p.pushed_head = None;
            p.push_repo = None;
            save(app, &p).await?;
            return Err(e);
        }
        p.remote_branch = Some(q.remote_branch.clone());
        save(app, &p).await?;
    }
    if let Err(e) = crate::home_git::push(app, &p.project_id, &target.remote, &head, &q.remote_branch).await {
        // Unlock only when the remote proves nothing arrived; otherwise the claim stays for Publish or Unlock.
        let _lock = app.predraft_lock.lock().await;
        if remote_branch_head(app, &mirror, &target.remote, &q.remote_branch).await.is_ok_and(|h| h.is_none()) {
            p = get(app, id).await?;
            release(&mut p);
            save(app, &p).await?;
            return Err(e.context(format!("push to {} failed; nothing was pushed and the Local PR is unlocked", target.repo)));
        }
        return Err(e.context(format!("push to {} failed; retry Publish or Unlock the Local PR", target.repo)));
    }
    let owner = target.repo.split('/').next().context("repository owner")?;
    let pr_head = if target.repo == origin { q.remote_branch.clone() } else { format!("{owner}:{}", q.remote_branch) };
    let repo = origin;
    let mut query = reqwest::Url::parse(&format!("https://api.github.com/repos/{repo}/pulls"))?;
    query.query_pairs_mut().append_pair("state", "all").append_pair("head", &format!("{owner}:{}", q.remote_branch));
    let candidates = app.github.get(&format!("{}?{}", query.path(), query.query().unwrap_or_default())).await?;
    let found = candidates
        .as_array()
        .and_then(|a| a.iter().find(|v| v["head"]["ref"] == q.remote_branch && v["head"]["sha"] == head && v["base"]["ref"] == p.base_branch))
        .cloned();
    let body = with_links(&p.body, &published_siblings(app, &p).await?);
    let pr = if let Some(v) = found {
        v
    } else {
        app.github.request(Method::POST, &format!("/repos/{repo}/pulls"), Some(json!({"title":p.title,"body":body,"head":pr_head,"base":p.base_branch,"draft":q.draft})), false).await?.value
    };
    let _lock = app.predraft_lock.lock().await;
    p = get(app, id).await?;
    p.published_url = Some(pr["html_url"].as_str().context("PR URL missing")?.to_string());
    p.published_number = pr["number"].as_u64();
    save(app, &p).await?;
    // Later commits of the branch reach the pull request through Home.
    let follow = crate::home_git::Follow { remote: target.remote.clone(), branch: q.remote_branch.clone(), url: p.published_url.clone().unwrap_or_default(), session_id: p.session_id.clone() };
    crate::home_git::set_follow(app, &p.project_id, &p.branch, &follow).await?;
    Ok(p)
}
/// Published pull requests of the other Local PRs of the change set.
async fn published_siblings(app: &App, p: &Predraft) -> Result<Vec<String>> {
    let Some(set) = &p.change_set else { return Ok(vec![]) };
    Ok(list(app).await?.into_iter().filter(|o| o.change_set.as_ref() == Some(set) && o.id != p.id).filter_map(|o| o.published_url).collect())
}
const LINKS: &str = "\n\n---\nPart of a change set: ";
fn with_links(body: &str, links: &[String]) -> String {
    let body = body.split(LINKS).next().unwrap_or(body);
    if links.is_empty() { body.to_string() } else { format!("{body}{LINKS}{}", links.join(", ")) }
}
/// Publish all: publishes every open Local PR of a change set (each with its default remote branch), then
/// links the pull requests to each other. A failure stops there; publishing again resumes.
pub async fn publish_set(app: &App, id: &str, draft: bool) -> Result<Vec<Predraft>> {
    ensure_home(app)?;
    let p = get(app, id).await?;
    let set = p.change_set.clone().context("this Local PR is not part of a change set")?;
    let members: Vec<Predraft> = list(app).await?.into_iter().filter(|o| o.change_set.as_ref() == Some(&set) && !o.archived && o.merged.is_none()).collect();
    for m in &members {
        if m.published_url.is_some() {
            continue;
        }
        let head = crate::home_git::tip(app, &m.project_id, &format!("refs/heads/{}", m.branch)).await?.with_context(|| format!("{} is not on Home", m.branch))?;
        let remote_branch = m.remote_branch.clone().unwrap_or_else(|| m.default_remote_branch.clone());
        publish(app, &m.id, Publish { remote_branch, draft, head, fork: false }).await.with_context(|| format!("publish {}", m.title))?;
    }
    let published: Vec<Predraft> = list(app).await?.into_iter().filter(|o| o.change_set.as_ref() == Some(&set) && o.published_url.is_some()).collect();
    for m in &published {
        let others: Vec<String> = published.iter().filter(|o| o.id != m.id).filter_map(|o| o.published_url.clone()).collect();
        let (Some(url), Some(number)) = (&m.published_url, m.published_number) else { continue };
        let Some(repo) = crate::git::github_repo(url.split("/pull/").next().unwrap_or_default()) else { continue };
        if let Err(e) = app.github.request(Method::PATCH, &format!("/repos/{repo}/pulls/{number}"), Some(json!({"body": with_links(&m.body, &others)})), false).await {
            tracing::warn!("link {url} to its change set: {e:#}");
        }
    }
    Ok(published)
}

/// The commit of a remote branch, `None` when the branch does not exist.
async fn remote_branch_head(app: &App, mirror: &Path, remote: &str, branch: &str) -> Result<Option<String>> {
    let out = crate::home_git::git_in(app, mirror, &["ls-remote", "--heads", remote, &format!("refs/heads/{branch}")], 60).await?;
    Ok(out.split_whitespace().next().map(str::to_string))
}
/// Ends an unfinished publication.
fn release(p: &mut Predraft) {
    p.pushed_head = None;
    p.remote_branch = None;
    p.push_repo = None;
}
/// Owner click: unlock a Local PR whose publication did not finish. A branch that already reached the
/// remote keeps its name and target, so the next Publish updates it.
pub async fn unlock(app: &App, id: &str) -> Result<Predraft> {
    ensure_home(app)?;
    let publication = app.suggestion_locks.get(&format!("predraft-publish:{id}"));
    let _publication = publication.lock().await;
    let p = get(app, id).await?;
    editable(&p)?;
    ensure!(p.pushed_head.is_some(), "publication has not started");
    let mirror = crate::home_git::mirror(app, &p.project_id);
    let remote = match &p.remote_branch {
        Some(branch) => {
            let target = push_target(&mirror, p.push_repo.as_deref()).await?;
            remote_branch_head(app, &mirror, &target.remote, branch).await.context("could not read the remote branch; try again")?
        }
        None => None,
    };
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    if remote.is_some() {
        p.pushed_head = None;
    } else {
        release(&mut p);
    }
    save(app, &p).await?;
    Ok(p)
}

#[cfg(test)]
#[path = "predrafts_tests.rs"]
mod tests;

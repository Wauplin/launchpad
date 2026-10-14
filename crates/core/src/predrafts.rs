//! Local pull requests. Only `publish` writes to a remote.
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
    pub root: String,
    pub branch: String,
    pub base_branch: String,
    pub initial_base: String,
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
    pub notes: Vec<Note>,
    pub archived: bool,
    pub published_url: Option<String>,
    pub published_number: Option<u64>,
    pub default_remote_branch: String,
    pub remote_branch: Option<String>,
    pub pushed_head: Option<String>,
    pub created_at: i64,
}
#[derive(Default, Deserialize)]
pub struct Create {
    pub project_id: Option<String>,
    pub cwd: Option<String>,
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
async fn git(app: &App, root: &str, args: &[&str]) -> Result<String> {
    crate::procs::output(&app.env, "git", args, Some(Path::new(root))).await
}
pub async fn list(app: &App) -> Result<Vec<Predraft>> {
    let mut rows: Vec<Predraft> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM predraft")?;
            let texts = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            texts
                .into_iter()
                .map(|s| Ok(serde_json::from_str(&s)?))
                .collect()
        })
        .await?;
    for p in &mut rows {
        // A crash between worktree creation and session persistence can still be recovered by cwd.
        if p.session_id
            .as_ref()
            .is_none_or(|id| app.sessions.get(id).is_none())
        {
            p.session_id = app
                .sessions
                .list()
                .into_iter()
                .find(|s| s.cwd == p.root)
                .map(|s| s.id);
        }
    }
    rows.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    Ok(rows)
}
pub async fn get(app: &App, id: &str) -> Result<Predraft> {
    list(app)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .context("pre-draft not found")
}
pub async fn check_session_work(app: &App, root: &str) -> Result<()> {
    if let Some(p) = list(app).await?.into_iter().find(|p| p.root == root) {
        ensure!(
            !p.archived && p.pushed_head.is_none(),
            "this pre-draft was discarded or publication has started; finish publication in its review page"
        );
    }
    Ok(())
}
pub async fn resolve(app: &App, cwd: &str) -> Result<Predraft> {
    let root = crate::hosts::loc::rehost(cwd, &git(app, cwd, &["rev-parse", "--show-toplevel"]).await?);
    list(app)
        .await?
        .into_iter()
        .find(|p| p.root == root && !p.archived && p.published_url.is_none())
        .context("no local pre-draft in this worktree")
}
pub async fn save(app: &App, p: &Predraft) -> Result<()> {
    let id = p.id.clone();
    let text = serde_json::to_string(p)?;
    app.state_db.call(move |c| { c.execute("INSERT INTO predraft (id,json) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![id,text])?; Ok(()) }).await?;
    app.bus.publish("predrafts", "updated", json!(p));
    Ok(())
}
pub async fn create(app: &App, q: Create) -> Result<Predraft> {
    let _lock = app.predraft_lock.lock().await;
    let (project, existing) = if let Some(id) = q.project_id {
        (
            crate::projects::get(app, &id)
                .await?
                .context("project not found")?,
            None,
        )
    } else {
        let cwd = q.cwd.context("project_id or cwd required")?;
        if let Ok(p) = resolve(app, &cwd).await {
            return Ok(p);
        }
        let root = crate::hosts::loc::rehost(&cwd, &git(app, &cwd, &["rev-parse", "--show-toplevel"]).await?);
        let source = crate::code::open(app, &root, None).await?;
        (source.project, Some(root))
    };
    git(app, &project.path, &["fetch", "origin"]).await?;
    let base_branch = match git(
        app,
        &project.path,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await
    {
        Ok(b) => b
            .strip_prefix("origin/")
            .context("origin default branch")?
            .to_string(),
        Err(_) => {
            let remote = git(
                app,
                &project.path,
                &["ls-remote", "--symref", "origin", "HEAD"],
            )
            .await?;
            remote
                .lines()
                .find_map(|l| {
                    l.strip_prefix("ref: refs/heads/")
                        .and_then(|s| s.split_once('\t').map(|(b, _)| b.to_string()))
                })
                .context("origin default branch not found")?
        }
    };
    let initial_base = git(
        app,
        &project.path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{base_branch}^{{commit}}"),
        ],
    )
    .await?;
    let id = uuid::Uuid::new_v4().to_string();
    let title = q.title.unwrap_or_else(|| "Local pull request".into());
    // A Launchpad task worktree becomes the pre-draft itself: same folder, branch and commits.
    if let Some(root) = &existing
        && let Ok(root) = crate::hostfs::canonicalize(root).await.map(|r| r.to_string_lossy().into_owned())
        && let Some(w) = crate::worktrees::find_by_path(app, &root).await?.filter(|w| w.root == root && w.primary)
    {
        let user = app.github.get("/user").await.ok().and_then(|u| u["login"].as_str().map(str::to_string));
        let slug = w.branch.trim_start_matches("lp/").to_string();
        let p = Predraft {
            default_remote_branch: format!("{}/{}", user.unwrap_or_else(|| "local".into()), slug),
            id,
            project_id: project.id,
            root: w.root.clone(),
            branch: w.branch.clone(),
            base_branch,
            initial_base,
            title,
            body: q.body.unwrap_or_default(),
            session_id: w.sessions.last().cloned(),
            notes: vec![],
            archived: false,
            published_url: None,
            published_number: None,
            remote_branch: None,
            pushed_head: None,
            created_at: crate::util::now_ms(),
        };
        save(app, &p).await?;
        crate::worktrees::mark_predraft(app, &w.root, &p.id).await?;
        return Ok(p);
    }
    let slug = format!("{}-{}", crate::compose::slug_from(&title), &id[..8]);
    let branch = format!("lp/{slug}");
    let parent = crate::projects::projects_root(app, crate::hosts::loc::host_of(&project.path))?
        .join(".worktrees")
        .join(&project.id);
    crate::hostfs::create_dir_all(&parent).await?;
    let root = parent.join(&slug).to_string_lossy().to_string();
    let start = if let Some(cwd) = &existing {
        ensure!(
            git(app, cwd, &["status", "--porcelain"]).await?.is_empty(),
            "commit existing changes before creating a pre-draft"
        );
        git(app, cwd, &["rev-parse", "HEAD"]).await?
    } else {
        initial_base.clone()
    };
    git(
        app,
        &project.path,
        &["worktree", "add", "-b", &branch, &root, &start],
    )
    .await?;
    let user = app
        .github
        .get("/user")
        .await
        .ok()
        .and_then(|u| u["login"].as_str().map(str::to_string));
    let default_remote_branch = format!("{}/{}", user.unwrap_or_else(|| "local".into()), slug);
    let root = crate::hostfs::canonicalize(&root).await?.to_string_lossy().to_string();
    let p = Predraft {
        default_remote_branch,
        id,
        project_id: project.id,
        root,
        branch,
        base_branch,
        initial_base,
        title,
        body: q.body.unwrap_or_default(),
        session_id: None,
        notes: vec![],
        archived: false,
        published_url: None,
        published_number: None,
        remote_branch: None,
        pushed_head: None,
        created_at: crate::util::now_ms(),
    };
    if let Err(e) = save(app, &p).await {
        let _ = git(app, &project.path, &["worktree", "remove", &p.root]).await;
        let _ = git(app, &project.path, &["branch", "-D", &p.branch]).await;
        return Err(e);
    }
    Ok(p)
}
pub fn instruction(app: &App, p: &Predraft) -> String {
    format!(
        "You are working in a dedicated git worktree on branch `{}`. Commit your work on this branch. Do not push and do not open a pull request on GitHub. Do not remove this worktree. When done, set the title and description with `{} pr update --title \"…\" --body-file <file>`. Pre-draft id: {}.",
        p.branch,
        app.cli_command_in(&p.root),
        p.id
    )
}

fn editable(p: &Predraft) -> Result<()> {
    ensure!(
        !p.archived && p.published_url.is_none(),
        "this pre-draft is archived or published"
    );
    Ok(())
}
fn idle(app: &App, p: &Predraft) -> Result<()> {
    ensure!(
        !app.sessions.list().iter().any(|s| s.cwd == p.root
            && (app.sessions.is_busy(s) || s.status == SessionStatus::InTerminal)),
        "the worktree has an active agent; wait for it to finish"
    );
    let open = app.terminals.alive_in(&p.root).len();
    ensure!(
        open == 0,
        "{open} terminal(s) are open in the worktree; close them first"
    );
    Ok(())
}
pub async fn update(app: &App, id: &str, q: Patch) -> Result<Predraft> {
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
pub async fn detail(app: &App, id: &str, fetch: bool) -> Result<Value> {
    let p = get(app, id).await?;
    let status = p
        .session_id
        .as_ref()
        .and_then(|id| app.sessions.get(id))
        .map(|s| s.status.as_str().to_string());
    // A removed project leaves the record readable so it can be discarded.
    let Some(project) = crate::projects::get(app, &p.project_id).await? else {
        return Ok(
            json!({"predraft":p,"status":status,"files":[],"commits":[],"base_ahead":0,"dirty":false,"project_missing":true}),
        );
    };
    if fetch {
        git(app, &project.path, &["fetch", "origin"]).await?;
    }
    if p.archived || p.published_url.is_some() && !crate::hostfs::exists(&p.root).await {
        return Ok(json!({"predraft":p,"status":status,"files":[],"commits":[],"base_ahead":0}));
    }
    let head = git(
        app,
        &project.path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}^{{commit}}", p.branch),
        ],
    )
    .await?;
    let base = git(
        app,
        &project.path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{}^{{commit}}", p.base_branch),
        ],
    )
    .await?;
    let merge_base = git(app, &project.path, &["merge-base", &base, &head]).await?;
    let patch = git(
        app,
        &project.path,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
            &format!("{base}...{head}"),
            "--",
        ],
    )
    .await?;
    let commits = git(
        app,
        &project.path,
        &["log", "--format=%h %s", &format!("{base}..{head}"), "--"],
    )
    .await?;
    let ahead = git(
        app,
        &project.path,
        &["rev-list", "--count", &format!("{head}..{base}")],
    )
    .await?
    .parse::<u64>()?;
    let dirty = git(app, &p.root, &["status", "--porcelain"]).await?;
    let remote = p
        .remote_branch
        .clone()
        .unwrap_or_else(|| p.default_remote_branch.clone());
    Ok(
        json!({"predraft":p,"status":status,"head":head,"base":merge_base,"base_ahead":ahead,"dirty":!dirty.is_empty(),"commits":commits.lines().collect::<Vec<_>>(),"files":crate::items::parse_diff(&patch),"project_id":project.id,"fallback":false,"default_remote_branch":remote}),
    )
}
pub async fn feedback(app: &App, id: &str) -> Result<()> {
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    idle(app, &p)?;
    let session = p
        .session_id
        .as_ref()
        .context("this pre-draft has no linked session")?;
    ensure!(
        app.sessions.get(session).is_some_and(|s| s.cwd == p.root),
        "session is not in the pre-draft worktree"
    );
    ensure!(!p.notes.is_empty(), "write review notes first");
    let message = format!(
        "Please address these local pre-draft review notes, commit your changes, and update the title and description with launchpad pr update. Do not push or publish.\n\n{}",
        p.notes
            .iter()
            .map(|n| format!("{}:\n{}", n.path.as_deref().unwrap_or("General"), n.text))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let message = format!("{message}\n\n{}", instruction(app, &p));
    let session = session.clone();
    let sent_notes = p.notes.clone();
    drop(_lock);
    app.sessions.send(app, &session, &message, vec![]).await?;
    let _lock = app.predraft_lock.lock().await;
    p = get(app, id).await?;
    if p.notes == sent_notes {
        p.notes.clear();
        save(app, &p).await?;
    }
    Ok(())
}
async fn cleanup(app: &App, p: &Predraft, delete_branch: bool) -> Result<()> {
    idle(app, p)?;
    let Some(project) = crate::projects::get(app, &p.project_id).await? else {
        // Without its project there is no repository to run git in: close the sessions and keep the files on disk.
        app.sessions.close_in(app, &p.root).await?;
        return Ok(());
    };
    if crate::hostfs::exists(&p.root).await {
        ensure!(
            git(
                app,
                &p.root,
                &["status", "--porcelain", "--untracked-files=all"]
            )
            .await?
            .is_empty(),
            "worktree has uncommitted changes; commit or remove them before cleanup"
        );
        app.sessions.close_in(app, &p.root).await?;
        git(app, &project.path, &["worktree", "remove", &p.root]).await?;
    }
    if delete_branch
        && git(
            app,
            &project.path,
            &["show-ref", "--verify", &format!("refs/heads/{}", p.branch)],
        )
        .await
        .is_ok()
    {
        git(app, &project.path, &["branch", "-D", &p.branch]).await?;
    }
    Ok(())
}
pub async fn discard(app: &App, id: &str) -> Result<Predraft> {
    let _work = app.sessions.allow_work().await;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    editable(&p)?;
    ensure!(
        p.pushed_head.is_none(),
        "branch was pushed during publication; retry Publish before archiving"
    );
    cleanup(app, &p, true).await?;
    p.archived = true;
    save(app, &p).await?;
    Ok(p)
}
pub async fn publish(app: &App, id: &str, q: Publish) -> Result<Predraft> {
    let _work = app.sessions.allow_work().await;
    let _lock = app.predraft_lock.lock().await;
    let mut p = get(app, id).await?;
    if p.published_url.is_some() {
        return Ok(p);
    }
    editable(&p)?;
    idle(app, &p)?;
    ensure!(!p.title.trim().is_empty(), "title is empty");
    git(
        app,
        &p.root,
        &["check-ref-format", "--branch", &q.remote_branch],
    )
    .await?;
    ensure!(!q.remote_branch.starts_with('-'), "invalid remote branch");
    ensure!(
        git(
            app,
            &p.root,
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .await?
        .is_empty(),
        "commit all changes before publishing"
    );
    let project = crate::projects::get(app, &p.project_id)
        .await?
        .context("project not found")?;
    let repo = crate::git::remotes(Path::new(&project.path))
        .await
        .into_iter()
        .find(|(n, _)| n == "origin")
        .and_then(|(_, u)| crate::git::github_repo(&u))
        .context("origin must be a GitHub repository")?;
    let head = git(app, &p.root, &["rev-parse", "HEAD"]).await?;
    let branch_head = git(
        app,
        &project.path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}^{{commit}}", p.branch),
        ],
    )
    .await?;
    ensure!(
        head == q.head && branch_head == q.head,
        "branch changed since review; refresh and review again"
    );
    ensure!(
        git(app, &p.root, &["branch", "--show-current"]).await? == p.branch,
        "worktree is on a different branch"
    );
    if let Some(remote) = &p.remote_branch {
        ensure!(
            remote == &q.remote_branch,
            "retry with the same remote branch"
        );
    } else {
        let exists = git(
            app,
            &p.root,
            &[
                "ls-remote",
                "--heads",
                "origin",
                &format!("refs/heads/{}", q.remote_branch),
            ],
        )
        .await?;
        ensure!(
            exists.is_empty(),
            "remote branch already exists; choose a new name"
        );
        p.remote_branch = Some(q.remote_branch.clone());
        save(app, &p).await?;
    }
    // Record the reviewed head before push: an uncertain response is safely retried without force.
    if let Some(pushed) = &p.pushed_head {
        ensure!(
            pushed == &head,
            "publication already started for a different head"
        );
    }
    p.pushed_head = Some(head.clone());
    save(app, &p).await?;
    git(
        app,
        &p.root,
        &[
            "push",
            "-u",
            "origin",
            &format!("{head}:refs/heads/{}", q.remote_branch),
        ],
    )
    .await?;
    git(
        app,
        &project.path,
        &[
            "branch",
            &format!("--set-upstream-to=origin/{}", q.remote_branch),
            &p.branch,
        ],
    )
    .await?;
    let owner = repo.split('/').next().context("repository owner")?;
    let mut query = reqwest::Url::parse(&format!("https://api.github.com/repos/{repo}/pulls"))?;
    query
        .query_pairs_mut()
        .append_pair("state", "all")
        .append_pair("head", &format!("{owner}:{}", q.remote_branch));
    let candidates = app
        .github
        .get(&format!(
            "{}?{}",
            query.path(),
            query.query().unwrap_or_default()
        ))
        .await?;
    let found = candidates
        .as_array()
        .and_then(|a| {
            a.iter().find(|v| {
                v["head"]["ref"] == q.remote_branch
                    && v["head"]["sha"] == head
                    && v["base"]["ref"] == p.base_branch
            })
        })
        .cloned();
    let pr = if let Some(v) = found {
        v
    } else {
        app.github.request(Method::POST, &format!("/repos/{repo}/pulls"), Some(json!({"title":p.title,"body":p.body,"head":q.remote_branch,"base":p.base_branch,"draft":q.draft})), false).await?.value
    };
    p.published_url = Some(
        pr["html_url"]
            .as_str()
            .context("PR URL missing")?
            .to_string(),
    );
    p.published_number = pr["number"].as_u64();
    save(app, &p).await?;
    if app.config.get().remove_published_worktree
        && let Err(e) = cleanup(app, &p, false).await
    {
        tracing::warn!("PR published; worktree kept: {e:#}");
    }
    Ok(p)
}

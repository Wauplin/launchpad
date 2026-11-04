//! Project registry: registered folders, discovery in the projects root,
//! the repo → project mapping, and temporary projects.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::model::Project;
use crate::util::now_ms;

const COLUMNS: &str = "id, name, path, remote_url, gh_repo, gh_upstream, description, is_temp, pinned, created_at, worktree_setup, when_done, workspace_id, runner_id";

fn from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        remote_url: r.get(3)?,
        gh_repo: r.get(4)?,
        gh_upstream: r.get(5)?,
        description: r.get(6)?,
        is_temp: r.get::<_, i64>(7)? != 0,
        pinned: r.get::<_, i64>(8)? != 0,
        created_at: r.get(9)?,
        worktree_setup: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        when_done: crate::model::WhenDone::parse(&r.get::<_, Option<String>>(11)?.unwrap_or_default()),
        workspace_id: r.get(12)?,
        runner_id: r.get(13)?,
    })
}

/// Runner: queues the stored folder for Home in the caller's transaction.
fn queue(c: &rusqlite::Connection, id: &str) -> Result<()> {
    if !crate::replica::enabled() {
        return Ok(());
    }
    if let Some(project) = c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE id = ?1"), [id], from_row).optional()? {
        crate::replica::queue(c, &crate::replica::Item::Project { project })?;
    }
    Ok(())
}

/// Home: stores a folder replicated from the runner whose disk has it.
pub(crate) async fn apply_replica(app: &App, runner_id: &str, n: u64, item: crate::replica::Item) -> Result<()> {
    let runner = runner_id.to_string();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            match item {
                crate::replica::Item::Project { project: p } => {
                    // A folder of this Home itself, or of another runner, is never overwritten.
                    let owner: Option<Option<String>> = tx.query_row("SELECT runner_id FROM project WHERE id = ?1", [&p.id], |r| r.get(0)).optional()?;
                    if owner.as_ref().is_none_or(|o| o.as_deref() == Some(runner.as_str())) {
                        tx.execute(&format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                            ON CONFLICT (id) DO UPDATE SET name = ?2, path = ?3, remote_url = ?4, gh_repo = ?5, gh_upstream = ?6, description = ?7, is_temp = ?8,
                            pinned = ?9, worktree_setup = ?11, when_done = ?12, workspace_id = ?13"),
                            params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, runner])?;
                    }
                }
                crate::replica::Item::ForgetProject { id } => {
                    tx.execute("DELETE FROM project WHERE id = ?1 AND runner_id = ?2", params![id, runner])?;
                }
                _ => {}
            }
            crate::replica::set_acked(&tx, &runner, n)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// A git repository found in the projects root.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Discovered {
    pub name: String,
    pub path: String,
    pub gh_repo: Option<String>,
    pub registered: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub pinned: Option<bool>,
    /// Command run in each new task worktree of this project before the agent starts.
    pub worktree_setup: Option<String>,
    pub when_done: Option<crate::model::WhenDone>,
    /// Moves the folder to another workspace (also assigns an Unassigned folder).
    pub workspace_id: Option<String>,
}

pub async fn list(app: &App) -> Result<Vec<Project>> {
    let projects = app
        .state_db
        .call(|c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM project ORDER BY is_temp, lower(name)"))?;
            let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    refresh_metadata(app, projects, false).await
}

pub async fn get(app: &App, id: &str) -> Result<Option<Project>> {
    let id = id.to_string();
    let project = app
        .state_db
        .call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE id = ?1"), [id], from_row).optional()?))
        .await?;
    Ok(refresh_metadata(app, project.into_iter().collect(), false).await?.pop())
}

/// Remote metadata is derived from disk, never from the registration-time snapshot. Projects on SSH hosts
/// are only read by the periodic refresh (`include_hosts`), so listing projects never waits for a network.
async fn refresh_metadata(app: &App, projects: Vec<Project>, include_hosts: bool) -> Result<Vec<Project>> {
    if projects.is_empty() {
        return Ok(projects);
    }
    let checks = projects.into_iter().map(|mut p| async move {
        let remote = crate::hosts::loc::host_of(&p.path).is_some();
        // Keep the last known mapping when a folder is temporarily unavailable.
        if (include_hosts || !remote) && crate::hostfs::is_dir(&p.path).await {
            let (remote_url, gh_repo, gh_upstream) = remote_info(Path::new(&p.path)).await;
            if p.remote_url != remote_url || p.gh_repo != gh_repo || p.gh_upstream != gh_upstream {
                (p.remote_url, p.gh_repo, p.gh_upstream) = (remote_url, gh_repo, gh_upstream);
                return (p, true);
            }
        }
        (p, false)
    });
    let results = futures_util::future::join_all(checks).await;
    let refreshed: Vec<Project> = results.iter().filter(|(_, changed)| *changed).map(|(p, _)| p.clone()).collect();
    let projects: Vec<Project> = results.into_iter().map(|(p, _)| p).collect();
    if refreshed.is_empty() {
        return Ok(projects);
    }
    let changed = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let mut changed = false;
            for p in refreshed {
                changed |= tx.execute(
                    "UPDATE project SET remote_url = ?1, gh_repo = ?2, gh_upstream = ?3
                     WHERE id = ?4 AND path = ?5
                     AND (remote_url IS NOT ?1 OR gh_repo IS NOT ?2 OR gh_upstream IS NOT ?3)",
                    params![p.remote_url, p.gh_repo, p.gh_upstream, p.id, p.path],
                )? > 0;
                queue(&tx, &p.id)?;
            }
            tx.commit()?;
            Ok(changed)
        })
        .await?;
    if changed {
        app.bus.publish("projects", "changed", serde_json::Value::Null);
    }
    Ok(projects)
}

/// Keep open windows current even when no project endpoint is being requested.
pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let all = app.state_db.call(|c| {
                let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM project"))?;
                let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            }).await;
            if let Err(e) = async { refresh_metadata(&app, all?, true).await }.await {
                tracing::warn!("project metadata refresh: {e:#}");
            }
        }
    });
}

/// Project whose folder contains `cwd` (the longest match), on the same machine.
pub async fn for_path(app: &App, cwd: &Path) -> Result<Option<Project>> {
    let all = list(app).await?;
    Ok(all.into_iter().filter(|p| cwd.starts_with(&p.path)).max_by_key(|p| p.path.len()))
}

/// The project to use for a GitHub repository: `origin` first, then `upstream` (forks).
pub async fn for_repo(app: &App, repo: &str) -> Result<Option<Project>> {
    let all = list(app).await?;
    let eq = |a: &Option<String>| a.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(repo));
    Ok(all.iter().find(|p| eq(&p.gh_repo)).or_else(|| all.iter().find(|p| eq(&p.gh_upstream))).cloned())
}

async fn remote_info(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let remotes = crate::git::remotes(path).await;
    let url = |name: &str| remotes.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone());
    let origin = url("origin");
    let gh_repo = origin.as_deref().and_then(crate::git::github_repo);
    let gh_upstream = url("upstream").as_deref().and_then(crate::git::github_repo);
    (origin, gh_repo, gh_upstream)
}

/// Registers a folder in a workspace. A folder already registered in the same workspace is returned, an
/// Unassigned one joins the workspace, and a folder of another workspace is refused: one folder belongs to
/// one workspace. A folder on an SSH host is `<host>:<path>`.
pub async fn register(app: &App, path: &Path, is_temp: bool, workspace_id: &str) -> Result<Project> {
    let workspace = crate::workspace::get(app, workspace_id).await.context("choose a workspace for the folder")?;
    if let Some(host) = crate::hosts::loc::host_of_path(path) {
        anyhow::ensure!(app.hosts.record(host).is_some(), "unknown host {host}; add it in Settings → Hosts");
    }
    let path = crate::hostfs::canonicalize(path).await.with_context(|| format!("{} does not exist", path.display()))?;
    if !crate::hostfs::is_dir(&path).await {
        bail!("{} is not a folder", path.display());
    }
    let (remote_url, gh_repo, gh_upstream) = remote_info(&path).await;
    let p = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        path: path.to_string_lossy().to_string(),
        remote_url,
        gh_repo,
        gh_upstream,
        description: String::new(),
        is_temp,
        pinned: false,
        created_at: now_ms(),
        worktree_setup: String::new(),
        when_done: Default::default(),
        workspace_id: Some(workspace.id.clone()),
        runner_id: None,
    };
    let saved = p.clone();
    let existing = app
        .state_db
        .call(move |c| {
            if let Some(mut e) = c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE path = ?1"), [&p.path], from_row).optional()? {
                if e.workspace_id.is_none() {
                    c.execute("UPDATE project SET workspace_id = ?1 WHERE id = ?2", params![p.workspace_id, e.id])?;
                    queue(c, &e.id)?;
                    e.workspace_id = p.workspace_id;
                }
                return Ok(Some(e));
            }
            c.execute(
                &format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
                params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, p.runner_id],
            )?;
            queue(c, &p.id)?;
            Ok(None)
        })
        .await?;
    if let Some(e) = &existing
        && e.workspace_id.as_deref() != Some(workspace.id.as_str())
    {
        let other = match e.workspace_id.as_deref() {
            Some(id) => crate::workspace::get(app, id).await.map(|w| w.name).unwrap_or_else(|_| "another workspace".into()),
            None => "another workspace".into(),
        };
        bail!("{} is already a folder of {other}; clone the repository again to use it in {}", e.path, workspace.name);
    }
    let project = refresh_metadata(app, vec![existing.unwrap_or(saved)], true).await?.pop().unwrap();
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(project)
}

pub async fn update(app: &App, id: &str, mut patch: ProjectPatch) -> Result<Project> {
    let workspace = match patch.workspace_id.take() {
        Some(w) => Some(crate::workspace::get(app, &w).await?.id),
        None => None,
    };
    let id2 = id.to_string();
    app.state_db
        .call(move |c| {
            if let Some(n) = patch.name {
                c.execute("UPDATE project SET name = ?1 WHERE id = ?2", params![n.trim(), id2])?;
            }
            if let Some(d) = patch.description {
                c.execute("UPDATE project SET description = ?1 WHERE id = ?2", params![d.trim(), id2])?;
            }
            if let Some(p) = patch.pinned {
                c.execute("UPDATE project SET pinned = ?1 WHERE id = ?2", params![p as i64, id2])?;
            }
            if let Some(done) = patch.when_done {
                c.execute("UPDATE project SET when_done = ?1 WHERE id = ?2", params![done.as_str(), id2])?;
            }
            if let Some(w) = patch.worktree_setup {
                c.execute("UPDATE project SET worktree_setup = ?1 WHERE id = ?2", params![w.trim(), id2])?;
            }
            if let Some(w) = workspace {
                c.execute("UPDATE project SET workspace_id = ?1 WHERE id = ?2", params![w, id2])?;
            }
            queue(c, &id2)?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such project")
}

/// Removes a project from the registry. The folder is left as is.
pub async fn unregister(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    app.state_db.call(move |c| {
        c.execute("DELETE FROM project WHERE id = ?1", [&id])?;
        crate::replica::queue(c, &crate::replica::Item::ForgetProject { id })
    }).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// The projects root of this computer, or of an SSH host (as a host location).
pub fn projects_root(app: &App, host: Option<&str>) -> Result<PathBuf> {
    match host {
        Some(h) => Ok(PathBuf::from(app.hosts.projects_root(h)?)),
        None => Ok(app.config.get().projects_root()),
    }
}

/// "Scan projects root": git repositories in the projects root (one level deep), here or on a host.
pub async fn discover(app: &App, host: Option<&str>) -> Result<Vec<Discovered>> {
    let root = projects_root(app, host)?;
    let registered: std::collections::HashSet<String> = list(app).await?.into_iter().map(|p| p.path).collect();
    let entries = crate::hostfs::read_dir(&root).await.with_context(|| format!("read {}", root.display()))?;
    let found = entries.into_iter().filter(|e| !e.name.starts_with('.') && matches!(e.kind.as_str(), "dir" | "symlink")).map(|e| {
        let path = root.join(&e.name);
        async move {
            crate::git::git_dir(&path).await?;
            let (_, gh_repo, _) = remote_info(&path).await;
            let canonical = crate::hostfs::canonicalize(&path).await.unwrap_or(path.clone());
            Some((e.name, canonical.to_string_lossy().to_string(), gh_repo))
        }
    });
    let mut out: Vec<Discovered> = futures_util::future::join_all(found).await.into_iter().flatten().map(|(name, path, gh_repo)| Discovered {
        registered: registered.contains(&path),
        name,
        path,
        gh_repo,
    }).collect();
    out.sort_by_key(|d| d.name.to_lowercase());
    Ok(out)
}

/// Creates `<projects_root>/tmp-<MMDD>-<slug>` and registers it as a temporary project of a workspace.
pub async fn create_temp(app: &App, prompt: &str, workspace_id: &str) -> Result<Project> {
    crate::workspace::get(app, workspace_id).await.context("choose a workspace for the scratch folder")?;
    let root = app.config.get().projects_root();
    let (month, day) = crate::util::local_month_day();
    let base = format!("tmp-{month:02}{day:02}-{}", crate::compose::slug_from(prompt));
    let mut path = root.join(&base);
    let mut n = 2;
    while path.exists() {
        path = root.join(format!("{base}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    register(app, &path, true, workspace_id).await
}

fn check_temp(app: &App, p: &Project) -> Result<PathBuf> {
    if !p.is_temp {
        bail!("{} is not a temporary project", p.name);
    }
    let root = std::fs::canonicalize(app.config.get().projects_root())?;
    let path = PathBuf::from(&p.path);
    if path.parent() != Some(root.as_path()) {
        bail!("{} is not directly in the projects root", p.path);
    }
    Ok(path)
}

/// Deletes the folder of a temporary project and unregisters it. The caller stops its sessions first.
pub async fn delete_temp(app: &App, id: &str) -> Result<()> {
    let p = get(app, id).await?.context("no such project")?;
    let path = check_temp(app, &p)?;
    if path.exists() {
        tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await??;
    }
    unregister(app, id).await
}

/// Renames a temporary folder to `<projects_root>/<name>` and makes it a normal project. Moves the
/// Native adapter history too, so past sessions stay resumable. Refused while a session runs there.
pub async fn promote(app: &App, id: &str, name: &str) -> Result<Project> {
    let p = get(app, id).await?.context("no such project")?;
    let old = check_temp(app, &p)?;
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        bail!("invalid folder name");
    }
    let new = old.parent().unwrap().join(name);
    if new.exists() {
        bail!("{} already exists", new.display());
    }
    // A shell would keep running in the renamed folder under its old path.
    app.terminals.close_in(&old.to_string_lossy()).await?;
    std::fs::rename(&old, &new).with_context(|| format!("rename to {}", new.display()))?;
    let (old_s, new_s) = (
        old.to_string_lossy().to_string(),
        new.to_string_lossy().to_string(),
    );
    for descriptor in app.harnesses.descriptors() {
        app.harnesses
            .get(descriptor.id)
            .project_moved(&old_s, &new_s);
    }
    let (id2, name2) = (id.to_string(), name.to_string());
    let new_path = new_s.clone();
    app.state_db
        .call(move |c| {
            c.execute("UPDATE project SET path = ?1, name = ?2, is_temp = 0 WHERE id = ?3", params![new_path, name2, id2])?;
            queue(c, &id2)
        })
        .await?;
    // Sessions keep working from the new folder.
    app.sessions.move_cwd(app, &old_s, &new_s).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such project")
}

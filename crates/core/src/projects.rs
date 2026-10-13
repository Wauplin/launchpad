//! Project registry (ARCHITECTURE.md, section 8): registered folders, discovery in the projects root,
//! the repo → project mapping, and temporary projects.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::model::Project;
use crate::util::now_ms;

const COLUMNS: &str = "id, name, path, remote_url, gh_repo, gh_upstream, description, is_temp, pinned, created_at, worktree_setup, merge_when_done";

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
        merge_when_done: r.get::<_, i64>(11)? != 0,
    })
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
    pub merge_when_done: Option<bool>,
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
    refresh_metadata(app, projects).await
}

pub async fn get(app: &App, id: &str) -> Result<Option<Project>> {
    let id = id.to_string();
    let project = app
        .state_db
        .call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE id = ?1"), [id], from_row).optional()?))
        .await?;
    Ok(refresh_metadata(app, project.into_iter().collect()).await?.pop())
}

/// Remote metadata is derived from disk, never from the registration-time snapshot.
async fn refresh_metadata(app: &App, projects: Vec<Project>) -> Result<Vec<Project>> {
    if projects.is_empty() {
        return Ok(projects);
    }
    let (projects, refreshed) = tokio::task::spawn_blocking(move || {
        let mut refreshed = Vec::new();
        let projects = projects.into_iter().map(|mut p| {
            // Keep the last known mapping when a folder is temporarily unavailable.
            if Path::new(&p.path).is_dir() {
                let (remote_url, gh_repo, gh_upstream) = remote_info(Path::new(&p.path));
                if p.remote_url != remote_url || p.gh_repo != gh_repo || p.gh_upstream != gh_upstream {
                    (p.remote_url, p.gh_repo, p.gh_upstream) = (remote_url, gh_repo, gh_upstream);
                    refreshed.push(p.clone());
                }
            }
            p
        }).collect::<Vec<_>>();
        (projects, refreshed)
    }).await?;
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
            if let Err(e) = list(&app).await {
                tracing::warn!("project metadata refresh: {e:#}");
            }
        }
    });
}

/// Project whose folder contains `cwd` (the longest match).
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

fn remote_info(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let remotes = crate::git::remotes(path);
    let url = |name: &str| remotes.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone());
    let origin = url("origin");
    let gh_repo = origin.as_deref().and_then(crate::git::github_repo);
    let gh_upstream = url("upstream").as_deref().and_then(crate::git::github_repo);
    (origin, gh_repo, gh_upstream)
}

/// Registers a folder (or returns the existing project for it).
pub async fn register(app: &App, path: &Path, is_temp: bool) -> Result<Project> {
    let path = std::fs::canonicalize(path).with_context(|| format!("{} does not exist", path.display()))?;
    if !path.is_dir() {
        bail!("{} is not a folder", path.display());
    }
    let (remote_url, gh_repo, gh_upstream) = remote_info(&path);
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
        merge_when_done: false,
    };
    let saved = p.clone();
    let existing = app
        .state_db
        .call(move |c| {
            if let Some(e) = c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE path = ?1"), [&p.path], from_row).optional()? {
                return Ok(Some(e));
            }
            c.execute(
                &format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"),
                params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.merge_when_done as i64],
            )?;
            Ok(None)
        })
        .await?;
    let project = refresh_metadata(app, vec![existing.unwrap_or(saved)]).await?.pop().unwrap();
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(project)
}

pub async fn update(app: &App, id: &str, patch: ProjectPatch) -> Result<Project> {
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
            if let Some(merge) = patch.merge_when_done {
                c.execute("UPDATE project SET merge_when_done = ?1 WHERE id = ?2", params![merge as i64, id2])?;
            }
            if let Some(w) = patch.worktree_setup {
                c.execute("UPDATE project SET worktree_setup = ?1 WHERE id = ?2", params![w.trim(), id2])?;
            }
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such project")
}

/// Removes a project from the registry. The folder is left as is.
pub async fn unregister(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    let id2 = id.clone();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM project WHERE id = ?1", [id2])?)).await?;
    crate::workspace::forget_project(app, &id).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// "Scan projects root": git repositories in the projects root (one level deep).
pub async fn discover(app: &App) -> Result<Vec<Discovered>> {
    let root = app.config.get().projects_root();
    let registered: std::collections::HashSet<String> = list(app).await?.into_iter().map(|p| p.path).collect();
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&root).with_context(|| format!("read {}", root.display()))? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !path.is_dir() || crate::git::git_dir(&path).is_none() {
                continue;
            }
            let (_, gh_repo, _) = remote_info(&path);
            let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
            out.push(Discovered {
                registered: registered.contains(&canonical.to_string_lossy().to_string()),
                name,
                path: canonical.to_string_lossy().to_string(),
                gh_repo,
            });
        }
        out.sort_by_key(|d| d.name.to_lowercase());
        Ok(out)
    })
    .await?
}

/// Creates `<projects_root>/tmp-<MMDD>-<slug>` and registers it as a temporary project.
pub async fn create_temp(app: &App, prompt: &str) -> Result<Project> {
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
    register(app, &path, true).await
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
        .call(move |c| Ok(c.execute("UPDATE project SET path = ?1, name = ?2, is_temp = 0 WHERE id = ?3", params![new_path, name2, id2])?))
        .await?;
    // Sessions keep working from the new folder.
    app.sessions.move_cwd(app, &old_s, &new_s).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such project")
}

//! Project registry: registered folders, discovery in the projects root,
//! the repo → project mapping, and temporary projects.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::model::Project;
use crate::util::now_ms;

const COLUMNS: &str = "id, name, path, remote_url, gh_repo, gh_upstream, description, is_temp, pinned, created_at, worktree_setup, when_done, workspace_id, runner_id, home_git, plain";

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
        home_git: r.get::<_, i64>(14)? != 0,
        plain: r.get::<_, i64>(15)? != 0,
    })
}

/// Runner: queues a temporary folder for Home in the caller's transaction, so Home knows where it is now. Git
/// and plain folders are Home's records: the runner only keeps Home's copy.
fn queue(c: &rusqlite::Connection, id: &str) -> Result<()> {
    if !crate::replica::enabled() {
        return Ok(());
    }
    if let Some(project) = c.query_row(&format!("SELECT {COLUMNS} FROM project WHERE id = ?1"), [id], from_row).optional()?.filter(|p| p.is_temp) {
        crate::replica::queue(c, &crate::replica::Item::Project { project })?;
    }
    Ok(())
}

/// Home: stores a temporary folder replicated from the runner that has it now. Every other folder is Home's.
pub(crate) async fn apply_replica(app: &App, runner_id: &str, n: u64, item: crate::replica::Item) -> Result<()> {
    let runner = runner_id.to_string();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            match item {
                crate::replica::Item::Project { project: p } if p.is_temp => {
                    // A folder of this Home itself, or of another runner, is never overwritten.
                    let owner: Option<Option<String>> = tx.query_row("SELECT runner_id FROM project WHERE id = ?1", [&p.id], |r| r.get(0)).optional()?;
                    if owner.as_ref().is_none_or(|o| o.as_deref() == Some(runner.as_str())) {
                        tx.execute(&format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, 0)
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
    let checks = projects.into_iter().map(|mut p| async move {
        // Keep the last known mapping when a folder is temporarily unavailable.
        // Git folders of Home keep the remote they were registered with: Home's mirror and runners' clones
        // are Launchpad's own copies.
        if !p.home_git && !p.plain && crate::files::is_dir(&p.path).await {
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
            if let Err(e) = async { refresh_metadata(&app, all?).await }.await {
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

/// Registers a temporary folder of this disk in a workspace. Every other folder is stored by Home (Home Git or a
/// plain folder) and runs on any runner: a disk path is never a folder by itself.
async fn register_temp(app: &App, path: &Path, workspace_id: &str, id: String) -> Result<Project> {
    let workspace = crate::workspace::get(app, workspace_id).await.context("choose a workspace for the folder")?;
    let path = crate::files::canonicalize(path).await.with_context(|| format!("{} does not exist", path.display()))?;
    if !crate::files::is_dir(&path).await {
        bail!("{} is not a folder", path.display());
    }
    let (remote_url, gh_repo, gh_upstream) = remote_info(&path).await;
    let p = Project {
        id,
        name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        path: path.to_string_lossy().to_string(),
        remote_url,
        gh_repo,
        gh_upstream,
        description: String::new(),
        is_temp: true,
        pinned: false,
        created_at: now_ms(),
        worktree_setup: String::new(),
        when_done: Default::default(),
        workspace_id: Some(workspace.id.clone()),
        runner_id: None,
        home_git: false,
        plain: false,
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
                &format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"),
                params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, p.runner_id, p.home_git as i64, p.plain as i64],
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
    let project = refresh_metadata(app, vec![existing.unwrap_or(saved)]).await?.pop().unwrap();
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(project)
}

/// Home (or `all`): registers a Git folder by its remote and creates its mirror. The same remote in the same
/// workspace returns the existing folder; in another workspace it becomes a second folder.
pub async fn register_remote(app: &App, name: &str, url: &str, workspace_id: &str) -> Result<Project> {
    anyhow::ensure!(crate::home_git::here(app), "folders are registered on Home");
    let workspace = crate::workspace::get(app, workspace_id).await.context("choose a workspace for the folder")?;
    let url = url.trim();
    let name = name.trim();
    anyhow::ensure!(!name.is_empty() && !name.contains('/'), "invalid folder name");
    if !url.is_empty()
        && let Some(e) = list(app).await?.into_iter().find(|p| p.home_git && p.remote_url.as_deref() == Some(url) && p.workspace_id.as_deref() == Some(workspace.id.as_str()))
    {
        return Ok(e);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = if app.paths.role == crate::role::Role::Home { crate::home_git::mirror(app, &id) } else { crate::repo_cache::dir(app, &id) };
    let gh_repo = crate::git::github_repo(url);
    let p = Project {
        id,
        name: name.to_string(),
        path: path.to_string_lossy().into_owned(),
        remote_url: Some(url.to_string()).filter(|u| !u.is_empty()),
        gh_repo,
        gh_upstream: None,
        description: String::new(),
        is_temp: false,
        pinned: false,
        created_at: now_ms(),
        worktree_setup: String::new(),
        when_done: Default::default(),
        workspace_id: Some(workspace.id.clone()),
        runner_id: None,
        home_git: true,
        plain: false,
    };
    crate::home_git::ensure(app, &p).await?;
    let saved = p.clone();
    app.state_db
        .call(move |c| {
            c.execute(
                &format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"),
                params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, p.runner_id, p.home_git as i64, p.plain as i64],
            )?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    if app.paths.role == crate::role::Role::All {
        let (app, p) = (app.clone(), saved.clone());
        tokio::spawn(async move {
            if let Err(e) = crate::repo_cache::sync(&app, &p).await {
                tracing::warn!("clone {} from Home Git: {e:#}", p.name);
            }
        });
    }
    Ok(saved)
}

/// Adds a local Git checkout as a Git folder of Home, named `name` (the checkout's folder name when None). Home
/// registers its remote; a checkout without a remote is imported into Home Git (every branch and tag). Agents
/// never work in the checkout itself: every runner clones the folder from Home.
pub async fn add_to_home(app: &App, path: &Path, name: Option<&str>, workspace_id: &str) -> Result<Project> {
    let path = crate::files::canonicalize(path).await.with_context(|| format!("{} does not exist", path.display()))?;
    let name = name.map(str::to_string).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let (remote_url, _, _) = remote_info(&path).await;
    let here = crate::home_git::here(app);
    let p: Project = if here {
        register_remote(app, &name, &remote_url.clone().unwrap_or_default(), workspace_id).await?
    } else {
        serde_json::from_value(crate::home_link::home_request("POST", "/api/projects/remote", Some(serde_json::json!({"name": name, "url": remote_url.clone().unwrap_or_default(), "workspace_id": workspace_id}))).await?)?
    };
    if remote_url.is_none() {
        let refs = ["refs/heads/*:refs/heads/*", "refs/tags/*:refs/tags/*"];
        if here {
            let mirror = crate::home_git::ensure(app, &p).await?;
            let mut cmd = crate::procs::command(&app.env, "git", Some(&path));
            cmd.args(["push", "--quiet"]).arg(&mirror).args(refs).stdin(crate::exec::Io::Null).env("LP_IMPORT", "1");
            let out = cmd.output().await?;
            ensure!(out.status.success(), "import the folder into Home Git: {}", String::from_utf8_lossy(&out.stderr).trim());
        } else {
            let header = crate::repo_cache::daemon_header(app)?;
            let url = crate::repo_cache::home_url(app, &p.id);
            crate::home_git::git_in(app, &path, &["-c", &header, "-c", &format!("http.extraHeader={}: 1", crate::home_git::IMPORT_HEADER), "push", "--quiet", &url, refs[0], refs[1]], 1800).await
                .context("import the folder into Home Git")?;
        }
    }
    if !here {
        store_home(app, vec![p.clone()], false).await?;
    }
    get(app, &p.id).await?.context("no such folder")
}

/// Runner: stores Home's Git folders (`all`: a full snapshot, so folders that left Home go too). Each gets
/// this runner's clone as its path.
pub async fn store_home(app: &App, projects: Vec<Project>, all: bool) -> Result<()> {
    let rows: Vec<Project> = projects.into_iter().filter(|p| p.home_git || p.plain).map(|mut p| {
        p.path = if p.plain { crate::plain::copy_dir(app, &p.id) } else { crate::repo_cache::dir(app, &p.id) }.to_string_lossy().into_owned();
        p.runner_id = None;
        p
    }).collect();
    let gone = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let mut gone = vec![];
            if all {
                let keep: std::collections::HashSet<String> = rows.iter().map(|p| p.id.clone()).collect();
                let mut st = tx.prepare("SELECT id FROM project WHERE home_git = 1 OR plain = 1")?;
                gone = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().filter(|id| !keep.contains(id)).collect();
                drop(st);
                for id in &gone {
                    tx.execute("DELETE FROM project WHERE id = ?1", [id])?;
                }
            }
            for p in rows {
                tx.execute(&format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, ?14, ?15)
                    ON CONFLICT (id) DO UPDATE SET name = ?2, path = ?3, remote_url = ?4, gh_repo = ?5, gh_upstream = ?6, description = ?7, is_temp = ?8,
                    pinned = ?9, worktree_setup = ?11, when_done = ?12, workspace_id = ?13, runner_id = NULL, home_git = ?14, plain = ?15"),
                    params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, p.home_git as i64, p.plain as i64])?;
            }
            tx.commit()?;
            Ok(gone)
        })
        .await?;
    for id in gone {
        crate::repo_cache::forget(app, &id).await;
        let _ = tokio::fs::remove_dir_all(crate::plain::copy_dir(app, &id)).await;
    }
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// Home's Git and plain folders, for runners.
pub async fn home_folders(app: &App) -> Result<Vec<Project>> {
    Ok(list(app).await?.into_iter().filter(|p| p.home_git || p.plain).collect())
}

/// Home (or `all`): a new, empty plain folder.
/// Home: turns a plain folder into a Git folder without a remote. Its current files become the first commit
/// of `main` in a new Home Git mirror; agents then work in worktrees. Refused while an agent holds the folder.
pub async fn plain_to_git(app: &App, id: &str) -> Result<Project> {
    anyhow::ensure!(crate::home_git::here(app), "folders are changed on Home");
    let mut p = get(app, id).await?.context("no such folder")?;
    anyhow::ensure!(p.plain, "{} is not a plain folder", p.name);
    if let Some(holder) = crate::plain::lock_holder(app, id).await?
        && app.sessions.get(&holder).is_some_and(|s| s.status != crate::model::SessionStatus::Closed)
    {
        anyhow::bail!("an agent works in {}; archive it first", p.name);
    }
    let tree = crate::plain::tree(app, id).await?;
    let tmp = app.paths.data_dir.join("tmp").join(format!("to-git-{id}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    for e in &tree.files {
        let path = tmp.join(&e.path);
        tokio::fs::create_dir_all(path.parent().context("file path")?).await?;
        tokio::fs::write(&path, crate::plain::read_blob(app, &e.sha256).await?).await?;
        #[cfg(unix)]
        if e.executable {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
        }
    }
    let home = app.config.get().home.clone();
    let name = Some(home.git_name).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "Launchpad".into());
    let email = Some(home.git_email).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "launchpad@localhost".into());
    let git = |args: Vec<String>| {
        let (app, tmp) = (app.clone(), tmp.clone());
        async move {
            let mut cmd = crate::procs::command(&app.env, "git", Some(&tmp));
            cmd.args(args.iter().map(String::as_str)).stdin(crate::exec::Io::Null).env("LP_IMPORT", "1");
            let out = cmd.output().await?;
            anyhow::ensure!(out.status.success(), "git {}: {}", args.first().cloned().unwrap_or_default(), String::from_utf8_lossy(&out.stderr).trim());
            anyhow::Ok(())
        }
    };
    git(vec!["init".into(), "--quiet".into(), "--initial-branch=main".into()]).await?;
    git(vec!["add".into(), "--all".into()]).await?;
    git(vec!["-c".into(), format!("user.name={name}"), "-c".into(), format!("user.email={email}"), "commit".into(), "--quiet".into(), "--allow-empty".into(), "-m".into(), format!("Files of {} (plain folder until now)", p.name)]).await?;
    p.plain = false;
    p.home_git = true;
    p.remote_url = None;
    p.path = if app.paths.role == crate::role::Role::Home { crate::home_git::mirror(app, &p.id) } else { crate::repo_cache::dir(app, &p.id) }.to_string_lossy().into_owned();
    let mirror = crate::home_git::ensure(app, &p).await?;
    git(vec!["push".into(), "--quiet".into(), mirror.to_string_lossy().into_owned(), "main:refs/heads/main".into()]).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let saved = p.clone();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE project SET plain = 0, home_git = 1, remote_url = NULL, path = ?2 WHERE id = ?1", params![saved.id, saved.path])?;
            // The files are in Git now; their earlier versions stay in the history until it expires.
            tx.execute("DELETE FROM plain_file WHERE store = ?1", [&saved.id])?;
            tx.execute("DELETE FROM plain_state WHERE store = ?1", [&saved.id])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such folder")
}

pub async fn create_plain(app: &App, name: &str, workspace_id: &str) -> Result<Project> {
    anyhow::ensure!(crate::home_git::here(app), "folders are registered on Home");
    let workspace = crate::workspace::get(app, workspace_id).await.context("choose a workspace for the folder")?;
    let name = name.trim();
    anyhow::ensure!(!name.is_empty() && !name.contains('/'), "invalid folder name");
    let id = uuid::Uuid::new_v4().to_string();
    let p = Project {
        path: crate::plain::copy_dir(app, &id).to_string_lossy().into_owned(),
        id,
        name: name.to_string(),
        remote_url: None,
        gh_repo: None,
        gh_upstream: None,
        description: String::new(),
        is_temp: false,
        pinned: false,
        created_at: now_ms(),
        worktree_setup: String::new(),
        when_done: Default::default(),
        workspace_id: Some(workspace.id.clone()),
        runner_id: None,
        home_git: false,
        plain: true,
    };
    let saved = p.clone();
    app.state_db
        .call(move |c| {
            c.execute(
                &format!("INSERT INTO project ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"),
                params![p.id, p.name, p.path, p.remote_url, p.gh_repo, p.gh_upstream, p.description, p.is_temp as i64, p.pinned as i64, p.created_at, p.worktree_setup, p.when_done.as_str(), p.workspace_id, p.runner_id, p.home_git as i64, p.plain as i64],
            )?;
            c.execute("INSERT INTO plain_state (store, version) VALUES (?1, 0) ON CONFLICT DO NOTHING", [&p.id])?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(saved)
}

/// Runner: a local folder without Git becomes a plain folder of Home; its files are uploaded once. The runner
/// then works in its own copy, never in the original folder.
pub async fn import_plain(app: &App, path: &Path, workspace_id: &str) -> Result<Project> {
    let path = crate::files::canonicalize(path).await.with_context(|| format!("{} does not exist", path.display()))?;
    anyhow::ensure!(path.is_dir(), "{} is not a folder", path.display());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let p: Project = if crate::home_git::here(app) {
        create_plain(app, &name, workspace_id).await?
    } else {
        serde_json::from_value(crate::home_link::home_request("POST", "/api/projects/plain", Some(serde_json::json!({"name": name, "workspace_id": workspace_id}))).await?)?
    };
    let importer = format!("import-{}", uuid::Uuid::new_v4());
    crate::plain::sync_down(app, &p.id, &crate::plain::copy_dir(app, &p.id), &importer).await?;
    let copy = crate::plain::copy_dir(app, &p.id);
    let mut cmd = crate::procs::command(&app.env, "cp", None);
    cmd.arg("-a").arg(format!("{}/.", path.display())).arg(&copy).stdin(crate::exec::Io::Null);
    let out = cmd.output().await?;
    anyhow::ensure!(out.status.success(), "copy {}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim());
    crate::plain::sync_up(app, &p.id, &copy, &importer).await?;
    crate::plain::release(app, &p.id, &importer).await?;
    if !crate::home_git::here(app) {
        store_home(app, vec![p.clone()], false).await?;
    }
    get(app, &p.id).await?.context("no such folder")
}

/// Every daemon: forgets folder records that only one disk had (registered before every folder was stored by
/// Home). Their files stay on disk; add them again to store them on Home. Temporary folders stay: their files
/// are on Home.
pub async fn forget_disk_folders(app: &App) -> Result<()> {
    let gone = app.state_db.call(|c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM project WHERE home_git = 0 AND plain = 0 AND is_temp = 0"))?;
        let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(st);
        c.execute("DELETE FROM project WHERE home_git = 0 AND plain = 0 AND is_temp = 0", [])?;
        Ok(rows)
    }).await?;
    for p in &gone {
        tracing::warn!("folder {} ({}) existed only on one disk: forgotten; add it again to store it on Home", p.name, p.path);
    }
    if !gone.is_empty() {
        app.bus.publish("projects", "changed", serde_json::Value::Null);
    }
    Ok(())
}

/// Home and `all`: Git checkouts registered before Home Git become Home Git folders.
pub async fn adopt_git_folders(app: &App) -> Result<()> {
    if !crate::home_git::here(app) {
        return Ok(());
    }
    let candidates = app.state_db.call(|c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM project WHERE home_git = 0 AND is_temp = 0 AND runner_id IS NULL"))?;
        let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    for p in candidates {
        if crate::git::git_dir(Path::new(&p.path)).await.is_some() {
            let id = p.id.clone();
            app.state_db.call(move |c| Ok(c.execute("UPDATE project SET home_git = 1 WHERE id = ?1", [id])?)).await?;
        }
    }
    Ok(())
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

/// "Scan projects root": git repositories in the projects root (one level deep).
pub async fn discover(app: &App) -> Result<Vec<Discovered>> {
    let root = app.config.get().projects_root();
    let folders = list(app).await?;
    // A checkout added before is a folder of Home with this checkout's remote; runners keep their own clones.
    let remotes: std::collections::HashSet<String> = folders.iter().filter_map(|p| p.remote_url.clone()).collect();
    let registered: std::collections::HashSet<String> = folders.into_iter().map(|p| p.path).collect();
    let entries = crate::files::read_dir(&root).await.with_context(|| format!("read {}", root.display()))?;
    let found = entries.into_iter().filter(|e| !e.name.starts_with('.') && matches!(e.kind.as_str(), "dir" | "symlink")).map(|e| {
        let path = root.join(&e.name);
        async move {
            crate::git::git_dir(&path).await?;
            let (remote, gh_repo, _) = remote_info(&path).await;
            let canonical = crate::files::canonicalize(&path).await.unwrap_or(path.clone());
            Some((e.name, canonical.to_string_lossy().to_string(), gh_repo, remote))
        }
    });
    let mut out: Vec<Discovered> = futures_util::future::join_all(found).await.into_iter().flatten().map(|(name, path, gh_repo, remote)| Discovered {
        registered: registered.contains(&path) || remote.is_some_and(|r| remotes.contains(&r)),
        name,
        path,
        gh_repo,
    }).collect();
    out.sort_by_key(|d| d.name.to_lowercase());
    Ok(out)
}

/// Where temporary folders live: `<data_dir>/tmp`, outside the folders root.
pub fn temp_root(app: &App) -> PathBuf {
    app.paths.data_dir.join("tmp")
}

/// Creates `<data_dir>/tmp/tmp-<MMDD>-<slug>` and registers it as a temporary folder of a workspace. Its files
/// are saved on Home at every turn end (`plain::temp_store`); this disk's copy is deleted when its last agent
/// is archived, Home's 30 days later, unless the owner keeps it as a folder (`promote`).
pub async fn create_temp(app: &App, prompt: &str, workspace_id: &str) -> Result<Project> {
    let (month, day) = crate::util::local_month_day();
    let base = format!("tmp-{month:02}{day:02}-{}", crate::compose::slug_from(prompt));
    temp_at(app, &base, workspace_id, uuid::Uuid::new_v4().to_string()).await
}

/// Creates `<data_dir>/tmp/<base>` (or `<base>-2`, …) and registers it as the temporary folder `id`.
async fn temp_at(app: &App, base: &str, workspace_id: &str, id: String) -> Result<Project> {
    crate::workspace::get(app, workspace_id).await.context("choose a workspace for the temporary folder")?;
    ensure!(!base.is_empty() && !base.contains('/') && !base.starts_with('.'), "invalid temporary folder name");
    let root = temp_root(app);
    let mut path = root.join(base);
    let mut n = 2;
    while path.exists() {
        path = root.join(format!("{base}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    register_temp(app, &path, workspace_id, id).await
}

/// The name of a session's temporary folder, from its working directory (`<data_dir>/tmp/tmp-…`).
pub fn temp_name(cwd: &str) -> Option<String> {
    let path = Path::new(cwd);
    let name = path.file_name()?.to_string_lossy().into_owned();
    (path.parent()?.file_name()? == "tmp" && name.starts_with("tmp-")).then_some(name)
}

/// A session's temporary folder on this disk: the registered one, or a new folder with the same ID and name,
/// recreated after a move to this runner or after Archive removed it (its files come down from Home with
/// `plain::prepare`). None: the session has no temporary folder.
pub async fn ensure_temp(app: &App, s: &crate::model::Session) -> Result<Option<Project>> {
    let Some(pid) = &s.project_id else { return Ok(None) };
    if let Some(p) = get(app, pid).await? {
        if !p.is_temp {
            return Ok(None);
        }
        if Path::new(&p.path).is_dir() {
            return Ok(Some(p));
        }
        // Registered, but its folder is gone: registered again below.
        let id = p.id.clone();
        app.state_db.call(move |c| Ok(c.execute("DELETE FROM project WHERE id = ?1", [id])?)).await?;
    }
    let Some(name) = temp_name(&s.cwd) else { return Ok(None) };
    let workspace = s.launch_spec.workspace_id.clone().context("this session's temporary folder has no workspace")?;
    Ok(Some(temp_at(app, &name, &workspace, pid.clone()).await?))
}

/// Home: a session in a temporary folder moves to another runner, and the folder's record with it.
pub async fn move_temp(app: &App, id: &str, runner: &str) -> Result<()> {
    let (id, runner) = (id.to_string(), runner.to_string());
    app.state_db.call(move |c| Ok(c.execute("UPDATE project SET runner_id = ?2 WHERE id = ?1 AND is_temp = 1 AND runner_id IS NOT NULL", params![id, runner])?)).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// Removes a temporary folder from this disk once Home holds all its files. `forget`: unregister it on Home
/// too (Archive); otherwise Home keeps the record for the session's next runner (a move).
pub async fn remove_temp_copy(app: &App, id: &str, forget: bool) -> Result<()> {
    let p = get(app, id).await?.context("no such project")?;
    let path = check_temp(app, &p)?;
    ensure!(crate::plain::in_sync(app, &crate::plain::temp_store(id), &path).await, "Home does not have every file of {} yet", p.path);
    app.terminals.close_in(&p.path).await?;
    if path.exists() {
        let dir = path.clone();
        tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&dir)).await??;
    }
    crate::plain::forget_copy(app, &path).await;
    if forget {
        return unregister(app, id).await;
    }
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM project WHERE id = ?1", [id])?)).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    Ok(())
}

/// The folder of a temporary project: directly in `<data_dir>/tmp`, or in the folders root (older ones).
fn check_temp(app: &App, p: &Project) -> Result<PathBuf> {
    if !p.is_temp {
        bail!("{} is not a temporary folder", p.name);
    }
    let path = PathBuf::from(&p.path);
    let parent = path.parent().and_then(|d| std::fs::canonicalize(d).ok());
    let roots = [temp_root(app), app.config.get().projects_root()];
    if !roots.iter().any(|r| std::fs::canonicalize(r).ok().is_some_and(|r| parent.as_deref() == Some(r.as_path()))) {
        bail!("{} is not a Launchpad temporary folder", p.path);
    }
    Ok(path)
}

/// Deletes the folder of a temporary project and unregisters it. The caller stops its sessions first.
pub async fn delete_temp(app: &App, id: &str) -> Result<()> {
    let p = get(app, id).await?.context("no such project")?;
    let path = check_temp(app, &p)?;
    app.terminals.close_in(&p.path).await?;
    if path.exists() {
        tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await??;
    }
    unregister(app, id).await
}

/// After Archive: a temporary folder whose agents are all archived leaves this disk and the folder list. Home
/// keeps its files 30 days, so a resume brings it back (`ensure_temp`).
pub async fn after_archive(app: &App, session_id: &str) {
    let Some(project_id) = app.sessions.get(session_id).and_then(|s| s.project_id) else { return };
    let Ok(Some(p)) = get(app, &project_id).await else { return };
    if !p.is_temp || app.sessions.list().iter().any(|s| s.project_id.as_deref() == Some(p.id.as_str()) && s.status != crate::model::SessionStatus::Closed) {
        return;
    }
    // Home's record of a runner's folder (a runner away, a stopped Burst job): the runner removes its copy
    // when the archive reaches it.
    if p.runner_id.is_some() && app.paths.role == crate::role::Role::Home {
        let id = p.id.clone();
        if let Err(e) = app.state_db.call(move |c| Ok(c.execute("DELETE FROM project WHERE id = ?1", [id])?)).await {
            tracing::warn!("forget the temporary folder {}: {e:#}", p.path);
        }
        app.bus.publish("projects", "changed", serde_json::Value::Null);
        return;
    }
    if let Err(e) = remove_temp_copy(app, &p.id, true).await {
        tracing::warn!("remove the temporary folder {}: {e:#}", p.path);
    }
}

/// Keep as folder, on the disk that has the temporary folder: it becomes a folder stored by Home, so it runs on
/// any runner. A Git repository becomes a new Git folder of Home (its agents keep the temporary folder); other
/// files become a plain folder with the same ID: every file goes up to Home, Home moves the store
/// (`keep_temp`), and this disk's copy moves to the plain folder's place with its sessions and the native
/// adapter history, so past sessions stay resumable. Refused while an agent works there.
pub async fn keep_as_folder(app: &App, id: &str, name: &str) -> Result<Project> {
    let p = get(app, id).await?.context("no such project")?;
    let old = check_temp(app, &p)?;
    let name = name.trim();
    ensure!(!name.is_empty() && !name.contains('/') && !name.starts_with('.'), "invalid folder name");
    let mine: Vec<crate::model::Session> = app.sessions.list().into_iter().filter(|s| s.project_id.as_deref() == Some(id)).collect();
    ensure!(!mine.iter().any(|s| app.sessions.is_busy(s)), "an agent works in {}; keep it when its turn ended", p.name);
    if old.join(".git").exists() {
        let workspace = p.workspace_id.as_deref().context("this temporary folder has no workspace")?;
        return add_to_home(app, &old, Some(name), workspace).await;
    }
    // The author of the last upload: an agent of the folder on this disk, else Launchpad itself.
    let author = mine.iter().max_by_key(|s| s.started_at).map(|s| s.id.clone()).unwrap_or_else(|| format!("keep-{}", uuid::Uuid::new_v4()));
    crate::plain::sync_up(app, &crate::plain::temp_store(id), &old, &author).await.context("save the files on Home")?;
    let kept = crate::plain::home::keep(app, id, name).await?;
    let new = crate::plain::copy_dir(app, id);
    ensure!(!new.exists(), "{} already exists", new.display());
    // A shell would keep running in the moved folder under its old path.
    app.terminals.close_in(&old.to_string_lossy()).await?;
    if let Some(parent) = new.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    move_dir(&old, &new).await?;
    crate::plain::moved_copy(app, &old, &new).await;
    let (old_s, new_s) = (old.to_string_lossy().to_string(), new.to_string_lossy().to_string());
    for descriptor in app.harnesses.descriptors() {
        app.harnesses.get(descriptor.id).project_moved(&old_s, &new_s);
    }
    if !crate::home_git::here(app) {
        store_home(app, vec![kept], false).await?;
    }
    // Sessions keep working from the new folder.
    app.sessions.move_cwd(app, &old_s, &new_s).await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such project")
}

/// Home: Keep as folder. A temporary folder becomes a plain folder with the same ID: the files and history of
/// its store `tmp-<id>` move to the folder's own store, and its one open agent, if any, holds the lock (a plain
/// folder has one agent at a time).
pub async fn keep_temp(app: &App, id: &str, name: &str) -> Result<Project> {
    ensure!(crate::home_git::here(app), "folders are changed on Home");
    let name = name.trim();
    ensure!(!name.is_empty() && !name.contains('/') && !name.starts_with('.'), "invalid folder name");
    let p = get(app, id).await?.context("no such folder")?;
    ensure!(p.is_temp, "{} is not a temporary folder", p.name);
    let open: Vec<String> = app.sessions.list().into_iter().filter(|s| s.project_id.as_deref() == Some(id) && s.status != crate::model::SessionStatus::Closed).map(|s| s.id).collect();
    ensure!(open.len() <= 1, "{} agents are open in {}; archive all but one first (a plain folder has one agent at a time)", open.len(), p.name);
    let (id2, name2, temp, path) = (id.to_string(), name.to_string(), crate::plain::temp_store(id), crate::plain::copy_dir(app, id).to_string_lossy().into_owned());
    let holder = open.into_iter().next();
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for table in ["plain_file", "plain_history", "plain_state"] {
                tx.execute(&format!("UPDATE {table} SET store = ?2 WHERE store = ?1"), params![temp, id2])?;
            }
            tx.execute("INSERT INTO plain_state (store, version) VALUES (?1, 0) ON CONFLICT DO NOTHING", [&id2])?;
            tx.execute("DELETE FROM plain_lock WHERE store = ?1 OR store = ?2", params![temp, id2])?;
            if let Some(s) = holder {
                tx.execute("INSERT INTO plain_lock (store, session_id, at) VALUES (?1, ?2, ?3)", params![id2, s, now_ms()])?;
            }
            tx.execute("UPDATE project SET name = ?2, path = ?3, is_temp = 0, plain = 1, home_git = 0, runner_id = NULL WHERE id = ?1", params![id2, name2, path])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("projects", "changed", serde_json::Value::Null);
    get(app, id).await?.context("no such folder")
}

/// Renames a folder, or copies it and deletes the original when the target is on another file system.
async fn move_dir(old: &Path, new: &Path) -> Result<()> {
    match std::fs::rename(old, new) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let out = tokio::process::Command::new("cp").arg("-a").arg(old).arg(new).output().await.context("cp")?;
            ensure!(out.status.success(), "copy to {}: {}", new.display(), String::from_utf8_lossy(&out.stderr).trim());
            let old = old.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(old)).await??;
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("move to {}", new.display())),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn temp_name_reads_temporary_folders_only() {
        assert_eq!(super::temp_name("/home/u/.local/share/launchpad-runner/tmp/tmp-1106-hfjs").as_deref(), Some("tmp-1106-hfjs"));
        assert_eq!(super::temp_name("/home/u/projects/tmp-1106-hfjs"), None);
        assert_eq!(super::temp_name("/data/tmp/notes"), None);
    }
}

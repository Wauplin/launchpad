//! Workspaces (ARCHITECTURE.md, section 9.1.2): named sets of projects. A window shows one workspace at a
//! time; its agents, scheduled launches, crons and inbox threads are the ones of its projects.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub project_ids: Vec<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Patch {
    pub name: Option<String>,
    pub project_ids: Option<Vec<String>>,
}

pub async fn list(app: &App) -> Result<Vec<Workspace>> {
    let mut all: Vec<Workspace> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM workspace")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    all.sort_by_key(|w: &Workspace| w.name.to_lowercase());
    Ok(all)
}

async fn get(app: &App, id: &str) -> Result<Workspace> {
    list(app).await?.into_iter().find(|w| w.id == id).context("no such workspace")
}

async fn save(app: &App, w: &Workspace) -> Result<()> {
    let (id, json) = (w.id.clone(), serde_json::to_string(w)?);
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO workspace (id, json) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET json = ?2", [id, json])?;
            Ok(())
        })
        .await?;
    app.bus.publish("workspaces", "changed", serde_json::Value::Null);
    Ok(())
}

async fn check_name(app: &App, name: &str, id: &str) -> Result<String> {
    let name = name.trim().chars().take(40).collect::<String>();
    if name.is_empty() {
        bail!("the workspace needs a name");
    }
    if list(app).await?.iter().any(|w| w.id != id && w.name.eq_ignore_ascii_case(&name)) {
        bail!("a workspace named {name} already exists");
    }
    Ok(name)
}

pub async fn create(app: &App, name: &str, project_ids: Vec<String>) -> Result<Workspace> {
    let id = uuid::Uuid::new_v4().to_string();
    let w = Workspace { name: check_name(app, name, &id).await?, id, project_ids, created_at: now_ms() };
    save(app, &w).await?;
    Ok(w)
}

pub async fn update(app: &App, id: &str, patch: Patch) -> Result<Workspace> {
    let mut w = get(app, id).await?;
    if let Some(name) = patch.name {
        w.name = check_name(app, &name, id).await?;
    }
    if let Some(ids) = patch.project_ids {
        w.project_ids = ids.into_iter().fold(Vec::new(), |mut acc, id| {
            if !acc.contains(&id) {
                acc.push(id);
            }
            acc
        });
    }
    save(app, &w).await?;
    Ok(w)
}

/// Deletes the workspace only: its projects and agents stay.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM workspace WHERE id = ?1", [id])?)).await?;
    app.bus.publish("workspaces", "changed", serde_json::Value::Null);
    Ok(())
}

/// Adds a project to a workspace, for example the project of an agent launched from it. A missing
/// workspace (deleted from another window) is ignored.
pub async fn add_project(app: &App, id: &str, project_id: &str) -> Result<()> {
    let Ok(mut w) = get(app, id).await else { return Ok(()) };
    if !w.project_ids.iter().any(|p| p == project_id) {
        w.project_ids.push(project_id.to_string());
        save(app, &w).await?;
    }
    Ok(())
}

/// Removes an unregistered project from every workspace.
pub async fn forget_project(app: &App, project_id: &str) -> Result<()> {
    for mut w in list(app).await? {
        if w.project_ids.iter().any(|p| p == project_id) {
            w.project_ids.retain(|p| p != project_id);
            save(app, &w).await?;
        }
    }
    Ok(())
}

/// Opens a new desktop window, on a workspace or on all projects. Each window keeps its own workspace.
pub fn open_window(app: &App, id: Option<&str>) -> Result<()> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("launchpad-desktop")))
        .filter(|p| p.exists())
        .or_else(|| app.env.which("launchpad-desktop"))
        .context("launchpad-desktop is not installed")?;
    let mut cmd = crate::procs::command(&app.env, &exe.to_string_lossy(), None);
    cmd.arg("--new-window").env("LAUNCHPAD_PROFILE", app.paths.profile.name());
    if let Some(id) = id {
        cmd.args(["--workspace", id]);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // The window must outlive the request and, for dev windows, the daemon.
    cmd.kill_on_drop(false);
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().with_context(|| format!("start {}", exe.display()))?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

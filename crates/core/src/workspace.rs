//! Workspaces: user-defined groups of projects. Every project belongs to exactly one workspace
//! (`project.workspace_id`). A window always shows one workspace; its agents, scheduled launches, crons and inbox
//! threads are the ones of its projects. There is always one: Home creates the first at its first start, and the
//! last one cannot be deleted.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Patch {
    pub name: Option<String>,
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

pub async fn get(app: &App, id: &str) -> Result<Workspace> {
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

pub async fn create(app: &App, name: &str) -> Result<Workspace> {
    let id = uuid::Uuid::new_v4().to_string();
    let w = Workspace { name: check_name(app, name, &id).await?, id, created_at: now_ms() };
    save(app, &w).await?;
    if crate::home_git::here(app)
        && let Err(e) = crate::ask_bot::seed_workspace(app, &w.id).await
    {
        tracing::warn!("Ask Launchpad of {}: {e:#}", w.name);
    }
    if crate::home_git::here(app)
        && let Err(e) = crate::librarian::seed_workspace(app, &w.id).await
    {
        tracing::warn!("Librarian of {}: {e:#}", w.name);
    }
    Ok(w)
}

pub async fn update(app: &App, id: &str, patch: Patch) -> Result<Workspace> {
    let mut w = get(app, id).await?;
    if let Some(name) = patch.name {
        w.name = check_name(app, &name, id).await?;
    }
    save(app, &w).await?;
    Ok(w)
}

/// Runner: replaces every workspace with Home's.
pub async fn replace_all(app: &App, all: Vec<Workspace>) -> Result<()> {
    let rows: Vec<(String, String)> = all.iter().map(|w| Ok((w.id.clone(), serde_json::to_string(w)?))).collect::<Result<_>>()?;
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM workspace", [])?;
            for (id, json) in rows {
                tx.execute("INSERT INTO workspace (id, json) VALUES (?1, ?2)", [id, json])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("workspaces", "changed", serde_json::Value::Null);
    Ok(())
}

/// The projects of a workspace.
pub async fn project_ids(app: &App, id: &str) -> Result<Vec<String>> {
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT id FROM project WHERE workspace_id = ?1")?;
            let rows = st.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

/// Deletes an empty workspace. Its folders must first move to another workspace or be removed, so every
/// folder keeps exactly one workspace.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    let all = list(app).await?;
    if !all.iter().any(|w| w.id == id) {
        bail!("no such workspace");
    }
    if all.len() <= 1 {
        bail!("the last workspace cannot be deleted: Launchpad always shows one");
    }
    let ids = project_ids(app, id).await?;
    let folders = ids.len();
    if folders > 0 {
        // Bots' own folders (created before bots could run without one) count: delete the bot first, then remove its folder.
        let bots = if crate::home_git::here(app) { crate::bots::list(app).await.unwrap_or_default() } else { vec![] };
        let own = bots.iter().filter(|b| b.workspace_id == id && b.own_folder && b.settings.folder.as_ref().is_some_and(|f| ids.contains(f))).count();
        let hint = if own > 0 { format!(" ({own} belong{} to bots: delete the bot in Bots, then remove its folder)", if own == 1 { "s" } else { "" }) } else { String::new() };
        bail!("move or remove its {folders} folder{} first{hint}", if folders == 1 { "" } else { "s" });
    }
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM workspace WHERE id = ?1", [id])?)).await?;
    app.bus.publish("workspaces", "changed", serde_json::Value::Null);
    Ok(())
}

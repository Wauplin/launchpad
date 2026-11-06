//! Default tools: which of the tools and MCP servers granted to a workspace a new session gets without a click.
//!
//! Grants stay in Home's registry (`vault`, `mcp`): a tool or server is reachable only in the workspaces it is
//! granted to. Among the granted ones, each layer may switch an item on or off by default, and the more specific
//! layer wins: the workspace, then the session's folder, then its bot (`bot.toml`, `[tools]`). Without any
//! choice an item is on. A launch then switches items on or off for itself (`LaunchSpec::tools_off`/`tools_on`).
//! Home resolves the choice when it builds a lease, so a changed default applies at the session's next start.
//!
//! Keys are tool IDs, and `mcp:<server id>` for MCP servers (`mcp::launch_off`).

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::App;

/// Item key → on (true) or off (false). Items not listed follow the layer above.
pub type ToolChoice = BTreeMap<String, bool>;

/// The workspace and folder layers, by ID (bots keep theirs in `bot.toml`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct ToolDefaults {
    pub workspaces: BTreeMap<String, ToolChoice>,
    pub folders: BTreeMap<String, ToolChoice>,
}

/// The layer a default is set on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Workspace,
    Folder,
}

impl Scope {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "workspace" => Ok(Self::Workspace),
            "folder" => Ok(Self::Folder),
            _ => bail!("defaults are set on a workspace or a folder"),
        }
    }
    fn key(self, id: &str) -> String {
        match self {
            Self::Workspace => format!("workspace:{id}"),
            Self::Folder => format!("folder:{id}"),
        }
    }
}

/// Whether `key` is on: on unless a layer (most general first) says otherwise, then the launch's own choice.
pub fn is_on(layers: &[&ToolChoice], launch_off: &[String], launch_on: &[String], key: &str) -> bool {
    if launch_on.iter().any(|k| k == key) {
        return true;
    }
    if launch_off.iter().any(|k| k == key) {
        return false;
    }
    layers.iter().rev().find_map(|l| l.get(key).copied()).unwrap_or(true)
}

fn valid_key(k: &str) -> bool {
    let id = k.strip_prefix("mcp:").unwrap_or(k);
    !id.is_empty() && id.len() <= 60 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn check(choice: &ToolChoice) -> Result<()> {
    if let Some(k) = choice.keys().find(|k| !valid_key(k)) {
        bail!("{k:?} is not a tool ID (or mcp:<server ID>)");
    }
    Ok(())
}

pub async fn all(app: &App) -> Result<ToolDefaults> {
    let rows: Vec<(String, String)> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT scope, json FROM tool_defaults")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let mut out = ToolDefaults::default();
    for (scope, json) in rows {
        let Ok(choice) = serde_json::from_str::<ToolChoice>(&json) else { continue };
        if let Some(id) = scope.strip_prefix("workspace:") {
            out.workspaces.insert(id.into(), choice);
        } else if let Some(id) = scope.strip_prefix("folder:") {
            out.folders.insert(id.into(), choice);
        }
    }
    Ok(out)
}

pub async fn get(app: &App, scope: Scope, id: &str) -> Result<ToolChoice> {
    let all = all(app).await?;
    let map = match scope {
        Scope::Workspace => all.workspaces,
        Scope::Folder => all.folders,
    };
    Ok(map.get(id).cloned().unwrap_or_default())
}

/// Replaces a layer (Home). An empty choice removes it.
pub async fn set(app: &App, scope: Scope, id: &str, choice: ToolChoice) -> Result<ToolChoice> {
    anyhow::ensure!(crate::vault::here(app), "default tools are kept on Home: change them there");
    check(&choice)?;
    match scope {
        Scope::Workspace => drop(crate::workspace::get(app, id).await?),
        Scope::Folder => anyhow::ensure!(crate::projects::get(app, id).await?.is_some(), "no such folder"),
    }
    let (key, json) = (scope.key(id), serde_json::to_string(&choice)?);
    let empty = choice.is_empty();
    app.state_db
        .call(move |c| {
            if empty {
                c.execute("DELETE FROM tool_defaults WHERE scope = ?1", [key])?;
            } else {
                c.execute("INSERT INTO tool_defaults (scope, json) VALUES (?1, ?2) ON CONFLICT (scope) DO UPDATE SET json = ?2", [key, json])?;
            }
            Ok(())
        })
        .await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(choice)
}

/// The layers of a session: its workspace, its folder and its bot.
pub async fn layers(app: &App, workspace: Option<&str>, folder: Option<&str>, bot: Option<&str>) -> Result<Vec<ToolChoice>> {
    let all = all(app).await?;
    let mut out = vec![];
    out.extend(workspace.and_then(|w| all.workspaces.get(w)).cloned());
    out.extend(folder.and_then(|f| all.folders.get(f)).cloned());
    if let Some(b) = bot
        && let Ok(b) = crate::bots::get(app, b).await
    {
        out.push(b.settings.tools);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(items: &[(&str, bool)]) -> ToolChoice {
        items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn most_specific_layer_then_launch_wins() {
        let ws = choice(&[("gh", false), ("hf", false)]);
        let folder = choice(&[("gh", true)]);
        let bot = choice(&[("gh", false), ("mcp:linear", false)]);
        let none: Vec<String> = vec![];
        assert!(is_on(&[], &none, &none, "gh"));
        assert!(!is_on(&[&ws], &none, &none, "gh"));
        assert!(is_on(&[&ws, &folder], &none, &none, "gh"));
        assert!(!is_on(&[&ws, &folder, &bot], &none, &none, "gh"));
        assert!(!is_on(&[&ws, &folder], &none, &none, "hf"));
        assert!(!is_on(&[&bot], &none, &none, "mcp:linear"));
        assert!(is_on(&[&ws, &folder, &bot], &none, &["gh".into()], "gh"));
        assert!(!is_on(&[], &["gh".into()], &none, "gh"));
    }

    #[test]
    fn keys() {
        assert!(check(&choice(&[("gh", true), ("mcp:linear-2", false)])).is_ok());
        assert!(check(&choice(&[("GH", true)])).is_err());
        assert!(check(&choice(&[("mcp:", true)])).is_err());
    }
}

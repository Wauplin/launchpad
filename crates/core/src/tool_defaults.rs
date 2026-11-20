//! Default tools: which of the tools and MCP servers granted to a workspace a new session gets without a click.
//!
//! Grants stay in Home's registry (`vault`, `mcp`): a tool or server is reachable only in the workspaces it is
//! granted to. Among the granted ones, each layer may switch an item on or off by default, and the more specific
//! layer wins: the workspace, then the session's folder, then its bot (`bot.toml`, `[tools]`). Without any
//! choice an item is on, except Slack profiles (`slack:<handle>`, `opt_in`), which follow their own default
//! (`slack::Profile::default_on`, off unless the owner chose otherwise); a workspace or folder layer never switches
//! one on for a bot's session, only the bot's own `bot.toml` does (`for_bot`), and an agent never switches one on
//! (`agent_switches_on`). A launch then switches items on or off for itself (`LaunchSpec::tools_off`/`tools_on`).
//! Home resolves the choice when it builds a lease (tools, MCP servers) or answers a Slack call (profiles), so a
//! changed default applies at the session's next start, and at once for Slack profiles.
//!
//! Keys are tool IDs, `mcp:<server id>` for MCP servers (`mcp::launch_off`) and `slack:<handle>` for Slack
//! profiles (`slack::key`).

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

/// Items that act in someone's name towards other people (Slack profiles): only the owner switches them on, and a
/// workspace or folder default never does for a bot.
pub fn opt_in(key: &str) -> bool {
    key.starts_with(crate::slack::KEY_PREFIX)
}

/// A workspace or folder layer as a bot's session reads it: it may switch an opt-in tool off, never on (every bot of
/// the workspace, Ask Launchpad included, would get it). A bot gets one only from its own bot.toml.
pub fn for_bot(mut layer: ToolChoice) -> ToolChoice {
    layer.retain(|k, on| !(opt_in(k) && *on));
    layer
}

/// Pure: the first opt-in item (a Slack profile) a request body switches on (`tools_on: [...]`, or `"<id>": true` in a `tools`
/// choice), anywhere in it: an agent never does (launch specs, schedules, routines, default tools, bot settings).
pub fn agent_switches_on(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.iter().find_map(|(k, v)| {
            let direct = match (k.as_str(), v) {
                ("tools_on", Value::Array(a)) => a.iter().filter_map(Value::as_str).find(|t| opt_in(t)).map(str::to_string),
                ("tools", Value::Object(t)) => t.iter().find(|(id, on)| opt_in(id) && **on == Value::Bool(true)).map(|(id, _)| id.clone()),
                _ => None,
            };
            direct.or_else(|| agent_switches_on(v))
        }),
        Value::Array(a) => a.iter().find_map(agent_switches_on),
        _ => None,
    }
}

/// Whether a tool or MCP server is on: on unless a layer (most general first) says otherwise, then the launch's own
/// choice.
pub fn is_on(layers: &[&ToolChoice], launch_off: &[String], launch_on: &[String], key: &str) -> bool {
    is_on_with(layers, launch_off, launch_on, key, true)
}

/// `is_on` with the item's own default when no layer names it (a Slack profile's `default_on`).
pub fn is_on_with(layers: &[&ToolChoice], launch_off: &[String], launch_on: &[String], key: &str, default: bool) -> bool {
    if launch_on.iter().any(|k| k == key) {
        return true;
    }
    if launch_off.iter().any(|k| k == key) {
        return false;
    }
    layers.iter().rev().find_map(|l| l.get(key).copied()).unwrap_or(default)
}

fn valid_key(k: &str) -> bool {
    let id = k.strip_prefix("mcp:").or_else(|| k.strip_prefix(crate::slack::KEY_PREFIX)).unwrap_or(k);
    !id.is_empty() && id.len() <= 60 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn check(choice: &ToolChoice) -> Result<()> {
    if let Some(k) = choice.keys().find(|k| !valid_key(k)) {
        bail!("{k:?} is not a tool ID (or mcp:<server ID>, slack:<profile handle>)");
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

/// The layers of a session: its workspace, its folder and its bot (a bot's session reads the first two `for_bot`).
pub async fn layers(app: &App, workspace: Option<&str>, folder: Option<&str>, bot: Option<&str>) -> Result<Vec<ToolChoice>> {
    let all = all(app).await?;
    let mut out = vec![];
    let shared = |l: Option<&ToolChoice>| l.cloned().map(|l| if bot.is_some() { for_bot(l) } else { l });
    out.extend(shared(workspace.and_then(|w| all.workspaces.get(w))));
    out.extend(shared(folder.and_then(|f| all.folders.get(f))));
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
    fn slack_profiles_follow_their_own_default() {
        let none: Vec<String> = vec![];
        let w = "slack:write-as-bot";
        // Off without a choice when the profile is off by default; on when the profile is on by default.
        assert!(!is_on_with(&[], &none, &none, w, false));
        assert!(is_on_with(&[], &none, &none, w, true));
        assert!(!is_on_with(&[&choice(&[(w, false)])], &none, &none, w, true));
        // An explicit choice of the owner: a workspace or folder default (non-bot sessions), a launch, a bot.toml.
        let ws_on = choice(&[(w, true)]);
        assert!(is_on_with(&[&ws_on], &none, &none, w, false));
        assert!(is_on_with(&[], &none, &[w.into()], w, false));
        assert!(is_on_with(&[&for_bot(ws_on.clone()), &choice(&[(w, true)])], &none, &none, w, false));
        // A bot's session ignores the workspace's and folder's "on", keeps their "off" and other tools.
        let shared = choice(&[(w, true), ("gh", false)]);
        assert!(!is_on_with(&[&for_bot(shared.clone())], &none, &none, w, false));
        assert!(!is_on(&[&for_bot(shared)], &none, &none, "gh"));
        assert_eq!(for_bot(choice(&[(w, false)])), choice(&[(w, false)]));
    }

    #[test]
    fn agents_never_switch_on_a_slack_profile() {
        use serde_json::json;
        let w = "slack:user-write";
        assert_eq!(agent_switches_on(&json!({"spec": {"tools_on": ["gh", w]}})).as_deref(), Some(w));
        assert_eq!(agent_switches_on(&json!({"tools": {w: true}})).as_deref(), Some(w));
        assert_eq!(agent_switches_on(&json!({"set": {"tools": {w: true}}})).as_deref(), Some(w));
        assert_eq!(agent_switches_on(&json!({"settings": {"tools": {w: true}}})).as_deref(), Some(w));
        assert!(agent_switches_on(&json!({"tools": {w: false, "gh": true}, "spec": {"tools_on": ["gh"], "tools_off": [w]}})).is_none());
        // Text that merely names the profile is not a switch.
        assert!(agent_switches_on(&json!({"text": "use slack:user-write", "tools": [w]})).is_none());
    }

    #[test]
    fn keys() {
        assert!(check(&choice(&[("gh", true), ("mcp:linear-2", false)])).is_ok());
        assert!(check(&choice(&[("GH", true)])).is_err());
        assert!(check(&choice(&[("mcp:", true)])).is_err());
        assert!(check(&choice(&[("slack:user-read", true)])).is_ok() && check(&choice(&[("slack:", true)])).is_err());
    }
}

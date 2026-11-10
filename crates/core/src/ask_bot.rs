//! Ask Launchpad: the bot every workspace starts with. It is a normal bot (BOT.md, bot.toml, memory/, no folder:
//! each session runs in a temporary folder) that reads Launchpad state, writes proposals and
//! changes Launchpad's settings with the Launchpad CLI tool at admin access (`launchpad = "admin"` in its bot.toml),
//! like any bot given that access. Home creates it once per workspace and the
//! Launchpad CLI tool once; the owner may edit, replace or delete both, and Home never recreates them.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::{App, bots, util::now_ms};

pub const NAME: &str = "Ask Launchpad";

pub const DEFAULT_INSTRUCTIONS: &str = "# Ask Launchpad

You help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.

## How you work

- Read the state you need with the Launchpad CLI before you answer or propose. Check facts instead of guessing.
- You do not do the work yourself: you propose it. One proposal per independent task.
- When the owner asks you to change Launchpad itself (a bot, its instructions or settings, a routine, a schedule, a workspace), make the change directly with your admin access, then say what you changed. To turn one bot.toml setting on or off (for example `inbox_done`, `proposals` or `launchpad`), send only that field: `launchpad bot update <bot-id> --body-file <file>` with `{\"set\": {\"inbox_done\": true}}` keeps the bot's other settings. Starting agents and messaging them stay proposals unless the owner says otherwise. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.
- Work that should wait for quota: a schedule with a quota condition.
- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron. A bot's BOT.md and routines are files of its folder (`routines/<name>.toml`): to change several of them, propose one `bot-files` card (one diff per file).
- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.
- Note only durable things about the owner (their habits, how they like proposals) or what they ask you to remember, in a few short lines of your memory (`launchpad bot memory write`). Facts other agents need go in the knowledge base. Never log your answers, proposals or runs.

## How you answer

- Short sentences, plain words. Lead with the answer.
- After proposing, say what each card does and check its returned state.
";

/// Its `starters` in bot.toml: the chips over its message box. Ending in "…": fills the box.
pub const STARTERS: &[&str] = &["What needs me?", "How are my agents doing?", "What happened today?", "Start an agent to …", "What is the status of …"];

fn starters() -> Vec<String> {
    STARTERS.iter().map(|s| s.to_string()).collect()
}

/// Marks `key` done; false when it already was.
pub(crate) async fn claim(app: &App, key: String) -> Result<bool> {
    app.state_db.call(move |c| Ok(c.execute("INSERT OR IGNORE INTO seeded (key, at) VALUES (?1, ?2)", params![key, now_ms()])? == 1)).await
}

pub(crate) async fn done(app: &App, key: String) -> Result<bool> {
    app.state_db.call(move |c| Ok(c.query_row("SELECT 1 FROM seeded WHERE key = ?1", [key], |_| Ok(())).optional()?.is_some())).await
}

/// Home: creates the Launchpad CLI tool and each workspace's Ask Launchpad, once.
pub async fn seed(app: &App) -> Result<()> {
    if !crate::home_git::here(app) {
        return Ok(());
    }
    if !done(app, "tool:launchpad".into()).await? {
        if !crate::vault::tools(app).await?.iter().any(|t| t.id == crate::vault::LAUNCHPAD_TOOL) {
            crate::vault::put_tool(app, crate::vault::launchpad_tool()).await?;
        }
        claim(app, "tool:launchpad".into()).await?;
    }
    for w in crate::workspace::list(app).await? {
        seed_workspace(app, &w.id).await?;
        // Not fatal: an Ask Launchpad whose settings no longer pass the checks gets them at a later start.
        if let Err(e) = seed_starters(app, &w.id).await {
            tracing::warn!("Ask Launchpad's starters in workspace {}: {e:#}", w.id);
        }
    }
    Ok(())
}

/// Creates the workspace's Ask Launchpad once (also right after a new workspace).
pub async fn seed_workspace(app: &App, workspace_id: &str) -> Result<()> {
    let key = format!("ask-launchpad:{workspace_id}");
    if done(app, key.clone()).await? {
        return Ok(());
    }
    // An earlier start may have created it before it could record it.
    let exists = bots::list(app).await?.iter().any(|b| b.workspace_id == workspace_id && b.slug == "ask-launchpad");
    if !exists {
        let instructions = DEFAULT_INSTRUCTIONS.to_string();
        bots::create(app, bots::Create { workspace_id: workspace_id.into(), name: NAME.into(), instructions, profile: None, folder: None, permission_mode: Default::default(), proposals: Default::default(), launchpad: crate::vault::LaunchpadAccess::Admin, starters: starters() }).await?;
    }
    claim(app, key).await?;
    Ok(())
}

/// Gives an Ask Launchpad created before bots had starters its starters, once per workspace (kept when it has some).
async fn seed_starters(app: &App, workspace_id: &str) -> Result<()> {
    let key = format!("ask-launchpad-starters:{workspace_id}");
    if done(app, key.clone()).await? {
        return Ok(());
    }
    if let Some(b) = bots::list(app).await?.into_iter().find(|b| b.workspace_id == workspace_id && b.slug == "ask-launchpad" && b.settings_error.is_none() && b.settings.starters.is_empty()) {
        let mut set = serde_json::Map::new();
        set.insert("starters".into(), serde_json::json!(starters()));
        bots::update(app, &b.id, bots::Patch { set: Some(set), ..Default::default() }).await?;
    }
    claim(app, key).await?;
    Ok(())
}

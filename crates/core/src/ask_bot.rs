//! Ask Launchpad: the bot every workspace starts with. It is a normal bot (BOT.md, bot.toml, its own plain
//! folder, notes in `bots/ask-launchpad/` of the knowledge base) that reads Launchpad state and writes proposals
//! with the Launchpad CLI tool, like any bot granted that tool. Home creates it once per workspace and the
//! Launchpad CLI tool once; the owner may edit, replace or delete both, and Home never recreates them.
//!
//! The first run also moves the former built-in assistant's records to the new bots: its routines (the morning
//! brief), its incoming hooks and a customized BOT.md.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::{App, bots, util::now_ms};

pub const NAME: &str = "Ask Launchpad";

pub const DEFAULT_INSTRUCTIONS: &str = "# Ask Launchpad

You help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.

## How you work

- Read the state you need with the Launchpad CLI before you answer or propose. Check facts instead of guessing.
- You do not do the work yourself: you propose it. One proposal per independent task. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.
- Work that should wait for quota: a schedule with a quota condition.
- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron.
- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.
- Keep what you learn about the owner (their folders, habits, how they like proposals) in your notes, so the next session knows it.

## How you answer

- Short sentences, plain words. Lead with the answer.
- After proposing, say what each card does and check its returned state.
";

/// Marks `key` done; false when it already was.
async fn claim(app: &App, key: String) -> Result<bool> {
    app.state_db.call(move |c| Ok(c.execute("INSERT OR IGNORE INTO seeded (key, at) VALUES (?1, ?2)", params![key, now_ms()])? == 1)).await
}

async fn done(app: &App, key: String) -> Result<bool> {
    app.state_db.call(move |c| Ok(c.query_row("SELECT 1 FROM seeded WHERE key = ?1", [key], |_| Ok(())).optional()?.is_some())).await
}

/// The former assistant's BOT.md, when the owner had edited it.
async fn legacy_instructions(app: &App) -> Option<String> {
    let sha: Option<String> = app.state_db.call(|c| Ok(c.query_row("SELECT sha256 FROM plain_file WHERE store = 'assistant' AND path = 'BOT.md'", [], |r| r.get(0)).optional()?)).await.ok()?;
    let text = bots::blob_string(app, sha.as_deref()).await;
    (!text.trim().is_empty()).then_some(text)
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
    }
    adopt_legacy(app).await
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
        let instructions = legacy_instructions(app).await.unwrap_or_else(|| DEFAULT_INSTRUCTIONS.into());
        bots::create(app, bots::Create { workspace_id: workspace_id.into(), name: NAME.into(), instructions, profile: None, folder: None, permission_mode: Default::default(), proposals: Default::default() }).await?;
    }
    claim(app, key).await?;
    Ok(())
}

/// The bot that takes over a former assistant record of `workspace` (None: Everything, the first workspace's).
async fn heir(app: &App, workspace: Option<&str>) -> Result<Option<bots::Bot>> {
    let all = bots::list(app).await?;
    let first_ws = crate::workspace::list(app).await?.into_iter().min_by_key(|w| w.created_at).map(|w| w.id);
    let ws = workspace.map(str::to_string).or(first_ws);
    Ok(all.into_iter().find(|b| Some(&b.workspace_id) == ws.as_ref() && b.slug == "ask-launchpad"))
}

/// Moves the former assistant's routines and hooks to the Ask Launchpad bot of their workspace, once.
async fn adopt_legacy(app: &App) -> Result<()> {
    if done(app, "legacy-assistant".into()).await? {
        return Ok(());
    }
    type Rows = Vec<(String, Option<String>)>;
    let crons: Rows = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT id, coalesce(json_extract(json, '$.spec.target.workspace_id'), json_extract(json, '$.spec.workspace_id')) FROM cron_agent WHERE json_extract(json, '$.spec.target.kind') = 'assistant' AND json_extract(json, '$.bot_id') IS NULL")?;
        Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    for (id, ws) in crons {
        let Some(bot) = heir(app, ws.as_deref()).await? else { continue };
        let (folder, ws, bot_id) = (bot.settings.folder.clone(), bot.workspace_id.clone(), bot.id.clone());
        app.state_db.call(move |c| Ok(c.execute(
            "UPDATE cron_agent SET json = json_set(json, '$.bot_id', ?2, '$.spec.target', json_object('kind', 'project', 'project_id', ?3), '$.spec.workspace_id', ?4) WHERE id = ?1",
            params![id, bot_id, folder, ws],
        )?)).await?;
    }
    let hooks: Rows = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT id, json_extract(json, '$.target.workspace_id') FROM incoming_hook WHERE json_extract(json, '$.target.kind') = 'ask'")?;
        Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    for (id, ws) in hooks {
        let Some(bot) = heir(app, ws.as_deref()).await? else { continue };
        let bot_id = bot.id.clone();
        app.state_db.call(move |c| Ok(c.execute("UPDATE incoming_hook SET json = json_set(json, '$.target', json_object('kind', 'bot', 'id', ?2)) WHERE id = ?1", params![id, bot_id])?)).await?;
    }
    app.state_db.call(|c| Ok(c.execute("DELETE FROM queued_message WHERE target LIKE 'ask:%'", [])?)).await?;
    claim(app, "legacy-assistant".into()).await?;
    app.bus.publish("crons", "changed", serde_json::Value::Null);
    Ok(())
}

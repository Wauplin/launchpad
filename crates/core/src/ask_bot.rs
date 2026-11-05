//! Ask Launchpad as a built-in bot: one record for every workspace. Its `BOT.md` (how the owner wants it to
//! work) is a versioned file Home stores like a bot's files (store `assistant`); its profile, model and proposal
//! mode stay in config.toml (`[assistant]`), since every daemon reads them. The safety rules and the proposal
//! reference are built into the prompt (`proposals::instruction`) and never editable. Reset puts back the
//! default `BOT.md` (as a new version) and the default settings; threads, proposals and routines stay.
//!
//! Home reads `BOT.md` when it composes a thread's first message (`proposals::ask_spec`) and sends it in the
//! launch spec, so a runner builds the same prompt. Follow-up messages never change the running thread.

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use ts_rs::TS;

use crate::{App, bots, plain};

/// The ID proposals use for Ask Launchpad's own `BOT.md` (`bot-instructions` with this `bot_id`).
pub const ID: &str = "ask-launchpad";
pub const STORE: &str = "assistant";

pub const DEFAULT_INSTRUCTIONS: &str = "# Ask Launchpad

You help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.

## How you work

- Read the state you need before you answer or propose. Check facts instead of guessing.
- One proposal per independent task. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.
- Work that should wait for quota: a schedule with a quota condition.
- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron.
- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.

## How you answer

- Short sentences, plain words. Lead with the answer.
- After proposing, say what each card does and check its returned state.
";

/// What the Ask page shows: the owner's part, the default and the built-in part of the prompt.
#[derive(Clone, Debug, Serialize, TS)]
pub struct AskBot {
    /// BOT.md.
    pub instructions: String,
    pub default_instructions: String,
    /// The built-in part of the first message, as the assistant reads it (without its token).
    pub builtin: String,
    /// BOT.md or the settings differ from the defaults.
    pub customized: bool,
    #[ts(type = "number")]
    pub version: i64,
}

fn ensure_here(app: &App) -> Result<()> {
    ensure!(crate::home_git::here(app), "Ask Launchpad's files are stored by Home");
    Ok(())
}

/// The current BOT.md, or the default before the first edit.
pub async fn instructions(app: &App) -> Result<String> {
    ensure_here(app)?;
    let tree = plain::tree(app, STORE).await?;
    Ok(if tree.files.iter().any(|f| f.path == bots::INSTRUCTIONS) { bots::text_of(app, &tree.files, bots::INSTRUCTIONS).await } else { DEFAULT_INSTRUCTIONS.to_string() })
}

pub async fn get(app: &App) -> Result<AskBot> {
    let instructions = instructions(app).await?;
    let version = plain::tree(app, STORE).await?.version;
    let customized = instructions != DEFAULT_INSTRUCTIONS || app.config.get().assistant != crate::config::Assistant::default();
    Ok(AskBot { instructions, default_instructions: DEFAULT_INSTRUCTIONS.into(), builtin: crate::proposals::builtin_preview(), customized, version })
}

/// Saves a new BOT.md; the next thread reads it.
pub async fn update(app: &App, text: String) -> Result<AskBot> {
    ensure_here(app)?;
    ensure!(!text.trim().is_empty(), "BOT.md is empty: use Reset to go back to the default");
    let text = if text.ends_with('\n') { text } else { format!("{text}\n") };
    bots::write_store(app, STORE, vec![(bots::INSTRUCTIONS, text)]).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": ID}));
    get(app).await
}

/// Reset to default: the default BOT.md (a new version, so Restore previous can undo it) and the default
/// profile, model and Manual mode.
pub async fn reset(app: &App) -> Result<AskBot> {
    update(app, DEFAULT_INSTRUCTIONS.into()).await?;
    let mut config = app.config.get();
    if config.assistant != crate::config::Assistant::default() {
        config.assistant = Default::default();
        app.config.save(config)?;
    }
    get(app).await
}

/// Changes of BOT.md in the last 30 days, newest first.
pub async fn history(app: &App) -> Result<Vec<plain::Change>> {
    ensure_here(app)?;
    plain::history(app, STORE).await
}

pub async fn diff(app: &App, change: i64) -> Result<String> {
    let c = history(app).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    Ok(crate::kb::unified(&bots::blob_string(app, c.sha256.as_deref()).await, &bots::blob_string(app, c.new_sha256.as_deref()).await))
}

/// Restore previous: BOT.md as it was before a change, as a new change by you.
pub async fn restore(app: &App, change: i64) -> Result<AskBot> {
    let c = history(app).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    if c.path != bots::INSTRUCTIONS {
        bail!("unknown file {}", c.path);
    }
    match c.sha256.as_deref() {
        // The first edit created the file: before it, Ask Launchpad used the default.
        None => update(app, DEFAULT_INSTRUCTIONS.into()).await,
        Some(sha) => update(app, bots::blob_string(app, Some(sha)).await).await,
    }
}

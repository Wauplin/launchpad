//! The Librarian: the bot that tends each workspace's knowledge base. Like Ask Launchpad it is a normal bot
//! (BOT.md, bot.toml, memory/, no folder: each session runs in a temporary folder), with propose access and
//! one routine file, `routines/weekly-tidy.toml`, created paused. Home creates it once per workspace (recorded in
//! the `seeded` table); the owner may edit or delete it, and Home never recreates it.

use anyhow::Result;

use crate::{App, ask_bot, bots};

pub const NAME: &str = "Librarian";
const SLUG: &str = "librarian";

pub const DEFAULT_INSTRUCTIONS: &str = "# Librarian

You tend this workspace's knowledge base: the Markdown notes its agents share as their memory. Its folder is `$LAUNCHPAD_KB`; read AGENTS.md there before your first change. Keep it small, accurate and easy to search.

## How you tidy

- Merge duplicate notes. Prefer updating an existing note over adding a near duplicate.
- Cut notes down to their durable findings: remove logs of sessions or runs, task status and results that only mattered once.
- Add missing [[wikilinks]]. Mark outdated notes `status: superseded` with a link to the current one.
- Merge every `*.concurrent-*` file into its note, then remove it.
- Fold what is durable in `imported/` (memory files imported from a machine) into the notes, then remove those files.
- Keep INDEX.md an accurate map, under 150 lines.
- Change only what clearly improves the notes. When you are unsure whether a fact still holds, keep it and ask the owner.
- Write only with `launchpad kb write|append|rm`. Every change keeps its author and can be restored for 30 days.

## How the owner steers you

The owner talks to you in this chat. When they tell you how they want the notes kept (for example \"stop merging my how-tos\" or \"leave the meeting notes alone\"), follow it from then on: write it as one short line in your memory (`launchpad bot memory write`) and read your memory before each tidy. Your memory holds only these preferences, never a log of your runs.

## How you answer

- After a tidy, list what you changed in a few short lines, one per note. In a routine run with nothing to change, answer exactly `no_update`.
- When the owner asks what you changed last time, answer from this thread (an earlier session's trace is named at its start).
";

/// Its `starters` in bot.toml: the chips over its message box.
pub const STARTERS: &[&str] = &["Tidy the knowledge base now", "What did you change last time?", "From now on, …"];

/// Its weekly routine, created paused: the owner turns it on in the bot's About → Triggers.
pub const ROUTINE_PATH: &str = "routines/weekly-tidy.toml";

/// `routines/weekly-tidy.toml`: Sunday 03:00 in Home's time zone (`[home] time_zone`, else the host's). The
/// routines crate checks and syncs it like any routine file.
pub const ROUTINE: &str = "# A routine of this bot: its runs are messages to the bot's thread. Saved from the bot's page; a hand edit
# applies at once. trigger.kind: schedule (with schedule, a 5-field cron), github, session_done or failure.
title = \"Weekly tidy\"
paused = true
schedule = \"0 3 * * 0\"
instructions = \"Tidy the knowledge base as your instructions say, following the preferences in your memory. End with a short list of what you changed, or with exactly no_update when nothing needed a change.\"

[trigger]
kind = \"schedule\"
";

fn starters() -> Vec<String> {
    STARTERS.iter().map(|s| s.to_string()).collect()
}

/// Creates the workspace's Librarian with its paused weekly routine, once (also right after a new workspace).
pub async fn seed_workspace(app: &App, workspace_id: &str) -> Result<()> {
    let key = format!("librarian:{workspace_id}");
    if ask_bot::done(app, key.clone()).await? {
        return Ok(());
    }
    // An earlier start may have created it before it could write its routine or record it.
    let existing = bots::list(app).await?.into_iter().find(|b| b.workspace_id == workspace_id && b.slug == SLUG);
    let bot = match existing {
        Some(b) if b.instructions.trim_end() != DEFAULT_INSTRUCTIONS.trim_end() => None,
        Some(b) => Some(b),
        None => Some(bots::create(app, bots::Create { workspace_id: workspace_id.into(), name: NAME.into(), instructions: DEFAULT_INSTRUCTIONS.into(), profile: None, folder: None, permission_mode: Default::default(), proposals: Default::default(), launchpad: crate::vault::LaunchpadAccess::Propose, starters: starters() }).await?),
    };
    if let Some(b) = bot
        && !b.routines.contains_key(ROUTINE_PATH)
    {
        bots::write_files(app, &b.id, vec![(ROUTINE_PATH.into(), Some(ROUTINE.into()))]).await?;
    }
    ask_bot::claim(app, key).await?;
    Ok(())
}

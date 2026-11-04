# Bots

A bot is an agent that stays. It has a name, a job you describe, one thread you come back to, and notes it keeps in the knowledge base between sessions. Bots are the first part of the [Bots plan](https://wauplin-launchpad-bots-research.static.hf.space/bots.html) (phase A). Routines (phase B) come later: today a bot works only when you send it a message.

Bots live on Home with your other records. Each session of a bot runs on a runner, contained like any agent.

## Create a bot

Open **Bots** in the sidebar and click **+** (or **New bot** on the Bots page). The form asks for four things:

- **Name**, for example "Scout".
- **Its job**: what it does and how you like it done. This text becomes `BOT.md`, the bot's instructions.
- **Profile**: the profile its sessions use.
- **Folder**: a folder of the workspace, or **Its own folder** (a new plain folder named after the bot, which Home syncs and versions).

The form shows what the bot can reach: its folder, the knowledge base of its workspace, and the tools granted to the workspace. Bots run in YOLO mode by default; change it in the bot's Settings tab. **Create bot** saves the bot and starts nothing. The bot's page then asks for its first task.

With a Git folder, **Start from: Repo maintainer** fills in a job for that folder.

You can also ask Ask Launchpad: "Create a bot that watches the Xet changelog." It writes a **New bot** card with the name, profile, folder and instructions; **Create bot** creates it.

## The thread

A bot has one thread: a chain of sessions. Your messages go to its current session, which is a normal agent session (listed in Agents too, with the bot's name).

A new session starts:

- for the first message,
- when the bot's profile or folder changed (the page says so before you send),
- when the current session was archived,
- when you click **New session**.

A new session gets a reference to the previous session's trace (`launchpad trace …`), never a copy of it, and the bot's notes in the knowledge base. The harness compacts a long context on its own.

With a Git folder, each session works in a fresh worktree of Home Git. The bot prepares branches, Local PRs and suggestions for your review, and acts directly on GitHub only when you explicitly ask.

## The bot's page

- **Memory**: the bot's notes in the knowledge base (`bots/<slug>/`) and the recent changes made by its sessions. Correct a note on the Knowledge page; every change can be restored for 30 days.
- **Files**: `BOT.md` (instructions) and `bot.toml` (name, profile, model, folder, permission mode), editable here. **Versions** lists every change of the last 30 days with its diff and **Restore previous**. The bot cannot edit these files; it tells you what to change.
- **Sessions**: every session of the thread, with its status and context use.
- **Settings**: name, profile, model, folder and permissions, what the bot can reach, and **Delete bot**. Deleting keeps the bot's sessions, its notes and its own folder.

The sidebar lists Ask Launchpad (the built-in bot) and the bots of the workspace. A dot shows when a bot's current session needs you; a spinner when it works. Counts stay in Agents, so a session is never counted twice.

## Files

Home stores each bot's files with versions, like a plain folder, and includes them in every backup. Home's backups also keep readable copies in the bucket: `<workspace>/bots/<slug>/BOT.md` and `bot.toml`, next to `<workspace>/kb/` for the knowledge base. Home never reads these copies back.

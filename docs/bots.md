# Bots

A bot is an agent that stays. It has a name, a job you describe, one thread you come back to, and notes it keeps in the knowledge base between sessions. Bots and their routines are phases A and B of the [Bots plan](https://wauplin-launchpad-bots-research.static.hf.space/bots.html). A bot works when you message it, and on its own through its routines.

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
- **Routines**: what the bot does on its own (see below).
- **Instructions**: `BOT.md` (instructions) and `bot.toml` (name, profile, model, folder, permission mode), editable here. **Versions** lists every change of the last 30 days with its diff and **Restore previous**. The bot cannot edit these files: when its instructions should change, it proposes a new `BOT.md`, shown as a diff in this tab (and on the Ask page) until you click **Apply to BOT.md**; applying fails if `BOT.md` changed in between.
- **Sessions**: every session of the thread, with its status and context use.
- **Settings**: name, profile, model, folder and permissions, what the bot can reach, and **Delete bot**. Deleting keeps the bot's sessions, its notes and its own folder.

**Pause** (in the header, once the bot has routines) stops every routine and incoming hook of the bot from starting runs; your own messages still go through, and **Stop** in its session ends a run that is working. **Resume** starts them again from their next trigger.

The sidebar lists Ask Launchpad (the built-in bot) and the bots of the workspace. Ask Launchpad has the same page layout, with its own `BOT.md`, versions and a **Reset to default**; see [Make Ask Launchpad yours](agents.md#make-ask-launchpad-yours). A dot shows when a bot's current session needs you; a spinner when it works. Counts stay in Agents, so a session is never counted twice.

## Files

Home stores each bot's files with versions, like a plain folder, and includes them in every backup. Home's backups also keep readable copies in the bucket: `<workspace>/bots/<slug>/BOT.md` and `bot.toml`, next to `<workspace>/kb/` for the knowledge base. Home never reads these copies back.

## Routines

A routine sends the bot a task by itself. Open the bot's **Routines** tab and click **New routine** (or start from **Daily check**, **Review requests**, **CI failures** or **After each agent**). A routine has a name, what the bot does at each run, and when it runs:

- **On a schedule**: a 5-field cron schedule in your browser's time zone (kept with the routine and shown next to it), for example weekdays at 9:00. A proposed routine takes the zone of the browser where you click **Add routine**; without a browser, `[home] time_zone` in config.toml, else the server's.
- **GitHub activity**: new activity in your Inbox (GitHub notifications) for the bot's repository or any repository of the workspace: review requested, mentioned, assigned, CI activity, new release, or any activity. Home is a private Space that GitHub cannot call, so Launchpad reads the Inbox about every minute; all new matching activity of one check starts one run, listed in the run's message.
- **When an agent is done**: another agent's turn ends Done in a folder (or any folder of the workspace). The run gets a reference to that agent's trace. Runs of routines never start other routines.

Optional conditions: at least a share of the profile's 5-hour or weekly quota left (with a reading from the last 15 minutes).

Each run is a message to the bot's thread, shown as one line (**Routine “Daily check” · Scheduled**) that opens to the instructions. The bot prepares work for your review unless the instructions ask it to publish, then ends with a short report (a normal Done that notifies you) or with `no_update`, shown as **✓ No update**: no notification, and the thread does not need you unless an earlier result is still unread. A run while the bot is working is skipped, with the reason. Messages from incoming hooks and Telegram never join a running turn: they wait, in order, until the bot's turn ends.

Every routine shows its last runs as small squares and its history in text: **Report**, **No update**, **Skipped** (why), **Failed** or **Interrupted**, the last successful run and the next one. **Run now** starts a run at once. **Pause** stops one routine; the bot's **Pause** stops all of them.

The bot can propose a routine for itself, for example after you say "check this every morning": the card waits in its Routines tab (and on the Ask page) until you click **Add routine**. Ask Launchpad can propose routines for any bot of the workspace.

All routines, of bots, of Ask Launchpad (the morning brief) and launches repeated on a schedule, are also on the **Schedules** page.

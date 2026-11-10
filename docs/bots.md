# Bots

A bot is an agent that stays. It has a name, a job you describe, one thread you come back to, and notes it keeps in the knowledge base between sessions. Bots and their routines are phases A and B of the [Bots plan](https://wauplin-launchpad-bots-research.static.hf.space/bots.html). A bot works when you message it, and on its own through its routines.

Bots live on Home with your other records. Each session of a bot runs on a runner, contained like any agent.

## Ask Launchpad

Every workspace starts with one bot, **Ask Launchpad**. Home creates it once, without a folder (each of its sessions runs in a temporary folder) and with admin access to the Launchpad CLI. It reads your folders, agents, Inbox, schedules and quotas, answers your questions, turns requests into proposals you start with one click, and changes Launchpad's settings (bots, routines, schedules, workspaces) when you ask it to. It keeps what it learns about you in its notes (`bots/ask-launchpad/` in the knowledge base), like any bot.

Ask Launchpad has no special treatment: its `BOT.md` and `bot.toml` are edited, versioned and restored like any bot's, and you can rename it, delete it or replace it with your own. Home never recreates a deleted Ask Launchpad; **New bot → Start from: Ask Launchpad** creates a new one with the default instructions. Any bot can do the same work, as long as the Launchpad CLI tool is granted to its workspace and its access level allows it.

## The Launchpad CLI tool

**Launchpad CLI** is a tool of Home's registry (Settings → Credentials and tools), added once and granted to every workspace. It needs no token: the CLI uses the session's own credential. What a session may do depends on its access level, set with `launchpad` in a bot's `bot.toml` (Settings tab → **Launchpad CLI**):

| Level | What it allows |
| --- | --- |
| `read` | Read Launchpad state: `launchpad folders\|bots\|sessions\|profiles\|presets\|inbox\|schedules\|proposals list`, `launchpad today`, `launchpad activity list`, `launchpad trace`. |
| `propose` (default) | Read, and write proposals (`launchpad propose …`) that wait for your click. Agents that are not bots always get this level. |
| `admin` | Propose, and change Launchpad itself with the admin actions of the action list, as `launchpad do <group>.<verb>` or the short verb `launchpad <group> <verb>` (for example `launchpad session archive <id>`), or with `launchpad api <method> <path>`: bots (their `BOT.md` and `bot.toml` included), routines, scheduled launches, workspaces, folder settings (name, description, When done, workspace), shared settings (presets, notifications, GitHub poll interval, worktrees kept), dismissing proposals, accepting a proposal whose action is an admin action (tool cards wait for you), launching agents and messaging sessions, MCP tool switches, and checking for and installing a newer Launchpad on Home's Space when you ask (`launchpad home check-update`, then `launchpad home update`, or `launchpad home update --channel main` for the newest packaged `main` commit; updating the runners afterwards stays your click). It also reads Launchpad's own state to diagnose failures (see [Diagnose Launchpad](#diagnose-launchpad)). Ask Launchpad starts with this level. |

**Inbox** (Settings tab, `inbox_done = true` in `bot.toml`, off by default): the bot may mark your Inbox threads done itself, in Launchpad and on GitHub (Done, not just read), with `launchpad inbox done <thread-id>…` (ids from `launchpad inbox list`). It needs `propose` or `admin`; Home checks the bot's current `bot.toml` at each call and only marks threads still in the Inbox. Other agents propose an `inbox.done` ready card instead. This is the only GitHub change the CLI makes.

Apart from that, no level changes or reaches GitHub, the Outbox, channels, credentials, devices, terminals, hosts, runners, setup fixes, backups, Burst, the rest of `config.toml` (profiles included), folder setup commands or paid usage: those stay your clicks. Each direct admin change is recorded on Home with the values before: the bot's chat shows it as a **Changed** line (each changed value as "before → after"; the record name opens its page), and `launchpad activity list` lists them. A change of level applies to running sessions within ten minutes. Remove a workspace from the tool (or switch it off for one launch) and its sessions can no longer read Launchpad state at all.

### Diagnose Launchpad

With admin access, a bot can find out by itself why something failed. These commands read state only, and never show a secret: tokens, credentials, webhook URLs and pairing codes are left out, and log lines are redacted when they are written.

| Command | What it shows |
| --- | --- |
| `launchpad health` | Everything failing now, by area: failed or stuck agents, runners offline with open sessions or on another build, failed Burst jobs and why Burst cannot start, failed routine runs (and runs skipped for want of a quota reading), scheduled launches and cards, quota and harness errors, automatic work held for 15 minutes by a missing quota reading, MCP, Telegram, Inbox sync and GitHub rate limits, backups, the Home update check, setup items left. Each problem names the ids to look at. |
| `launchpad logs` | The recent log lines of Home and every connected runner (the last hour by default). `--session <id>` keeps the lines about one agent and its Burst runners; `--runner <id or name>`, `--home` or `--local` pick one daemon; `--since 2h`, `--level warn`, `--grep <text>`, `--json`. |
| `launchpad sessions show <id>` | An agent with its runner, generation, launch spec, last error, Burst runs and latest handoff. |
| `launchpad failures` | What Home noticed failing in the last 24 hours (`--since`). |
| `launchpad runners list`, `launchpad burst runs`, `launchpad burst job <runner id>` | Runners (connection, build, guard, open sessions); Burst runs with their errors; the HF Job of a run (stage, message, last log lines). |
| `launchpad home`, `launchpad tools list`, `launchpad channels list` | Home's uptime, its last starts (and whether each run stopped cleanly), backups and update state; tools, credential names and MCP servers with their last error; channels and the Telegram link. |

A bot's CLI acts on an agent of another runner (or of a stopped Burst job) like your buttons do: Home routes the action to the agent's runner, and a launch goes where Home sends launches.

## Proposals

A proposal is an inert card: a launch, a message to a session, a scheduled launch, a cron, a status card, a preset, a tool, a bot, a routine, a new `BOT.md`, a ready click or a report. Cards wait in [Today](today.md) → Approve, on the bot's **Proposals** tab and under the session that wrote them. **Start** (or Send, Schedule, Add routine, Apply to BOT.md) acts. The card's **⋯** holds **Dismiss**, which drops it, and **Edit**, which opens a launch in the composer and marks the card “Edited in the composer”, so it no longer waits. **Start all** and **Dismiss all** act on every waiting card of every kind, after one confirmation that lists them. Paid usage is always off for proposals.

A **ready click** prepares an action that stays yours: merge a branch, publish or merge a Local PR, post a suggestion, send an Outbox message, approve the waiting GitHub Actions runs of a pull request from a fork (**Approve and run workflows**), mark Inbox threads done, update Home's Space (`home.update`). The card shows what it acts on and the button of that action, with its page link and **Dismiss** in **⋯**; the click runs the same call as the button on its page, with the same confirmation. Accepting the card never runs it, and Start all and auto mode skip it.

A **report** tells you what the bot did about one item, usually a pull request: `launchpad propose report --title "#2716 cro-bpe: close it?" --spec-file r.json` with `{"outcome": "needs_you", "github": "owner/repo#2716", "body": "<Markdown>"}`. Outcomes: `needs_you` (waits in Today → Answer), `merged`, `approved`, `commented`, `waiting`, `closed`, `info` (Today → Review). It starts nothing: **Mark read** moves it to Done, and **Reply** sends your text to the bot's thread with the card's title, item and ID, so the bot acts on that item. A new report about the same item from the same bot replaces the waiting one. Every agent can write reports, not only bots; a reply to an agent's report goes to that session.

`[tools]` in `bot.toml` sets the bot's [default tools](home-and-runners.md#default-tools) over those of its workspace and folder (Settings tab → **Default tools**).

`proposals` in `bot.toml` sets the bot's proposal mode: `manual` (the default: every card waits for your click) or `auto` (new launches, messages and schedules apply at once; crons, routines, presets, bots, tools, `BOT.md` changes and ready clicks still wait).

## Create a bot

Open **Bots** in the sidebar and click **+** (or **New bot** on the Bots page). The form asks for four things:

- **Name**, for example "Scout".
- **Its job**: what it does and how you like it done. This text becomes `BOT.md`, the bot's instructions.
- **Profile**: the profile its sessions use. Without one, the bot uses your first subscription profile.
- **Folder** (optional): **None** (the default), or a folder of the workspace for a bot that always works on one. Without a folder, each session starts in a temporary folder: the bot finds the workspace's folders with `launchpad folders list` and its notes, and adds a fresh worktree of a Git folder with `launchpad work add-project` when a task needs it. To give a bot a folder of its own, create the folder first (Folders → Add folder), then choose it here.

The form shows what the bot can reach: its folder, the knowledge base of its workspace, and the tools granted to the workspace. Bots run in YOLO mode by default; change it in the bot's Settings tab. **Create bot** saves the bot and starts nothing. The bot's page then asks for its first task.

With a Git folder, **Start from: Repo maintainer** fills in a job for that folder; **Ask Launchpad** fills in its default instructions.

You can also ask Ask Launchpad: "Create a bot that watches the Xet changelog." It writes a **New bot** card with the name, profile, folder and instructions; **Create bot** creates it.

## The thread

A bot has one thread: a chain of sessions. Your messages go to its current session, which is a normal agent session (listed in Agents too, with a small teal chip of the bot's name before its title, also in the session header, where it links to the bot).

A new session starts:

- for the first message,
- when the bot's profile or folder changed (the page says so before you send),
- when the current session was archived,
- when you check **New session** next to the message box before you send (offered while the bot waits for you).

A new session gets a reference to the previous session's trace (`launchpad trace …`), never a copy of it, and the bot's notes in the knowledge base. The harness compacts a long context on its own.

With a Git folder, each session works in a fresh worktree of Home Git. Without a folder, a session's temporary folder is saved on Home at every turn end and follows the session to any runner; Home deletes it 30 days after the session is archived (a new session archives the previous one), so keep what matters in the knowledge base or on a branch. The bot prepares branches, Local PRs and suggestions for your review, and acts directly on GitHub only when you explicitly ask.

## The bot's page

The bot's page is a chat over the whole thread. It opens at the newest message; scroll up (or click **Load earlier messages**) to read older turns and older sessions. Each turn shows:

- your message, and the bot's last answer;
- one steps line ("12 steps · 3 min 10 s") that opens the list of steps, and **Open trace**, which opens the session in Agents with its full trace and files;
- its open questions and approval cards, answered in place;
- the cards it wrote during the turn, at that turn: proposals, drafts and suggestions that wait for your click (**Send**, **Post**, **Review & publish**), **Changed** lines for its knowledge base notes and for what it changed directly (a note path opens the note in Knowledge, a bot, routine, launch, session or folder name opens its page), and a **Changes** card with the branch (**Merge** when it is safe, otherwise **Review changes**);
- a line when the turn failed or was interrupted, with **Continue** on the last turn.

A routine run that ends with no update is one muted line. A divider marks the start of each session, with the reason when the profile or folder changed, and **Open in Agents**. While the bot works, a live line shows the current step and **Stop**.

The message box sends to the current session. Its **+** icon, next to **Attach**, adds context to the next message like an agent's message box (see [Launching an agent](agents.md#launching-an-agent)): other folders as read-only references, GitHub links, the traces of other agents and the presets checked for **Bots**. The chips show above the box; you can send context without text. The context also goes to a new session when the message starts one. When the bot asks a question, typed text answers it and the **+** icon is hidden. During an approval request the box is disabled: choose on the approval card (typed text never grants a permission). When the harness cannot take messages during a turn, the box waits until the turn ends. The page counts as showing the current session: it marks it seen after 1.5 seconds and suppresses its notifications, like Agents.

The header has the bot's state, **Pause** and **Open in Agents**; **ⓘ** next to the name explains sessions, routines and notes. On a laptop, the top bar shows **Bots / bot** to go back to the list. On a laptop, a side panel next to the chat has four tabs; drag the line between the chat and the panel to resize it (double-click resets it). This browser remembers the width. On a phone, the header's **⋯** menu holds the header buttons and the same four panels; a panel replaces the chat, and Back returns to it. The panels:

- **Memory**: the bot's notes in the knowledge base (`bots/<slug>/`): short and current, with your preferences, the facts it needs next time and what you asked it to remember, never a journal of its runs; the recent changes made by its sessions, and every session of its thread with its status and context use. Correct a note on the Knowledge page; every change can be restored for 30 days.
- **Proposals**: the cards its sessions wrote, waiting or handled, with **Start all** and **Dismiss all**.
- **Routines**: what the bot does on its own (see below).
- **Settings**: `BOT.md` (its instructions), shown short: **Expand** (or a click on it) opens a large editor with a live preview (on the phone, a full-screen sheet with **Read** and **Edit**), and an unsaved draft stays until you save or discard it; how it runs (name, profile, model, folder, permissions, proposal mode, Launchpad CLI access and Inbox); what it can reach; `bot.toml`, the same settings as a file; **Versions**, every change of both files in the last 30 days with its diff and **Restore previous**; and **Delete bot**. Deleting keeps the bot's sessions, its notes and its folder. A bot cannot edit its files unless its Launchpad CLI access is admin: when its instructions should change, it proposes a new `BOT.md`, shown as a diff at the top of this tab (and in Today) until you click **Apply to BOT.md**; applying fails if `BOT.md` changed in between.

**Pause** (in the header, once the bot has routines) stops every routine and incoming hook of the bot from starting runs; your own messages still go through, and **Stop** in its session ends a run that is working. **Resume** starts them again from their next trigger.

The sidebar's **Bots** group lists the bots of the workspace, Ask Launchpad among them. A dot shows when a bot's current session needs you; a spinner when it works. Counts stay in Today, so a decision is never counted twice. The chevron collapses the group on this device; collapsed, it shows one state dot.

## Files

Home stores each bot's files with versions, like a plain folder, and includes them in every backup. Home's backups also keep readable copies in the bucket: `<workspace>/bots/<slug>/BOT.md` and `bot.toml`, next to `<workspace>/kb/` for the knowledge base. Home never reads these copies back.

## Routines

A routine sends the bot a task by itself. Open the bot's **Routines** tab and click **New routine** (or start from **Daily check**, **Review requests**, **CI failures**, **After each agent** or **Morning brief**). A routine has a name, what the bot does at each run, and when it runs:

- **On a schedule**: a 5-field cron schedule in your browser's time zone (kept with the routine and shown next to it), for example weekdays at 9:00. The routine shows it as a sentence (“Weekdays at 09:00”); hover it to see the cron. A proposed routine takes the zone of the browser where you click **Add routine**; without a browser, `[home] time_zone` in config.toml, else the server's.
- **GitHub activity**: new activity in your Inbox (GitHub notifications) for the bot's repository or any repository of the workspace: review requested, mentioned, assigned, CI activity, new release, or any activity. Home is a private Space that GitHub cannot call, so Launchpad reads the Inbox about every minute; all new matching activity of one check starts one run, listed in the run's message.
- **When an agent is done**: another agent's turn ends Done in a folder (or any folder of the workspace). The run gets a reference to that agent's trace. Runs of routines never start other routines.
- **When Launchpad fails**: an agent of the workspace fails or is interrupted, a Burst job fails, a routine run cannot start (or is skipped after waiting an hour for a quota reading), a scheduled launch fails, a runner goes offline while agents run on it, automatic work has waited 15 minutes for a quota reading, or Home starts again. Runner, Home, quota and Burst failures reach every workspace. Home checks every 30 seconds; the new failures of one check start one run, whose message lists each failure with its ids and reason. A routine never runs for its own runs or for its bot's own agents. While the bot works, the failures wait (up to one hour). With admin access, the bot then investigates them with the commands of [Diagnose Launchpad](#diagnose-launchpad).

Optional conditions: at least a share of the profile's 5-hour or weekly quota left (with a reading from the last 15 minutes).

Each run is a message to the bot's thread, shown in the chat as one line (**Routine “Daily check” · Scheduled**). The bot prepares work for your review unless the instructions ask it to publish, then ends with a short report (a normal Done that notifies you) or with `no_update`, shown as one muted line (**Daily check · Scheduled · no update**): no notification, and the thread does not need you unless an earlier result is still unread. A run while the bot is working is skipped, with the reason. Messages from incoming hooks and Telegram never join a running turn: they wait, in order, until the bot's turn ends.

Every routine shows its last runs as small squares and its history in text: **Report**, **No update**, **Skipped** (why), **Failed** or **Interrupted**, the last successful run and the next one. **Run now** starts a run at once. **Pause** (in the routine's **⋯**, with **Delete**) stops one routine; the bot's **Pause** stops all of them.

The bot can propose a routine for itself, for example after you say "check this every morning": the card waits in its Routines tab (and in Today) until you click **Add routine**. A session can propose routines and `BOT.md` changes for any bot of its own workspace.

All routines, of bots (the morning brief of Ask Launchpad included) and launches repeated on a schedule, are also on the **Schedules** page. **Add the morning brief** on that page gives Ask Launchpad (or the workspace's first bot) a weekday 8:30 routine.

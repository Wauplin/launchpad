# UI glossary

One word per concept, and one concept per word, in every button, tooltip, toast, phone header and empty state. Code identifiers (`project`, `predraft`, `suggestion`, `code`) stay as they are.

| Use | Instead of | Meaning |
| --- | --- | --- |
| Folder | project, Add project, projects root folder | A Git folder or plain folder stored by Home, added with Clone from GitHub, New plain folder or Keep as folder. "Project" stays in the code. |
| Temporary folder | scratch folder, New scratch folder | An empty folder Launchpad created for an agent in `<data_dir>/tmp/`; saved on Home at every turn end; removed when its last agent is archived and deleted from Home 30 days later (Resume brings it back until then), unless you click **Keep as folder** (it becomes a folder stored by Home, under the name you give it). The `tmp-MMDD-` prefix stays hidden. |
| Worktree / Checkout | task worktree, Agent branches, Open worktree, Project checkout | Worktree: a task's own branch and folder. Checkout: the folder's main working copy. |
| Local PR | pre-draft PR, pre-draft, Local pre-draft | A PR prepared in Launchpad on a branch of Home Git, published or merged with a click. |
| Plain folder | non-Git folder | A folder without Git whose files Home stores, with versions and trash. |
| Asset folder | assets | Large or generated files of a Git folder, stored by Home instead of Git. |
| Bot | assistant, teammate, persona | A named agent that stays: one thread of sessions, `BOT.md` instructions, routine files in `routines/`, notes in its own memory. Ask Launchpad is the bot every workspace starts with. Its page is a chat, with **About** (BOT.md · Triggers · Memory · Access · Versions) and **⋯** (Pause all triggers, Start a new session, Open current session in Agents, Edit as file, Delete). |
| Routine | cron, cron agent, repeating agent | Work that starts on its own again and again: on a schedule, on GitHub activity or when another agent is done. A bot's routine is a file of the bot (`routines/<name>.toml`). "Cron" stays in the code (`crons.rs`, `/api/launch/crons`) and names only the schedule format. |
| Schedules | Cron group, Scheduled group | The one page with Routines · Scheduled launches. |
| Knowledge | memory, notes, wiki | The knowledge base of a workspace: notes and media that are the memory agents share (bots also keep their own). |
| Home Git | Git server, mirror | Home's copy of every Git folder, with the Launchpad branches. Runners clone from it and push to it. |
| Suggestion | agent draft, Agent draft banner, suggested comment | A comment or review written by an agent, posted with a click. |
| Draft | (5 meanings) | Only an unsent composer launch. GitHub draft PRs say "GitHub draft"; unsent replies are "unsent reply". |
| Changes | Files changed, Diff | Every diff view (session, Local PR, PR, commit). |
| Files | Code, code viewer, Open worktree, View file | The read-only file browser. "Browse files" in menus; the commit toggle is Diff · Files. |
| Shell / TUI | Terminal, Open in terminal, Show TUI | A shell in the dock / the harness's own terminal UI, on a runner, opened from Home's window. "Continue in Claude Code TUI" (harness name from metadata) replaces "Open in terminal"; "Show TUI tab" shows it. The dock and its status-bar chip are "Terminals". |
| Archive · Discard · Remove · Delete | Done, Archive, Discard, Remove, Delete folder, Close | Archive: hide but keep (agents, inbox threads; the Inbox Done button keeps its GitHub name "Done"). Discard: drop a local draft, suggestion or Local PR. Remove: take something out of a list, data kept ("Remove from Launchpad" keeps the files, "Remove runner" unpairs); a folder changes workspace with "Move to <workspace>". Delete: erase data (notes go to the trash, credentials, tools, MCP servers, channels, "Delete Telegram setup"); "Delete files" when files are deleted from disk, such as a temporary folder. |
| Launch / Launch here / Send later… | New agent, New agent here, Follow up, Schedule follow-up | Start an agent ("Launch an agent" is the composer title; "Start" only for proposals) / a separate agent in the same folder / a message delivered later to the same session. |
| Pull requests · Channels · Disk cleanup · Search in folder | PRs, Telegram and channels, Maintenance, Search in project | The folder page tab, the Settings page for Telegram, webhooks and hooks, the Settings page that frees disk space, and the Files search field. |
| ⋯ | More ▾, More actions | The menu of the other actions of a row, card or header. |
| Open agent | Agent, Agent session, View session, Back to session | Every link to a session ("← Back to agent"). |
| Workspace | Everything, All workspaces, project group | A group of folders with their agents, bots, schedules and knowledge base. A window shows one workspace; the rail shows the others as counts. There is no view of all workspaces. |
| Today | Now, Needs you page, Proposals page, Outbox page, Review page, dashboard | The first page (`/`) and phone tab: every decision that waits for you in the workspace, one row each, with filters Answer · Review · Approve · Send · GitHub · Fix and Done. |
| Decision list | attention list, needs-you list | What Today shows, computed by Home. Every attention count (sidebar, rail, phone, window title, app badge) is its length. "Needs you" stays the rule for agents: an open question, or finished, failed or interrupted and unseen. |
| Outbox | outgoing queue | Messages to other people that wait for your Send. A concept and code name, not a page: drafts show in Today → Send. |
| Tour | walkthrough, guide, onboarding (for the steps) | The guided steps on demo data (Essentials or Full, chapters). Settings → Onboarding holds the tours and the setup. |
| Demo | sandbox, playground | The fake world the tour runs on. Nothing in it is real or saved; Exit demo reloads the real app. |
| Home | server, main daemon, Home daemon | The one daemon that keeps every record, credential, folder and branch and serves the app. It runs no agents. |
| Runner | worker, host, agent machine | A Linux machine (or Local VM, or Burst job) that runs agents for Home. Its own window shows only its status. "This computer" is not used. |
| Saving · Not on Home | unsynced, pending, dirty | The marks of a session whose last turn is not on Home yet: Saving while the runner sends it, Not on Home (amber) when that failed and it retries. |

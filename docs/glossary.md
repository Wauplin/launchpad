# UI glossary

One word per concept, and one concept per word, in every button, tooltip, toast, phone header and empty state. Code identifiers (`project`, `predraft`, `suggestion`, `code`) stay as they are.

| Use | Instead of | Meaning |
| --- | --- | --- |
| Folder | project, Add project, projects root folder | A directory registered in Launchpad (local or on a host). "Project" stays in the code. The projects root is the "Folders root". |
| Temporary folder | scratch folder, New scratch folder | An empty folder Launchpad created for an agent in `<data_dir>/tmp/`; deleted when its last agent is archived, unless you click **Keep as folder** (move and rename into the folders root). The `tmp-MMDD-` prefix stays hidden. |
| Worktree / Checkout | task worktree, Agent branches, Open worktree, Project checkout | Worktree: a task's own branch and folder. Checkout: the folder's main working copy. |
| Local PR | pre-draft PR, pre-draft, Local pre-draft | A PR prepared in Launchpad on a branch of Home Git, published or merged with a click. |
| Plain folder | non-Git folder | A folder without Git whose files Home stores, with versions and trash. |
| Asset folder | assets | Large or generated files of a Git folder, stored by Home instead of Git. |
| Bot | assistant, teammate, persona | A named agent that stays: one thread of sessions, `BOT.md` instructions, notes in the knowledge base. Ask Launchpad is the bot every workspace starts with. Its page has Memory · Proposals · Routines · Instructions (`BOT.md`, `bot.toml`) · Sessions · Settings. |
| Routine | cron, cron agent, repeating agent | Work that starts on its own again and again: on a schedule, on GitHub activity or when another agent is done. "Cron" stays in the code (`crons.rs`, `/api/launch/crons`) and names only the schedule format. |
| Schedules | Cron group, Scheduled group | The one page with Routines · Scheduled launches. |
| Knowledge | memory, notes, wiki | The knowledge base of a workspace: notes and media that are the only memory agents have. |
| Home Git | Git server, mirror | Home's copy of every Git folder, with the Launchpad branches. Runners clone from it and push to it. |
| Suggestion | agent draft, Agent draft banner, suggested comment | A comment or review written by an agent, posted with a click. |
| Draft | (5 meanings) | Only an unsent composer launch. GitHub draft PRs say "GitHub draft"; unsent replies are "unsent reply". |
| Changes | Files changed, Diff | Every diff view (session, Local PR, PR, commit). |
| Files | Code, code viewer, Open worktree, View file | The read-only file browser. "Browse files" in menus; the commit toggle is Diff · Files. |
| Shell / TUI | Terminal, Open in terminal, Show TUI | A shell in the dock / the harness's own terminal UI. "Continue in Claude Code TUI" (harness name from metadata) replaces "Open in terminal"; "Show TUI tab" shows it. The dock and its status-bar chip are "Terminals". |
| Archive · Discard · Remove · Delete files | Done, Archive, Discard, Remove, Delete folder, Close | Archive: hide but keep (agents, inbox threads; the Inbox Done button keeps its GitHub name "Done"). Discard: drop a local draft, suggestion or Local PR. Remove: unregister, files kept ("Remove from Launchpad"); a folder changes workspace with "Move to <workspace>". "Delete files" only when files are deleted from disk. |
| New agent here / Send later… | Follow up / Schedule follow-up | A separate agent in the same folder / a message delivered later to the same session. |
| Open agent | Agent, Agent session, View session, Back to session | Every link to a session ("← Back to agent"). |
| Workspace | Everything, All workspaces, project group | A group of folders with their agents, bots, schedules and knowledge base. A window shows one workspace; the rail shows the others as counts. There is no view of all workspaces. |
| Today | Now, Needs you page, Proposals page, Outbox page, Review page, dashboard | The first page (`/`) and phone tab: every decision that waits for you in the workspace, one card each, with filters Answer · Review · Approve · Send · GitHub · Fix and Done. |
| Decision list | attention list, needs-you list | What Today shows, computed by Home. Every attention count (sidebar, rail, phone, window title, app badge) is its length. "Needs you" stays the rule for agents: an open question, or finished, failed or interrupted and unseen. |
| Outbox | outgoing queue | Messages to other people that wait for your Send. A concept and code name, not a page: drafts show in Today → Send. |
| Tour | walkthrough, guide, onboarding (for the steps) | The guided steps on demo data (Essentials or Full, chapters). Settings → Onboarding holds the tours and the setup checklist. |
| Demo | sandbox, playground | The fake world the tour runs on. Nothing in it is real or saved; Exit demo reloads the real app. |

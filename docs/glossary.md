# UI glossary

One word per concept, and one concept per word, in every button, tooltip, toast, phone header and empty state. Code identifiers (`project`, `predraft`, `suggestion`, `code`) stay as they are.

| Use | Instead of | Meaning |
| --- | --- | --- |
| Folder | project, Add project, projects root folder | A directory registered in Launchpad (local or on a host). "Project" stays in the code. The projects root is the "Folders root". |
| Scratch folder | temporary folder, New temporary folder | A folder Launchpad created for a one-off agent; Keep or Delete files. The `tmp-MMDD-` prefix stays hidden. |
| Worktree / Checkout | task worktree, Agent branches, Open worktree, Project checkout | Worktree: a task's own branch and folder. Checkout: the folder's main working copy. |
| Local PR | pre-draft PR, pre-draft, Local pre-draft | A PR prepared locally, published or merged locally with a click. |
| Suggestion | agent draft, Agent draft banner, suggested comment | A comment or review written by an agent, posted with a click. |
| Draft | (5 meanings) | Only an unsent composer launch. GitHub draft PRs say "GitHub draft"; unsent replies are "unsent reply". |
| Changes | Files changed, Diff | Every diff view (session, Local PR, PR, commit). |
| Files | Code, code viewer, Open worktree, View file | The read-only file browser. "Browse files" in menus; the commit toggle is Diff · Files. |
| Shell / TUI | Terminal, Open in terminal, Show TUI | A shell in the dock / the harness's own terminal UI. "Continue in Claude Code TUI" (harness name from metadata) replaces "Open in terminal"; "Show TUI tab" shows it. The dock and its status-bar chip are "Terminals". |
| Archive · Discard · Remove · Delete files | Done, Archive, Discard, Remove, Delete folder, Close | Archive: hide but keep (agents, inbox threads; the Inbox Done button keeps its GitHub name "Done"). Discard: drop a local draft, suggestion or Local PR. Remove: unregister, files kept ("Remove from workspace", "Remove from Launchpad"). "Delete files" only when files are deleted from disk. |
| New agent here / Send later… | Follow up / Schedule follow-up | A separate agent in the same folder / a message delivered later to the same session. |
| Open agent | Agent, Agent session, View session, Back to session | Every link to a session ("← Back to agent"). |
| Everything | All workspaces | The workspace choice that applies no filter. |
| Review | To publish, /review | The one page with To publish · Agent branches · Review requests. |
| Tour | walkthrough, guide, onboarding (for the steps) | The guided steps on demo data (Essentials or Full, chapters). Settings → Onboarding holds the tours and the setup checklist. |
| Demo | sandbox, playground | The fake world the tour runs on. Nothing in it is real or saved; Exit demo reloads the real app. |

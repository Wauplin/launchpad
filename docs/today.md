# Today

**Today** is the first page (`/`) on the laptop and the first tab on the phone. It lists every decision that waits for you in the window's workspace, one compact row per decision, so a long list (30 review requests) fits on a few screens. Each row has its buttons, so you decide without leaving the page. Nothing reaches GitHub or other people without your click and its confirmation.

## Rows

Rows are grouped by filter, with a heading per group under **All**, then newest first. Each filter is a chip with its count and its own address (for example `/?filter=send`).

The icon at the start of a row says what it is; hover it for its name. GitHub items use GitHub's icons (a green pull request icon for a review request). Each row has one button, its main action, with a short label (**Merge**, **Fix merge**, **Review**, **Resume**, **Answer**, **Start**, **Open**); a finished or failed agent also shows **Archive** next to it. The other actions are in **⋯** at the end of the row. A row with something to read (a question, an agent's last message and branch, a proposal, a message, a suggestion) opens its full card below the line when you click its title. An agent's last message shows its beginning; when it is cut, the arrow after **…** shows the full message, and the arrow at its end folds it back. Rows that need reading first have **Answer** or **Review** as their main button, which opens the card.

Each agent has one row. When an agent that needs you has a Local PR, the Local PR shows as a line under the agent's row ("Local PR · branch → base"), and the row counts once; click the line to open the Local PR. A Local PR whose agent does not need you (seen, running or gone) has its own row.

On the phone, the page starts with the filter chips (**All** shows the count) and **Start all**. Each row shows its title on up to two lines and only its main button, with a one-word label (**Merge**, **Fix**, **Review**, **Open**, **Start**, **Add**, **Create**, **Send**), and no **⋯**. Tap the row to open it: it shows the card or the agent's last message, then the other actions as a list (**Open agent**, **Archive**, **Mark seen**, **Done**, **Dismiss**).

A **report** is a card a bot or an agent writes about one thing it handled, usually a GitHub pull request or issue: what it did (merged, approved, commented, waiting, closed) or a question for you. Each report is its own row, so a bot that handles ten pull requests in one run gives ten rows, each with the pull request's link, the text and **Open agent**. A new report about the same pull request from the same bot (or agent) replaces the one that still waits.

A link to one row is `/?card=<id>`: it highlights the row and opens its card. Telegram notifications use it (`/?card=session:<id>`). When the item no longer waits, the link opens the session.

| Filter | Rows | Buttons |
| --- | --- | --- |
| **Answer** | An agent asks a question; a report that needs you. | **Answer** opens the answer choices in place; **⋯ → Open agent** opens the agent to type another answer. A report: **Reply** opens it, with a field whose text goes to the bot or agent that wrote it (the card then counts as answered); **Mark read** in **⋯**. |
| **Review** | A finished agent you have not seen; a failed or interrupted agent; a Local PR not yet published; a report of what an agent did. | Report: **Mark read** moves it to Done; a click on the title opens its text. Finished with a merge that stopped (a conflict or a failure): the row says **Not merged**, its card gives the reason, and **Fix merge** sends the problem back to the agent. Finished with a Local PR: **Review** opens the Local PR. Finished without one: **Merge** when it is safe (its tooltip names the branch and the base), otherwise **Review** when there are changes not yet merged or in a pull request (it opens the agent's Changes), otherwise **Open agent** (it opens at the agent's last message). Failed: **Resume**. Both: **Archive** next to the main button (in the list of the opened row on the phone), and **⋯** with **Open agent** (when it is not the main button) and **Mark seen**. Local PR on its own row: **Review** opens its page, to publish or merge it. |
| **Approve** | A proposal card from a bot or an agent. | **Start** (or Send, Schedule, Create routine, Apply) on the row, **Dismiss** in **⋯**; the card's **⋯** adds **Edit**. **Start all** starts every waiting proposal after one confirmation. A ready card opens with **Review** and has the button of its action (for example **Merge** or **Post**), with **Dismiss** in **⋯**; Start all skips it. |
| **Send** | A message an agent drafted for an outgoing channel. | **Review** opens it: edit it, then **Send** (with a confirmation) or **Discard**. An uncertain send offers **It arrived** or **Discard**. |
| **GitHub** | A suggestion (a comment or review written by an agent) and an open review request. | Suggestion: **Review** opens it (**Edit**, **Post**, **Discard**); **⋯** has **Open pull request** and **Open agent**. Review request: **Review** opens a launch draft for an agent that reviews it (the review comes back as a suggestion); **⋯** has **Open pull request** and **Done**, which marks the notification done here and on GitHub and removes the row. |
| **Fix** | A routine whose last run failed, a runner that runs another build, and a newer Launchpad for Home's Space (or a Home update whose build failed). | **Open** opens the routine or Home and runners; a Home update card has **Update Home** with **Then update the runners**. |

**Merge** shows only when the merge is safe: the agent is Done and its turn is saved on Home (no **Saving** or **Not on Home** mark), its runner is connected (or its folder was removed and Home merges from Home Git), the branch is on Home with new commits, nothing is uncommitted and the base branch did not move. Merge sends the commit the row showed; Home refuses it when the branch moved since. The row stays until the merge is complete; a merge that waits keeps the row with **Review**, and a merge that stopped keeps it with **Fix merge**.

**Done** lists what you handled in the last 30 days and what happened (for example "Sent to team-chat"), with a link to the agent.

## Counts

Every attention count comes from this list: the **Today** count in the sidebar and on the phone tab, the workspace squares of the rail, the phone's workspace sheet, the window title and the app badge. Counts that need you show as amber pills, the others as plain grey numbers. **Agents** shows only how many agents run (grey) or wait for quota (amber). The **Inbox** count (unread review requests, mentions and assignments) stays on Inbox.

## Workspaces

A row belongs to the workspace of its folder, its agent or its repository. Rows about a repository without a folder, or an agent outside every workspace, are not listed. A few things belong to no workspace and show in every one: a runner to update, a tool or preset proposal without a workspace, and an Outbox message without an agent.

## Where the history is

- Proposals: Today → Done, and the bot's chat at the turn that wrote them.
- Sent or discarded messages and posted suggestions: Today → Done.
- Agent branches: the **Branches** tab of the folder page.
- Review requests: Today → GitHub, and the [Inbox](github.md#inbox).

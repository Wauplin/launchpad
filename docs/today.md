# Today

**Today** is the first page (`/`) on the laptop and the first tab on the phone. It lists every decision that waits for you in the window's workspace, one compact row per decision, so a long list (30 review requests) fits on a few screens. Each row has its buttons, so you decide without leaving the page. Nothing reaches GitHub or other people without your click and its confirmation.

## Rows

Rows are grouped by filter, with a heading per group under **All**, then newest first. Each filter is a chip with its count and its own address (for example `/?filter=send`).

The icon at the start of a row says what it is; hover it for its name. GitHub items use GitHub's icons (a green pull request icon for a review request). A row with something to read (a question, an agent's last message and branch, a proposal, a message, a suggestion) opens its full card below the line: click its title or the arrow at the end. Rows that need reading first have **Answer**, **Review** or **Review & post** as their main button, which opens the card.

On the phone, each row is one line with only its main button, with a shorter label (**Review**, **Merge**). Tap the title to open the row: it shows the card or the agent's last message, then the other buttons (**Open**, **Archive**, **Mark seen**, **Done**, **Dismiss**).

A link to one row is `/?card=<id>`: it highlights the row and opens its card. Telegram notifications use it (`/?card=session:<id>`). When the item no longer waits, the link opens the session.

| Filter | Rows | Buttons |
| --- | --- | --- |
| **Answer** | An agent asks a question. | **Answer** opens the answer choices in place; **Open** opens the agent to type another answer. |
| **Review** | A finished agent you have not seen; a failed or interrupted agent; a Local PR not yet published. | Finished: **Merge into &lt;base&gt;** when it is safe, otherwise **Review changes** when there are changes, otherwise **Archive**. Failed: **Resume**. Both: **Open**, and on hover **Archive** and **Mark seen**. Local PR: **Review & publish**. |
| **Approve** | A proposal card from a bot or an agent. | **Start** (or Send, Schedule, Add routine, Apply to BOT.md) on the row, **Dismiss** on hover; the card adds **Edit**. **Start all** starts every waiting proposal after one confirmation. A ready card opens with **Review** and has the button of its action (for example **Merge** or **Post**) and **Dismiss**; Start all skips it. |
| **Send** | A message an agent drafted for an outgoing channel. | **Review** opens it: edit it, then **Send** (with a confirmation) or **Discard**. An uncertain send offers **It arrived** or **Discard**. |
| **GitHub** | A suggestion (a comment or review written by an agent) and an open review request. | Suggestion: **Review & post** opens it (**Edit**, **Post**, **Discard**), **Open PR**. Review request: **Review with agent** (the agent's review comes back as a suggestion), **Open**, and on hover **Done**, which marks the notification done here and on GitHub and removes the row. |
| **Fix** | A routine whose last run failed, and a runner that runs another build. | **Open routine**, **Open runners**. |

**Merge into &lt;base&gt;** shows only when the merge is safe: the agent is Done, its runner is connected, the branch is on Home with new commits, nothing is uncommitted and the base branch did not move. The row stays until the merge is complete; a merge that waits or fails keeps the row with **Review changes**.

**Done** lists what you handled in the last 30 days and what happened (for example "Sent to team-chat"), with a link to the agent.

## Counts

Every attention count comes from this list: the **Today** count in the sidebar and on the phone tab, the workspace squares of the rail, the phone's workspace sheet, the window title and the app badge. **Agents** shows only how many agents run or wait for quota. The **Inbox** count (unread review requests, mentions and assignments) stays on Inbox.

## Workspaces

A row belongs to the workspace of its folder, its agent or its repository. Rows about a repository without a folder, or an agent outside every workspace, are not listed. A few things belong to no workspace and show in every one: a runner to update, a tool or preset proposal without a workspace, and an Outbox message without an agent.

## Where the history is

- Proposals: Today → Done, and the bot's **Proposals** tab.
- Sent or discarded messages and posted suggestions: Today → Done.
- Agent branches: the **Branches** tab of the folder page.
- Review requests: Today → GitHub, and the [Inbox](github.md#inbox).

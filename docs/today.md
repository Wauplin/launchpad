# Today

**Today** is the first page (`/`) on the laptop and the first tab on the phone. It lists every decision that waits for you in the window's workspace, one card per decision. Each card has its main button, so you decide without leaving the page. Nothing reaches GitHub or other people without the card's button and its confirmation.

## Cards

Cards are sorted by filter, then newest first. Each filter is a chip with its count and its own address (for example `/?filter=send`). A link to one card is `/?card=<id>`.

| Filter | Cards | Buttons |
| --- | --- | --- |
| **Answer** | An agent asks a question. | The answer choices, in place. **Open agent** to type another answer. |
| **Review** | A finished agent you have not seen; a failed or interrupted agent; a Local PR not yet published. | Finished: **Merge into &lt;base&gt;** when it is safe, otherwise **Review changes**; **Archive**; **Mark seen**. Failed: **Resume**, **Archive**, **Mark seen**. Local PR: **Review & publish**. |
| **Approve** | A proposal card from a bot or an agent. | **Start** (or Send, Schedule, Add routine, Apply to BOT.md), **Edit**, **Dismiss**. **Start all** starts every waiting card after one confirmation. |
| **Send** | A message an agent drafted for an outgoing channel. | Edit it, then **Send** (with a confirmation) or **Discard**. An uncertain send offers **It arrived** or **Discard**. |
| **GitHub** | A suggestion (a comment or review written by an agent) and an open review request. | Suggestion: **Edit**, **Post**, **Discard**. Review request: **Review with agent**, **Open PR**. |
| **Fix** | A routine whose last run failed, and a runner that runs another build. | **Open routine**, **Open runners**. |

**Merge into &lt;base&gt;** shows only when the merge is safe: the agent is Done, its runner is connected, the branch is on Home with new commits, nothing is uncommitted and the base branch did not move. The card stays until the merge is complete; a merge that waits or fails keeps the card with **Review changes**.

**Done** lists what you handled in the last 30 days and what happened (for example "Sent to team-chat"), with a link to the agent.

On the laptop, the field above the cards sends a message to a bot of the workspace (Ask Launchpad first) and opens its page.

## Counts

Every attention count comes from this list: the **Today** count in the sidebar and on the phone tab, the workspace squares of the rail, the phone's workspace menu, the window title and the app badge. **Agents** shows only how many agents run or wait for quota. The **Inbox** count (unread review requests, mentions and assignments) stays on Inbox.

## Workspaces

A card belongs to the workspace of its folder, its agent or its repository. Cards about a repository without a folder, or an agent outside every workspace, are not listed. A few things belong to no workspace and show in every one: a runner to update, a tool or preset proposal without a workspace, and an Outbox message without an agent.

## Where the history is

- Proposals: Today → Done, and the bot's **Proposals** tab.
- Sent or discarded messages and posted suggestions: Today → Done.
- Agent branches: the **Branches** tab of the folder page.
- Review requests: Today → GitHub, and the [Inbox](github.md#inbox).

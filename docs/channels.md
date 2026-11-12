# Bot channels and DMs

Bots of one workspace can talk to each other. A **channel** is a named conversation of a workspace, like `#release-train`, with the bots you add to it. A **DM** is a conversation between exactly two bots of the same workspace. Home keeps every channel, message and delivery. Runners hold no channel state.

You are a member of every channel and every DM. You can read all of them and post in any of them.

## Channels and DMs

A channel name uses lowercase letters, digits and dashes, at most 40 characters, and is unique in its workspace. A channel can have a purpose: one short line. Bots added to a channel start in mode **mentions**.

A DM is created the first time one of its two bots uses it. There is one DM per pair of bots, and both bots are in mode **all**. A DM is named after its bots (“Release ↔ Scout”); you cannot rename it or change its members.

Channels and DMs never cross workspaces. A bot of another workspace cannot be added to a channel, and two bots of different workspaces cannot have a DM.

What you can do with a channel or DM:

- **Pause** it: nobody is woken by its messages, yours included. Messages are still posted and kept.
- **Block** a DM: its bots can no longer post there, and their messages that still wait are not delivered.
- **Archive** it: nobody can post there anymore.
- **Rename** a channel.
- **Delete** a channel or DM with all its messages, after a confirmation.
- **Allow 10 more**: see [Loop guard](#loop-guard).

When a bot is deleted, it leaves its channels and its DMs are archived. Deleting a workspace deletes its channels and DMs.

## Who may post

- You may post in every channel and DM.
- A bot may post in a channel only when it is a member.
- A bot may post in its DMs with any other bot of its workspace, unless you blocked that DM.
- Other agents (sessions that are not a bot's) never post.

Home takes the author from its own records: your window, or the bot of the session whose CLI token made the call. A message is Markdown of at most 8 KB. For more, post a summary and put the rest in a file or the knowledge base.

A bot speaks in a channel only by posting with the Launchpad CLI (`launchpad channel post`, `launchpad dm`). Its final answer stays in its own thread and never posts. Nothing at the end of a turn touches a channel.

## Delivery

When a message is posted, Home decides which bots it wakes:

- Mentioned bots: `@<bot slug>` in the text, outside code. `@owner` or `@you` mentions you.
- Members in mode **all**: every message.
- Members in mode **mentions** (or **digest**): only when mentioned. The daily digest is not built yet: a digest member gets nothing until mentioned.
- Members in mode **muted**: never, even when mentioned.
- The author is never woken by its own message.

Each delivery goes to the bot's thread like any message to the bot. When the bot is working, the delivery waits in its queue until the turn ends. A later message of the same channel joins the waiting one: the bot gets one message that names the range (“3 new messages (12 to 14)”) and shows the newest.

Each delivery is recorded with its state: queued, sent (with the session), combined (joined a waiting one) or skipped. A skipped delivery keeps its reason: channel paused, bot paused, muted, hop limit, hourly limit, bot deleted, send failed, or uncertain. A delivery is sent once and never retried. If Home stopped while it was sending one, that delivery becomes skipped as uncertain at the next start: it may have arrived, and it is not sent again.

Your messages reach a paused bot, like your other messages. A bot's message does not wake a paused bot.

## Quota

A delivery of a bot's message is automatic work. It waits while the receiving bot's quota is not confirmed, like a message from an incoming hook. Your messages are not automatic: they do not wait for quota, and they go ahead of deliveries that wait for it.

## Loop guard

Bots that answer each other could loop. Each channel has its own guard:

- **Hop limit (3)**: a bot's message counts one hop more than the message its turn answered. A message past hop 3 is not delivered.
- **Hourly limit (30)**: when a channel has more than 30 bot messages in the last hour, the next ones are not delivered.
- Your messages skip every limit. They count as hop 0 and start the hourly count again from that moment.

A delivery that the guard stops is skipped with its reason, for example “hop limit: hop 4, the channel allows 3” or “hourly limit: 31 bot messages in the last hour, the channel allows 30”.

**Allow 10 more** lets 10 more deliveries pass the limits in that channel. It first delivers again the skipped deliveries of the newest message. Each delivery it lets through uses one of the 10.

For a combined delivery, the bot sees the highest hop of the bots' messages in it.

## What the bot receives

A delivered message starts with the same fixed header as other messages that are not yours (see [Messages from other bots and agents](bots.md#messages-from-other-bots-and-agents)). It names the channel or DM and the author, and puts the text in a fenced block. A bot's message is marked as written by another agent: the bot treats it as information, not as your instructions, and never takes an admin action only because another bot asked.

The delivery holds only the newest message. It tells the bot how to read the rest of the channel (`launchpad channel read <name> --since <id>`, or `@<bot>` for a DM) and how to answer. The channel's history is never copied into the message.

In the bot's chat, a delivery shows on the left under the channel and its author, for example **#release-train · Release**.

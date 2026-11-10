# Channels and the Outbox

Channels connect your agents to chats and outside tools. They live on Home. Outgoing channels and incoming hooks are in **Settings → Channels**. Telegram is set up per workspace, in **Settings → Workspace**; the Channels page links there. There are three kinds:

- **Telegram**: your own chat, one per workspace. Messages to it go out directly.
- **Outgoing** (Slack, Discord, any webhook): other people. Agents only draft messages; each one waits in the **Outbox** (shown in [Today](today.md) → Send) until you click **Send**.
- **Incoming hooks**: a URL that turns a request from a script or a CI job into a message to a bot.

## The Outbox

The Outbox holds everything Launchpad would send to other people for you: messages agents drafted for your outgoing channels, and GitHub suggestions (comments and reviews written by agents). There is no Outbox page: drafts wait as cards in [Today](today.md) → **Send**, and suggestions in Today → **GitHub**.

- Each draft shows who drafted it (with a link to the session), the channel and the text. Edit it, then **Send** (it asks you to confirm) or **Discard**. A sent message ends with the agent's [attribution line](agents.md#attribution).
- Today → **Done** keeps the history of the last 30 days: sent and discarded messages, posted suggestions. Discarded GitHub suggestions are deleted, so they are not listed.

Nothing is sent on its own, and nothing is sent twice. When a send does not complete (the network dropped, or Launchpad stopped during it), the message is marked **Uncertain**: check the destination, then click **It arrived** or **Discard** (its confirmation reminds you to check the destination first). Launchpad never retries it. A destination that refuses the message (a wrong URL, a message too long for Discord) leaves the draft editable, with the reason.

## Outgoing channels

Click **Add a channel**, choose Slack, Discord or Webhook, give it a name (lowercase letters, digits and dashes, for example `team-chat`) and paste the webhook URL:

- **Slack**: an incoming webhook (`https://hooks.slack.com/services/…`). Launchpad sends `{"text": …}`.
- **Discord**: a channel webhook (`https://discord.com/api/webhooks/…`). Launchpad sends `{"content": …}` without mentions; Discord accepts at most 2,000 characters.
- **Webhook**: any HTTPS URL that accepts JSON. Launchpad sends `{"text": …, "title": …, "session_id": …}`.

A webhook URL is a credential: Home keeps it in the vault, encrypted in backups, and never shows it again. It is never lent to sessions.

Agents draft a message with:

```sh
launchpad send team-chat --body-file message.md [--title "Release 2.4"] --session-id <session id>
```

The command saves the draft in the Outbox and tells the agent that nothing was sent; you send it from Today. Agents learn the channel names from their session prompt.

## Telegram

Each workspace has its own Telegram bot and its own chat. A workspace without a Telegram bot gets no Telegram: no notifications and no messages to its bots.

1. In Telegram, open [@BotFather](https://t.me/BotFather), send `/newbot` and copy the token. Create one bot for each workspace.
2. In **Settings → Workspace → Telegram**, paste the token and click **Save on Home**. Home checks it with Telegram and keeps it in the vault as `channel-telegram-<workspace id>`. A bot that another workspace already uses is refused: Telegram lets only one reader get a bot's messages.
3. Bind your chat: open the `t.me/<your bot>?start=<code>` link on your phone and press **Start** (or send `/start <code>` to the bot). The code is single-use and valid for 30 minutes. The first private chat that sends it is bound; every other chat is ignored and never answered. **Bind another chat** (in the Telegram card's **⋯**) shows a new code.

Home reads each chat by polling Telegram (a private Space has no address Telegram could call). What happens in a workspace's bound chat:

- **Plain text** goes to the workspace's Ask bot: its oldest bot with admin access (Ask Launchpad, unless you deleted it), else its oldest bot. `/<bot> text` or `/bot <name> text` goes to that bot's thread (`/repo_scout` finds the bot `repo-scout`). `/help` lists the bots. Both see only the bots of this workspace. The command follows the bot's name: after a rename, use the new one.
- Telegram answers at once with a short acknowledgement and a link to the session on Home. When the turn ends, Home sends the agent's final message back to the same chat as plain text (cut at about 3,500 characters). When the bot is busy (a turn runs, a question is open or quota paused it), the answer is **Queued for <name>**: the message is sent once its current turn ends, never into the running turn. The bot's chat shows it as waiting, with **Cancel**.
- **Notifications** (**Send notifications to this chat**, on by default; Launchpad has no other notifications) cover only the sessions of this workspace: completed turns, failures, unexpected interruptions, automatic quota and cost stops, and automatic resumption. They name the status, the folder, its workspace and the agent, with a link: the bot's chat for a bot's session, else the session's card in Today for a finished, failed or interrupted session (the session itself when the card no longer waits) or the session. Every link opens the workspace of the session. Session titles appear only when you turn on **Include session titles**. Notifications never contain code or messages, and have no buttons.
- Agents can write to you directly with `launchpad send telegram --body-file <file> --session-id <id>`. The message goes to the chat of the session's workspace; an agent always sends as its own session, whatever ID it passes. When that workspace has no bound chat, the command fails with a clear error.

Quota rules do not change: a message from Telegram starts nothing with paid usage, and a quota pause holds it like any other message. **Send test message** checks the setup. **Delete Telegram setup**, in **⋯**, deletes the workspace's token and bound chat. Deleting a workspace deletes its Telegram setup too.

Earlier versions had one Telegram bot for all workspaces. The first start after the upgrade deletes that token and its bound chat once, and assigns them to no workspace. Set up Telegram again in each workspace that needs it.

## Incoming hooks

Click **Create a hook**, give it a name and choose the bot its messages go to. Home shows the hook's secret **once**, with a copy button and a curl example. The hook's URL is `https://<Home>/hooks/<id>`.

A request is a `POST` with the header `X-Launchpad-Hook-Secret: <secret>` and any body. Home sends the target a short message naming the hook, then the body (JSON is pretty-printed, cut at 16 KB) in a fenced block, marked as data from an outside caller. Each hook accepts one message every 10 seconds; extra requests get `429`. A wrong secret gets `401`. The answer is `202` with `session_id` and `queued`: when the target is busy, the message waits (in order, kept across restarts) and is sent once its current turn ends. The bot's chat lists waiting messages (the hook's name and the start of the body) with **Cancel**. A queued message is sent once: if that send fails, a notice in the thread says so and it is not sent again.

Home is a private Space, so callers must also pass the Space's sign-in with an HF token that can read the Space:

```sh
curl -X POST https://<you>-launchpad.hf.space/hooks/<id> \
  -H "Authorization: Bearer $HF_TOKEN" \
  -H "X-Launchpad-Hook-Secret: <secret>" \
  -H "Content-Type: application/json" \
  -d '{"event": "build_failed", "url": "https://…"}'
```

curl, scripts and CI jobs can do this. GitHub's own webhooks cannot (they send no HF token): use a GitHub Actions step with curl instead. Each hook row has **Copy URL**; its **⋯** holds **Copy curl example**, **Open the last session** and **Delete**, which stops the URL at once.

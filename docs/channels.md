# Channels and the Outbox

Channels connect your agents to chats and outside tools. They live on Home, in **Settings → Channels**; a runner's window shows Home's. There are three kinds:

- **Telegram**: your own chat. Messages to it go out directly.
- **Outgoing** (Slack, Discord, any webhook): other people. Agents only draft messages; each one waits in the **Outbox** until you click **Send**.
- **Incoming hooks**: a URL that turns a request from a script or a CI job into a message to a bot.

## The Outbox

**Outbox** in the sidebar lists everything Launchpad would send to other people for you: GitHub suggestions (comments and reviews written by agents) and messages agents drafted for your outgoing channels. The sidebar shows how many wait.

- **To send**: each draft shows who drafted it (with a link to the session), the channel and the text. Edit it, then **Send** (it asks you to confirm) or **Discard**.
- **Sent** and **Discarded** keep the history. Discarded GitHub suggestions are deleted, so they are not listed.

Nothing is sent on its own, and nothing is sent twice. When a send does not complete (the network dropped, or Launchpad stopped during it), the message is marked **Uncertain**: check the destination, then click **It arrived** or **Discard**. Launchpad never retries it. A destination that refuses the message (a wrong URL, a message too long for Discord) leaves the draft editable, with the reason.

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

The command saves the draft and says that you send it from the Outbox. Agents learn the channel names from their session prompt.

## Telegram

1. In Telegram, open [@BotFather](https://t.me/BotFather), send `/newbot` and copy the token.
2. In **Settings → Channels → Telegram**, paste the token and click **Save on Home**. Home checks it with Telegram and keeps it in the vault.
3. Bind your chat: open the `t.me/<your bot>?start=<code>` link on your phone and press **Start** (or send `/start <code>` to the bot). The code is single-use and valid for 30 minutes. The first chat that sends it is bound; every other chat is ignored and never answered. **Bind another chat** shows a new code.

Home reads the chat by polling Telegram (a private Space has no address Telegram could call). What happens in the bound chat:

- **Plain text** goes to Ask Launchpad (Everything). `/<bot> text` or `/bot <name> text` goes to that bot's thread (`/repo_scout` finds the bot `repo-scout`). `/help` lists the bots.
- Telegram answers at once with a short acknowledgement and a link to the session on Home. When the turn ends, Home sends the agent's final message back as plain text (cut at about 3,500 characters).
- **Notifications** (a switch, on by default): the same events as Web Push, that is completed turns, failures, unexpected interruptions, automatic quota and cost stops, and automatic resumption. They name the status, the folder and the agent, with a link; session titles only when you turn on **Include session titles**. They never contain code or messages.
- Agents can write to you directly with `launchpad send telegram --body-file <file> --session-id <id>`.

Quota rules do not change: a message from Telegram starts nothing with paid usage, and a quota pause holds it like any other message. **Send test message** checks the setup. **Remove Telegram** deletes the token and the bound chat.

## Incoming hooks

Click **Create a hook**, give it a name and choose where its messages go: Ask Launchpad or a bot. Home shows the hook's secret **once**, with a copy button and a curl example. The hook's URL is `https://<Home>/hooks/<id>`.

A request is a `POST` with the header `X-Launchpad-Hook-Secret: <secret>` and any body. Home sends the target a short message naming the hook, then the body (JSON is pretty-printed, cut at 16 KB) in a fenced block, marked as data from an outside caller. Each hook accepts one message every 10 seconds; extra requests get `429`. A wrong secret gets `401`.

Home is a private Space, so callers must also pass the Space's sign-in with an HF token that can read the Space:

```sh
curl -X POST https://<you>-launchpad.hf.space/hooks/<id> \
  -H "Authorization: Bearer $HF_TOKEN" \
  -H "X-Launchpad-Hook-Secret: <secret>" \
  -H "Content-Type: application/json" \
  -d '{"event": "build_failed", "url": "https://…"}'
```

curl, scripts and CI jobs can do this. GitHub's own webhooks cannot (they send no HF token): use a GitHub Actions step with curl instead. **Delete** stops the URL at once.

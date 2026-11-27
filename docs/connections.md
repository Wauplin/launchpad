# Connections and the Outbox

Connections link your agents to chats and outside tools. They live on Home. Outgoing webhooks and incoming hooks are in **Settings → Connections**. There are two kinds:

- **Outgoing** (Slack, Discord, any webhook): other people. Agents only draft messages; each one waits in the **Outbox** (shown in [Today](today.md) → Send) until you click **Send**. To let agents read and post in Slack themselves, use [Slack profiles](slack.md) instead.
- **Incoming hooks**: a URL that turns a request from a script or a CI job into a message to a bot.

## The Outbox

The Outbox holds everything Launchpad would send to other people for you: messages agents drafted for your outgoing connections, and GitHub suggestions (comments and reviews written by agents). There is no Outbox page: drafts wait as cards in [Today](today.md) → **Send**, and suggestions in Today → **GitHub**.

- Each draft shows who drafted it (with a link to the session), the connection and the text. Edit it, then **Send** (it asks you to confirm) or **Discard**. A sent message ends with the agent's [attribution line](agents.md#attribution).
- Today → **Done** keeps the history of the last 30 days: sent and discarded messages, posted suggestions. Discarded GitHub suggestions are deleted, so they are not listed.

Nothing is sent on its own, and nothing is sent twice. When a send does not complete (the network dropped, or Launchpad stopped during it), the message is marked **Uncertain**: check the destination, then click **It arrived** or **Discard** (its confirmation reminds you to check the destination first). Launchpad never retries it. A destination that refuses the message (a wrong URL, a message too long for Discord) leaves the draft editable, with the reason.

## Outgoing connections

Click **Add a connection**, choose Slack, Discord or Webhook, give it a name (lowercase letters, digits and dashes, for example `team-chat`) and paste the webhook URL:

- **Slack**: an incoming webhook (`https://hooks.slack.com/services/…`). Launchpad sends `{"text": …}`.
- **Discord**: a channel webhook (`https://discord.com/api/webhooks/…`). Launchpad sends `{"content": …}` without mentions; Discord accepts at most 2,000 characters.
- **Webhook**: any HTTPS URL that accepts JSON. Launchpad sends `{"text": …, "title": …, "session_id": …}`.

A webhook URL is a credential: Home keeps it in the vault, encrypted in backups, and never shows it again. It is never lent to sessions.

Agents draft a message with:

```sh
launchpad send team-chat --body-file message.md [--title "Release 2.4"] --session-id <session id>
```

The command saves the draft in the Outbox and tells the agent that nothing was sent; you send it from Today. Agents learn the connection names from their session prompt.

## Incoming hooks

Click **Create a hook**, give it a name and choose the bot its messages go to. Home shows the hook's secret **once**, with a copy button and a curl example. The hook's URL is `https://<Home>/hooks/<id>`.

A request is a `POST` with the header `X-Launchpad-Hook-Secret: <secret>` and any body. Home sends the target a short message naming the hook, then the body (JSON is pretty-printed, cut at 16 KB) in a fenced block, marked as data from an outside caller. Each hook accepts one message every 10 seconds; extra requests get `429`. A wrong secret gets `401`. Each hook has its own thread of the bot, so its messages continue one conversation next to the bot's others. The answer is `202` with `session_id` and `queued`: when that thread is busy, the message waits (in order, kept across restarts) and is sent once its current turn ends. It also waits while the bot's quota is not confirmed, or while the bot works in as many threads as it may (see [Threads](bots.md#threads)), because a hook message is automatic work. The bot's chat lists waiting messages (the hook's name and the start of the body) with **Cancel**. A queued message is sent once: if that send fails, a notice in the thread says so and it is not sent again.

In the bot's chat, a hook message shows on the left under **From hook <name> · not from you**, never as your own message. The line links to Settings → Connections.

### Loops between bots

A bot can call a hook that messages another bot, which can call a hook again. To stop such loops, each hook message carries a hop count, like the messages agents send each other (see [Bots](bots.md)):

- A request without the `X-Launchpad-Hops` header counts as hop 1. With the header, the count is its value, and at least 1. A value that is not a whole number gets `400`.
- Past 3 hops, the hook answers `409` with "loop guard: N hops" and delivers nothing.
- When the bot answers, the messages it sends to other bots count on from the hook's hop.

Launchpad's agents send the header themselves. `launchpad hops` prints the value for the agent's next call:

```sh
curl -X POST https://<you>-launchpad.hf.space/hooks/<id> \
  -H "X-Launchpad-Hops: $(launchpad hops)" …
```

An agent that leaves out the header starts again at hop 1, so the guard is best effort. Your own scripts and CI jobs do not need the header.

Home is a private Space, so callers must also pass the Space's sign-in with an HF token that can read the Space:

```sh
curl -X POST https://<you>-launchpad.hf.space/hooks/<id> \
  -H "Authorization: Bearer $HF_TOKEN" \
  -H "X-Launchpad-Hook-Secret: <secret>" \
  -H "Content-Type: application/json" \
  -d '{"event": "build_failed", "url": "https://…"}'
```

curl, scripts and CI jobs can do this. GitHub's own webhooks cannot (they send no HF token): use a GitHub Actions step with curl instead. Each hook row has **Copy URL**; its **⋯** holds **Copy curl example**, **Open the last session** and **Delete**, which stops the URL at once.

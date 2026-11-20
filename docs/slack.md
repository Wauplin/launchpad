# Slack

Agents read and write Slack with `launchpad slack --profile <handle> <command>`. The commands and options are those of [slackline](https://github.com/co42/slackline): `launchpad slack --profile user-read search messages "deploy"` runs `slackline search messages "deploy"`. Home runs slackline with the profile's token and sends back its output and exit code. Agents and runners never hold a Slack token, and runners do not need slackline.

## Profiles

A **Slack profile** (Settings → **Slack**) is:

- a **name**, for example "Read as me" or "Post as Release bot";
- a **handle**, lowercase with dashes, made from the name by default (`read-as-me`), used as `--profile <handle>`;
- a **description**: what it can do and in whose name. Agents get the description of every profile they hold, so they pick the right one;
- a **token**: a User OAuth Token (`xoxp-…`), which acts as the person who installed the Slack app, or a Bot User OAuth Token (`xoxb-…`), which acts as the app's bot user;
- **On by default for every session** (off by default);
- **Add Launchpad's attribution line to messages** (on by default).

Launchpad has no read or write mode of its own: a profile can do what its token's scopes allow. Some usual profiles: "Read as me" (a user token with read scopes), "Write as me" (a user token with write scopes too), "Post as a bot" (a bot token with write scopes). A good habit: read as yourself, write as a bot, so people see that an agent wrote the message.

### Token types

Slack has three kinds of tokens ([Slack's token types](https://docs.slack.dev/authentication/tokens)):

| Prefix | What it is | In Launchpad |
| --- | --- | --- |
| `xoxp-` | A user token: it acts as the user who installed the app, sees what they see, and writes in their name. | Supported. The only kind that can search messages. |
| `xoxb-` | A bot token: it acts as the app's bot user, only in the channels the bot was added to. | Supported. Messages show the app's name. |
| `xapp-` | An app-level token: it opens Socket Mode connections and calls a few app-wide methods. It cannot read or send messages. | Refused, with an explanation. |

### Create the Slack app

When you add a profile, **Create the Slack app for this profile** builds the link for you:

1. Choose **Acts as**: **You (user token)** or **A bot (bot token)**.
2. Choose **Access**: **Read only** or **Read and write**.
3. Change the **App name** if you like (the profile's name by default).
4. Open the link: Slack's "Create app" page opens with the app's name and permissions filled in. Pick your Slack workspace, click **Next** and **Create**, open **Install App**, click **Install to Workspace**, then **Allow**.
5. Copy the **User OAuth Token** (`xoxp-…`) or the **Bot User OAuth Token** (`xoxb-…`) and paste it. For a bot, add the app to the channels it should read or post in.

The read scopes are those of `slackline token create`; write adds `chat:write`, `files:write`, `im:write`, `pins:write`, `reactions:write` (and `users.profile:write` for a user, `channels:join` and `chat:write.public` for a bot). Bot tokens cannot use `search:read`.

**Test** (in the profile's **⋯**) asks Slack who the token acts as. **Delete** removes the profile and its token from Home; agents lose it at once. Deleting does not revoke the token in Slack: remove the app there.

## Who gets a profile

No session gets a profile unless you decide so:

- **On by default for every session**: every session holds it, unless a default or a launch switches it off.
- **A bot**: Settings → Slack lists the bots of the window's workspace with a choice per profile (Default, On, Off), which writes `"slack:<handle>" = true` or `false` in the bot's `[tools]` (also in the bot's settings → Access → **Edit** → **Default tools**). A bot gets a profile only from its own setting or from the profile's default, never from its workspace's or folder's defaults.
- **One launch**: the composer's **Tools** lists each profile as **Slack · <name>**.
- **The agents you launch in a workspace or folder**: their [default tools](home-and-runners.md#default-tools).

Only you switch a profile on. An agent never does: a launch, schedule, routine, default tools or bot change of an agent that switches one on is refused, and an agent's launch card drops it. Home checks every call against the session's profiles at that moment, so a change applies at once; the session's instructions list its profiles as of its start.

## What agents see

A session that holds profiles gets a **Slack profiles** part in its instructions (also in **What it sees at start**): how to call `launchpad slack`, and each profile's handle, name, token kind and description. Without `--profile`, or with a profile it does not hold, the call fails and names the profiles the session holds.

## Attribution line

When **Add Launchpad's attribution line** is on, Home ends the text of `messages send`, `dms send` and the comment of `files upload` with the session's line in Slack's link syntax, for example `_Written by Claude Code via <https://github.com/Wauplin/launchpad|Launchpad>, posted automatically_`. A file shared in a channel without a comment gets the line as its comment. Agents never add the line themselves; a message that already ends with one keeps one. Turn it off for a bot profile whose app name already says that an agent wrote the message.

## Limits

- **Files.** `files upload <path>` reads the file in the agent's folder and sends it to Home (at most 8 MB). `files download <id> -o <path>` writes the file in the agent's folder; without `-o`, the file goes to standard output. An answer is at most 8 MB.
- **Not available:** `watch` (Socket Mode streams events without end, and needs an `xapp-` token) and `--token` (Home gives the token). For `messages send`, `dms send`, `files upload` and `files download`, Launchpad accepts only the options it knows, so it finds the text and the files for sure.
- **Time.** A call ends after 2 minutes. Slack's rate limits apply per token: slackline waits and retries, which counts in that time.
- **Home.** Every call goes through Home, which runs at most 4 slackline processes at once; more calls wait their turn. When Home is unreachable (an update, a restart), `launchpad slack` fails and the agent can try again later.
- **No log.** Launchpad does not record Slack calls or messages yet.

Slack profiles are different from a Slack [connection](connections.md): a connection is an incoming webhook whose messages wait in the Outbox for your click; a profile lets agents act in Slack directly.

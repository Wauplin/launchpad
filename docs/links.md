# Links and references

Every object you see in Launchpad has a link: an agent and each of its messages, a chat and each of its messages, a bot channel or DM and each of its messages, a folio with a page and a section, a topic, a bot, a Local PR, a folder, a card, an oracle, a routine and a knowledge base note. Paste a link into any message box and the agent that receives it can read exactly that object. Paste it into **Go to** and the window opens it at that place.

## Copy a link

- **A message** in a chat, a channel or DM, or an agent's trace: hover it (on a phone, it is always visible) and click the link icon next to its time.
- **A chat or channel**: **Copy link** in its row's **⋯** in Chats (on a phone, also in the chat's **⋯**).
- **An agent**: **Copy link** in the session's **⋯** in Agents.
- **A folio section**: **Link** next to the section's heading (next to **Comment**).
- **Any other page**: its address in the browser is its link.

A copied link opens the object in its workspace (`?ws=` in the link switches the window). The toast also shows the object's short ID, for example `lp:chat/<bot>/<thread>/<session>/<seq>`.

## Short IDs

A short ID names the same object as its link, without a host. Agents print them and the `launchpad` CLI reads them.

| Object | Short ID | Page |
| --- | --- | --- |
| Agent session, one message | `lp:session/<id>`, `lp:session/<id>/<seq>` | `/agents?session=<id>&msg=<seq>` |
| Chat (a bot thread and its conversation), one message | `lp:chat/<bot>/<thread>`, `lp:chat/<bot>/<thread>/<session>/<seq>` | `/bots/<bot>?thread=<thread>&msg=<session>.<seq>` |
| Bot channel or DM, one message | `lp:channel/<id>`, `lp:channel/<id>/<message>` | `/channels/<id>?msg=<message>` |
| Folio, page, section | `lp:folio/<slug>/<page>#<section>` | `/folios/<slug>?page=<page>&section=<section>` |
| Topic, bot, Local PR, folder | `lp:topic/<id>`, `lp:bot/<id>`, `lp:local-pr/<id>`, `lp:folder/<id>` | their pages |
| Card, oracle, routine, note | `lp:card/<id>`, `lp:oracle/<slug>`, `lp:routine/<id>`, `lp:note/<path>` | Today, Schedules, Knowledge |

Older addresses keep working: Today card links (`/?card=session:<id>`), a bot's or a channel's settings page, and folio anchors.

## Go to

The top bar's **Go to** button (on a phone, **More → Go to a link or ID**, on a laptop also **Ctrl+K** or **Cmd+K**) takes a link or a short ID and opens its page at the exact place: a chat or channel loads older messages until the linked one is there, scrolls to it and outlines it; an agent's trace does the same; a folio opens the page and scrolls to the section. An object of another workspace switches the window to it. An unknown, deleted or malformed link says so.

Links in agents' messages and in channel messages open the same way, and short IDs in their text are links too.

## References as context

Paste a link or short ID into a message box: the launch composer, an agent's message box, a bot's chat, the Ask panel, the top-bar field or a channel. Home finds every Launchpad link and short ID in the text and checks each against the receiving agent's workspace. The prompt then lists each one with the command that reads it, never its text:

```
Launchpad references: …
- Chat "Fix the login flow", one message (chat): `launchpad ref read lp:chat/…/…/…/31`
```

In the composer and an agent's message box, a pasted link becomes a token (`@chat:Fix-the-login-flow#msg`) with a chip, like a `#` pick; in a bot's chat it stays in the text and adds a chip. **+ Add context → Chat or channel…** adds a whole chat or channel the same way. Removing the chip or token removes the reference. A link of another workspace, or one that names nothing, is not added: the box says why. A link inside a message sent by another bot is read the same way, within the receiving agent's workspace.

## How agents read references

`launchpad ref read <link or ID> [--before 6] [--after 3]` prints the object as Markdown:

- **A message** (of a chat, a channel or an agent): the message with six earlier and three later ones, each with who wrote it, when and its own short ID, fenced as someone's words. A chat lists its messages and each bot's final answer per turn, like the chat itself; an agent lists the messages it received and its answers, without steps and tool calls.
- **A chat, channel or agent** without a message: its newest twelve messages, plus the commands that read more (`launchpad trace <id> --through <seq>` for a full trace).
- **Other objects**: a short summary and the command that reads more (`launchpad folio read`, `launchpad pr show`, `launchpad topics show`, `launchpad oracle show`).

`launchpad ref resolve <link or ID>` prints what it names (JSON). Agents read only objects of their own workspace; anything else reads as unknown, with the same words as a deleted object.

# Launchpad for Chrome

The extension brings Launchpad into github.com and adds a small chat about any page. On GitHub, it shows what your agents do about the repository, issue or pull request you look at, and puts their text into GitHub's own fields when you click **Insert**. You post with GitHub's own button. Heavy work stays in Launchpad, one click away with **Open in Launchpad**.

The extension never posts, submits or clicks on GitHub for you. It never acts in pages and never calls a model API: agents run on your runners, as always.

## Install

Use Google Chrome 116 or newer.

**From a release.** Each GitHub release has `launchpad-chrome-extension.zip`, and the release package holds it in `extension/`. Unzip it into a folder of its own.

**From a source checkout.** Use Node 24 (`web/.nvmrc`) and pnpm 10.33.0 through Corepack:

```sh
cd web && pnpm install --frozen-lockfile
cd ../extension && pnpm install --frozen-lockfile && pnpm build
```

The build goes to `extension/dist`. To ship a Hugging Face client ID with the build, copy `extension/.env.example` to `extension/.env.local` and set `VITE_HF_CLIENT_ID` first (see [Sign in with Hugging Face](#sign-in-with-hugging-face)).

Then open `chrome://extensions`, turn on **Developer mode**, click **Load unpacked** and choose the folder. Pin Launchpad to the toolbar.

The manifest's public key pins the extension ID to `bbefehapjjlbeodihifmpgidgjhmcklj`. Home trusts only this ID. Keep these three in sync:

- the `key` in `extension/public/manifest.json`;
- `EXTENSION_ID` in `crates/server/src/auth.rs` (the daemon);
- `EXTENSION_ORIGIN` in `deploy/hf-home/gateway/proxy.py` (the Space gateway).

## Connect to Home

Click the toolbar button to open the side panel.

### Home in a Hugging Face Space

1. Click **Sign in with Hugging Face** and allow the app on Hugging Face's page.
2. The extension looks for your Launchpad Home: Spaces tagged `launchpad-home` (or titled “Launchpad Home”) in your account, then in your organizations.
3. A running Home of your own account connects at once. Otherwise, pick one in the list and click **Connect**. A sleeping Space starts when you choose it; keep the panel open for a minute or two.

The extension sends your Hugging Face token only to the Hub and to the Space's own address (`https://<name>.hf.space`). It asks only running Spaces of your own account whether they are a Home, so it never wakes a sleeping Space. A Space of an organization runs code that other members may change, so it is contacted only after you pick it. To see an organization's Spaces, allow that organization on Hugging Face's sign-in page.

On **Connect**, the Space's gateway checks your token (the Hub's private-Space check) and creates a device for this browser. The device shows in Launchpad's **Settings → Devices** as “Chrome on <system>”.

### Sign in with Hugging Face

Sign in needs an OAuth app on Hugging Face. Register it once under your account, in [Connected Apps](https://huggingface.co/settings/connected-applications) (**Create App**):

| Field | Value |
| --- | --- |
| Name | Launchpad for Chrome |
| Homepage | `https://github.com/Wauplin/launchpad` |
| Scopes | `openid profile read-repos` |
| Redirect URL | `https://bbefehapjjlbeodihifmpgidgjhmcklj.chromiumapp.org/` |
| Type | Public client (PKCE, no secret) |

Release and main builds include the client ID from the GitHub repository variable `HF_CLIENT_ID`, passed to the build as `VITE_HF_CLIENT_ID`. Set that variable under **Settings → Secrets and variables → Actions → Variables** before building.

For a source build or a different OAuth app, give its client ID to the extension in one of two ways:

- at build time, with `VITE_HF_CLIENT_ID` in `extension/.env.local`;
- in the panel's **Settings → Sign in with Hugging Face → OAuth client ID** (this wins over the build's value; leave it empty to use the build's).

Without a client ID, the button says that sign-in is not configured yet. The address path below still works.

### Home at an address (dev or self-hosted)

For a dev Home, or a Home on another computer:

1. Click **Use an address** and enter Home's address, such as `http://127.0.0.1:7318` or `https://your-machine.ts.net`.
2. Chrome asks for access to that address. Allow it.
3. The panel shows a pairing code. On a paired device, open **Settings → Devices** and approve the code. Keep the panel open until then.

### Disconnect

- In the panel: **Settings → Disconnect**. It removes this browser from Home's devices and signs out of Hugging Face in the extension.
- In Launchpad: **Settings → Devices → Revoke**. The extension stops at once and shows **This browser was disconnected**, with **Connect again**.

## On GitHub

The marks sit inside GitHub's pages, in GitHub's colors with a teal Launchpad accent. Each place shows at most one button, plus **⋯** for the rest. A mark whose place GitHub changed simply does not show; the side panel still works.

The extension follows the workspace of the repository you look at: when a folder of that repository belongs to a workspace, the panel and the toolbar badge switch to it.

### Repository

- **Badge** next to the repository name: **In Launchpad · Work / folder**, **Your fork of …**, **Fork of … in Launchpad**, or **Not in Launchpad**. Its menu opens the folder or its Files in Launchpad.
- A repository with no folder offers **Add to Launchpad** (Clone from GitHub into a workspace you pick) and **Ask without cloning** (a bot answers with the repository as context, without a clone).
- **Strip** on the Code page: the folder's suggestions waiting for you, Local PRs to publish, branches that GitHub does not have yet, and **Launch** (ask a bot with the repository as context).
- **+N local** after the Pull requests tab: the folder's open Local PRs.

### Pull requests and Issues lists

- **Local PRs** section above GitHub's pull requests: the Local PRs of the folder that are not on GitHub yet, with **Publish…** (the same options and confirmation as in Launchpad, naming the reviewed commit) and links to review, merge or discard them in Launchpad.
- One chip per row, by priority: needs you, working, a bot's report (such as “merged”), commits to push, Not in Launchpad.
- On hover, a row offers **Review with agent** (pull requests) or **Ask** (issues). The popover has the bot, an optional preset and a note. Review with agent remembers its bot and preset per repository.
- When you select rows with GitHub's checkboxes, a bar offers one agent for all of them.

### Pull request and issue conversation

- **Launchpad box** at the top of GitHub's sidebar. It lists the agents on the item, suggestions waiting for you, Local PRs and the agent's branch. Its one main button depends on the state:
  - pull request: **Insert reply**, then **Push to this PR**, **Continue**, **Review with agent**;
  - issue: **Insert reply**, then **Publish…** (its Local PR), **Fix in a Local PR**, **Investigate**.
- **Ask** on each comment: a bot gets the item and that comment as context (by its ID, never the page text). Messages about one item go to that item's thread of the bot, so each pull request or issue has its own conversation. A comment that mentions you gets an amber **Ask <bot> what to answer**.
- **Live agent card** under the comment an agent was asked about: its current step and time, then **Reply ready** with **Insert** when it wrote a reply.
- **Investigate** and **Check with an agent** launch a read-only agent; **Fix in a Local PR** launches an agent that ends with a Local PR. These are one-time launches in the folder, with a profile you pick in the popover.
- **Before you approve**: next to GitHub's **Approve and run** banner, a note lists the files of the pull request that change what CI runs (from Home's copy of the diff), with **Check with an agent**. GitHub's button stays your click.

### Insert

An agent's suggested comment shows as a banner over GitHub's comment box. **Insert** writes it into GitHub's box:

- The text goes in as one undo step, so Ctrl+Z removes it.
- When the box already has text, you choose **Replace it**, **Add below** or **Cancel**.
- The [attribution line](../docs/agents.md#attribution) of the agent or bot comes last, after a blank line. It is plain text: keep it or delete it.
- A reply to a review thread goes into that thread's reply box. If the box is closed, the extension scrolls to it and asks you to open it, then Insert again.

You edit the text and post it with GitHub's button. The suggestion then says **Inserted on GitHub** in Launchpad, and Launchpad's own Post is no longer offered for it (Discard stays).

### Files changed

- **Ask about these lines**: select lines with GitHub's line numbers (shift-click for a range). A bar shows the range with **Ask about these lines**. The bot gets a reference to the lines; Home reads them from the pull request's diff. The answer shows under the lines, with **Turn into line comment**.
- **Ghost comments**: the line comments of an agent's suggested review show under their lines as dashed teal boxes. **Add to my review** fills GitHub's line comment form; you then click GitHub's **Add single comment** or **Start a review**. **Dismiss** in ⋯ drops the comment from the suggestion. The extension never opens GitHub's form itself: when it is closed, it scrolls to the line and fills the form once you open it with GitHub's **+** (within two minutes).
- **Review banner** above the files: the commit the agent reviewed, its event, the number of line comments, and **Next comment**.
- **Insert summary**: over GitHub's **Review changes** dialog, it fills the summary field. The suggested event shows as text; you choose the event and submit with GitHub's button.
- **Outdated guard**: a review written for an older commit says **outdated**. Its comments insert only on a second click (**Insert anyway**), which names the commit it was written at.

### Push to this PR

When an agent's Launchpad branch holds the pull request's head plus new commits, the box says **N commits ready to push**. **Push to this PR** opens a confirmation that names the branch, the commit and the target. On your click, Home checks that the branch still points at that commit, then pushes it to the pull request's branch as a fast-forward, never forced. After that, Home follows the branch: the agent's next commits go to the pull request too.

Home pushes only to the pull request's own repository, your own fork, or a fork whose author lets maintainers edit. The box shows **Maintainers can edit** or **Maintainers cannot edit**. A branch whose last commits did not reach GitHub shows **Not on GitHub yet** with the last error.

### Notifications

- **Strip** at the top of github.com/notifications: what waits for you in Launchpad's Today of the workspace (questions, finished agents, suggestions to post, Local PRs to publish), with **Open Today**.
- **Done all handled**: threads that a bot already handled (its newest report says merged, closed, approved or commented, and nothing happened after it). The confirmation lists them; **Mark N Done** marks them Done in Launchpad and on GitHub. Home checks each thread again first.
- Compact controls beside the repository and item number, without an extra row: one status chip (needs you, Reply ready, Review ready, a bot's report, Not in Launchpad), and hover actions: **Review with agent** or **Ask**, and in ⋯ **Draft a reply** and **Open in Launchpad**.
- **Done on GitHub**: when you click GitHub's own **Done**, **Mark as read** or **Unsubscribe**, the extension tells Home once the page shows the change. Launchpad's Inbox updates at once (the Inbox header says “N done on GitHub today”). Home writes nothing more to GitHub.

## Side panel

The side panel has a header with the workspace picker and Settings, then three tabs.

- **Chat**: talk to a bot of the workspace about the page or anything else. The workspace's Ask bot comes first; pick another one above the box. The **This tab** chip attaches the page: when you send, the extension reads the page once (readable text, URL and your selection) and attaches it as `page.md`. Remove the chip to send without it. Starters: **Summarize this page**, **Explain my selection**, **Find…**. The chat is one of the bot's threads in Launchpad (Main by default); pick another thread above the box, or **New conversation** to start a new thread with your next message. While the bot works, the chat shows its step with **Stop**; a question shows with **Answer in Launchpad**.
- **This item** (on GitHub pull requests and issues): the bot's thread for this item and its state, with **Continue** to open it in Chat, and the same facts and buttons as the page's box: agents with **Stop**, suggestions with **Insert**, **Show on Files changed** and **Review in Launchpad**, Local PRs, and the branch with **Push to this PR…** (a confirm step in the panel). Use it when a mark does not show on the page.
- **Agents**: the workspace's agents of the last day, grouped as Needs you, Running and Earlier today.
- **Settings**: the connection and Hugging Face account, the OAuth client ID, text size (100–150%), **GitHub anchors** (“3 of 4 work”, naming the places the extension could not find on GitHub's pages) and **Disconnect**.

**Ask Launchpad about selection** in Chrome's right-click menu adds the selected text as a quote to the chat's box and opens the panel. It never sends by itself.

## Toolbar badge

The badge shows how many items wait in Today of the extension's workspace, in amber. A grey **…** means Home is starting or does not answer; the tooltip says which. The extension shows no Chrome notifications.

## Feedback ledger

For each suggestion, Home records what happened to it, part by part (the review's summary and each line comment):

- shown on the page, inserted, posted by hand on GitHub, then confirmed or unconfirmed;
- posted by Launchpad, dismissed, discarded or outdated;
- the final text, the size of your edits and whether you kept the attribution line.

“Posted by hand” is a claim until Home finds your comment on GitHub. Home checks the item's comments 20 s, 2 min, 10 min and an hour after the claim. A comment of yours that keeps at least 85% of the text confirms it; otherwise the claim becomes unconfirmed after an hour. Nothing reads the ledger yet: it is kept for a future feedback view. No marker is added to the text you post.

## Permissions

- `sidePanel`, `storage`, `alarms`: the panel, the connection and the live updates.
- `identity`: Sign in with Hugging Face.
- `contextMenus`: Ask Launchpad about selection.
- `activeTab`, `scripting`: read the page you chat about, once, when you send.
- Hosts `github.com` (the GitHub layer), `huggingface.co` (sign-in and Space discovery) and `*.hf.space` (Home in a Space).
- Optional hosts: the address of a Home you pair by address, and a site you attach to a chat. Chrome asks in the click.

## Safety

- **Never posts or submits.** The extension never clicks, submits or posts on GitHub. Insert only fills GitHub's own fields, and you post with GitHub's button. The extension observes that click to record it in the ledger.
- **GitHub writes only through its own confirmations.** Three actions write to GitHub: **Publish** of a Local PR, **Done all handled** and **Push to this PR**. Each runs only from the confirm button of its own dialog, on your real click, and Home checks it again.
- **Tokens stay in the service worker.** Only the service worker talks to Home and Hugging Face. It keeps the Hugging Face tokens and the device secret in extension storage that content scripts cannot read. Content scripts reach Home only through narrow messages, one per Home route.
- **Page text stays on the page.** Marks are built from Home's data and the URL. Titles read from the page show only in the extension's popovers; agents get item numbers and comment IDs, never page text. Only Chat sends a page, when you send with its chip.
- Agents cannot use the extension's routes (`/api/ext/*` and the ledger routes are the owner's).

## Known limits

- Sign in with Hugging Face has not been tested against a real Space yet: register the OAuth app first.
- Some GitHub selectors were checked only on signed-out pages and copies of GitHub's markup: the comment box of the newer issue pages, notifications, line comment forms, the newer Files changed view, the Review changes dialog and the Approve and run banner. A mark that does not find its place does not show.
- Not built yet: diagnosis of failed CI runs, filling a new pull request's form, release notes, sync of viewed files, and a view of the feedback ledger.

## Development

- Code: `src/background/` (service worker: connection, `GET /api/events` stream, page context cache, badge, message router), `src/content/github/` (the GitHub layer; every insertion point is an anchor in `anchors.ts`), `src/panel/` (side panel), `src/shared/messages.ts` (the message protocol).
- Checks: `pnpm typecheck` and `pnpm build`. CI runs both.
- See [Architecture](../docs/architecture.md#chrome-extension) for how it talks to Home.

Sessions linked to a GitHub item automatically join a workspace topic named `repository#number` (for example, `huggingface_hub#5352`). Agents can link their own session with `launchpad link --repo owner/repo --number 5352 --session-id <id>` and optionally `--comment-url <url>`. Creating a suggested comment or review also links its session. Repository references without an item number do not create topics. Agents cannot rename or delete topics or link other sessions.

A Local PR body can name issues in its folder’s repository with `Fixes #5352`, `Closes #5352`, or `Resolves #5352`. These references link its session to each issue and show the Local PR beside the session trace and suggestions in the Chrome extension’s box on that issue. Set or edit the body with `launchpad pr create|update --body-file <file>`. Removing a closing reference removes the Local PR from that issue’s box; the session’s historical link remains.

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
- **Strip** on the Code page: agents working, the folder's suggestions waiting for you, Local PRs to publish, branches that GitHub does not have yet, and **Launch** (ask a bot with the repository as context). Click **N suggestions to post** to unfold them: each shows its item, its kind, its author and the start of its text, with **Open item** (a comment waits in the item's timeline with **Post this comment**) including reviews that can be posted from the item’s timeline. Click **N Local PRs to publish** to unfold the Local PRs, each with **Publish…** and its details (see below).
- The extension adds nothing to GitHub's tab bar: an extra element there makes GitHub fold every tab into its **More** menu.

### Pull requests and Issues lists

- **Local PRs** section above GitHub's pull requests: the Local PRs of the folder that are not on GitHub yet, with **Publish…** (the same options and confirmation as in Launchpad, naming the reviewed commit) and links to review, merge or discard them in Launchpad. Click a Local PR's title to unfold its details without leaving GitHub: the reviewed commit, the number of files, lines and commits, whether its base moved, its description, its commits and its files with their added and removed lines.
- One chip per row, by priority: needs you, working, a bot's report (such as “merged”), commits to push.
- A notes tag when your agents wrote private notes about the item: a teal **2 new notes** while one is unread, a quiet **2 notes** once you read them. A click opens the item.
- On hover, a row offers **Review with agent** (pull requests) or **Ask** (issues). The popover has the bot, an optional preset and a note. Review with agent remembers its bot and preset per repository.
- When you select rows with GitHub's checkboxes, a bar offers one agent for all of them.

### Notifications

On github.com/notifications, each row keeps GitHub's height: the marks sit at the end of its repository line (“owner/repo #123”).

- The row's chip, as on the lists (an agent asks you, works or finished, a review or reply ready, a bot's report).
- The notes tag: a teal **N new notes** while your agents' private notes about the item are unread (you have not opened its page since), a quiet **N notes** once you read them.
- On hover, **Review with agent** (pull requests) or **Ask** (issues): pick the bot and write your message (a review request, a question…), and the bot starts on the item. **⋯** has **Ask a bot…**, **Draft a reply…** and, for a repository not in Launchpad, **Add to Launchpad**.

The extension only reads the list to find each row's item. It never marks, archives or clicks anything there.

### Pull request and issue conversation

- **Launchpad box** at the top of GitHub's sidebar. It lists the agents on the item (click one to read its work on the page), suggestions waiting for you, Local PRs and the agent's branch. Its one main button depends on the state (**Edit the reply** when a reply was prefilled into GitHub's box):
  - pull request: **Insert reply**, then **Push to this PR**, **Continue**, **Review with agent**;
  - issue: **Insert reply**, then **Publish…** (its Local PR), **Fix in a Local PR**, **Investigate**.
- **Ask** on each comment: a bot gets the item and that comment as context (by its ID, never the page text). Messages about one item go to that item's thread of the bot, so each pull request or issue has its own conversation. A comment that mentions you gets an amber **Ask <bot> what to answer**.
- **Live agent card** under the comment an agent was asked about: its current step and time, then **Reply ready** with **Insert** when it wrote a reply.
- **Investigate** and **Check with an agent** launch a read-only agent; **Fix in a Local PR** launches an agent that ends with a Local PR. These are one-time launches in the folder, with a profile you pick in the popover.

### Private notes

Private notes are a private timeline of an issue or pull request, shown inside GitHub's own timeline at the moment they were written, like comments from a Launchpad agent. Only you and your agents see them; they are never posted. Agents add a note when something changed (`launchpad note`), and a note may carry a suggested comment. A suggested comment shows as its own highlighted section at the bottom of its note, rendered like a GitHub comment, with **Post this comment** (one click: Launchpad posts it as you, with the attribution line), the agent's actions ticked under it (close, with a reason or as a duplicate; reopen; request reviewers; ready for review or convert to draft; update branch): untick one to skip it, or tick **close** to close after posting, and the button names what the click does (**Comment and close**, **Comment, mark ready for review and request reviews**). A suggestion of actions only (a close without a comment) shows just the actions and one button (**Close as not planned**). Then **Edit**, **Insert into comment box** and **Discard**, so GitHub's own comment box stays yours. Long notes fold; **Show work** opens the agent's work on the page (see below); **⋯** opens the agent in Launchpad or deletes the note.

A note you have not read yet has a **New** tag. Opening the item's page counts as reading its notes once the page was visible in a focused window for 1.5 s: lists, notifications and the panel then stop flagging them.

Above GitHub's comment box, **Private note to your agents** stores your own note in Launchpad and sends it to every agent that wrote about the item (else to its default bot); they answer with a note when they have something useful. The panel's **This item** lists the same timeline and has the same box. **Ctrl+Enter** (**Cmd+Enter** on a Mac) in either box adds the note.

Notes link to each other with a note's link (⋯ → **Copy link**, or `url` of `launchpad notes` for agents): `https://github.com/owner/repo/issues/123#lp-note-<id>`. The link shows the note's author ("Reviewer's note"); a click scrolls to that note on this page and marks it, or opens the other item's page at that note. Each note lists the notes of the item that reference it (**Referenced in**), and an answer's "answers …" tag links to the note it answers. Like GitHub's comments, `#123`, `owner/repo#123`, `@login` and GitHub URLs (`#123 (comment)`, `#123 (review)`, short commits) show as links.

### An agent's work on the page

Click an agent in the Launchpad box (or **Show work** on a note, or **Show the agent's work** in a banner's **⋯**). A drawer opens on the right with what the agent was asked, its answer for each turn (Markdown) and its steps, folded under a line such as “12 steps · 3 min”. It refreshes every 5 seconds while it is open. **Close** or Esc closes it. **Open in Launchpad** opens the full trace, with Stop.

### Prefill and Insert

Prefill is off by default, since the timeline shows suggested comments; turn it on in the panel's **Settings → GitHub**. When it is on and an agent suggested a comment for the pull request or issue you open, the extension puts the text into GitHub's comment box on its own if the box is empty. The banner says **Prefilled with <bot>'s draft**. Edit it and post it with GitHub's button. The page does not scroll and the focus does not move. Nothing is claimed in Launchpad until you submit GitHub's form, so Launchpad's own Post stays available. If you discard or post the suggestion from Launchpad and the box still holds the untouched text, the extension takes it out again; **⋯ → Take it out of GitHub's box** does the same. It fills each suggestion once per tab: if you empty the box, it stays empty.

A suggested comment shows in the item's timeline (see [Private notes](#private-notes)); a reply to a review comment also shows as a banner over that thread's reply box. **Insert** writes it into GitHub's box (and **Insert again** when it was prefilled or inserted before; the comment box then shows a banner for it):

- The text goes in as one undo step, so Ctrl+Z removes it.
- When the box already has text, you choose **Replace it**, **Add below** or **Cancel**.
- The [attribution line](../docs/agents.md#attribution) of the agent or bot comes last, after a blank line, ending with `posted manually` since you post it with GitHub's button. It is plain text: keep it or delete it.
- A reply to a review thread goes into that thread's reply box. If the box is closed, the extension scrolls to it and asks you to open it, then Insert again.

You edit the text and post it with GitHub's button. The suggestion then says **Inserted on GitHub** in Launchpad, and Launchpad's own Post is no longer offered for it (Discard stays).

### Files changed

- **Ask about these lines**: select lines with GitHub's line numbers (shift-click for a range). A bar shows the range with **Ask about these lines**. The bot gets a reference to the lines; Home reads them from the pull request's diff on GitHub when it sends the message. The answer shows under the lines, with **Turn into line comment**.
- **Ghost comments**: the line comments of an agent's suggested review show under their lines as dashed teal boxes. **Add to my review** fills GitHub's line comment form; you then click GitHub's **Add single comment** or **Start a review**. **Dismiss** in ⋯ drops the comment from the suggestion. The extension never opens GitHub's form itself: when it is closed, it scrolls to the line and fills the form once you open it with GitHub's **+** (within two minutes).
- **Review banner** above the files: the commit the agent reviewed, its event, the number of line comments, and **Next comment**.
- **Insert summary**: over GitHub's **Review changes** dialog, it fills the summary field. The suggested event shows as text; you choose the event and submit with GitHub's button.

### Push to this PR

When an agent's Launchpad branch of the pull request has commits since the agent started and Home does not push it to GitHub yet, the box says **N commits ready to push**. **Push to this PR** opens a confirmation that names the branch and the commit. On your click, Home checks that the branch still points at that commit, reads the pull request on GitHub, then pushes the branch to the pull request's branch only as a fast-forward of its head, never forced. After that, Home follows the branch: the agent's next commits go to the pull request too.

When an agent prepared the push as a ready card (`launchpad propose ready` with `branch.push-to-pr`), the timeline shows **Ready to push** under the agent's newest private note, with **Push to this PR** and **View its changes**. The branch and commit come from the card (Home's `ExtContext.ready_pushes`), never from the note's text. The button opens the same confirmation; after the push, the extension starts the card (`/api/proposals/{id}/ran`), so Today no longer asks. One push runs at a time per branch and commit; a card whose branch moved, is gone from Home or already goes to the pull request shows why instead of the button.

Home pushes only to the pull request's own repository, your own fork, or a fork whose author lets maintainers edit; otherwise the push is refused and the dialog says why. A branch whose last commits did not reach GitHub shows **Not on GitHub yet** with the last error.

## Side panel

The side panel has a header with Today's count, the workspace picker and Settings, then up to four tabs. Click the amber count to open the **Today** tab.

- **Chat**: talk to a bot of the workspace about the page or anything else. The workspace's Ask bot comes first; pick another one above the box. The **This tab** chip attaches the page: when you send, the extension reads the page once (readable text, URL and your selection) and attaches it as `page.md`. Remove the chip to send without it. Starters: **Summarize this page**, **Explain my selection**, **Find…**. The chat is one of the bot's threads in Launchpad (Main by default); pick another thread above the box, or **New conversation** to start a new thread with your next message. While the bot works, the chat shows its step with **Stop**; a question shows with **Answer in Launchpad**.
- **This item** (on GitHub pull requests and issues): the bot's chat for this item and its state, with **Continue** to open it in Chat, the item's private timeline with the **Add private note** box, and the same facts and buttons as the page's box: agents with **Stop**, suggestions with **Insert**, **Show on Files changed** and **Review in Launchpad**, Local PRs, and the branch with **Push to this PR…** (a confirm step in the panel). Use it when a mark does not show on the page.
- **Today**: first **Unread notes**, the items your agents wrote private notes about that you have not read yet (item, how many, who and the start of the newest note). Click one to open the item on GitHub (in the current tab when it shows GitHub); the check mark marks it read, and **Mark all read** marks every note of the workspace. Then the workspace's Today, grouped like Launchpad's Today page (To answer, To review, To approve, To send, GitHub, To fix). Click a card to open it in Launchpad, where you decide; the GitHub mark opens its item on GitHub.
- **Agents**: the workspace's agents of the last day, grouped as Needs you, Running and Earlier today.
- **Settings**: the connection and Hugging Face account, the OAuth client ID, **Prefill suggested comments** (off by default), text size (100–150%), **GitHub anchors** (“3 of 4 work”, naming the places the extension could not find on GitHub's pages) and **Disconnect**.

**Ask Launchpad about selection** in Chrome's right-click menu adds the selected text as a quote to the chat's box and opens the panel. It never sends by itself.

## Toolbar badge

The badge shows how many items wait in Today of the extension's workspace, in amber. A grey **…** means Home is starting or does not answer; the tooltip says which. The extension shows no Chrome notifications.

## Feedback ledger

For each suggestion, Home records what happened to it, part by part (the review's summary and each line comment):

- shown on the page, inserted, posted by hand on GitHub, then confirmed or unconfirmed;
- posted by Launchpad, dismissed, discarded or outdated;
- the final text, the size of your edits and whether you kept the attribution line.

“Posted by hand” is a claim until Home finds your comment on GitHub. Home reads the item's comments on GitHub 20 s, 2 min, 10 min and just after an hour after the claim. A comment of yours that keeps at least 85% of the text confirms it; otherwise the claim becomes unconfirmed after an hour. Nothing reads the ledger yet: it is kept for a future feedback view. No marker is added to the text you post.

## Permissions

- `sidePanel`, `storage`, `alarms`: the panel, the connection and the live updates.
- `identity`: Sign in with Hugging Face.
- `contextMenus`: Ask Launchpad about selection.
- `activeTab`, `scripting`: read the page you chat about, once, when you send.
- Hosts `github.com` (the GitHub layer), `huggingface.co` (sign-in and Space discovery) and `*.hf.space` (Home in a Space).
- Optional hosts: the address of a Home you pair by address, and a site you attach to a chat. Chrome asks in the click.

## Safety

- **Never clicks or submits GitHub's forms.** Insert and prefill only fill GitHub's own fields, and you post with GitHub's button. The extension observes that click to record it in the ledger.
- **GitHub writes only through its own confirmations.** These actions write to GitHub, through Home: **Publish** of a Local PR, **Push to this PR** and **Post this comment** or a suggestion's own button (with the actions you left ticked), or the review actions in the timeline. Publish and Push run only from the confirm button of their own dialog, and suggestion actions ask for confirmation from their buttons under the full suggestion; each needs your real click, and Home checks it again.
- **Tokens stay in the service worker.** Only the service worker talks to Home and Hugging Face. It keeps the Hugging Face tokens and the device secret in extension storage that content scripts cannot read. Content scripts reach Home only through narrow messages, one per Home route.
- **Page text stays on the page.** Marks are built from Home's data and the URL. Titles read from the page show only in the extension's popovers; agents get item numbers and comment IDs, never page text. Only Chat sends a page, when you send with its chip.
- Agents cannot use the extension's routes (`/api/ext/*` and the ledger routes are the owner's).

## Known limits

- Sign in with Hugging Face has not been tested against a real Space yet: register the OAuth app first.
- Some GitHub selectors were checked only on signed-out pages and copies of GitHub's markup: the comment box of the newer issue pages, line comment forms, the newer Files changed view and the Review changes dialog. A mark that does not find its place does not show.
- Not built yet: diagnosis of failed CI runs, filling a new pull request's form, release notes, sync of viewed files, and a view of the feedback ledger.

## Development

- Code: `src/background/` (service worker: connection, `GET /api/events` stream, page context cache, badge, message router), `src/content/github/` (the GitHub layer; every insertion point is an anchor in `anchors.ts`), `src/panel/` (side panel), `src/shared/messages.ts` (the message protocol).
- Checks: `pnpm typecheck`, `pnpm test:suggestions` (pure checks of posting and editing suggestions) and `pnpm build`. CI runs all three.
- See [Architecture](../docs/architecture.md#chrome-extension) for how it talks to Home.

Sessions linked to a GitHub item automatically join a workspace topic named `repository#number` (for example, `huggingface_hub#5352`). Agents can link their own session with `launchpad link --repo owner/repo --number 5352 --session-id <id>` and optionally `--comment-url <url>`. Creating a suggested comment or review also links its session. Repository references without an item number do not create topics. Agents cannot rename or delete topics or link other sessions.

A Local PR body can name issues in its folder’s repository with `Fixes #5352`, `Closes #5352`, or `Resolves #5352`. These references link its session to each issue and show the Local PR beside the session trace and suggestions in the Chrome extension’s box on that issue. Set or edit the body with `launchpad pr create|update --body-file <file>`. Removing a closing reference removes the Local PR from that issue’s box; the session’s historical link remains.

Suggested reviews also appear in the GitHub timeline with their event and line comments, under a header that names what the agent recommends (“suggests an approving review and merge”, “a review requesting changes”). **Approve and merge (squash)** posts an approving review and merges using its suggested method at the reviewed commit; **… and run CI** also approves waiting workflow runs; **Approve**, **Request changes** and **Post review** only post the review. These actions ask for confirmation and use Home’s publication safeguards. A suggestion's actions are ticked under it and the button names them (**Comment and close without merging** on a pull request, **Request changes and convert to draft**); untick one to skip it. **Edit** works on comments and reviews before you post: a comment's text, or a review's summary and each line comment as raw Markdown (`suggestion` blocks stay as written), with **Leave out** to drop a line comment and **Keep it** to bring it back. Line comments keep their place, and the event, follow-up and ticked actions stay. **Save** stores the edit in Launchpad only; Post sends the saved text, and **Cancel** drops your changes. Insert remains available for comments. Inserted suggestions must be posted with GitHub’s own button.

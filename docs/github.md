# GitHub

GitHub's own website owns notifications, issues and pull requests; the [Chrome extension](#on-githubcom) adds Launchpad to it. Launchpad keeps what it owns: agents and their branches, Local PRs, Suggestions (agent comments and reviews), private notes, item links and reports. It never caches or polls GitHub items: every link about an issue or pull request opens github.com, and folder pages link to the repository's pull requests and issues there.

Sign in with `gh auth login` on the machine that runs the daemon. Launchpad reads the token with `gh auth token` and calls the GitHub REST and GraphQL APIs directly, only when a feature needs the answer, and stores no response. Settings → GitHub shows the login and the rate limit.

**Rule: nothing reaches GitHub without your click.** Agents can write comments, reviews and pull requests, but they stay local until you publish them. Once a branch is on GitHub, agents you launch on it push their commits to it (see [Continue a pull request](#continue-a-pull-request)).

## Issues and pull requests as context

- Name an item in a prompt (a link, or `#` in the composer, which searches the folder's repository on GitHub as you type) or add it as context. The agent gets the item's repository, number and link, and reads it with its own tools.
- **Review with agent** (from the extension) gives an agent the PR and review instructions. The agent writes its review as a suggestion.
- Clone from GitHub suggests the repositories you pushed to recently, read from GitHub when the dialog opens; you can also type any `owner/repo`.

## Continue a pull request

Paste a link to a pull request, or to one of its comments, in the composer prompt (or add it as context) and say what to change. When the folder's `origin` or `upstream` is the PR's repository and **From** is a new worktree, **Push to PR #N** is checked by default:

- Home reads the PR on GitHub with its own login (runners need no GitHub credential), fetches its head (`pull/N/head`) into the session's branch, and the runner creates the worktree from it.
- The agent commits; its branch goes to Home at the end of every turn, and Home pushes it to the PR branch (`origin`, or the fork's URL derived from it). Home never force-pushes; if the PR branch moved, the session shows it and the agent rebases on it.
- The worktree is never merged by Launchpad and cannot become a Local PR: the pull request is the review.
- Uncheck **Push to PR** to keep the work on Home in a normal worktree.

Pushing to a fork's branch needs write access to it ("Allow edits by maintainers" or your own fork).

## Suggestions

Agents write comments and reviews with the `launchpad suggest` CLI. They are saved as suggestions, never posted:

```sh
launchpad suggest review --repo owner/repo --number 123 --head <commit> --body-file review.md --event approve --comments-file comments.json
```

`--comments-file` (reviews only) holds the review's line comments, a JSON array of `{"path", "line", "side", "start_line", "body"}` (`side` is `RIGHT` by default or `LEFT` for a removed line; `start_line` is optional, for a range). A comment body may hold a ```` ```suggestion ```` block. On a Local PR, the line comments become notes on those lines.

A suggested review is a Today card (and a line in its bot's chat): **Review** shows its summary and line comments, **Edit** changes the summary and event or removes line comments, and **Post review** sends the summary and every line comment to GitHub as one review at the commit the agent reviewed. Post first reads the pull request's head on GitHub: when it moved, nothing is posted, the line comments count as outdated and you ask the agent for a fresh review. **Open on GitHub** opens the pull request. Posting a suggestion claims it, so it cannot be posted twice. Agent text published from a suggestion ends with its [attribution line](agents.md#attribution).

A suggested comment can answer one comment: the agent adds `--reply-to <comment>` (`issuecomment-<id>`, `discussion_r<id>`, the comment's URL, a review thread's `PRRT_…` ID or a bare comment ID). Home reads that comment on GitHub when the suggestion is saved. Its card says **Reply to <author>'s comment**. **Post** sends a reply to a review comment into its thread, and a reply to a conversation comment as a new comment that starts with a quote line linking it.

With the Chrome extension, you can also **Insert** a suggestion into GitHub's own comment box and post it there with GitHub's button. The card then says **Inserted on GitHub** and offers no Post (Discard stays); once you post it, it says **Posted by you on GitHub** and leaves Today. See [On github.com](#on-githubcom).

A suggested review can carry a follow-up: an approval with `--merge squash|merge|rebase` merges the pull request at the reviewed head, and `--run-ci` approves the head's GitHub Actions runs that wait for a maintainer. Uncheck the follow-up to post only the review. Follow-ups run after the review is recorded as posted. A failure reports that the review was posted and never retries the follow-up.

Agents can also prepare **Approve and run workflows** for a pull request whose workflow runs wait for a maintainer, as a ready card (`github.run-ci`, see [Bots](bots.md#proposals)). It runs the pull request's code in the repository's CI, so it stays your click, like GitHub's own button.

## Local PRs

A Local PR is a pull request that exists only in Launchpad: a branch on Home Git, a title, a description and review notes. Review and publish it from any device, also while the machine that ran the agent sleeps. Publish it to GitHub when it is worth it, or merge it on Home.

![A Local PR with a note for the agent](images/localpr.webp)

```mermaid
flowchart LR
    A["Launch with<br/>When done: Local PR"] --> B[Agent commits on lp/branch]
    C["launchpad pr create<br/>(from an agent)"] --> B
    B --> D[Review: Description, Changes, Commits]
    D -->|Notes| E[Agent updates the branch]
    E --> D
    D --> F[Publish to GitHub]
    D --> G[Merge on Home]
    D --> H[Discard]
```

- **Create**: choose **When done → Local PR** in the composer's options, or let an agent run `launchpad pr create` in its worktree. **When done → Draft PR** or **Open PR** also publishes it on GitHub once the agent's turn is saved (your own launches only; see [Worktrees and merging](agents.md#worktrees-and-merging)). Local PRs opened by one agent form a **change set**: their pages link to each other, and **Publish all** publishes them together and links the pull requests to each other. Publish all first checks that every branch is still at the commit you reviewed, and publishes nothing when one moved.
- **Review**: line notes store their path, line and side separately from editable text. Notes save on Home without a Save notes button. **Publish…** opens branch, draft and fork options; the bar also has **Merge** and **⋯ → Discard**. The agent keeps the title and description up to date with `launchpad pr update`. **Notes** (general or per file) go back to the agent as one message. While notes wait, **Send notes to agent** is the primary button; otherwise **Publish** is.
- **Publish**: choose a remote branch name and GitHub draft or ready, then confirm. Home pushes exactly the commit you reviewed (a newer push asks you to review again) and creates the PR, its description ending with the agent's [attribution line](agents.md#attribution). Check **From my fork** when you cannot push to `origin`: Home pushes to your existing GitHub fork and opens the PR from it. After publication, the Local PR stays open to agents: new commits on its branch reach the published branch through Home, never forced. Its page then shows **PR #N** instead of Publish and Merge: review and merge it on github.com (**Open on GitHub**). Until GitHub has the newest commit, the Local PR and the agent's branch show **Not on GitHub yet**, with the last push error; **Refresh** retries. If the push fails and the remote shows that nothing arrived, the Local PR unlocks itself. Otherwise a banner offers **Unlock**, and a branch that was already pushed keeps its name for the next Publish.
- **Merge**: Home pushes the reviewed commit to the base branch on the folder's remote, then fast-forwards the base branch on Home. When the remote refuses the push (a protected branch, for example), nothing is merged and the Local PR shows why; publish it instead. It never forces or retries automatically. If the base branch moved, send a note asking the agent to rebase.
- **Discard** deletes the branch on Home. Its tip stays restorable for 30 days (folder page → Launchpad branches).

Publication and merges wait until the Local PR's agent is idle.

## On github.com

The [Chrome extension](../extension/README.md) shows Launchpad inside github.com: a badge on each repository, Local PRs and row states on the Pull requests and Issues lists, a box in the sidebar of each issue and pull request, private notes inside the item's timeline, Ask on comments, live agent cards, suggestions to **Insert** into GitHub's own fields, the line comments of a suggested review under their lines in Files changed. It reads all of this from Launchpad's own records (`/api/ext/*`), never from GitHub.

- **Insert, never submit.** The extension never clicks or submits GitHub's forms. **Insert** fills GitHub's field and adds the attribution line at the end, visible and removable; you post with GitHub's button. Prefill (off by default) does the same without a click when the comment box is empty.
- **Private notes**: each issue and pull request has a private timeline in Launchpad. Agents add a note when something changed (`launchpad note`), optionally with a suggested comment, and you add yours with **Private note to your agents** above GitHub's comment box. Your note goes to every agent that wrote about the item in the workspace (else to its default bot), and they answer with a note when useful. The extension draws each note, and each suggested comment without a note, inside GitHub's timeline at the moment it was written. A suggested comment there has **Post this comment** (Launchpad posts it as you after your confirmation), **Edit**, **Insert into comment box** and **Discard**. Notes are never posted.
- **Bot on GitHub**: a folder setting (folder page → **Bot on GitHub**) that names the bot the extension asks on the repository's pages. **Automatic** uses the last bot you used on the repository, else the workspace's Ask bot.
- **Ask about these lines**: on Files changed, a bot gets a reference to the lines you selected (repository, pull request, path, side, range and head). Home reads the lines from the pull request's diff on GitHub when it sends the message, and gives them to the agent marked as text from the pull request's author. A bot or agent may name only a repository of its own workspace.
- **Push to this PR**: when an agent's Launchpad branch of the pull request has commits since the agent started and Home does not push it to GitHub yet, the extension offers to push it to the pull request's branch. After your confirmation, Home checks that the branch still points at the commit you saw, reads the pull request on GitHub, pushes the branch only as a fast-forward of its head (never forced) and then follows the branch, so the agent's next commits go to the pull request too, as with [Continue a pull request](#continue-a-pull-request). Home pushes only to the pull request's own repository, your own fork, or a fork whose author lets maintainers edit. Agents can prepare the same click as a ready card (`branch.push-to-pr`).

### Feedback ledger

Home keeps a ledger of what happened to each suggestion, part by part (a review's summary and each line comment): shown, inserted, posted by hand, confirmed or unconfirmed, posted by Launchpad, dismissed, discarded or outdated, with the final text, the size of your edits and whether you kept the attribution line. A “posted by hand” record is only a claim: Home confirms it when it reads the item's comments on GitHub and finds one by you, written after the insert, with at least 85% of the text (it reads them 20 s, 2 min, 10 min and just after an hour after the claim), else marks it unconfirmed after an hour. Nothing reads the ledger yet; it is kept for a later feedback view. No marker is added to what you post.

### Roadmap

- Attachments (images and files) on private notes.
- An `Idempotency-Key` on `note.write`, so a retried click never stores the note twice.
- Timeline ages that update while the page stays open.

## What waits for you

[Today](today.md) lists what waits for your decision in the current workspace:

- **GitHub**: unposted suggestions. Review requests stay on github.com.
- **Review**: finished agents with their branch (**Merge** when it is safe) or their Local PR (**Local PR** opens it), and Local PRs not yet published whose agent does not need you.

The **Branches** tab of a folder page lists the agent branches with work to look at: commits ahead of their base, uncommitted files or a merge request. Each row has **Changes**, which opens the agent's Changes, and **⋯ → Open agent**.

Sessions linked to a GitHub item automatically join a workspace topic named `repository#number` (for example, `huggingface_hub#5352`). Agents can link their own session with `launchpad link --repo owner/repo --number 5352 --session-id <id>` and optionally `--comment-url <url>`. Creating a suggested comment or review also links its session. Repository references without an item number do not create topics. Agents cannot rename or delete topics or link other sessions.

A Local PR body can name issues in its folder’s repository with `Fixes #5352`, `Closes #5352`, or `Resolves #5352`. These references link its session to each issue and show the Local PR beside the session trace and suggestions in the Chrome extension’s box on that issue. Set or edit the body with `launchpad pr create|update --body-file <file>`. Removing a closing reference removes the Local PR from that issue’s box; the session’s historical link remains.

# GitHub

Launchpad replaces the parts of the GitHub UI used every day: notifications, issues, pull requests, reviews and merges. GitHub data is cached locally in `cache.db`, so every screen renders at once and refreshes in the background.

Sign in with `gh auth login` on the machine that runs the daemon. Launchpad reads the token with `gh auth token` and calls the GitHub REST and GraphQL APIs directly.

**Rule: nothing reaches GitHub without your click.** Agents can write comments, reviews and pull requests, but they stay local until you publish them. Once a branch is on GitHub, agents you launch on it push their commits to it (see [Continue a pull request](#continue-a-pull-request)).

## Inbox

![The Inbox with reason filters and a preview](images/inbox.webp)

- Filter by reason: **Review requested**, **Mentioned**, **Assigned**, **Subscribed** or **All**, plus **Unread only** and a repository filter.
- Each row has **✓ Done**, **▶ Launch** (start an agent with this item as context) and **+ Add** (add it to the current draft).
- The preview shows the latest comments and a reply box. **Read/Unread** and **Unsubscribe** are in the preview.
- The sidebar count is unread review requests, mentions and assignments.

GitHub's API cannot mirror the web inbox exactly, so Launchpad tracks inbox membership locally. Read and Done changes made on github.com may not appear.

## Issues and pull requests

![A pull request with an agent suggestion](images/pr.webp)

An item page has **Conversation**, **Changes** and **Review threads** tabs, and a side panel with status, checks, reviewers and labels.

- **Changes** loads the diff from local Git first (`pull/N/head`), with the GitHub API as fallback. Files load one at a time, so very large PRs stay fast. Viewed markers and unsent review comments are saved by the daemon and shared across your devices.
- **Merge** is the primary button only when GitHub reports the PR as mergeable and clean. Otherwise the panel shows the blocking reason.
- **Review with agent** opens the composer with the PR head and review instructions. The agent writes its review as a suggestion. If the PR head changes, the suggested review is blocked.
- Folder pages list the 30 most recently updated open issues and PRs of the folder's repository. Select several to launch one agent with all of them.

## Continue a pull request

Paste a link to a pull request, or to one of its comments, in the composer prompt (or add it as context) and say what to change. When the folder's `origin` or `upstream` is the PR's repository and **Run in** is **New worktree**, **Push to PR #N** is checked by default:

- Launchpad reads the PR on GitHub, fetches its head (`pull/N/head`) and creates the worktree from it.
- The agent commits, then pushes with `git push <remote> HEAD:<PR branch>`. The remote is `origin`, a remote pointing to the PR's branch repository, or that fork's URL derived from `origin`. It never force-pushes; if the branch moved, it rebases on it and pushes again.
- The worktree is never merged locally and cannot become a Local PR: the pull request is the review.
- Uncheck **Push to PR** to keep the work local in a normal worktree.

Pushing to a fork's branch needs write access to it ("Allow edits by maintainers" or your own fork).

## Suggestions

Agents write comments and reviews with the `launchpad suggest` CLI. They are saved as suggestions, never posted:

```sh
launchpad suggest review --repo owner/repo --number 123 --head <commit> --body-file review.md --event approve
```

A suggestion appears as a card on the item's Conversation tab and in **Review → To publish**. **Edit**, then **Post review** (or **Post comment**) publishes it under your account. **Discard** deletes it.

## Local PRs

A Local PR is a pull request that exists only on your machine: a branch in its own worktree, a title, a description and review notes. Publish it to GitHub when it is worth it, or merge it locally.

![A Local PR with a note for the agent](images/localpr.webp)

```mermaid
flowchart LR
    A["Launch with<br/>Run in: New Local PR"] --> B[Agent commits on lp/branch]
    C["launchpad pr create<br/>(from an agent)"] --> B
    B --> D[Review: Description, Changes, Commits]
    D -->|Notes| E[Agent updates the branch]
    E --> D
    D --> F[Publish to GitHub]
    D --> G[Merge locally]
    D --> H[Discard]
```

- **Create**: choose **Run in → New Local PR** in the composer, or let an agent run `launchpad pr create`.
- **Review**: the agent keeps the title and description up to date with `launchpad pr update`. **Notes** (general or per file) go back to the agent as one message.
- **Publish**: choose a remote branch name and GitHub draft or ready, then confirm. Launchpad pushes the branch and creates the PR. Check **From my fork** when you cannot push to `origin`: Launchpad pushes to your existing GitHub fork (a configured remote, or the origin URL with your login) and opens the PR from it. While a publication runs, agents cannot change the branch. After publication, the Local PR stays open to agents: its agent (or a new one launched in it) pushes new commits to the published branch. If the push fails and the remote shows that nothing arrived, the Local PR unlocks itself. Otherwise a banner offers **Unlock**: agents can work again, and a branch that was already pushed keeps its name for the next Publish.
- **Merge locally**: rebases the branch onto the local base branch and fast-forwards it. Then it tries to push the merged base branch to its upstream (or the same branch on `origin`) in the background. Push failure keeps the local merge and is shown on the Local PR; it never forces or retries automatically.

Publication and merges wait until the agents are idle and the worktree is clean.

## Review

**Review** collects everything waiting for your decision, scoped to the current workspace:

- **To publish**: unposted suggestions and unpublished Local PRs.
- **Agent branches**: worktrees with unmerged work from finished agents.
- **Review requests**: PRs where your review is requested.

![The Review page](images/review.webp)

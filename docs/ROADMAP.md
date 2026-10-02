# Launchpad roadmap

Status: proposal (2026-10-02). See [ARCHITECTURE.md](ARCHITECTURE.md) for the design behind each item and [design/sketches.html](design/sketches.html) for the UI.

## How to read this

- Phases are ordered by value and by risk. Each one ends with something you use every day, so the app replaces part of your current workflow as early as possible.
- There are no dates. Each phase has a **Done when** line: a concrete check from your own routine, done by using the app.
- Tests are kept to the minimum (see [ARCHITECTURE.md, section 5.2](ARCHITECTURE.md#52-testing)). No phase includes writing a test suite.
- Phase 0 starts with short spikes on the unknowns. Each spike answers one question, and its notes go into `docs/spikes/`. If a spike fails, its fallback is listed so the plan does not stall.
- Agents come before GitHub because launching agents is the main job of the app and the agent adapters carry the most technical risk.

## Overview

| Phase | Theme | Main result | Status |
|---|---|---|---|
| 0 | Foundations and spikes | Dock icon opens a window served by the daemon, with versions and quotas in the status bar | Done |
| 1 | Launch and run agents | You stop launching agents from the terminal | In progress |
| 2 | GitHub inbox | You triage notifications without opening github.com | Not started |
| 3 | Issues and pull requests | Reviewing and merging a PR is faster than on github.com | Not started |
| 4 | Pre-draft PRs | Exploratory PRs no longer reach GitHub | Not started |
| 5 | Workspace polish | Several sessions are easy to arrange, find and resume | Not started |
| 6 | Agent-written suggestions | Agents draft comments and reviews; you post them in one click | Not started |
| 7 | Hardening and options | Sessions survive restarts, embedded terminal, remote access | Not started |

Agents keep the Status column up to date (Next, In progress, Done) and note what is done in each phase section.

## Phase 0: foundations and spikes

**Spikes** (timeboxed, before writing product code):

1. **Claude stream-json:** several turns on one process; a message sent while a turn runs; `interrupt` control request; `--resume` continuity; content of `rate_limit_event` on your plan; how AskUserQuestion behaves with `--dangerously-skip-permissions` in print mode. *Fallback:* drive Claude through the TypeScript Agent SDK in a small Node sidecar.
2. **Claude usage endpoint:** response shape of `/api/oauth/usage`, behavior when the access token expires. *Fallback:* quotas only from `rate_limit_event` while a session runs.
3. **Codex app-server:** `thread/start` with `approvalPolicy: never` and full access, `turn/steer`, `turn/interrupt`, `thread/resume` after the app-server restarts, mapping of item events to the normalized event list. *Fallback:* `codex exec --json` plus `codex exec resume` for each turn.
4. **WebKitGTK:** render a 5,000-line virtualized diff and a 2,000-event session; check scrolling smoothness and memory. *Fallback:* a Chromium `--app=` window instead of the Tauri webview (same UI, nothing else changes).
5. **Claude TUI trust prompt:** does `claude --resume` in a new temporary folder ask for trust, and how can it be skipped safely?

**Build:**

- Cargo workspace (`core`, `server`, `cli`), Vite React app, Tauri shell.
- Daemon: `config.toml` loading and watch (including `projects_root`, default `~/projects`), `state.db` and `cache.db`, axum API, WebSocket with an event bus, auth (token file, cookie handshake, `Host` and `Origin` checks), static UI serving, dev and prod profiles.
- Packaging: systemd user unit, `.desktop` file, `packaging/install.sh` (build release, install to `~/.local/bin`, restart the service, warn if agents are running).
- App shell: sidebar, top bar with the Launch button, empty views, status bar with Claude and Codex versions, Codex quota (and Claude quota if spike 2 succeeds).

**Done when:** Launchpad is pinned in the dock, opens in under 300 ms with a warm daemon, and the status bar shows both versions and quotas.

**Progress:**

- All five spikes done on 2026-10-02, no fallback needed. Notes: [1](spikes/1-claude-stream-json.md), [2](spikes/2-claude-usage-endpoint.md), [3](spikes/3-codex-app-server.md), [4](spikes/4-webkitgtk.md), [5](spikes/5-claude-trust-prompt.md). The findings are in the architecture (decisions 11 to 16).
- Build done: Cargo workspace, daemon (config watch, `state.db`, `cache.db`, axum API, WebSocket event bus, token and cookie auth with `Host` and `Origin` checks, embedded UI, dev and prod profiles, login shell environment), React app shell (sidebar, top bar with the Launch button, empty views, status bar with versions and both quotas), Tauri shell (single instance, warm hidden window), packaging (`packaging/install.sh`, systemd user unit, `.desktop` entry, autostart entry, icons).
- Measured: the window opens in about 100 ms from a dock click with the warm shell (cold start of the shell: about 0.5 s).

## Phase 1: launch and run agents

- **Projects:** registry, "Scan projects root" discovery, descriptions, pinning, repo ↔ project mapping.
- **Composer:** harness toggle, project picker, free text, presets (the 3 initial ones) with a settings editor, context projects with a branch picker (local branches), GitHub links added with a "+ GitHub link" field (title fetched), model and effort pickers showing "Default (…)", prompt preview, Launch button, unsent drafts kept. In the prompt, `@` suggests projects and `#` suggests issues and PRs (only at the start of a word).
- **Radial launcher:** Launch button in the top bar and on notifications, issues, PRs and projects; two round buttons with the Claude and Codex logos (no labels); the last harness used has a ring.
- **Temporary projects:** create, clean up, promote (including moving Claude transcripts).
- **Adapters:** Claude and Codex, normalized events, event log in `state.db`, WebSocket streaming with replay.
- **Agents view:** list grouped as Running, Pending (waiting for your input, or idle while a background command runs) and Closed; session pane (messages, collapsed tool calls, inline diffs, copy buttons, input, interrupt, close, resume), layouts with 1, 2 and 4 panes.
- **Signals:** tokens, context usage and cost per session; desktop notification when a session becomes pending.
- **Maintenance:** background `codex update`; "Open in terminal" and "Re-attach".
- **Settings → General:** projects root, terminal used by "Open in terminal" (Terminator by default, Ghostty, GNOME Terminal or a custom command), editor.

**Done when:** for one week, every agent you start goes through Launchpad.

## Phase 2: GitHub inbox

- GitHub client: token from `gh`, ETag store, GraphQL batching, rate-limit tracking.
- Notification poller with `If-Modified-Since` and `X-Poll-Interval`, periodic full reconciliation, enrichment (state, participants, review decision, checks).
- Inbox: repo filter chips with counts, unread-only toggle, two-line rows (reason, participants, time since update), hover action buttons (done, unsubscribe, read, local unread) with optimistic updates.
- Preview pane: cached summary of the selected thread (description, latest comments), quick reply.
- Launch from a notification: the project is chosen from the repo mapping and the thread is added as context.
- Prefetch of the top unread threads.

**Done when:** a full morning of notification triage happens in Launchpad, and github.com is only opened for CI logs.

## Phase 3: issues and pull requests

- Detail query and timeline renderer: `bodyHTML`, sanitization, image proxy with a disk cache.
- Issue view: conversation, close as completed or not planned, reopen, comment.
- PR view: header, conversation, checks summary with links, reviewers and request review, labels, merge box (method remembered per repo), review (comment, approve, request changes), convert to draft and ready for review, close.
- Files changed: local git diff through PR refs, API fallback, virtualization, syntax highlighting, unified and split views, "viewed" checkboxes.
- Project page: latest open PRs and issues (cached on demand), multi-select to launch an agent with the selection, Refresh and Open on GitHub everywhere.
- Composer "cart": while the composer is open, every issue, PR and comment shows "+ Add".

**Done when:** you review and merge a typical PR in Launchpad faster than on github.com, and moving between items never shows a loading page.

## Phase 4: pre-draft PRs

- `predraft` table and API; branch and worktree management under `<projects_root>/.worktrees/`.
- Composer target "Pre-draft PR".
- `launchpad pr create|update|show|list` CLI, and the `launchpad` skill linked into `~/.agents/skills`.
- Pre-draft PR view (teal), listed with the GitHub PRs of the project.
- Review notes (general and per file) and "Send feedback to agent", which resumes the same session.
- Publish as draft or ready for review (with remote branch name), discard, "base moved" banner.

**Done when:** for two weeks, no exploratory draft PR is opened on GitHub and then closed.

## Phase 5: workspace polish

- Drag and drop sessions into panes, saved layouts, focus movement between panes, session rename, pin and search.
- Import sessions started outside Launchpad (Claude transcripts in `~/.claude/projects/`, Codex `thread/list`) as history that can be resumed.
- Git panel per session (status and diff of its cwd), "Open in editor".
- Composer: recent contexts offered as one-click chips.

**Done when:** running 4 agents at once feels comfortable, and finding last week's session takes seconds.

## Phase 6: agent-written suggestions

- `launchpad suggest comment|review` CLI and the matching skill section.
- Draft comment cards in issue, PR and pre-draft timelines: edit, post, discard.
- "Review with agent" quick action on PRs that produces a suggested review.

**Done when:** you post agent-drafted reviews after editing them, and nothing an agent writes reaches GitHub without your click.

## Phase 7: hardening and options

Pick items as they become useful:

- Sessions that survive daemon restarts (session shim, or Codex's shared app-server daemon).
- Embedded terminal mode (xterm.js + PTY) for full TUI fidelity when needed.
- Remote access through a tunnel and a browser.
- Another harness (one adapter module).
- Inline review comments.
- Optional inlining of cached GitHub threads in prompts.

## Not planned

Keyboard shortcuts (for now), a large test suite or mocks, dark theme, macOS and Windows, multiple users, GitHub search, Actions logs, release versioning, data migrations for the cache.

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
| 2 | GitHub inbox | You triage notifications without opening github.com | In progress |
| 2b | Code viewer | You read code (an agent's worktree, a branch, a PR head) without opening an editor | In progress |
| 3 | Issues and pull requests | Reviewing and merging a PR is faster than on github.com | In progress |
| 4 | Pre-draft PRs | Exploratory PRs no longer reach GitHub | In progress |
| 5 | Workspace polish | Several sessions are easy to arrange, find and resume | In progress |
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

- Basic CI implemented for `Wauplin/launchpad`: three Ubuntu jobs for strict Clippy and UI types, existing Rust tests, and release binaries with the embedded UI plus downloadable build artifacts. Workflow syntax and commands checked locally; the first hosted run is pending the GitHub push.
- All five spikes done on 2026-10-02, no fallback needed. Notes: [1](spikes/1-claude-stream-json.md), [2](spikes/2-claude-usage-endpoint.md), [3](spikes/3-codex-app-server.md), [4](spikes/4-webkitgtk.md), [5](spikes/5-claude-trust-prompt.md). The findings are in the architecture (decisions 11 to 16).
- Build done: Cargo workspace, daemon (config watch, `state.db`, `cache.db`, axum API, WebSocket event bus, token and cookie auth with `Host` and `Origin` checks, embedded UI, dev and prod profiles, login shell environment), React app shell (sidebar, top bar with the Launch button, empty views, status bar with versions and both quotas), Tauri shell (single instance, warm hidden window), packaging (`packaging/install.sh`, systemd user unit, `.desktop` entry, autostart entry, icons).
- App icon updated to the selected sculpted launch artwork, with the unchanged master, PNG sizes, WebP and SVG exports, a browser favicon, the top bar logo, and an icon-only installer that does not restart running agents.
- Measured: the window opens in about 100 ms from a dock click with the warm shell (cold start of the shell: about 0.5 s).

## Phase 1: launch and run agents

- **Projects:** registry, "Scan projects root" discovery, descriptions, pinning, repo ↔ project mapping.
- **Composer:** harness toggle, project picker, free text, presets (the 3 initial ones) with a settings editor, context projects with a branch picker (local branches), GitHub links added with a "+ GitHub link" field (title fetched), model and effort pickers showing "Default (…)", prompt preview, Launch button, unsent drafts kept. In the prompt, `@` suggests projects and `#` suggests issues and PRs (only at the start of a word).
- **Delayed starts:** Now, a duration, a local date and time, all selected agents Done, or a reported Claude/Codex 5h or weekly quota reset; durable local queue and Scheduled group with Start now and Cancel.
- **Direct launcher:** Launch buttons open the composer sidebar with the harness, model and effort selections from the latest launched agent.
- **Temporary projects:** create, clean up, promote (including moving Claude transcripts).
- **Adapters:** Claude and Codex, normalized events, event log in `state.db`, WebSocket streaming with replay.
- **Agents view:** list grouped as Done (waiting for input), Running (including background and long commands) and Archived; session pane (messages, collapsed tool calls, inline diffs, copy buttons, input, interrupt, archive, close panel, resume), layouts with 1, 2 and 4 panes.
- **Signals:** tokens, context usage and cost per session; desktop notification when a session becomes Done.
- **Maintenance:** background `codex update`; "Open in terminal" and "Re-attach".
- **Settings → General:** projects root, terminal used by "Open in terminal" (Terminator by default, Ghostty, GNOME Terminal or a custom command), editor.

**Done when:** for one week, every agent you start goes through Launchpad.

**Progress:**

- Built on 2026-10-02, and checked against the real CLIs (Claude Haiku and Codex GPT-6-Luna) through the API and the UI:
  - Projects: registry, "Scan projects root" (149 repositories in 48 ms), descriptions, pinning, add by path, repo → project mapping (origin, then upstream).
  - Composer: harness toggle, project picker (or a new temporary folder), prompt with `@` and `#` suggestions, context projects with a branch picker, "+ GitHub link" with the title fetched, the three presets and their settings editor, model and effort pickers showing "Default (…)", live prompt preview, "Open after launch", draft kept on the daemon.
  - Direct launcher on the top bar, projects and notifications, with the latest launched harness, model and effort selections remembered.
  - Temporary projects: create, delete with a second click, promote (the folder, the Claude transcripts and the session folders move).
  - Adapters: Claude and Codex, normalized events in `state.db`, WebSocket streaming with replay after a reconnect.
  - Agents view: independently collapsible Done, Running and Archived groups with remembered expansion state; session pane with messages, collapsed tool calls, inline diffs, plan, question cards, background commands with their output, copy buttons, message box (`Enter` sends), Stop, Archive, Close panel, Resume, Open in terminal and Re-attach; layouts with 1, 2 and 4 panes.
  - Signals: context tokens and usage, cost (Claude), desktop notification when a session becomes Done.
  - Maintenance: `codex update` at daemon start and every 6 hours.
  - Settings → General (projects root, terminal with "found" markers, editor) and Presets.
- Usability updates on 2026-10-02: direct Launch with remembered selections; image attachments by file picker, console drop and clipboard paste for both harnesses; larger 120% default UI with stronger contrast and five saved zoom positions; Done/Running/Archived with long commands kept Running; separate colored sidebar counts and simultaneous project dots/spinners; virtualized session list; AI-generated session titles based on the initial prompt.
- The bottom status bar displays the running Launchpad commit and offers Update when local main has newer commits. Updates build in the background from clean committed main and reserve idle agents before reinstalling; Settings shares the same updater and progress.
- Settings → General now offers a production update button that runs UI type checks and Rust tests, builds release assets, then installs and relaunches from the local checkout. The updater runs outside the daemon and reports its stage and failure log in the UI.
- Checked these updates in the dev browser and desktop: all five saved zoom positions, native Ctrl+V image paste, console drop and removal, native image delivery to Claude Haiku and Codex GPT-6-Luna (both identified a red test image), AI titles from both harnesses, saved launch selections across reloads, simultaneous Done and Running indicators on a pinned project, image-only input on a resumed Codex session, and a silent 35-second Codex command that stayed Running. UI and desktop builds and the five existing Rust tests pass.
- Agent list update on 2026-10-02: Done, Running and Archived headings independently collapse their rows, keep counts visible and remember expansion across navigation and reloads. Verified against real dev sessions in the browser at all five zoom positions; UI type checks and build pass.
- Session panel update on 2026-10-02: the X only closes the panel, preserving Done or Running; a separate Archive button keeps conversations resumable in the renamed Archived group. Verified in the dev browser: X preserves session data, Archive offers Resume, and X closes an archived panel with one click.
- Checked delayed starts in an isolated dev daemon and browser: a real Codex timed launch, a two-agent Done dependency, reported Codex quota snapshots, manual start and cancellation, future queue persistence and overdue launch after restart, all selector modes and validation at every saved zoom position, independently collapsing Scheduled, and closing its panel without cancelling. Combined checks confirm that code-viewer file references survive a scheduled launch, deleted projects produce a visible queued failure, and ready/manual starts remain queued under the installation guard until it is released. Strict workspace Clippy, UI type checks/build, and the nine fast Rust tests pass.
- Delayed launches added on 2026-10-02: a compact Start selector, saved delay drafts, a durable local queue, and a virtualized, independently collapsible Scheduled group with condition details, prompt preview, Start now, and Cancel. Time and quota reset deadlines are captured at scheduling time; all selected dependency agents must be Done simultaneously. Queued starts survive daemon restarts; interrupted claims require manual attention instead of automatic duplicate launches.
- Left for the **Done when** check: a week of daily use.

## Phase 2: GitHub inbox

- GitHub client: token from `gh`, ETag store, GraphQL batching, rate-limit tracking.
- Notification poller with `If-Modified-Since` and `X-Poll-Interval`, periodic full reconciliation, enrichment (state, participants, review decision, checks).
- Inbox: repo filter chips with counts, unread-only toggle, two-line rows (reason, participants, time since update), hover action buttons (done, unsubscribe, read, local unread) with optimistic updates.
- Preview pane: cached summary of the selected thread (description, latest comments), quick reply.
- Launch from a notification: the project is chosen from the repo mapping and the thread is added as context.
- Prefetch of the top unread threads.

**Done when:** a full morning of notification triage happens in Launchpad, and github.com is only opened for CI logs.


**Progress:**

- Built on 2026-10-02: direct GitHub REST and batched GraphQL client, token from `gh` with a retry on 401, persistent ETag/Last-Modified response cache, REST and GraphQL rate budgets in the status bar, polling that respects `X-Poll-Interval`, and a full paginated reconciliation at startup and every 30 minutes.
- Corrected after initial daily use: `all=true` includes Done history. The default inbox now uses durable local membership, excludes explicit ignored/unsubscribed threads, and can be restored from a known GitHub inbox snapshot. Verified the restored four-item inbox (three unread) in the dev API and browser. The public API cannot continuously mirror Read/Done actions taken on github.com.
- Inbox: virtualized fixed-height two-line rows and repository chips with counts, unread-only filter, participants, state, review and check summaries, optimistic Done, Unsubscribe (ignore future notifications), Read and local Unread actions with rollback on failure. Local overrides live in `state.db`, expire on new activity and survive daemon restarts and cache rebuilds.
- Preview: cached description and latest eight issue/PR comments, background refresh, explicit Refresh and Open on GitHub, and quick replies published only by clicking Post reply. Release and commit notifications have REST summaries; other subject types link to GitHub. Top unread threads are enriched first and nearby preview caches are prefetched. Launch buttons choose the mapped origin/upstream project and add the thread as context, including subjects without an issue number.
- Checked with the dev daemon against the real GitHub account: 240 notifications, 230 enriched issues/PRs, conditional polling and cached previews; UI and desktop builds, existing Rust tests, browser layout, unread filters, release links and composer context checks, and local unread persistence across a dev-daemon restart. Reply publishing and ignoring real threads were not exercised during automated verification.
- Left for the **Done when** check: a full morning of daily notification triage.

## Phase 2b: code viewer

Added on 2026-10-02. It only needs Phase 1, and it comes before Phase 3, which reuses its code renderer for Files changed. Design in [ARCHITECTURE.md, section 8.1](ARCHITECTURE.md#81-code-viewer) and sketch S7.

- Daemon `code` module: sources (checkout, worktrees, branches, tags, fetched PR refs), file list with change letters, file content with changed line ranges, `git grep` search, path checks.
- Viewer screen `/projects/{id}/code`: source picker, virtualized file tree with "Go to file" and "Changed only", tabs, code lines (virtualized, fixed row height, Shiki in a Web Worker) with bars on changed lines, Markdown preview, images.
- Find bar: Find button and `Ctrl+F`, match case, whole word, regex; `Enter`, `Shift+Enter` and `Escape` in its field.
- Search in project, grouped by file.
- Line selection: copy, copy reference, GitHub link, open in editor at the line, add to agent (composer chip or a session's message box).
- Ways in: Code button on project pages and worktrees, Files button in session headers, linked paths in tool calls, file changes and agent messages. "View file" on diffs comes with Phases 3 and 4.
- The open file reloads when a session working in the same folder edits it.

**Done when:** for a week, reading code (an agent's worktree, a branch, a file an agent mentions) happens in Launchpad, and the editor is opened only to change code.

**Progress:**

- Built on 2026-10-02:
  - Daemon `code` module and `/api/code/*`: sources (checkout, `git worktree list`, local and remote branches, tags, `refs/launchpad/pr/*`), file lists with M, A and U letters since the merge base with the default branch, file contents with changed line ranges (modified, added, deleted), images, binary and size limits, `git grep` search capped at 1,000 results and cancelled with its request, the commit and its GitHub repository for permalinks. Roots must be registered projects or their git worktrees, paths stay inside the root, and revisions are resolved to a commit id first.
  - Viewer `/projects/{id}/code`: source picker, virtualized tree with compact folders, "Go to file" and "Changed only", tabs, fixed-height virtualized code lines with Shiki colors from a Web Worker, change bars, Markdown preview (relative images and links resolved in the same source), images.
  - Find bar: Find button and `Ctrl+F`, match case, whole word, regex, `Enter`, `Shift+Enter` and `Escape`; matches over the whole file.
  - Search in project as you type, grouped by file with four lines until "Show more".
  - Line selection: Copy, Copy reference, GitHub link (copied; only when a remote branch has the commit and the file has no uncommitted change), Add to agent (composer chip or a session's message box), Editor at the line (working trees).
  - Ways in: Code on project pages and worktrees, Files in session headers, linked file paths in tool calls, file changes and inline code of agent messages.
  - The open file and the file list reload when a session reports a `file_change` in the same working tree.
- Checked with the dev daemon and Chrome through the DevTools protocol: this repository, its `code-viewer` worktree and branch, huggingface_hub (503 files in 0.3 s, a 15,442-line file from `origin/main`, permalinks to `huggingface/huggingface_hub`), an empty repository without commits; colors, find navigation, selection by click and shift-click, Go to file, search, README preview, image, Changed only, the composer prompt with a file chip, a session message box reference, and the 150% zoom position. A Claude Haiku session that edited the open file reloaded it in place within 5 seconds, with the scroll position kept. Opening the editor was not exercised during automated checks.
- Left for the **Done when** check: a week of daily use. "View file" on diffs comes with Phases 3 and 4.

## Phase 3: issues and pull requests

- Detail query and timeline renderer: `bodyHTML`, sanitization, image proxy with a disk cache.
- Issue view: conversation, close as completed or not planned, reopen, comment.
- PR view: header, conversation, checks summary with links, reviewers and request review, labels, merge box (method remembered per repo), review (comment, approve, request changes), convert to draft and ready for review, close.
- Files changed: local git diff through PR refs, API fallback, virtualization, syntax highlighting (the code renderer of Phase 2b), unified and split views, "viewed" checkboxes, "View file" in the code viewer.
- Project page: latest open PRs and issues (cached on demand), multi-select to launch an agent with the selection, Refresh and Open on GitHub everywhere.
- Composer "cart": while the composer is open, every issue, PR and comment shows "+ Add".

**Done when:** you review and merge a typical PR in Launchpad faster than on github.com, and moving between items never shows a loading page.

**Progress:**

- Built on 2026-10-02: cached issue and PR detail routes, paginated GraphQL timelines and read-only review threads, sanitized GitHub `bodyHTML`, raw Markdown retained, and a disk image/avatar cache with a restricted authenticated proxy. Cached items render immediately and refresh in the background; unchanged timelines reuse their cache while headers and checks refresh. The top 15 unread inbox items and nearby selected rows are prefetched.
- Issue controls: comment, close as completed or not planned, reopen. PR controls: comments and reviews (comment, approve, request changes), check summary and log links, reviewers and team review requests, labels, draft/ready, close/reopen, and merge/squash/rebase with a remembered method per repository. Reviews and merges include the displayed head commit; merging requires a second explicit click. Unsent comment/review text survives navigation and reloads.
- Files changed: fetched PR heads in the private ref namespace, three-dot local git diffs from the exact base/head commits through the matching origin/upstream remote, a paginated API fallback, cached by commit pair. Virtualized file list and diff rows, syntax colors from the existing worker, unified and synchronized split views, viewed markers scoped to the head commit, and View file in the code viewer (GitHub for an unmapped repository). Deleted files open at the base commit.
- Project pages list the 30 most recently updated open PRs and issues, cache for five minutes, and support selection across both tabs to launch with several context references. Full conversation opens from inbox previews; issue, PR and comment cards expose + Add while the composer is open. Refresh and Open on GitHub controls remain visible.
- Checked against the real account in the dev daemon and Chrome: 30 PRs and 30 issues from huggingface_hub, full issue/PR conversations, a reviewed PR with 15 timeline entries and three review threads, disk-cached avatars, a three-file local diff, and an eight-file API fallback from an unmapped repository. Browser checks confirmed multi-select composer references, syntax colors, unified/split views, viewed persistence, View file, unsent draft persistence, composer cart controls and review-thread rendering with no browser errors. Diff controls and virtualization were also checked at all five zoom positions. The dev desktop window starts successfully. Strict workspace Clippy, UI type checks/build, the desktop build and ten fast Rust tests pass. Publishing controls are implemented but deliberately not exercised against real GitHub items during automated checks.
- Left for the **Done when** check: review and merge a typical PR in daily use, and compare the workflow speed to github.com.

## Phase 4: pre-draft PRs

- `predraft` table and API; branch and worktree management under `<projects_root>/.worktrees/`.
- Composer target "Pre-draft PR".
- `launchpad pr create|update|show|list` CLI, and the `launchpad` skill linked into `~/.agents/skills`.
- Pre-draft PR view (teal), listed with the GitHub PRs of the project.
- Review notes (general and per file) and "Send feedback to agent", which resumes the same session.
- Publish as draft or ready for review (with remote branch name), discard, "base moved" banner.

**Done when:** for two weeks, no exploratory draft PR is opened on GitHub and then closed.

**Progress:**

- Built on 2026-10-02: durable local pre-draft records and notes in `state.db`; fetch of the origin default branch; unique `lp/<slug>` branches and worktrees under `<projects_root>/.worktrees/<project-id>/`; new and existing pre-draft composer targets; exact CLI path/profile instructions in the launch and feedback prompts.
- `launchpad pr create|update|show|list` implemented, with title/body-file inputs and worktree lookup through the existing registered-root checks. Creating from an existing checkout copies its committed HEAD into a new worktree and preserves the original checkout. A shared repository skill is linked into `~/.agents/skills/launchpad` at daemon startup. The old draft-PR preset is replaced by the composer target.
- Teal local rows share the project PR list with GitHub PRs, with a toggle for discarded records and links from session panels. The local review page has an editable description, virtualized commits, the existing colored unified/split diff renderer and viewed markers, full-file navigation, saved general/file notes, and feedback that resumes the linked session. CLI-created records offer Launch agent.
- Explicit confirmed publication takes a remote branch name (default `<github-user>/<slug>`) and draft/ready choice, checks the displayed commit and a clean idle worktree, pushes without force, and creates the GitHub PR. Publication checkpoints persist for retries and matching PR lookup avoids a duplicate after an uncertain response. Published records keep the GitHub link; Settings can remove a clean worktree afterward. Confirmed discard removes the local worktree and branch and archives the record. Refresh fetches the base and displays its ahead count.
- Checked in the dev daemon and Chrome using a throwaway repository and local bare origin: real Codex GPT-6-Luna launch, committed edits and CLI title/body updates, same-session file feedback and a second commit, teal list rows, colored diffs, split view, viewed markers, full-file navigation, saved notes across reloads, all five zoom positions, base advancement, dirty-worktree refusals, CLI creation from existing committed work, original-checkout preservation, discard cleanup, and persistence across a dev restart. Strict Clippy, UI type checks/build, desktop build, and the ten existing fast Rust tests pass. Real GitHub push/PR creation and production installation were not exercised.
- Left for the **Done when** check: two weeks of daily use, including publication on an intended PR.


## Phase 5: workspace polish

- Drag and drop sessions into panes, saved layouts, focus movement between panes, session rename, pin and search.
- Import sessions started outside Launchpad (Claude transcripts in `~/.claude/projects/`, Codex `thread/list`) as history that can be resumed.
- Git panel per session (status and diff of its cwd; reading its files is the code viewer of Phase 2b), "Open in editor".
- Composer: recent contexts offered as one-click chips.

**Done when:** running 4 agents at once feels comfortable, and finding last week's session takes seconds.

**Progress:**

- Built on 2026-10-02: session list and title drag/drop into any pane (swap when both are occupied), visible Focus pane and Move to pane controls, up to twelve named saved layouts with restore/delete, persistent pins sorted first within each state group and a Pinned filter, manual rename, and search over titles, projects, working directories, initial prompts and stored conversation messages. Group counts reflect the current filter and long lists remain virtualized.
- Explicit Import history discovers paginated Claude transcripts and Codex threads (including Codex's archived history), excludes already imported conversations, and stores selected conversations atomically as Archived snapshots without launching an agent. Resume uses the original harness conversation id and cwd. Imported history does not change launch defaults. Native transcripts remain untouched; complete a conversation in its other client before resuming it here.
- Each session has a read-only Git panel with branch, two-column status, renames, selected-file staged and working-tree diffs, safe untracked-file reading through the code viewer, five-second refresh and virtualized file/diff rows. Files opens the existing code viewer; Open in editor opens the session folder.
- Composer offers six recent context chips from launched sessions, covering projects and their selected branches, GitHub links and file references. Already selected contexts are omitted.
- Checked with real external Claude Haiku and Codex GPT-6-Luna conversations: discovery, duplicate prevention, imported messages, manual rename, stored-message search, and resume with remembered conversation context. Four real agents ran together and finished cleanly. Dev browser checks cover pins, filters, staged/unstaged/untracked patches, drag/drop, move and focus, named layout restore after reload, and import selectors. All five zoom sizes fit the four-pane workspace, and recent context chips add context in one click. Imported event history, renamed titles and message search survive a dev-daemon restart. The dev desktop opens successfully. Strict workspace Clippy, UI type checking/build, desktop build and the eleven fast Rust tests pass. A small UTF-8 output truncation regression test covers large Unicode patches.
- Left for the **Done when** check: daily use with four agents and older conversations.

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
- Light editing in the code viewer (CodeMirror 6), if reading is not enough.

## Not planned

Keyboard shortcuts other than `Ctrl+F` in the code viewer (for now), a large test suite or mocks, dark theme, macOS and Windows, multiple users, GitHub search, Actions logs, release versioning, data migrations for the cache.

# Launchpad architecture

Status: Phase 0 done; Phases 1, 2, 2b, 3, 4, 5 and 6 built and awaiting their daily-use acceptance checks. Phase 7 is deferred after its 2026-10-03 implementation was reverted; its candidate ideas require a new scope before implementation. Main decisions confirmed (see [Decisions log](#17-decisions-log)). Companion documents: [ROADMAP.md](ROADMAP.md), the UI sketches in [design/sketches.html](design/sketches.html), and the spike notes in [spikes/](spikes/).

## 1. What Launchpad is

Launchpad is a desktop app for Linux (primary target), macOS and Windows, launched from the dock, that combines two things:

- an **agent launcher and workspace** for Claude Code and Codex: start a session in the right folder with the right context in a few clicks, run several sessions and view one at a time per window;
- a **fast GitHub client** for the parts of GitHub used every day: notifications, issues, pull requests, reviews, merges. Everything is cached locally so moving between items never reloads a page.

On top of these, Launchpad adds **local-first objects** that GitHub does not have: pre-draft PRs (a branch + title + description that exists only on the laptop) and, later, agent-written comments and reviews that wait for manual approval before being posted.

A read-only **code viewer** connects both sides: it shows the files of a project, of an agent's worktree, of a branch or of a PR head, and turns selected lines into context for an agent (see [8.1](#81-code-viewer)).

It is a single-user tool. Light theme only. Linux is the primary target, with macOS and Windows support. No versioning, no backward compatibility, no migration framework.

### Design principles

1. **Never wait on the network to show something.** Every screen renders from the local database first and refreshes in the background. Network latency appears as a small "syncing" indicator, never as a blank page.
2. **Launching an agent is the main action.** A minimal launch is three clicks plus the prompt. A rich launch uses the same flow with more chips. Both must feel natural.
3. **The core does not depend on a harness.** Claude Code and Codex are adapters behind one session model. Adding a third harness means writing one adapter.
4. **Local first, publish later.** Anything that would create noise on GitHub (pre-draft PRs, agent-written comments) stays local until you publish it. These objects share one color in the UI (teal).
5. **Click only.** No keyboard shortcuts, at least at first, with one exception: `Ctrl+F` opens the find bar in the code viewer (see [8.1](#81-code-viewer)). Every action is a visible button where it is needed, and the UI should be good enough that shortcuts are not missed. Text fields only type text, with three exceptions: in the composer prompt, `@` and `#` at the start of a word open a suggestion list (projects, issues and PRs, see [9.7](#97-prompt-composition)); in the session message box `Enter` sends, `Shift+Enter` adds a new line and standard clipboard paste accepts images; in the find field of the code viewer, `Enter` and `Shift+Enter` go to the next and previous match and `Escape` closes the bar.
6. **Owned and easy to extend.** A mainstream stack that coding agents handle well (Rust, React, TypeScript, Tailwind), configuration in a plain TOML file, GitHub payloads stored as JSON so new fields need no schema change.
7. **Move fast, test little.** This is a personal project: speed of change matters more than protection against breakage. Most code has no tests. See [5.2](#52-testing).

## 2. Key decisions

| Topic | Decision | Why |
|---|---|---|
| Process model | A background daemon (`launchpad daemon`, systemd on Linux, launchd on macOS, detached on Windows) plus a thin Tauri window | Agents keep running when the window is closed or the UI is rebuilt. The daemon is always warm, so the window opens instantly. The same server can later run on a remote machine. |
| UI ↔ backend | HTTP + WebSocket on `127.0.0.1`. The Tauri window loads the UI from the daemon | One transport for desktop and future remote access. The frontend has no Tauri dependency and also runs in a normal browser. |
| Agent integration | Structured protocols: `claude -p` with stream-json, `codex app-server` (JSON-RPC) | Launchpad renders the conversation itself: exact copy of messages, token and cost numbers, cards for questions. An "Open in terminal" button covers features that only exist in the TUI. |
| Frontend | React + TypeScript + Vite, TanStack Query / Router / Virtual, Tailwind, Radix primitives | Largest ecosystem, and the stack that coding agents write best, which matters for a codebase you extend with agents. |
| Storage | SQLite (WAL). `state.db` for your data, `cache.db` for GitHub data | The cache can be deleted and rebuilt on any schema change, so no migrations are needed for it. |
| GitHub access | Token from `gh auth token`, direct REST + GraphQL calls from Rust | Much faster than spawning `gh` for each call. GraphQL fetches participants and PR details in batches. |
| Diffs | Local git first (fetch `pull/N/head`), GitHub API as fallback | Instant, handles very large PRs, and the same code path serves pre-draft PRs. |
| Code viewer | Read-only, rendered by Launchpad with the code lines of the diff view (TanStack Virtual, Shiki in a Web Worker). Files come from disk and git | Reading agent work and looking up code are frequent; editing stays in the editor and with the agents. One code renderer, tuned for WebKitGTK, serves files and diffs. |
| Pre-draft PRs | Launchpad creates the branch and worktree; agents use a `launchpad pr` CLI documented by a shared skill | Deterministic paths, no GitHub noise, 1-click publish or discard. |
| Configuration | `~/.config/launchpad/config.toml` (settings and presets), editable in the UI and by hand | Can be versioned in dotfiles and edited by agents. |
| Projects root | Setting `projects_root`, default `~/projects` | Used for discovery, temporary folders and worktrees. Every path in this document that starts with `~/projects` means "the projects root". |

## 3. System overview

```
                          ┌───────────────────────────────────────────┐
  Ubuntu dock ──click──▶  │ launchpad-desktop (Tauri v2, WebKitGTK)   │
                          │ native window, single instance            │
                          │ loads http://127.0.0.1:7317/              │
                          └─────────────────────┬─────────────────────┘
                                                │ HTTP (REST) + WebSocket (events)
  agent shell ──▶ launchpad CLI ────────────────┤ cookie (window) / bearer token (CLI)
  (pr update, suggest…)                         │
                          ┌─────────────────────▼──────────────────────────────────┐
                          │ launchpad daemon (Rust: tokio + axum, systemd --user)  │
                          │                                                        │
                          │  REST API + static UI        event bus (seq numbers)   │
                          │  session manager ──▶ harness adapters                  │
                          │  GitHub client + sync        git service    projects   │
                          │  prompt composer             quota monitor  notifier   │
                          │  SQLite: state.db, cache.db  config.toml watcher       │
                          └────┬───────────────┬───────────────┬───────────────┬───┘
                               │               │               │               │
                        api.github.com    git CLI (SSH)    claude -p        codex app-server
                        REST + GraphQL    ~/projects/*     stream-json      JSON-RPC over stdio
                                                           (1 per session)  (1 shared process)
```

## 4. Process model

### 4.1 Daemon

- One Rust binary, `launchpad`, with subcommands: `launchpad daemon` (the server) and agent-facing commands such as `launchpad pr …` and `launchpad suggest …`.
- On Linux, runs as a systemd user unit `launchpad.service`: starts at login, restarts on crash, logs to journald (`journalctl --user -u launchpad -f`).
- Owns everything stateful: SQLite, GitHub sync loops, agent processes, quota polling, desktop notifications (D-Bus through `notify-rust`), and opening URLs, editors and terminals.
- Listens on `127.0.0.1:7317` only.
- On Unix, resolves the environment of your login shell once at startup (`$SHELL -l -i -c 'env -0'`, about 0.5 s) and uses it for every process it starts. Under systemd the daemon only gets a minimal `PATH`, and agents must see the same tools as in a terminal (nvm, cargo, `~/.local/bin`). Variables left by a parent agent session (`CLAUDECODE`, `CLAUDE_CODE_*`, …) are removed.
- On Linux, harness processes get `PR_SET_PDEATHSIG`, so they stop when the daemon stops, even outside systemd (dev profile).

### 4.2 Desktop shell

- A Tauri v2 app of roughly 100 lines of Rust. On start it checks `GET /api/health`. If the daemon is down, it runs `systemctl --user start launchpad` (or spawns `launchpad daemon` directly as a fallback), then opens a window on `http://127.0.0.1:7317/` after the auth handshake described in [Security](#14-security).
- Plugins: `single-instance` (prod only: clicking the dock icon focuses the existing window; `launchpad-desktop --open <route>` focuses it on a given screen, used by desktop notifications; `--new-window` and `--workspace` open more windows, see [9.1.2](#912-workspaces)) and `window-state` (size and position).
- **Warm window (prod):** a cold start of the shell takes about 0.5 s (GTK and WebKit processes), so one shell process stays warm. Closing the window hides it; the page stays loaded and connected. An autostart entry runs `launchpad-desktop --hidden` at login, and `install.sh` starts it after an install. A dock click starts a second process, which hands over to the warm one and exits: the window shows in about 100 ms ([spike 4](spikes/4-webkitgtk.md)). `launchpad-desktop --quit` stops the shell. Dev windows (`LAUNCHPAD_PROFILE=dev`) are not single-instance and quit on close.
- Links to other sites never open inside the window: navigation outside the daemon origin is cancelled and the URL goes to `xdg-open`.
- The window loads the UI from the daemon, not from bundled assets. This gives one origin (no CORS) and the same UI works in a browser for the future remote mode. The UI imports no `@tauri-apps/*` package.
- `packaging/install.sh` installs a `.desktop` entry, which can be pinned to the dock. On Wayland, the window's app id is `launchpad-desktop` (the binary name), so the entry has `StartupWMClass=launchpad-desktop` and the dock groups the window with the pinned icon. No `.deb` bundle is built.
- App identity: the sculpted launch icon uses an ivory folded arrow, mint core and coral exhaust on a teal rounded tile. `packaging/icons/source.png` is the unchanged raster master; `packaging/generate-icons.py` derives PNG sizes, lossless WebP, a 64px web export (browser favicon and top bar logo) and a self-contained SVG wrapper. Tauri, Ubuntu packaging and the in-app top bar share this artwork. `packaging/install-icons.sh` updates only the installed icon files and GTK icon cache, leaving processes running.
- After a daemon restart with a new UI build, the open page reloads itself: the WebSocket `hello` message carries a build id, and the UI compares it with the one it started with.
- Settings → General offers a production-only update button. The daemon embeds and starts a platform update script outside its lifetime: a transient systemd service on Linux, detached Bash on macOS, detached PowerShell on Windows. Updates resolve `Wauplin/launchpad` GitHub `main`, require a successful trusted `ci.yml` run for that exact commit (push or manual dispatch), and download the matching native artifact with `gh run download`. SHA-256 and the `COMMIT` manifest are checked before installation. Missing, expired, failed or unfinished builds fail visibly without selecting an older commit or building locally. The existing idle reservation serializes installation with agent starts and input. Update stage/log persist under the data directory. The status bar compares the embedded running commit to GitHub main, cached for 60 seconds; network failures do not imply an available update.

### 4.3 Lifecycle consequences

- Closing the window does not affect running agents or sync.
- Restarting the daemon kills agent processes (they are children in the service cgroup). Their sessions are marked `interrupted` and can be resumed in one click (`claude --resume`, Codex `thread/resume`). Only the turn in progress is lost. A later phase removes this limitation (see [9.8](#98-surviving-daemon-restarts-later)).
- Dev and prod profiles: `LAUNCHPAD_PROFILE=dev` uses port 7318, `~/.local/share/launchpad-dev` and `~/.config/launchpad-dev`. Agents launched from the prod instance can work on the Launchpad codebase, run the dev instance and restart it without killing themselves.

## 5. Repository layout and testing

### 5.1 Layout

```
launchpad/
├─ Cargo.toml                   # Rust workspace
├─ crates/
│  ├─ core/                     # domain types and services: db, github, git, projects,
│  │                            #   sessions, harness adapters, compose, predraft, quota, code
│  ├─ server/                   # axum routes, WebSocket, auth, static UI serving
│  └─ cli/                      # `launchpad` binary: daemon + agent-facing subcommands
├─ desktop/                     # Tauri v2 shell (src-tauri), and fallback/ (page shown while the daemon starts)
├─ web/                         # React app (Vite)
│  └─ src/
│     ├─ app/                   # shell: sidebar, top bar, status bar, routing
│     ├─ features/              # inbox, launch, agents, github, projects, code, predraft, settings
│     ├─ components/            # owned UI components (Radix based)
│     ├─ lib/                   # api client, ws client, markdown, diff, query keys
│     └─ gen/                   # TypeScript types generated from Rust (ts-rs)
├─ skills/launchpad/SKILL.md    # linked into ~/.agents/skills/launchpad
├─ packaging/                   # systemd unit, .desktop file, install script
└─ docs/
```

Rust structs shared with the UI derive `ts_rs::TS`, so the API is typed end to end without maintaining an OpenAPI spec.

### 5.2 Testing

Tests are kept to the minimum:

- Most code has no tests. Changes are checked by building and using the app (the dev profile, see [4.3](#43-lifecycle-consequences)).
- A test is added only for a small, specific part that clearly benefits from it, typically a pure function with tricky edge cases. Likely candidates: the unified diff parser, the prompt renderer, the harness event normalizers (fed with output recorded from real sessions).
- No mocks: no mocked HTTP APIs, harness processes, git or database.
- No tests that need a server, the network, GitHub, a running agent or the daemon.
- The full test run (`cargo test` plus the frontend tests, if any) takes a few seconds at most. A test that becomes slow or flaky is deleted or rewritten.

### 5.3 Continuous integration

GitHub Actions runs Linux Clippy and UI type checks plus native debug tests on main pushes, pull requests, tags and manual dispatch. Release builds/tests and CLI smoke checks run only when the ref is a tag, on Ubuntu 22.04, macOS 15 (ARM64 and Intel) and Windows 2025 (x86_64). The build job produces `web/dist` before compiling the daemon, which embeds those assets. It uploads a tar archive with executable binaries, UI and installation resources plus its SHA-256, retained for 90 days. CI only builds artifacts; installation and production restarts stay local. Rust is pinned to 1.93.1 so new Clippy versions do not change the lint gate unexpectedly; Node and pnpm use the existing repository declarations and both package managers use locked dependencies.

## 6. Backend

### 6.1 Stack

`tokio`, `axum` + `tower-http`, `reqwest` (rustls), `serde`/`serde_json`, `rusqlite` (bundled SQLite) behind a small DB actor, `tracing`, `ts-rs`, `notify` (config file watch), `uuid`, `time`, `notify-rust`. Git operations shell out to the `git` CLI: it uses your SSH configuration as-is and is simpler than libgit2.

### 6.2 Modules (in `crates/core`)

| Module | Responsibility |
|---|---|
| `config` | Load and watch `config.toml`: presets, external commands (editor, terminal), poll intervals |
| `projects` | Registry, discovery in the projects root, repo ↔ project mapping, temporary projects |
| `workspace` | Workspaces (named sets of projects), new desktop windows (see [9.1.2](#912-workspaces)) |
| `git` | Status, branches, worktrees, PR ref fetch, diff parsing into files/hunks/lines |
| `code` | Code viewer: sources (checkout, worktrees, revisions), file lists, file contents with changed line ranges, `git grep` search, path checks (see [8.1](#81-code-viewer)) |
| `github` | HTTP client (REST + GraphQL), ETag store, rate-limit tracking |
| `sync` | Notification poller, prioritized on-demand refresh queue, prefetch |
| `sessions` | Session registry, state machine, event log, input routing |
| `harness::{claude, codex}` | Harness adapters (see [9](#9-agent-sessions)) |
| `compose` | Turns a launch spec into a prompt and harness arguments |
| `predraft` | Pre-draft PR records, worktrees, publish and discard flows |
| `quota` | Harness versions, rate limits, Codex background update |
| `events` | In-process broadcast bus. Every state change emits an event, forwarded to WebSocket clients |

### 6.3 API shape

REST (JSON) for reads and mutations, one WebSocket for push events. First draft:

```
GET    /api/health
GET    /api/status                              versions, quotas, sync state, counts
GET    /api/notifications?repo=                 from cache
POST   /api/notifications/{id}/done|read|unread|unsubscribe
GET    /api/gh/{owner}/{repo}/items/{n}         cached issue/PR (refresh queued if stale)
GET    /api/gh/{owner}/{repo}/pulls/{n}/diff
POST   /api/gh/{owner}/{repo}/pulls/{n}/review|merge|draft|ready|reviewers|close
POST   /api/gh/{owner}/{repo}/issues/{n}/comments|close|reopen
GET    /api/gh/{owner}/{repo}/pulls?state=open  project lists (cache on demand)
POST   /api/refresh                             {keys: [...]} force a refresh
GET    /api/projects    POST /api/projects    POST /api/projects/discover
GET    /api/workspaces  POST /api/workspaces  PATCH|DELETE /api/workspaces/{id}
POST   /api/window                              {workspace_id?}: another desktop window
POST   /api/projects/temp    POST /api/projects/{id}/promote    DELETE /api/projects/{id}
GET    /api/code/sources?project={id}           checkout, worktrees, branches, tags, PR refs
GET    /api/code/files?root={path}&rev=         every file path, with change letters
GET    /api/code/file?root={path}&rev=&path=    content, language, size, changed lines
GET    /api/code/raw?root={path}&rev=&path=     image bytes
GET    /api/code/search?root={path}&rev=&q=&case=&word=&regex=   git grep
GET    /api/code/commit?root={path}&rev=        commit id and the GitHub repo that has it (permalinks)
POST   /api/open-editor                         {path, line?}: VS Code and Cursor open at the line
POST   /api/launch/preview                      launch spec → rendered prompt, no side effect
POST   /api/sessions                            launch spec → new session
GET    /api/sessions    GET /api/sessions/{id}/events?after={seq}
POST   /api/sessions/{id}/input|interrupt|answer|stop|resume|open-terminal
GET    /api/predrafts   POST /api/predrafts   PATCH /api/predrafts/{id}
POST   /api/predrafts/{id}/publish|discard|feedback
GET    /img?u={url}                             image proxy with disk cache
GET    /ws                                      event stream
```

WebSocket messages look like `{seq, topic, type, payload}` with topics `session/{id}`, `gh`, `status`. After a reconnect, the client sends its last `seq` per session and the daemon replays missing events from the database.

### 6.4 Storage

SQLite in WAL mode, two files in `~/.local/share/launchpad/`:

- `state.db` holds data you would not want to lose: projects, workspaces, sessions and their events, launch drafts (one per workspace), pre-draft PRs, draft comments, local notification overrides. Schema changes are a list of SQL statements applied in order using `PRAGMA user_version`. This takes about 20 lines of code and is not a framework.
- `cache.db` mirrors GitHub. It stores a hash of its schema. If the code's schema hash differs, the file is deleted and rebuilt by the next sync. No migrations, ever.

Cache tables keep the raw GraphQL or REST JSON plus a few indexed columns:

```sql
gh_notification(id PRIMARY KEY, repo, subject_type, subject_number, reason, unread,
                updated_at, last_read_at, json)
gh_item(repo, number, kind, state, is_draft, title, author, updated_at, fetched_at, json,
        PRIMARY KEY (repo, number))
gh_timeline(repo, number, fetched_at, json, PRIMARY KEY (repo, number))
gh_diff(repo, number, base_sha, head_sha, json, PRIMARY KEY (repo, number, head_sha))
gh_etag(url PRIMARY KEY, etag, last_modified, fetched_at)
image_cache(url PRIMARY KEY, path, content_type, fetched_at)
```

State tables:

```sql
project(id, name, path, remote_url, gh_repo, description, is_temp, pinned, created_at)
session(id, harness, harness_session_id, project_id, cwd, title, status, model, effort,
        launch_spec_json, prompt, predraft_id, cost_usd, usage_json, started_at, ended_at)
session_event(session_id, seq, ts, kind, json, PRIMARY KEY (session_id, seq))
session_raw(session_id, n, ts, dir, line, PRIMARY KEY (session_id, n))   -- raw harness lines
launch_draft(id, spec_json, updated_at)
predraft(id, project_id, branch, base, worktree_path, title, body, status, session_id,
         gh_pr_number, created_at, updated_at)
draft_comment(id, target, repo, number, predraft_id, path, line, body, event,
              author_session_id, status, created_at)
notification_override(thread_id PRIMARY KEY, local_unread, set_at)
```

Because GitHub payloads are stored as JSON, showing a new field in the UI means adding it to a GraphQL query. No schema change is needed.

### 6.5 Configuration file

`~/.config/launchpad/config.toml` is watched by the daemon and rewritten by the Settings screen. Missing keys use their defaults.

```toml
zoom = 2                            # 100%, 110%, 120% (default), 135%, 150%
projects_root = "~/projects"        # discovery, temporary folders, worktrees

[commands]
editor = "code"
browser = "xdg-open"

[terminal]                          # used by "Open in terminal"
app = "terminator"                  # terminator | ghostty | gnome-terminal | custom
custom = ["kitty", "--directory", "{cwd}", "{command}"]   # only used when app = "custom"

[github]
poll_interval_seconds = 60          # GitHub's X-Poll-Interval wins when larger

[agents]
notify = true                       # desktop notification when a session becomes Done

[[presets]]                         # see 9.7
id = "worktree"
label = "worktree"
text = "Please use a local git worktree and delete it once you're done."
```

Settings → General edits the projects root, the terminal and the editor. For the terminal, it lists the terminals found on `PATH` and offers a custom command. Built-in templates:

| Terminal | Command |
|---|---|
| Terminator (default) | `terminator --working-directory={cwd} -x {command}` |
| Ghostty | `ghostty --working-directory={cwd} -e {command}` |
| GNOME Terminal | `gnome-terminal --working-directory={cwd} -- {command}` |
| Custom | the `custom` list, with `{cwd}` and `{command}` replaced |

## 7. GitHub integration

### 7.1 Auth and client

- Token: `gh auth token`, read when the daemon starts and again after a `401`. It is kept in memory only. Your current scopes (`repo`, `read:org`, `gist`) cover notifications and every action listed below.
- REST requests use conditional headers (`ETag`, `If-Modified-Since`). A `304` response does not count against the rate limit. GraphQL is used wherever REST would need several calls.

### 7.2 Notifications

- **Polling:** `GET /notifications?all=true` returns read, unread and historical Done threads. It does not expose Done or inbox membership. Each tick checks the first page with `If-Modified-Since` and respects `X-Poll-Interval` (usually 60 s); a changed first page triggers a complete paginated pass, and an unconditional pass runs at startup and every 30 minutes. Fetching every page on a changed poll also catches read changes on old unread threads.
- **Inbox membership:** `state.db.inbox_active` stores threads observed unread or explicitly restored from a known GitHub inbox snapshot. Reading a thread in Launchpad keeps it in the inbox; Done removes it through the durable override. Historical read threads are excluded from the default list. `POST /api/inbox/restore` replaces the local membership with supplied thread ids and clears overrides for those threads; it never mutates GitHub. This can repair the current list from a screenshot, but ongoing exact parity with Read/Done actions on github.com is not available through the public API.
- **Unsubscribed threads:** the subscription endpoint is checked for new/changed threads and at least every 30 minutes. Explicit `ignored: true` or `subscribed: false` excludes a thread. A 404 means there is no explicit thread subscription and must not hide it: repository watching can still deliver that notification. The results are cached in `cache.db.inbox_subscription`, with conditional REST responses.
- **Enrichment:** the notifications payload has no participants and no subject state. For new or changed threads, one batched GraphQL query (aliases, about 20 subjects per request) fetches for each subject: state, `isDraft`, merged, author, `participants(first: 8)` with `totalCount`, labels, comment count, `updatedAt`, and for PRs `reviewDecision` and `statusCheckRollup.state`.
- **Reasons:** the `reason` field maps to a badge: `review_requested`, `mention`, `team_mention`, `assign`, `author`, `comment`, `subscribed`, `state_change`, `ci_activity`, `manual`, `security_alert`, …
- **Actions:**

| Action | API |
|---|---|
| Mark as read | `PATCH /notifications/threads/{id}` |
| Done | `DELETE /notifications/threads/{id}` |
| Unsubscribe | `PUT /notifications/threads/{id}/subscription` with `ignored: true`, then mark as done. Ignoring also mutes watched repositories; deleting a subscription does not. |
| Mark as unread | **No public API.** Checked on 2026-10-02: no REST endpoint and no GraphQL mutation. Implemented as a local override: Launchpad shows the thread as unread until you open it or GitHub reports new activity. github.com still shows it as read. |

- Opening a thread (preview or full view) marks it as read, like github.com. The "Mark as unread" button sets it back.
- All actions are optimistic: the cache and the UI change immediately, the API call follows, and a failure rolls back with a toast. Sync and notification mutations share a lock to prevent a poll from overwriting an action. Local read, unread and done overrides are stored in `state.db`, keyed by thread id and notification update time; new activity expires them.
- Phase 2 stores normalized notifications in `cache.db` (`inbox_thread`), description and latest eight comment summaries in `inbox_preview`, and REST responses with ETag and Last-Modified in `gh_response`. New or changed subjects are enriched in batches of 20, unread first, and their summaries are warmed at the same time. Selecting a thread renders its cached summary immediately and refreshes in the background if it is older than 60 seconds. Releases and commits use REST summaries; other subject types have a GitHub link. Full timelines, HTML rendering and the image proxy are available in Phase 3 through the Full conversation button.
- Quick reply drafts stay in the UI when changing the selection or navigating. Posting requires clicking Post reply. The reply input has no Enter shortcut. A successful post stays successful even if refreshing its summary fails.

### 7.3 Issues and pull requests (cached on demand)

- **Detail query:** a GraphQL query per timeline page fetches the header fields, `bodyHTML` and `body`, `timelineItems` (comments, reviews, review threads as read-only, commits, label/assign/rename/reference events, ready-for-review, merged, closed), participants, review requests, latest reviews, assignees, labels, milestone, `closingIssuesReferences`, check contexts (name, conclusion, details URL), `mergeable`, `mergeStateStatus`, `headRefName`, `baseRefName`, `headRefOid` and `baseRefOid`. Review threads and their comments are paginated separately.
- **Freshness:** the cached version renders immediately. A background refresh runs when an opened item is older than 60 s, or when you click Refresh. Before refetching a full timeline, the daemon compares `updatedAt` from the list or notification.
- **Prefetch:** when the inbox loads, details for the top 15 unread threads are prefetched. Selecting a row prefetches the rows just below it, the likely next clicks. A project page fetches its first page of open PRs and issues (30 most recently updated) when opened, and refreshes it if older than 5 minutes. Nothing else is prefetched.
- **Rendering:** GitHub's own `bodyHTML` (exact rendering: mentions, issue references, task lists, suggestion blocks), sanitized with DOMPurify and styled with `github-markdown-css` (light). The raw `body` is kept for quoting and editing.
- **Images:** private repositories embed short-lived signed image URLs. The daemon rewrites image URLs to its `/img` proxy and fetches the bytes at sync time, while the URLs are still valid. Avatars go through the same proxy, so they show instantly after the first load. The authenticated proxy accepts HTTPS URLs on known GitHub image hosts, validates every redirect, sends no GitHub token, and limits raster images to 5 MB. Images that cannot be cached keep their original URL.
- **Mutations:**

| Action | API |
|---|---|
| Comment | `POST /repos/{o}/{r}/issues/{n}/comments` |
| Review: comment, approve, request changes | `POST /repos/{o}/{r}/pulls/{n}/reviews` with `event` |
| Request review | `POST /repos/{o}/{r}/pulls/{n}/requested_reviewers` |
| Merge (merge, squash, rebase) | `PUT /repos/{o}/{r}/pulls/{n}/merge` |
| Close or reopen a PR | `PATCH /repos/{o}/{r}/pulls/{n}` |
| Close an issue as completed or not planned ("won't fix") | `PATCH /repos/{o}/{r}/issues/{n}` with `state_reason` |
| Convert to draft, mark ready for review | GraphQL `convertPullRequestToDraft`, `markPullRequestReadyForReview` (no REST equivalent) |

- **Out of scope for now:** posting inline review comments, GitHub Actions logs (the UI links to the browser), search.
- Every cached object has a Refresh button and an Open on GitHub button (`xdg-open` on `html_url`).

### 7.4 Diffs

- When the repository is a registered project: `git fetch origin +refs/pull/{n}/head:refs/launchpad/pr/{n}` (a private ref namespace, so no local branches are created), then `git diff --find-renames {base_sha}...{head_sha}` using the exact commits from the detail. The fetch uses the remote matching the item repository, including upstream for forks. The three-dot form diffs from the merge base, like GitHub. The output is parsed into files, hunks and lines in Rust and cached by `(base_sha, head_sha)`.
- Otherwise: `GET /repos/{o}/{r}/pulls/{n}/files` (paginated; GitHub truncates very large patches).
- The UI renders diffs with virtualization (TanStack Virtual), highlights code with Shiki in a Web Worker (per file, only when visible), offers unified and split views, and stores "viewed" checkboxes locally.
- Each file of a diff has a "View file" button that opens the full file in the code viewer ([8.1](#81-code-viewer)), at the PR head or in the pre-draft worktree.

### 7.5 Rate-limit budget

5,000 REST requests and 5,000 GraphQL points per hour. Notification polling is nearly free thanks to `304` responses. Enrichment costs about 1 point per 20 threads. A PR detail query costs 1 to 5 points. The status bar shows the remaining budget.

## 8. Projects

- **Project page:** Running agents come first, followed by GitHub pull requests/issues, Worktrees, and All agents (including running sessions). Both agent lists use memoized, fixed-height virtualized rows and the shared Running predicate. Empty GitHub tabs shrink to their message instead of reserving the full list height.

- A project is a folder in the projects root (setting `projects_root`, default `~/projects`) plus an optional GitHub remote (`owner/repo` parsed from `origin`, SSH or HTTPS form). A project can also be registered from another location by path.
- **Remote metadata:** Project reads refresh `remote_url`, `gh_repo` (origin) and `gh_upstream` from local Git configuration, including linked worktrees. A daemon loop checks every 30 seconds and broadcasts `projects/changed` only when persisted metadata changes, so open windows update without re-registering folders or restarting. Added, changed and removed remotes replace the registration snapshot; temporarily missing folders retain their last known metadata. User-edited names, descriptions and pins remain independent. No network request is needed.
- **Discovery:** "Scan projects root" lists git repositories with a GitHub `origin` (you have 184 folders in `~/projects`; most do not need to be registered). Registered projects can be pinned in the sidebar. Each project has a one-line description (for example "server-side codebase") that is used when the project is given as context.
- **Repo → project mapping:** tells notifications, issues and PRs where to launch an agent and which local clone to use for diffs. For forks, the `upstream` remote is matched too.
- **Temporary projects:**
  - Create: `<projects_root>/tmp-<MMDD>-<slug>`, where the slug comes from the first words of the prompt (for example `tmp-1002-xet-upload-bench`). Optional `git init` (setting). Registered with `is_temp = 1` and listed in a "Temporary" group with their age.
  - Trust: headless modes show no trust prompt (`claude -p` skips the trust dialog; `codex app-server` does not prompt). For "Open in terminal", Codex gets `-c 'projects."<path>".trust_level="trusted"'`. The Claude TUI shows its trust dialog in a git repository whose root is not trusted, even under a trusted `~/projects` and even with `--dangerously-skip-permissions` ([spike 5](spikes/5-claude-trust-prompt.md)). For temporary projects, "Open in terminal" sets `CLAUDE_CODE_SANDBOXED=1` in the command's environment, which skips the dialog. Launchpad never writes `~/.claude.json`.
  - Clean up: one button stops the sessions running there, deletes the folder and unregisters it. The confirmation is part of the button (second click), not a dialog.
  - Promote: pick a name, the folder is renamed (by default the `tmp-<MMDD>-` prefix is dropped) and `is_temp` is cleared. Renaming is refused while a session runs there. Claude stores transcripts in a folder derived from the path (`~/.claude/projects/<encoded-path>/`), so promotion also moves that folder to keep past sessions resumable.

### 8.1 Code viewer

A read-only viewer for the code of a project: file tree, tabs, syntax highlighting, find in file and search in the project (sketch S7). It is for reading: what an agent changed in its worktree, the full file behind a diff, a file an agent mentions, or code you want to give an agent as context. Editing stays in the editor chosen in Settings, and with the agents. Route: `/projects/{id}/code?root=&rev=&path=&line=`.

- **Sources.** The viewer shows one source at a time:
  - A working tree: the project checkout or one of its worktrees (`git worktree list`: pre-draft PRs, worktrees created by agents). Files are read from disk, uncommitted changes included.
  - A git revision: a local or remote branch, a tag, a commit, or a PR head fetched for its diff (`refs/launchpad/pr/{n}`, see [7.4](#74-diffs)). Files are read with `git ls-tree` and `git show <rev>:<path>`, without touching any checkout.
  - A temporary project that is not a git repository is read from disk with a directory walk (the `ignore` crate, which skips hidden and ignored files).
- **File list.** `git ls-files --cached --others --exclude-standard -z` for a working tree, `git ls-tree -r --name-only -z <rev>` for a revision. The full list is sent once per source. The UI builds the tree (virtualized, fixed row height) and the "Go to file" filter from it, so neither waits on the daemon. A folder with a single child folder shares its row (`src/huggingface_hub`).
- **What the branch changes.** The base is the merge base with the default branch (`git merge-base HEAD origin/<default>`; the default is `origin/HEAD`, then `origin/main`, `origin/master`, `main`, `master`). `git diff --name-status --no-renames <base>` (plus untracked files for a working tree) gives the M, A and U letters in the tree and the "Changed only" filter; files deleted in the working tree leave the list. For the open file, `git diff -U0 <base> -- <path>` gives the line ranges of the bars next to the line numbers: in each hunk, the first min(removed, added) new lines are modified (amber), the others added (green), and a hunk without new lines puts a red mark after its position. On the default branch, the merge base is usually `HEAD`, so the viewer shows the uncommitted changes. Without a default branch, a working tree compares with `HEAD`; in a repository without commits, every file is untracked.
- **File content.** `GET /api/code/file` returns the text, its size, the language (a Shiki id and a display name, from a table of file names and extensions in `code.rs`), the changed line ranges and whether the working tree file differs from `HEAD`. Images are shown as images, through `GET /api/code/raw` (served with a sandbox Content-Security-Policy, so an SVG opened on its own runs no script). Other binary files (a NUL byte in the first 8 KB, as git detects them) only show their size. Files over 5 MB are not loaded ("Open in editor" instead), and files over 1 MB or 20,000 lines are shown without colors. The first visit of a source opens its README.
- **Rendering.** The code lines of the diff view ([7.4](#74-diffs)): TanStack Virtual with fixed row heights (20 px), no line wrapping, a line number gutter that stays visible when scrolling sideways, Shiki in a Web Worker (plain text first, colors when ready). The worker is an ES module that loads one grammar chunk per language and uses Shiki's JavaScript regex engine (no WASM); the last twelve results are kept for tab switches. Markdown files also have a Preview (markdown-it, as in sessions), where relative images and links point to files of the same source. HTML files have the same Preview/Source toggle, using DOMPurify and a sandboxed `srcdoc` frame with no permissions and a restrictive Content-Security-Policy. Inline CSS and data images/fonts render; scripts, links, forms and external resources are disabled. This preview supports self-contained static documents without a local web server. Opened files stay as tabs for each source; tabs with the same file name show the end of their folder.
- **Find in file.** A find bar over the code, opened by its Find button or by `Ctrl+F` while the viewer is on screen. It searches the file text in memory, not the DOM: most rows of a virtualized file are not in the DOM, and the Tauri webview has no find bar of its own. Options: match case, whole word, regular expression. `Enter` goes to the next match, `Shift+Enter` to the previous one, `Escape` closes the bar, and the virtualizer scrolls the current match into view. This is the only keyboard shortcut in Launchpad (decision 22).
- **Search in project.** `git grep -n --column -I -z` on the same source (`--untracked` for a working tree, the commit id for a revision, `--no-index` for a folder without git): fixed strings by default (`-F`), `-E` in regex mode, `-i` unless match case is on, `-w` for whole words. It runs as you type (two characters or more, 250 ms after the last key), so the field needs no `Enter`. Results are grouped by file (4 lines per file until "Show more") and capped at 1,000; very long lines are cut around the match. A new query cancels the running one: the UI aborts the request, and the daemon kills `git grep` when the request is dropped.
- **Line selection.** Click a line number to select the line (a second click clears it); drag over the numbers or shift-click to extend the range. The selection bar, at the bottom right of the code, offers Copy (the code), Copy reference (`src/huggingface_hub/hf_file_system.py:405-411`), GitHub link (copies a permalink on the commit, offered only when a remote branch contains it, `git branch -r --contains <sha>`, and the working tree file has no uncommitted change), Add to agent, and Open in editor (`--goto <file>:<line>` for VS Code and Cursor, the file alone for other editors; working trees only, since a revision is not on disk). The file header has the same GitHub and editor actions for the whole file, the editor opening at the first visible line.
- **Add to agent.** Adds the file and the line range as context: to the composer as a context chip (see [9.7](#97-prompt-composition); an empty composer also gets the project as its target), or to the message box of a session of the same project (not archived) as a `path:405-411` reference. The reference is relative when the session works in the same folder, absolute otherwise, and names the revision for a revision source. Unsent message box text lives in a UI store, so it survives switching views. The prompt never includes the code itself; the agent reads the file, as it does for GitHub links.
- **Ways in.** A Code button on project pages and on each worktree (listed on the project page), a Files button in each session header (opens the session's cwd), file paths in tool calls and file changes, `path:line` references in inline code and local file links in agent messages (linked when the path, or a unique file name, is in the file list of the session's cwd), and "View file" on each file of a diff (Phases 3 and 4). Session links with HTTP or HTTPS URLs open in the configured browser; unresolved local paths stay in Launchpad and show a message instead of becoming daemon URLs.
- **Live updates.** No file watcher. When a session reports a `file_change` for a file of the open working tree (Claude's Edit, MultiEdit and Write, Codex file changes), the viewer reloads that file in place and keeps the scroll position, and reloads the file list a second later. The file list also reloads when the viewer opens and with the Reload button. Edits made by shell commands need the Reload button.
- **State.** Open tabs, expanded folders and the last position for each source, and the last source and file of each project, are UI state (Zustand, kept in `localStorage`). Nothing is stored in `state.db`.

## 9. Agent sessions

### 9.1 Session model

States are the same for every harness:

The agent session groups are Running, Quota paused, Done and Archived (Cron and Scheduled remain separate):

- **Done:** `waiting`, when a turn has finished or the agent asks for user input. It stays here until the next prompt or Archive. Closing its panel does not change its state.
- **Running:** `starting`, `working` and `background`. Long commands stay Running even when they produce no output. Codex no longer uses a silence timer to change the session state.
- **Quota paused:** `quota_paused`, between Running and Done. A protected session is held locally or interrupted when a reported included 5-hour or weekly quota reaches 100%. It never satisfies Done dependencies. Stop cancels its automatic continuation and moves it to Done; Resume with paid usage allows additional spending for the rest of that session and continues the pending input or interrupted task.
- **Archived:** closed, stopped, interrupted, errored and externally attached sessions, as before. Archived conversations remain resumable. The separate Archive button stops the process and uses the existing `closed` state and `/close` endpoint; the header X only removes the session from the pane, including while Running.

Each group heading is a button with a chevron and an always-visible count. Groups start expanded and collapse independently; their collapse state is kept with the UI layout in `localStorage` across navigation and reloads. Hidden session rows and Archived’s Show more button are omitted from the virtualized list. Collapsing a group does not change session state or close an open pane.

The Agents sidebar item shows separate amber Done, outlined Quota paused and blue Running counts; a discreet spinner appears beside a nonzero Running count. Pinned projects show an amber dot when any associated session is Done and a spinner when any is Running, including both when applicable. Sessions match projects by project id or folder, so worktree sessions retain their project marker.

A session that becomes Done sends a desktop notification (`agents.notify`) unless the focused window is displaying it. Each session stores its id, harness id, cwd, project, title, model, effort, usage and initial launch spec. After launch, a separate naming request generates a concise AI title from the initial rendered prompt and persists it through the usual session update event. This request uses Claude Haiku or a small available Codex model, is ephemeral, runs outside the project and times out after 45 seconds. Launch never waits for naming; if naming fails, the first-line prompt title remains.

Normalized event kinds (stored in `session_event`, streamed over WebSocket):

| Event | Content |
|---|---|
| `user_message` | text, attachments, `queued` flag |
| `assistant_delta` / `assistant_message` | streamed text / final markdown |
| `reasoning` | reasoning summary, when the harness exposes one |
| `tool_call` / `tool_result` | name, input summary / output, exit code, duration |
| `file_change` | path and unified diff |
| `plan` | todo list with item states |
| `usage` | tokens (input, cached, output), context window usage, cost |
| `ask_user` | question and options from the agent, answered with `/answer` |
| `status`, `rate_limit`, `error` | state changes, quota updates, failures |

Raw harness output lines are also stored (`session_raw`, capped at 20,000 lines per session) so the normalizer can be improved and replayed on old sessions. Text deltas and live command output are streamed but not stored; the final message and the command result are.

A Done session has no harness process after a daemon restart; sending it a message resumes it first. A session that was mid-turn becomes `interrupted`.

### 9.1.1 Agent selection

The Agents view shows one detail panel per window. Clicking a session replaces the selected conversation; Cron and Scheduled details use the same panel. There are no layout, pane focus, move or session drag/drop controls. The selected session and pins live in the existing persisted browser layout store. Only the session actually visible in the current workspace is reported for notification suppression. Pins sort first inside each state group rather than introducing another session state. Search filters titles, project names, folders and initial prompts immediately, and debounces a daemon query over stored user/assistant message text. Filtered groups retain their counts and independent collapse controls.

Session titles are edited in the header: click the title to replace it with a text field and visible Save title and Cancel buttons. Saving persists the trimmed, nonempty title through the existing rename endpoint; failures keep the draft open. A star to the right toggles the persisted pin, with an accessible label and pressed state. Git and Open in editor sit in the action row beside Files and Archive.

### Session summary (S8)

Each panel load of a non-running session (including a repeated click, startup and workspace switch) opens Summary. Running always shows the virtualized trace. A user-selected Trace stays selected through turn completion, with a thin state bar offering Final message (scrolls to the last assistant row) and Summary. The header retains its existing actions and adds Summary/Trace; the message box remains at the bottom and successful input or a structured answer selects Trace. No generated summary, free-text action parsing or harness output convention is used.

The summary is one white column separated by thin lines and small uppercase headings: state band, Run, Task, Final message, Next steps. Task is the first saved user prompt, folded after four lines; the exact last assistant markdown folds after about 30 display lines and supports raw markdown and rendered text copies. Missing metadata is hidden. Run duration sums reported `turn_end.duration_ms` only when every recorded turn has a duration, and the count is recorded turn ends, excluding idle time and unrecorded unfinished turns. Tokens use recorded input/cache/output totals; cost is Claude-only. Launch context comes from the saved spec. The last Bash/Shell event supplies the command and its reported exit code. The read-only Git endpoint includes tracked differences against local `main` and commits absent from cached remote refs, with no fetch or claim about current remote state; binary line counts are hidden. Git’s existing panel adds a Compared with main mode for those tracked patches. Untracked files remain in Working tree.

A stop request is kept in memory until the harness ends the turn, which then persists `stopped`. It stays Running while interrupt confirmation is pending, including silent commands. A restart during a pending stop is Interrupted. Reported failed turns use Error (displayed as Failed); a process exit without a failure is generically Interrupted. Restart detection records when the interruption was detected, rather than inventing the turn’s end time. Stopped/Error/Interrupted never satisfy the Done predicate. Next steps use only Git status, linked local pre-drafts, saved GitHub references, session state and structured questions. Opening a new pre-draft copies the session’s committed head through the existing cwd-based creation path; the UI remembers that source-session-to-pre-draft link in localStorage, so repeated opens return to that record. Uncommitted changes must be committed before the existing creation path can copy them. Unfinished Resume sends the visible action’s fixed instruction “Continue the unfinished turn.” in that session; archived Resume uses the existing re-attach action. Question choices use `/answer` while attached, or a new message with those structured questions and selected answers after a restart; multiple questions collect one choice each before sending.

### 9.1.2 Workspaces

A workspace is a named set of registered projects, stored as JSON in the `workspace` table of `state.db`. A project can belong to several workspaces. Everything else follows from the projects: a session belongs to a workspace when its project id or folder belongs to one of its projects, or when it was launched from that workspace; scheduled launches and crons use their target project or the workspace that created them. Inbox threads belong to a workspace when their repository is the `origin` or `upstream` of one of its projects. "All projects" is the absence of a workspace and shows everything, as before.

Each window shows one workspace. The choice lives in the window's `sessionStorage`, so several windows show different workspaces and each one keeps its own after a reload; `localStorage` remembers the last choice for the next window. The top bar is a single row: logo, page title, then, on the right, All projects and every workspace as visible one-click buttons followed by Launch. The selected workspace is highlighted and each button has a separate New window button. When the window is too narrow (smaller windows, larger zoom sizes, many workspaces), the workspace buttons and Launch move together to a second line and wrap there, so every workspace stays accessible. Switching changes the workspace in place (no reload). The window title names the workspace, so the dock and the window switcher tell windows apart. Settings → Workspaces (`/settings/workspaces`, next to the General tab) creates, renames and deletes workspaces and picks their projects; changes are saved at once, and deleting a workspace keeps its projects and agents.

In a workspace: the sidebar lists all its projects and its temporary folders, and its counts; Agents filters Cron, Scheduled, Running, Done and Archived; Projects lists its projects first with Add/Remove buttons and the other projects below; the composer offers its projects first; the status bar counts its agents with the sidebar predicates. The Inbox shows the workspace's repositories by default, with an All repos button to see every notification (the repository chips and the unread count follow the same list). A launch sends the window's `workspace_id` in its launch spec, and the daemon adds the target project, including a new temporary folder, to that workspace, so a new agent never disappears from the window that started it. Adding a folder or a scanned repository from Projects also adds it to the window's workspace. Unregistering a project removes it from every workspace.

Selected sessions, pins and collapsed groups are stored per workspace (`launchpad.layout.<id>`; all projects keep `launchpad.layout`). The unsent composer draft is also per workspace (`launch_draft` row `workspace:<id>`), so two windows do not overwrite each other's draft. Desktop notifications suppression uses the presence of every connected window: a session shown in any focused window is not notified.

New windows: `POST /api/window` runs `launchpad-desktop --new-window [--workspace <id>]` next to the daemon binary. In prod, the single-instance shell opens one more window in the warm process (labels `window-2`, `window-3`, …, so window-state reuses their sizes); only the main window hides on close. In dev, each `launchpad-desktop` is its own process with its own window, like separate VS Code instances. `launchpad-desktop --workspace <id or name>` starts on a workspace; on a running prod shell it switches the hidden main window, or opens a new window when combined with `--new-window`. In a normal browser, New window opens a tab with `?ws=<id>`.

### 9.1.3 Session titles and history import

Manual rename updates the session title in `state.db` and broadcasts the usual session update. A late AI naming result only applies if the original launch title is still present. Session headers wrap their visible actions so titles remain readable at narrow window sizes. The status bar wraps at larger zoom positions instead of extending past the window, and the desktop shell leaves drag/drop to the HTML controls.

Import history is an explicit read-only discovery and import flow. Claude candidates come from top-level JSONL transcripts in `~/.claude/projects/`, with a bounded metadata scan; Codex candidates come from paginated `thread/list`, including a separate archived selector. Import reads the selected native transcript or `thread/read` with turns, normalizes conversation and tool history, and writes the session and events in one database transaction. Duplicate harness/id pairs return the existing session. The result is Archived with no attached process, and retains the native conversation id and cwd for resume. Native transcripts are never moved or rewritten. Imports have no launch harness in their launch spec, so they cannot change remembered launch defaults or recent contexts. This is a snapshot, not continuous synchronization; finish a conversation in its other client before resuming it here. Missing folders and inaccessible transcripts remain visible errors. Claude imports over 64 MB are left to the terminal.

The per-session Git panel only runs read-only commands against that session's saved cwd. Status preserves both porcelain columns and rename paths; selecting a file loads separate index and working-tree patches with literal pathspecs and external diff/textconv disabled. Untracked text is read through `code::open` and `code::file`, preserving registered-root and symlink checks. Large patches show their beginning and end with a truncation label. The panel refreshes every five seconds while open, and both file lists and patch rows are virtualized. Open in editor uses the existing configured-editor endpoint on the session folder.

Recent composer chips come from the launch specs of successfully launched sessions, newest first. They preserve project branches, GitHub references and file line references, skip selected contexts, and show at most six suggestions. Unsent edits and imported history do not feed these suggestions.

### 9.2 Harness adapter interface

```rust
#[async_trait]
trait Harness: Send + Sync {
    async fn start(&self, launch: &ResolvedLaunch, sink: EventSink) -> Result<Box<dyn LiveSession>>;
    async fn resume(&self, session: &SessionRecord, sink: EventSink) -> Result<Box<dyn LiveSession>>;
    async fn version(&self) -> Result<String>;
    async fn models(&self) -> Result<Vec<ModelInfo>>;           // composer pickers
    async fn defaults(&self) -> Result<ModelDefaults>;           // model + effort used when not overridden
    async fn rate_limits(&self) -> Result<Option<RateLimits>>;
    fn terminal_command(&self, session: &SessionRecord) -> std::process::Command;
}

#[async_trait]
trait LiveSession: Send + Sync {
    async fn send(&self, text: String) -> Result<()>;            // new turn, or steer while working
    async fn interrupt(&self) -> Result<()>;
    async fn answer(&self, request_id: String, answer: serde_json::Value) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}
```

### 9.3 Claude Code adapter

- **Command** (cwd = project path, one process per session, kept alive between turns because stdin stays open):

  ```
  claude -p --input-format stream-json --output-format stream-json --verbose \
         --include-partial-messages --dangerously-skip-permissions \
         --replay-user-messages --permission-prompt-tool stdio \
         --session-id <uuid> --name "<title>" \
         [--model <model>] [--effort <level>] [--add-dir <context project>…]
  ```

  Verified in [spike 1](spikes/1-claude-stream-json.md).
- **Environment:** harness processes get `LAUNCHPAD_PROFILE` (the profile of the daemon that started them) and `LAUNCHPAD_SESSION_ID`, so `launchpad` commands run by an agent reach the right daemon and session. The adapters remove `CLAUDECODE`, `CLAUDE_CODE_*`, `CLAUDE_PID` and `CLAUDE_EFFORT` from the environment before spawning a harness. A dev daemon started from an agent session would otherwise pass them on, and Claude then turns transcript saving off ("inherited CLAUDE_CODE_CHILD_SESSION marker").
- **Input:** one JSON line per user message on stdin (`{"type":"user","uuid":"<uuid>","message":{"role":"user","content":[…]}}`). A message sent while a turn runs does not start a new turn: Claude adds it to the running turn at the next model request (after the current tool result). Claude reports `command_lifecycle` events (`queued`, `started`, `completed`) with the message `uuid`, and `--replay-user-messages` echoes the message when Claude reads it. The UI shows the message as "queued" until then.
- **Interrupt:** a `control_request` with `subtype: "interrupt"` on stdin. Claude replies with a `control_response`, rejects the running tool, and ends the turn with a `result` of subtype `error_during_execution`. The process stays alive for the next message.
- **Output → events:** `system`/`init` (model, tools, cwd; sent again at the start of every turn), `stream_event` (text deltas), `assistant` (text, thinking and `tool_use` blocks), `user` (`tool_result` blocks), `result` at the end of each turn (`total_cost_usd`, `usage`, duration; switches the session to `waiting`), `rate_limit_event` (sent once per process: `utilization` as a fraction and `resetsAt` for both the five-hour and the seven-day window). `total_cost_usd` is cumulative for the whole session, including earlier processes of a resumed session, so Launchpad stores the latest value instead of adding them; `usage` is per turn.
- **Background commands:** `system` messages with subtype `background_tasks_changed` (the list of running background tasks), `task_started` (`is_backgrounded: true`), `task_updated` and `task_notification` (final status, summary, output file) track commands started with `run_in_background`. A turn that ends while the list is not empty puts the session in `background`. When the task ends, Claude starts a new turn by itself, so the session moves to `working`, then to `waiting` at the next `result`.
- **Questions from the agent:** AskUserQuestion is only offered to the model when the host takes permission prompts (`--permission-prompt-tool stdio`). It then arrives as a `can_use_tool` control request (`requires_user_interaction: true`), rendered as a card. The answer is a `control_response` with `behavior: "allow"` and `updatedInput.answers` (question text → chosen label). All other tools still run without a request because of `--dangerously-skip-permissions`. Any other `can_use_tool` request is allowed automatically and logged.
- **Resume:** the same command with `--resume <session-id>` instead of `--session-id <uuid>` (Claude refuses both together unless `--fork-session` is given).
- **Open in terminal:** wait for `waiting`, close stdin (the process exits), then run the terminal chosen in Settings (see [6.5](#65-configuration-file)) with `claude --resume <id> --dangerously-skip-permissions` in the session cwd (plus `CLAUDE_CODE_SANDBOXED=1` for temporary projects, see [8](#8-projects)). The session shows "in terminal". "Re-attach" resumes it in Launchpad later. A session never has two live processes.
- **Defaults:** without `--model` or `--effort`, Claude Code uses `~/.claude/settings.json` (today `model: opus[1m]`, `effortLevel: xhigh`). The composer reads that file and shows "Default (opus[1m] · xhigh)".
- Subscription login works in print mode (OAuth). Do not use `--bare`: it disables OAuth.

### 9.4 Codex adapter

- **Process:** one long-lived `codex app-server` (JSON-RPC over stdio), child of the daemon and shared by all Codex sessions (the server handles multiple threads). It also serves `account/rateLimits/read` and `model/list`. Both were tested on this machine: the first returns the 5-hour and weekly windows with `usedPercent` and `resetsAt`, the second returns models with their supported effort levels.
- **Flow:** `initialize` → `thread/start {cwd, model?, approvalPolicy: "never", sandbox: "danger-full-access"}` → `turn/start {threadId, input: [{type: "text", text}], effort?}`.
- **Typing while the agent works:** `turn/steer {threadId, expectedTurnId, input}`. The steered text appears as a `userMessage` item when Codex reads it (after the running command), in the same turn. The Stop button sends `turn/interrupt {threadId, turnId}`, which ends the turn with `status: "interrupted"`. A running command is not stopped by the interrupt: it finishes on its own and its `item/completed` arrives later. The adapter marks open items as interrupted when the turn ends, and the late result replaces that mark. Verified in [spike 3](spikes/3-codex-app-server.md), with the full event mapping.
- **Notifications → events:** `item/agentMessage/delta`, `item/started` and `item/completed` (user messages, agent messages with `phase` `commentary` or `final_answer`, command executions with `item/commandExecution/outputDelta`, file changes, MCP tool calls, reasoning summaries), `turn/plan/updated`, `turn/diff/updated`, `thread/tokenUsage/updated` (`total`, `last`, `modelContextWindow`), `turn/completed` (switches to `waiting`), `account/rateLimits/updated` (after every model response).
- **Long commands:** an open `commandExecution` stays Running until the turn finishes or Codex asks for user input. Silence is not a signal that the agent is Done.
- **Server requests:** `item/tool/requestUserInput` becomes an `ask_user` card. Approval requests should not happen with `approvalPolicy: "never"`; if one arrives, it is declined automatically and logged.
- **Resume after a restart:** `thread/resume {threadId}`.
- **Open in terminal:** `codex resume <thread-id>` with `-c check_for_update_on_startup=false` and the trust override for the cwd.
- **Protocol types:** `codex app-server generate-json-schema` produces the full schema (104 client methods, 83 notifications in 0.159.3). Launchpad hand-writes Rust types for the subset it uses. After a Codex update, a small dev script regenerates the schema and lists the changes in the methods Launchpad uses. It is run by hand, not as a test.
- **Option for later:** Codex now ships a shared local app-server daemon (`codex app-server daemon`, `codex agents`, `codex --remote unix://…`). Connecting to it instead of a private child process would let Codex threads survive Launchpad restarts and be attached from a terminal TUI. It is marked experimental, so it is not part of v1.

### 9.5 Versions, quotas and updates

- **Versions:** `claude --version` and `codex --version` at startup and every 30 minutes, shown in the status bar.
- **Codex quota:** `account/rateLimits/read` every 2 minutes when idle and every 30 seconds while protected sessions run or are quota paused, plus `account/rateLimits/updated` notifications.
- **Claude quota:** two sources. First, `rate_limit_event` messages from any running session. Second, the endpoint behind Claude Code's `/usage` screen (`GET https://api.anthropic.com/api/oauth/usage`, `Authorization: Bearer <token>`, no beta header), polled every 5 minutes when idle and every 60 seconds while protected sessions run or are quota paused, with the OAuth access token read from `~/.claude/.credentials.json` (approved). It returns `five_hour` and `seven_day` with `utilization` (percent) and `resets_at` ([spike 2](spikes/2-claude-usage-endpoint.md)). Launchpad never refreshes that token itself: refresh tokens rotate, so a refresh by Launchpad would log Claude Code out. Before each call it checks `expiresAt` in the same file; when the token has expired, it keeps the last values and waits for Claude Code to refresh it. This endpoint is not documented, so the module is isolated and the UI shows "unknown" when it fails.
- **Codex updates:** `codex update` runs at daemon start and every 6 hours. You use the standalone install (`~/.codex/packages/standalone`, currently 0.159.3 with 0.160.0 available). Running sessions keep the binary they started with. Headless modes never show the update prompt, and "Open in terminal" passes `-c check_for_update_on_startup=false`.
- **Claude updates:** the native installer updates Claude Code by itself. Launchpad only shows the version.

### 9.5.1 Per-session quota protection

The composer has an unchecked Allow paid usage checkbox beside Open after launch. `LaunchSpec.allow_paid_usage` is persisted for sessions, scheduled launches and cron runs, but is excluded from remembered launch defaults. Unsent drafts retain their explicit choice; discarding or completing a launch restores the unchecked default. This is best-effort control of subscription overage, not a guaranteed zero-charge limit, and does not change provider billing settings. External terminal sessions are outside its control.

`quota.rs` evaluates included 5-hour and weekly windows without rounding. The daemon checks protected sessions every second, in addition to faster quota polling. Known exhaustion blocks new input even if the reading is old; an unknown reading permits manual launch/input with a trace notice. Automatic continuation requires both windows below 100%, future reported reset timestamps, no quota error and a reading at most 90 seconds old. A passed reset timestamp alone never authorizes continuation. Partial Claude events replace the snapshot instead of refreshing the age of retained windows. Claude OAuth is never refreshed by Launchpad.

`session.quota_pause_json` stores blocking windows, reported reset times, the pending message and images, queued client id, phase (pausing, paused, resuming or needs_attention) and errors. A launch at exhausted quota attaches no harness and holds the original rendered prompt verbatim. Ordinary unsent input is held similarly. Mid-turn exhaustion saves a continuation instruction instead of replaying the task. Adapter status events cannot overwrite the pause, detached adapters cannot overwrite session status, and the same conversation is continued only after the turn and outstanding commands finish. Codex command ids remain tracked after interruption until their real completion events arrive. An idle unanswered question is left for the user; its answer is blocked while included quota is exhausted.

User actions and quota decisions share an action lock. Automatic continuation claims the pause before attaching or sending, honors the updater guard and pre-draft publication checks, and never retries uncertain sends. A paused record survives restart; a claimed continuation becomes needs_attention and requires inspection before manual paid resume or Stop. Stop on a quota pause cancels held continuation and enters Done, while ordinary user interruptions retain Stopped. Scheduled follow-ups wait behind quota pauses. A cron condition requiring its previous run to finish also waits while that run is quota paused. Publication and installation idle checks include commands that outlive an interrupt. Title-generation requests for protected sessions require fresh available quota and are cancelled when it becomes unavailable.

The quota band appears in Summary and Trace, shows blocking windows and reset times, explains failed readings and interruption limits, and has visible Stop and Resume with paid usage buttons. Quota paused has its own virtualized, independently collapsible group and workspace-scoped counts in the sidebar and status bar; project markers use the same predicate.

### 9.6 Session view (UI)

- **Assistant text:** markdown (markdown-it, Shiki for code blocks), rendered incrementally. Finished blocks are memoized; only the last block re-renders while streaming.
- **Tool calls:** collapsed to one line (`Bash · pytest tests/test_hf_api.py · 12 s · exit 0`). Consecutive calls are grouped ("Ran 6 commands, read 4 files"). File changes show a small inline diff. Click to expand.
- **Copy:** every message keeps its raw markdown. "Copy" puts the exact markdown on the clipboard, without the line breaks and padding that terminals add. Code blocks have their own copy button. Tool output copies as raw text.
- **Input:** a message box under each session, with a Send button. `Enter` also sends and `Shift+Enter` adds a new line, like in the terminal. A Stop button in the session header interrupts the agent. TUI-only slash commands point to "Open in terminal".
- **Header:** tokens (input, cached, output), context window usage, cost estimate (Claude reports `total_cost_usd`; Codex reports tokens only), elapsed time.
- **Files:** file paths in tool calls and file changes, and `path:line` references in messages, open the code viewer on the session's cwd ([8.1](#81-code-viewer)). A Files button in the header opens the viewer on that folder.

### 9.7 Prompt composition

A launch is described by a spec:

```rust
struct LaunchSpec {
    harness: HarnessKind,                    // Claude | Codex
    target: Target,                          // Project(id) | NewTempProject | NewPredraft { project, base }
    text: String,                            // free text typed by you
    presets: Vec<PresetId>,
    context_projects: Vec<ContextProject>,   // { project_id, branch: Option<String> }
    github_refs: Vec<GitHubRef>,             // issue | pr | comment | review comment: URL + cached title
    files: Vec<FileRef>,                     // from the code viewer: root, revision, path, optional line range
    model: Option<String>,                   // None = harness default
    effort: Option<String>,                  // None = harness default
}
```

The prompt is rendered by a deterministic template and shown in the composer before launch:

```
Check if #3412 is still relevant and whether #3420 fixes it.

Context:
- Issue: https://github.com/huggingface/huggingface_hub/issues/3412 ("hf download ignores --revision when used with --include")
- Pull request: https://github.com/huggingface/huggingface_hub/pull/3420 ("Fix revision handling in snapshot_download filters")
- Reference project: ~/projects/moon-landing (server-side codebase), branch `xet-refresh-token`. Read it without changing its checkout, for example with `git -C ~/projects/moon-landing show xet-refresh-token:<path>` or a temporary worktree. Do not modify it.
Use `gh` to read issues, pull requests and comments.

Please use a local git worktree and delete it once you're done. No need to add any unit tests for that PR.
```

- Order: free text, then context, then presets (in the order defined in settings).
- Paths are absolute, because sessions may run in worktrees outside the project folder. Context projects are also passed to Claude as `--add-dir`.
- **Mentions in the prompt:** typing `@` suggests registered projects and `#` suggests issues and PRs of the selected project (from the cache, matched by number or title words; an exact number not in the cache is fetched on demand). Rules that keep typing predictable:
  - The list opens only when `@` or `#` starts a word (after a space or at the start of a line). `user@host`, `C#` and Markdown headings (`#` followed by a space) never open it.
  - Picking an item (click, or arrow keys and `Enter` while the list is open) adds a context chip and leaves a readable `@moon-landing` or `#3412` in the text.
  - The list closes on a space, `Escape` or a click elsewhere. Ignoring it never changes what you typed.
  - Only the composer prompt has mentions. Other text fields (comments, session messages) do not.
- GitHub references are links by default: the agent reads them with `gh`, so they are always fresh. A later option can inline the cached thread to save the agent a round trip.
- Files added from the code viewer are references too: `- File: <absolute path>, lines 405-411` (or `, line 405`). For a revision that is not checked out: ``- File: src/x.py at origin/main (read it with `git -C <project path> show origin/main:src/x.py`), lines 405-411``.
- Presets live in `config.toml`. The three initial ones:

```toml
[[presets]]
id = "worktree"
label = "worktree"
text = "Please use a local git worktree and delete it once you're done."

[[presets]]
id = "draft-pr"
label = "draft PR"
text = "Open a draft PR once you are done and check CI if anything related to the PR is failing."

[[presets]]
id = "no-unit-test"
label = "no unit test"
text = "No need to add any unit tests for that PR."
```

### 9.7.1 Delayed starts

The composer Start section launches now by default. An "Add a start condition…" selector adds conditions, each in its own card with a Remove button: After a delay (minutes with 15/30/60-minute buttons), At a date and time (local calendar/time input), After agents are Done (searchable multi-select of sessions and scheduled launches, at most one such condition) or When a quota resets (Claude/Codex and 5h/weekly window). Several conditions combine with AND: for example, the weekly Claude reset and agent X Done. The launch button becomes Schedule when at least one condition is present. The condition list belongs to the saved composer draft, separately from the launch spec, and is cleared after submission. Scheduling does not change remembered harness/model/effort defaults; an actual successful launch does.

`scheduled_launch` in `state.db` keeps the immutable launch spec, the list of conditions, title, creation time, resolved deadline, claim state and any pre-launch error. Duration, datetime and quota conditions resolve to Unix milliseconds, and the deadline is the latest of them. A launch is ready when the deadline has passed (or there is none) and every agent condition is satisfied. Records written before multiple conditions stored a single `delay`; `schedules::list` reads it as a one-item list. Quota delays require a successful reading less than 15 minutes old and a future reported reset; that deadline is frozen when scheduling, so later quota windows cannot move it. Unknown or stale quotas are unavailable. Agent dependencies require every selected session to satisfy `SessionStatus::is_done()` at the same check; archived, interrupted, errored, terminal and background sessions do not qualify. Already-Done selections can start immediately. Scheduled launches are valid dependencies: their stable ids become session ids, so a dependent launch waits through their queue and execution until those sessions are Done. Cancelled or missing dependencies stay unavailable and never satisfy the condition; failed launches require manual attention. New dependencies can only refer to existing sessions or queued records, so launch chains cannot form cycles.

A daemon loop checks the queue once per second. The DB actor serializes cancellation and claims before any launch work, then launches ready entries independently. The scheduled id becomes the session id. Queued entries survive restarts and overdue deadlines start when the daemon returns. A claim left behind on restart is removed if its session was persisted; otherwise it is marked failed for manual retry. There is no automatic retry after a claimed start, avoiding duplicate agent execution. Harness start failures use the usual session Error state and remain in the session list; failures before a session exists remain in Scheduled with the reason and Start now/Cancel controls. Temporary folders are created only at actual launch. The scheduler honors the updater installation guard; ready entries stay queued during installation and can start after the replacement daemon returns.

`GET/POST /api/launch/schedules`, `DELETE /api/launch/schedules/{id}` and `POST /api/launch/schedules/{id}/start` expose the queue. WebSocket `schedules` events refresh it. The Agents screen adds a virtualized, independently collapsible Scheduled group and a detail pane with the conditions, dependency states, prompt preview, Start now and Cancel launch. Queued entries are neither Running nor Done and do not change sidebar session counts. Closing their detail panel leaves the queue intact.

### Scheduled follow-ups

`scheduled_launch.resume_session_id` is optional: absent means the existing new-session launch; present means `spec.text` is sent verbatim to that session through the same input and installation lock. `POST /api/sessions/{id}/follow-up` captures the message, actual project for workspace filtering and the existing AND start conditions. There is at most one pending follow-up per session, enforced by the DB actor; self-dependencies and dependencies on follow-up record ids are rejected. Follow-ups are visible in Scheduled and in a teal session band with Cancel. The scheduled detail shows the saved message rather than recomposing a launch prompt; when it starts, the selected detail opens the existing target session in Trace. The form reuses the time, quota and agent condition selector; quota reset deadlines retain the same fresh-reading requirement.

Ready follow-ups stay queued while their target runs, is in a terminal or has an unanswered question, and while installation blocks work. Sending uses the existing resumed input path and clears the archived end timestamp. A claim is persisted before input, and an interrupted claim always becomes Failed for manual inspection: the target already exists, so its existence cannot prove whether the message was delivered. Start now is a manual retry and the error explicitly warns that the message may already have been sent. Follow-up ids never become session ids and cannot be selected as dependencies for a launch. Cancel deletes the queue record without archiving or changing the target session. No new session or Relaunch button is introduced.

### 9.7.2 Cron agents

The composer has a Repeat selector under the launch options: Run once (default) or On a cron schedule…. A cron replaces the Start selector and the button becomes Create cron. The schedule is a standard 5-field cron expression (minute, hour, day of month, month, day of week) in the daemon's local time zone, with preset buttons (every hour, every day at 9:00, weekdays at 9:00, Mondays at 9:00). The daemon parses it with the `croner` crate; `GET /api/launch/crons/next?schedule=…` returns the next three run times, so the UI previews and validates with the same parser as the scheduler.

Conditions are optional and all must hold at the due time: "the previous run is no longer Running" (`SessionStatus::is_running()`, the same predicate as the sidebar) and "the harness has at least N% left of its 5-hour or weekly quota". A quota condition needs a successful reading less than 15 minutes old; an unknown, stale or failed reading blocks the run. A run whose conditions fail is skipped with the reason saved in `last_outcome`; it is not retried before the next scheduled time.

`cron_agent` in `state.db` keeps each cron as JSON: launch spec, title, schedule, conditions, paused flag, next run time, last check, last outcome and up to 50 run session ids. A daemon loop checks every 5 seconds. Each due run is claimed in the DB actor by moving `next_run_at` to the next match before any launch work, so a crash never repeats a run. A run more than 1 hour late (daemon stopped, update installing) is skipped instead of started; the loop does nothing while the updater blocks new work. Each run is a normal session with a new id and appears in Running, Done and Archived like other agents. Run now starts one run immediately, ignores the conditions and keeps the schedule.

`GET/POST /api/launch/crons`, `PATCH/DELETE /api/launch/crons/{id}` (schedule, conditions, paused) and `POST /api/launch/crons/{id}/run` expose crons. WebSocket `crons` events refresh them. The Agents screen lists crons in their own collapsible Cron group above Scheduled, and the detail pane shows the schedule, conditions, next run, last outcome, the run list, Run now, Edit schedule, Pause/Resume and Delete cron. Deleting a cron keeps its past sessions.

### 9.8 Surviving daemon restarts (later)

Two options, to decide when it becomes a real pain:

- **Session shim:** each session runs under `launchpad shim`, started with `systemd-run --user --scope` so it lives outside the daemon cgroup. The shim owns the harness stdin/stdout, appends raw output to a file and accepts input on a Unix socket. On start, the daemon reattaches to existing shims and replays their output files.
- **Codex only:** use Codex's shared app-server daemon (see [9.4](#94-codex-adapter)).

Not needed for v1, because of the dev/prod profiles and one-click resume.

## 10. Pre-draft PRs

### 10.1 Concept

A pre-draft PR is a pull request that exists only on your laptop: a local branch in a dedicated worktree, a title, a description and a status, stored in `state.db`. It appears next to GitHub PRs in project lists, in teal. Nothing is pushed, no CI runs and no collaborator is notified until you click Publish.

### 10.2 Lifecycle

```
            launch with target "Pre-draft PR"
                          │
               ┌──────────▼──────────┐   agent runs `launchpad pr update`
               │ in progress (agent) │ ◀───────────────────────────────┐
               └──────────┬──────────┘                                 │
                          │ turn finished                              │
               ┌──────────▼──────────┐   "Send feedback to agent"      │
               │  ready for review   │ ────────────────────────────────┘
               └─────┬──────────┬────┘   (same session, your notes as a message)
             Publish │          │ Discard
    (draft or ready) ▼          ▼
         GitHub PR #N           branch and worktree deleted,
         (record kept as a      record archived
         link to #N)
```

### 10.3 How it works

1. In the composer, choose **Pre-draft PR** as the place to run (next to the project). It replaces the "draft PR" preset.
2. Launchpad runs `git fetch`, creates branch `lp/<slug>` from `origin/<default branch>` in the worktree `<projects_root>/.worktrees/<project-id>/<slug>`, creates the pre-draft record, and starts the agent in the worktree with this instruction appended to the prompt:

   > You are working in a dedicated git worktree on branch `lp/<slug>`. Commit your work on this branch. Do not push and do not open a pull request on GitHub. When you are done, set the title and description with `launchpad pr update --title "…" --body-file <file>`.

3. `launchpad pr create|update|show` finds the pre-draft PR from the current worktree and calls the daemon. `create` also works from any committed branch in a registered project or worktree: it copies the current HEAD into a new dedicated pre-draft worktree and returns its path, preserving the source checkout. The local review page offers Launch agent to run in that worktree and link a session. Commit uncommitted work before creating the record.
4. A shared skill at `~/.agents/skills/launchpad/SKILL.md` (Claude reads it through your existing `~/.claude/skills` symlinks; Codex reads `~/.agents/skills`) explains when and how to use `launchpad pr`, the expected PR description format, and later `launchpad suggest`. Launchpad links it from the repository at startup, so it always matches the code.
5. Review in the PR view: description, commits, and files changed computed locally (`git diff origin/main...lp/<slug>`). A banner shows when the base branch moved ("main is 3 commits ahead").
6. **Feedback loop:** write notes (general, or attached to a file). "Send feedback to agent" resumes the same session with the notes as a message. The agent amends the branch and you review again.
7. **Publish:** choose the remote branch name (default `<github-user>/<slug>`), Launchpad pushes the reviewed commit with `git push origin <reviewed-head>:refs/heads/<name>`, sets the local branch upstream, then `POST /repos/{o}/{r}/pulls` with `draft: true` or `false`, the title and the body. The record now links to `#N`. The worktree is kept by default, or removed when clean if `remove_published_worktree` is enabled in General settings. The displayed head must still match both the branch and worktree HEAD, and no agent may be running in the worktree.
8. **Discard:** after confirmation, removes a clean worktree, deletes the branch and archives the record. Running agents and uncommitted changes block cleanup. Published and discarded records remain accessible in the local list.

## 11. Agent-written suggestions

`launchpad suggest comment|review --repo <owner/repo> --number <n> --body-file <file> [--event comment|approve|request-changes] [--session-id <id>]` saves a local `draft_comment` record. Reviews require `--head <40-character-inspected-commit>`; the CLI never substitutes a cached or current head. Replace repo/number with `--predraft <id>` for a local PR. Creation and editing do not call GitHub. Text is limited to 64 KB. Session attribution comes from the daemon’s session record, with “Agent” as the fallback when no session id is supplied. Every launched session receives the daemon’s exact binary path, profile and its own session id, and the linked Launchpad skill describes the commands.

Issue and PR conversations put teal suggestion cards in the existing virtualized timeline. The cards render safe Markdown and offer Edit, Post comment/review and Discard. Edit includes a review event selector. Saved edits live in `state.db`; unsaved edits survive reloads in browser storage. WebSocket events invalidate the suggestion query. Review with agent opens the normal composer with the PR reference, displayed head and instructions to inspect it and save a local suggested review, leaving launch choices to the owner.

Posting uses the normal GitHub mutation service. Before a suggested review posts, a forced detail refresh verifies that the item is an open PR and still has the inspected head; the review also sends that head as `commit_id`. The daemon serializes edits, discard and publication through the existing local PR lock. Before any GitHub mutation, it persists a posting claim. A successful response saves the posted state and URL independently of the later UI refresh. Failures retain an uncertain state; restart preserves a posting claim. Neither state can be posted again. The owner must inspect GitHub before discarding and replacing an attempted suggestion. This avoids automatic duplicate submissions when a response is lost.

Local PR description cards use Add to local notes instead of a GitHub publication button. The note and posted marker are saved together in a SQLite transaction; notes remain local until the owner explicitly sends feedback. Publishing a pre-draft does not publish its suggestions. Line-specific suggestions wait until inline comments are supported.

## 12. Frontend

- **Libraries:** React 19 + TypeScript + Vite, with pnpm as the package manager; TanStack Router (typed routes); TanStack Query (cache, background revalidation, optimistic mutations; WebSocket events call `invalidateQueries` or `setQueryData`); Zustand for UI state (pins, collapsed groups, composer draft, selection); Tailwind CSS v4 with Radix primitives (shadcn-style components copied into the repo); TanStack Virtual (lists, diffs, long sessions); Shiki in a Web Worker; markdown-it; DOMPurify; github-markdown-css.
- **Theme:** light only. One set of CSS variables, no dark tokens.
- **Layout:** left sidebar (Inbox, Agents, Projects, pinned projects, temporary projects, Settings; collapses to icons), main area, composer drawer on the right, status bar at the bottom.
- **Harness marks:** the official Claude and Codex logos (SVG files in `web/src/assets/`) everywhere a harness appears: session lists, pane headers, status bar.
- **Speed tactics:** render from cache; prefetch on selection; keep list views mounted so scroll position and selection survive navigation; optimistic actions; virtualized lists; every frequent action one click away.
- **Click only:** no keyboard shortcuts except `Ctrl+F` in the code viewer (see principle 5). Frequent actions are visible buttons where they are used: hover actions on notification rows, a Launch button on every notification, issue, PR and project, Refresh and Open on GitHub on every cached object. Icon-only buttons have a hover title. Context is added to the composer with buttons, or with `@` and `#` suggestions in the prompt (see [9.7](#97-prompt-composition)).
- **WebKitGTK:** the Tauri webview on Linux is WebKit, not Chromium. Measured in [spike 4](spikes/4-webkitgtk.md): a virtualized 5,000-line diff and a 2,000-event session scroll at 60 fps on an idle machine. With all CPU threads busy (a large build), WebKitGTK drops to about 20 fps where Chrome stays at 60 fps, and mounting thousands of plain rows is about 14 times slower than in Chrome. Rules that follow: virtualize every long list, memoize row components, create virtualizers with `useFlushSync: false`, prefer fixed row heights, keep overscan small. The page `/dev/perf?auto=1` repeats the measurement. If the window renders blank with some GPU drivers, `WEBKIT_DISABLE_DMABUF_RENDERER=1` is the usual workaround. Because the UI is a plain web app, it can be developed in Chrome against the dev daemon, and switching the shell to Chromium (`--app=` mode) stays possible if scrolling during builds becomes a problem.

## 13. Desktop integration

- Desktop notifications are sent by the daemon, so they work when the window is closed. Examples: "Claude is waiting in huggingface_hub", "Review requested: huggingface_hub#3420". Clicking one focuses the window on the item (`launchpad-desktop --open <route>`).
- External apps are chosen in Settings → General (stored in `config.toml`, see [6.5](#65-configuration-file)): the terminal used by "Open in terminal" (Terminator by default; Ghostty, GNOME Terminal or a custom command), the editor (`code` by default; Cursor is also installed) and the browser (`xdg-open`).

## 14. Security

- The daemon can start agents that run any command. Its API must be treated like remote code execution.
- It binds `127.0.0.1` only. At first start it writes a random 256-bit token to `~/.local/share/launchpad/token` (mode 0600). The desktop shell opens `/auth?t=<token>`, which sets an `HttpOnly`, `SameSite=Strict` cookie. The CLI sends `Authorization: Bearer <token>`.
- Requests whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>` are rejected (DNS rebinding). WebSocket upgrades and mutations with a foreign `Origin` are rejected (CSRF). No CORS headers.
- The GitHub token stays in daemon memory. Logs redact tokens.
- The code viewer endpoints only read inside a registered project or one of its worktrees: `root` must be one of them (a linked worktree is checked against git's own records: its `.git` file points to `<main>/.git/worktrees/<name>`, whose `gitdir` points back), `path` must be relative without `..` or `.`, and the resolved path (after following symlinks) must stay inside `root`. A revision is resolved once with `git rev-parse --verify --end-of-options <rev>^{commit}`, and later commands only get the commit id (`git grep` in git 2.34 does not accept `--end-of-options`). Search patterns are passed with `-e` and pathspecs are literal, so user input is never read as an option or a pathspec pattern.
- For remote access later, keep the daemon on localhost and reach it through an SSH tunnel or Tailscale before adding any other authentication.

## 15. Future: remote access

The UI is already served by the daemon over HTTP and WebSocket. Remote access means running `launchpad daemon` on the server (agents and projects live there), connecting through a tunnel, and opening the UI in a browser or pointing the Tauri shell at the remote URL. Features tied to the local machine ("open in editor", "open in terminal") become "copy command".

## 16. Risks and unknowns

| Risk | Impact | Mitigation |
|---|---|---|
| Claude stream-json output changes after an auto-update | Adapter breaks | Raw event log, so the normalizer can be fixed and re-run on recorded sessions; `claude install <version>` to pin a known-good version |
| Claude usage endpoint is undocumented | Quota bar shows "unknown" | Fallback to `rate_limit_event`; isolated module |
| `codex app-server` is marked experimental | Breaking protocol changes | Schema diff script after updates; version shown in the status bar |
| TUI-only features are missing in headless sessions (slash commands, rewind, interactive pickers) | Some workflows still need the terminal | "Open in terminal" and "Re-attach" |
| No "mark as unread" API on GitHub | Launchpad and github.com disagree | Local override, documented in the UI |
| WebKitGTK performance | Slow large diffs or long sessions | Virtualization, early measurement, Chromium shell as fallback |
| Expiring image URLs in private repositories | Broken images | Image proxy with a disk cache, filled at sync time |
| Daemon restart kills running agents | In-flight turn lost | Dev/prod profiles, resume, session shim later |
| Very large repositories or files | Slow code viewer | File list sent once per source and virtualized; no colors above 1 MB or 20,000 lines, nothing loaded above 5 MB; search capped at 1,000 results |

## 17. Decisions log

Confirmed on 2026-10-02:

1. **Headless sessions.** Launchpad renders sessions itself. Losing TUI-only features is fine; "Open in terminal" is the escape hatch. No embedded terminal for now.
2. **Background daemon.** The systemd user service with a thin Tauri window is the target architecture.
3. **Agent groups.** Updated by decision 23: Done waits for input; Running includes background and long commands; Archived remains resumable (see [9.1](#91-session-model)).
4. **Pre-draft PRs.** Launchpad creates the branch and worktree under `<projects_root>/.worktrees/` and links a `launchpad` skill into `~/.agents/skills`.
5. **Temporary folders.** `<projects_root>/tmp-<MMDD>-<slug>`. The projects root is configurable, default `~/projects`.
6. **Claude quota.** Launchpad may read the OAuth access token in `~/.claude/.credentials.json` to call the usage endpoint, never refreshing it.
7. **Harness icons.** Harness marks use the Claude and Codex logos. Launch buttons open the composer directly (updated by decision 23).
8. **Click-only UI.** No keyboard shortcuts, at least at first. Two typing behaviors only: `@` and `#` suggestions in the composer prompt (added on 2026-10-02, see [9.7](#97-prompt-composition)), and `Enter` to send in a session's message box. Shortcuts may be added later if they are really missed. Amended on 2026-10-02 by decision 22 (`Ctrl+F` in the code viewer).
9. **Terminal is a setting.** "Open in terminal" uses the terminal chosen in Settings: Terminator by default, Ghostty, GNOME Terminal, or a custom command.
10. **Minimal tests.** Move fast and break things. Test only small, specific parts that clearly benefit from it. No mocks, no tests that need a server, and the whole run takes a few seconds at most. Details in [5.2](#52-testing).

Phase 0 spikes, on 2026-10-02 (notes in [docs/spikes/](spikes/)):

11. **Claude adapter flags.** Sessions add `--replay-user-messages` and `--permission-prompt-tool stdio` (needed for AskUserQuestion cards). A resume uses `--resume <id>` instead of `--session-id`. Harness processes get an environment without the `CLAUDECODE` and `CLAUDE_CODE_*` variables. A message typed during a turn joins the running turn. The Agent SDK sidecar fallback is not needed.
12. **Claude quota endpoint.** `/api/oauth/usage` works with the read-only token. Launchpad checks `expiresAt` and never calls with an expired token.
13. **Codex adapter.** `codex app-server` works as planned (steer, interrupt, resume after restart). The adapter closes open items when a turn ends.
14. **Claude TUI trust.** "Open in terminal" sets `CLAUDE_CODE_SANDBOXED=1` for temporary projects, so the trust dialog does not appear. Launchpad does not write `~/.claude.json`.
15. **WebKitGTK stays.** The Tauri shell is fast enough on an idle machine; the Chromium `--app=` fallback is kept for later if scrolling during agent builds bothers you. The UI follows the list rules of [section 12](#12-frontend).
16. **Warm window.** The prod shell keeps one hidden process (autostart at login, hide on close), because a cold start takes about 0.5 s and the goal is 300 ms.

Phase 2 implementation, on 2026-10-02:

17. **Ignore on unsubscribe.** Use `PUT` with `ignored: true` followed by Done, so watched repository threads stop notifying too. The [GitHub notification API documentation](https://docs.github.com/en/rest/activity/notifications#set-a-thread-subscription) distinguishes ignoring from deleting a subscription.
18. **Durable inbox overrides.** Local unread, read and done flags belong in `state.db`, scoped to the last notification update time. The disposable GitHub cache can be rebuilt without losing the owner's triage choices; fresh GitHub activity expires an override.
19. **Summary previews first.** Phase 2 uses safe Markdown rendering of the description and latest eight comments, plus REST summaries for releases and commits. Full HTML timelines, images and reviews arrive in Phase 3. GitHub context references allow a missing issue number for release, commit and other notification subjects. An unmapped notification clears any previous composer project target, so it cannot launch accidentally in an unrelated repository.

Inbox correction after daily use, on 2026-10-02:

20. **REST history is not the web inbox.** Real account verification found 242 REST notifications, including 72 explicitly ignored threads, while the owner's GitHub inbox had four items. The original assumption that `all=true` excluded Done threads was wrong. Subscription filtering removes ignored threads; durable local membership excludes the remaining historical read threads. The current membership can be restored from the owner's known four-item screenshot. This is a local repair, not a claim that the public API supports continuous exact web-inbox synchronization.

Code viewer, asked for on 2026-10-02 (designed in sketch S7, built the same day, see decisions 30 to 32):

21. **Code viewer.** A read-only viewer (file tree, tabs, find in file, search in project) over a checkout, a worktree or any git revision, with selected lines usable as agent context. It uses the code lines of the diff view (TanStack Virtual, Shiki in a Web Worker) instead of Monaco or CodeMirror: nothing needs editing, Monaco is heavy and brings its own keyboard model, and one renderer tuned for WebKitGTK is simpler. If light editing is wanted later, CodeMirror 6 is the option. Details in [8.1](#81-code-viewer).
22. **Ctrl+F.** The first keyboard shortcut. `Ctrl+F` opens the find bar while the code viewer is on screen. In the find field, `Enter` and `Shift+Enter` go to the next and previous match and `Escape` closes the bar. A visible Find button does the same, so every action still has a button. This amends decision 8.

Usability updates after daily use, on 2026-10-02:

23. **Direct launch and readable status.** Every Launch button opens the composer with the harness, model and effort selections from the latest launched session, kept in `state.db` with its launch spec. Picking a harness or editing an unsent draft does not change these remembered defaults. Done means waiting for input; background commands stay Running. The Agents navigation and pinned project markers share the same state predicates.
24. **Readable scale.** The whole light interface scales through CSS zoom, including fixed-height virtualized rows, icons and panels. General settings persist five zoom positions in `config.toml`: 100%, 110%, 120% (default), 135% and 150%. Secondary text and borders have stronger contrast.
25. **Image messages.** Session message boxes accept image file selection, drops anywhere in the console and clipboard paste. Previews can be removed before Send; image-only messages are supported. PNG, JPEG, GIF and WebP images are limited to 5 MB each, eight images and 20 MB per message. The authenticated input endpoint validates data URLs and raises its request limit to 32 MB. Attachments remain in the local event log, appear in the conversation and are sent as native image content to Claude or Codex, including steer and resumed turns. Clipboard paste remains normal text-field behavior.
26. **AI session titles.** Every newly launched agent gets an AI title based on its initial prompt. A separate short request runs in the background and broadcasts the persisted title without blocking launch. A prompt-derived title covers naming failures.
27. **Self-update.** The production settings button runs checks and release builds in a transient systemd user unit, then uses the existing installer. The updater survives the daemon restart, and the replacement desktop window starts in its own user unit so it survives the updater's exit. Running agent sessions delay installation; an idle reservation prevents new turns during replacement. The source is the current local checkout; fetching changes remains a separate action.

Continuous integration, on 2026-10-02:

28. **Three Ubuntu CI jobs.** GitHub Actions checks strict Rust linting and UI types, runs the existing fast Rust tests, and builds downloadable release binaries with the UI embedded. Ubuntu 24.04 x86_64 matches the Linux-only scope. Pin Rust for repeatable linting, preserve binary permissions inside a tar archive, and keep release publishing and installation outside CI.

29. **Separate panel closing and session archiving.** The session header X only closes the panel, leaving Done or Running unchanged. A labeled Archive button performs the former session-close action and keeps the conversation resumable. The list and closed-state badges say Archived; storage and API names stay `closed` and `/close`.

Code viewer implementation, on 2026-10-02:

30. **Revisions become commit ids.** The daemon resolves a revision once (`rev-parse --verify --end-of-options`) and passes only the commit id to `ls-tree`, `cat-file`, `diff` and `grep`, because `git grep` has no `--end-of-options` in the installed git 2.34. Roots are accepted only when they are registered projects or worktrees recorded by git.
31. **Shiki with the JavaScript regex engine.** The worker uses Shiki's JavaScript engine in forgiving mode instead of the Oniguruma WASM build, and loads grammars on demand from an ES module worker. Files over 1 MB or 20,000 lines keep plain text.
32. **No Enter in the viewer's other fields.** "Go to file" filters as you type and opens a file on click; search in the project runs as you type. The find field keeps its `Enter`, `Shift+Enter` and `Escape` keys (decision 22). The selection bar sits at the bottom right of the code, so the line numbers stay clickable, and its GitHub link copies a permalink instead of opening the browser.

33. **Commit status and deferred installation.** The status bar displays the running build’s Git commit and offers a small Update button when local main is ahead. Checks and builds run in the background; installation waits for no Running sessions, using the same predicate as the sidebar and counts. The updater never interrupts active agents or builds uncommitted work.

34. **Collapsible agent groups.** Cron, Scheduled, Running, Done and Archived (in that order, top to bottom) have independent collapse buttons with visible counts. Expansion is persisted as UI layout state; only expanded rows enter the fixed-height virtualized list.

Delayed launches, on 2026-10-02:

35. **Durable delayed starts.** A small explicit Start selector covers durations, local datetimes, all-selected-agent Done conditions and reported quota reset deadlines. Persist the queue separately from sessions, claim before starting, use a stable session id for crash recovery, and require manual retry for an interrupted claim without a session. Scheduled work stays separate from Running and Done.

Phase 3 implementation, on 2026-10-02:

36. **Cached GitHub detail pages.** `/github/{owner}/{repo}/{number}` shows a paginated conversation, Files changed and read-only Review threads; commits remain in the conversation. Detail, project list and diff JSON live in the disposable `gh_response` cache with separate key prefixes. GET serves cached content immediately; the UI refreshes stale details after 60 seconds and project lists after five minutes. Project list responses require both PR and issue arrays; a missing GitHub repository or incomplete response returns an error instead of poisoning the cache, and an older invalid cache entry is ignored. The UI tolerates older null lists so project pages and agents remain accessible. Background detail refresh compares `updatedAt` before loading a timeline, but refreshes header fields and checks even when the timeline is unchanged. Explicit Refresh always fetches the full detail. GitHub publication success is kept separate from a later cache-refresh failure.
37. **Review the displayed head.** Review submissions include `commit_id` and merge submissions include `sha` from the displayed detail. The merge box requires a second click and remembers the last successful method per repository. GitHub enforces repository rules and permissions; API failures remain visible with the unsent draft preserved. No automated verification publishes or merges real GitHub items.
38. **One selected diff file.** Files changed virtualizes the file list and the selected file's rows, highlighting only that file through the existing worker. Split columns have independent horizontal scrolling and synchronized vertical scrolling. View file uses the PR head, or the base for a deleted file, through the existing code viewer root/revision checks; an unmapped repository opens the GitHub blob instead. API fallback patches can be truncated by GitHub and are labeled accordingly. Viewed markers are scoped to repository, PR, head commit and path; diff mode, viewed markers, merge methods and unsent comment/review drafts persist in browser local storage.


Phase 4 implementation, on 2026-10-02:

39. **Owned local PR worktrees.** The `predraft` table stores each record as JSON, including title, body, notes, session link and publication checkpoints. Worktrees use a stable project id and unique slug under `.worktrees`; local branches use `lp/`. CLI creation from existing work copies committed HEAD into an owned worktree instead of adopting or removing another checkout. Local status follows the linked session: only Waiting is Ready for review, and Starting/Working/Background remain In progress. Local detail GETs run git against committed refs without a network request; explicit Refresh fetches origin. General and file notes persist on the daemon, and unsent edits also survive browser reloads.
40. **Publication is a separate operation.** Publish requires a second click, the remote name, draft/ready choice, the reviewed head, a clean worktree and an idle agent. The daemon serializes publication/discard with session starts and input through the installation guard, and records the remote name and reviewed head before pushing without force. Further turns in that worktree are blocked once publication starts. Retry uses the same head and remote name and looks for a matching PR (branch, commit and base) before creating one. A successful GitHub URL is saved before optional cleanup; cleanup failure keeps the published record and worktree. Discard never deletes a remote branch, and a publication that already attempted a push must be completed through Publish. The `launchpad` skill is linked from the main checkout at startup; agents receive the daemon's exact CLI path and profile so dev sessions cannot update production records accidentally.


Phase 5 implementation, on 2026-10-02:

41. **Local workspace arrangements and explicit history import.** Named layouts and pins use the existing browser layout store; titles and imported event snapshots use `state.db`. Imports remain archived until the owner resumes them through the native harness id. Read-only session Git inspection uses literal pathspecs and the existing code viewer checks for untracked content. Visible focus and move controls accompany drag/drop, preserving the click-only interaction model.
42. **Labels in project lists.** Fetch label names and colors alongside the existing recent open issue and PR summaries. Shared label chips use a GitHub-colored edge with readable text on a light background. A visible selector filters the cached 30 recent open items per tab by one label or by no labels; counts and empty states make that scope explicit. Old cached summaries without labels refresh in the background. Selection for agent context remains independent of the filter. Local PR rows remain visible separately from the filtered GitHub count.

Phase 6 implementation, on 2026-10-03:

43. **Local agent suggestions with explicit publication.** Durable suggestion records belong in `state.db`, with session attribution and the inspected review head. GitHub posting requires a visible card action, checks the PR head and persists a claim before the mutation. Failed or interrupted claims require manual inspection instead of automatic retries. Local PR cards add notes atomically and never publish text.

Scope correction, on 2026-10-03:

44. **Defer Phase 7 until an item is scoped.** Revert commit `7b670af`, including session workers, private session sockets, the embedded PTY/TUI and remote browser options. Keep the daemon-owned headless adapters, external terminal handoff and native resume from Phases 0–6. A daemon restart interrupts running turns; restart survival remains a future candidate. Phase 7 is an unselected options list, not a commitment to build several extensions together. Before selecting an item, define its user need, ownership and failure behavior, non-goals and acceptance check. Phases 1–6 still need their daily-use checks.

Scheduled launch conditions, on 2026-10-03:

45. **Combined start conditions.** A scheduled launch stores a list of conditions that must all be met, instead of one delay. Time and quota conditions still resolve once to a single deadline (the latest one); agent conditions are checked live. There is no OR: one launch per alternative covers that case without a condition tree.

Cron agents, on 2026-10-03:

46. **Cron agents claim by advancing the schedule.** Recurring launches are separate `cron_agent` records with a 5-field local-time schedule (`croner`) and optional conditions (previous run finished, minimum quota left). The scheduler moves `next_run_at` forward before launching, so runs are never repeated after a crash; failed conditions and runs more than 1 hour late are skipped, not caught up. Runs are ordinary sessions; crons have their own Cron group and never count as Running or Done.

Workspaces, on 2026-10-04:

47. **Workspaces are sets of projects, chosen per window.** A workspace stores only project ids; sessions, scheduled launches, crons and inbox threads are filtered through their project or repository, so there is no second ownership model to keep in sync. Launches carry the window's workspace id and add their project to it. The selection is per window (`sessionStorage`), and switching is instant in the same window; another workspace in parallel is one click on New window. Several windows share the one daemon, which keeps running all agents; prod windows live in the single warm shell process, dev windows are separate processes. The existing session Git module was renamed `session_git` so "workspace" only names this concept, and the Agents toolbar label became "Panes" (that toolbar was removed on 2026-10-04).

Platform support and GitHub builds, on 2026-10-04:

48. **Linux first, native macOS and Windows builds.** The owner expanded the supported platforms. Unix-only signal, executable permission and session APIs are gated; Linux keeps systemd and parent-death signals, macOS uses launchd and detached processes, Windows uses login shortcuts and a detached daemon. Defaults use each platform’s browser and terminal; agent CLI instructions use Unix shell or PowerShell syntax for the host. Home-relative config/data locations remain the same. CI tests and builds natively on Ubuntu (22.04 release baseline, 24.04 lint/debug tests), both macOS architectures and Windows x86_64; unsigned archives include installation resources and the agent skill. This supersedes the Linux-only scope in decision 28. Native build/test success does not replace daily-use desktop and harness acceptance on the new systems.
49. **Update from GitHub-built latest main.** Continuous CI artifacts supply updates until release cycles are introduced. The exact GitHub main commit must have a successful trusted workflow run and a retained platform artifact; there is no source-build or older-commit fallback. Downloads and checks precede the existing idle installation reservation. This supersedes the local checkout update source in decisions 27 and 33. Agent suggestions remain local and only owner clicks publish them.

50. **Keep only the current pane arrangement.** Removed the Agents Panes toolbar and named layout save/restore/delete on 2026-10-04. At that point, the top-bar pane count switch, click-to-focus and session Move controls, drag/drop, and per-workspace persistence of the current arrangement remained; decision 54 removes them.

Project metadata refresh, on 2026-10-04:

51. **Derive project remotes from current local configuration.** Registration is not the source of truth for origin and upstream. Refresh metadata on project reads and every 30 seconds in the daemon, and invalidate open UI project lists through change events. Missing project folders retain the last known mapping; local project edits remain independent of Git metadata.

51. **Visible workspace switching.** Replaced the workspace dropdown on 2026-10-04 with a wrapping button row below the top bar, so every workspace can be selected in one click. Keep All projects, the current selection, New window buttons and Manage workspaces visible.

52. **Single-row top bar.** Moved the workspace buttons into the top bar row on 2026-10-09 to save vertical space, and moved workspace management to a Settings → Workspaces page. The buttons stay one click away and only wrap to a second line when the window is too narrow. This supersedes the dedicated row and the visible Manage workspaces button of decision 51.

53. **Edit session titles in place.** On 2026-10-09, replaced the separate Rename button and rename row with an inline title field and Save title/Cancel controls. Replaced Pin with a star on the right of the title, and grouped Git and Open in editor with Files and Archive in the header action row.

54. **One agent panel per window.** Removed the 1/2/4-pane switch, pane arrays and focus state, session move/swap controls and session drag/drop on 2026-10-09. Each workspace remembers one selected session, pins and collapsed groups. Cron and Scheduled details replace the conversation; notification presence reports only the visible session. Several agents can still run concurrently. Image paste and drop remain supported.

55. **Tag-only release builds.** On 2026-10-09, restricted native release builds, release tests and artifact uploads to tag refs. Main pushes and pull requests retain lint and debug tests; manual dispatch builds binaries only on a tag. This supersedes continuous artifact production in decision 49. The updater still searches for the exact main commit in main-branch runs, so adapting it to tagged builds remains pending.

56. **Static HTML previews.** On 2026-10-10, added HTML previews alongside Markdown in the code viewer. Sanitize document markup while retaining inline styles, remove navigation targets and resource links, and render in a frame with no sandbox permissions. A document CSP permits only inline styles and data images/fonts. Source remains the default; interactive pages and external assets stay outside this preview.

57. **Outcome-first session panels and same-session follow-ups.** On 2026-10-10, implemented the owner-approved S8 scope: non-running panel loads show recorded run facts, the initial task, the exact last message and factual actions on one white surface; Running and explicitly selected Trace remain the conversation. Track owner stops separately until acknowledged and persist Stopped, while failed turns use Failed. Extend the existing schedule JSON with an optional resume target instead of a separate queue/table, preserving AND conditions, reported quota deadlines and updater serialization. Claimed follow-ups require manual inspection after a crash because an existing target cannot prove delivery. No AI summary, free-text action parsing, skill changes or GitHub publication.

58. **Per-session paid usage control.** Owner-approved scope: default to best-effort included-quota protection; persist quota pauses and pending input, automatically continue only on fresh confirmed capacity, allow an explicit paid override, and let Stop cancel the continuation into Done. Keep all enforcement in the daemon and account for commands that finish after interruption. Unknown quota readings do not prevent manual work, but never authorize automatic continuation. Uncertain continuation claims need manual inspection after restart.

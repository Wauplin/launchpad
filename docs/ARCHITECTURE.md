# Launchpad architecture

Status: proposal (2026-10-02), main decisions confirmed (see [Decisions log](#17-decisions-log)). Nothing is implemented yet. Companion documents: [ROADMAP.md](ROADMAP.md) and the UI sketches in [design/sketches.html](design/sketches.html).

## 1. What Launchpad is

Launchpad is a desktop app for Ubuntu, launched from the dock, that combines two things:

- an **agent launcher and workspace** for Claude Code and Codex: start a session in the right folder with the right context in a few clicks, watch and drive several sessions side by side;
- a **fast GitHub client** for the parts of GitHub used every day: notifications, issues, pull requests, reviews, merges. Everything is cached locally so moving between items never reloads a page.

On top of these, Launchpad adds **local-first objects** that GitHub does not have: pre-draft PRs (a branch + title + description that exists only on the laptop) and, later, agent-written comments and reviews that wait for manual approval before being posted.

It is a single-user tool. Light theme only. Linux only. No versioning, no backward compatibility, no migration framework.

### Design principles

1. **Never wait on the network to show something.** Every screen renders from the local database first and refreshes in the background. Network latency appears as a small "syncing" indicator, never as a blank page.
2. **Launching an agent is the main action.** A minimal launch is three clicks plus the prompt. A rich launch uses the same flow with more chips. Both must feel natural.
3. **The core does not depend on a harness.** Claude Code and Codex are adapters behind one session model. Adding a third harness means writing one adapter.
4. **Local first, publish later.** Anything that would create noise on GitHub (pre-draft PRs, agent-written comments) stays local until you publish it. These objects share one color in the UI (teal).
5. **Click only.** No keyboard shortcuts, at least at first: every action is a visible button where it is needed, and the UI should be good enough that shortcuts are not missed. Text fields only type text, with two exceptions: in the composer prompt, `@` and `#` at the start of a word open a suggestion list (projects, issues and PRs, see [9.7](#97-prompt-composition)), and in the session message box `Enter` sends and `Shift+Enter` adds a new line, like in the terminal.
6. **Owned and easy to extend.** A mainstream stack that coding agents handle well (Rust, React, TypeScript, Tailwind), configuration in a plain TOML file, GitHub payloads stored as JSON so new fields need no schema change.
7. **Move fast, test little.** This is a personal project: speed of change matters more than protection against breakage. Most code has no tests. See [5.2](#52-testing).

## 2. Key decisions

| Topic | Decision | Why |
|---|---|---|
| Process model | A background daemon (`launchpad daemon`, systemd user service) plus a thin Tauri window | Agents keep running when the window is closed or the UI is rebuilt. The daemon is always warm, so the window opens instantly. The same server can later run on a remote machine. |
| UI ↔ backend | HTTP + WebSocket on `127.0.0.1`. The Tauri window loads the UI from the daemon | One transport for desktop and future remote access. The frontend has no Tauri dependency and also runs in a normal browser. |
| Agent integration | Structured protocols: `claude -p` with stream-json, `codex app-server` (JSON-RPC) | Launchpad renders the conversation itself: exact copy of messages, token and cost numbers, grid layouts, cards for questions. An "Open in terminal" button covers features that only exist in the TUI. |
| Frontend | React + TypeScript + Vite, TanStack Query / Router / Virtual, Tailwind, Radix primitives | Largest ecosystem, and the stack that coding agents write best, which matters for a codebase you extend with agents. |
| Storage | SQLite (WAL). `state.db` for your data, `cache.db` for GitHub data | The cache can be deleted and rebuilt on any schema change, so no migrations are needed for it. |
| GitHub access | Token from `gh auth token`, direct REST + GraphQL calls from Rust | Much faster than spawning `gh` for each call. GraphQL fetches participants and PR details in batches. |
| Diffs | Local git first (fetch `pull/N/head`), GitHub API as fallback | Instant, handles very large PRs, and the same code path serves pre-draft PRs. |
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
- Runs as a systemd user unit `launchpad.service`: starts at login, restarts on crash, logs to journald (`journalctl --user -u launchpad -f`).
- Owns everything stateful: SQLite, GitHub sync loops, agent processes, quota polling, desktop notifications (D-Bus through `notify-rust`), and opening URLs, editors and terminals.
- Listens on `127.0.0.1:7317` only.

### 4.2 Desktop shell

- A Tauri v2 app of roughly 100 lines of Rust. On start it checks `GET /api/health`. If the daemon is down, it runs `systemctl --user start launchpad` (or spawns `launchpad daemon` directly as a fallback), then opens a window on `http://127.0.0.1:7317/` after the auth handshake described in [Security](#14-security).
- Plugins: `single-instance` (clicking the dock icon focuses the existing window; `launchpad-desktop --open <route>` focuses it on a given screen, used by desktop notifications) and `window-state`.
- The window loads the UI from the daemon, not from bundled assets. This gives one origin (no CORS) and the same UI works in a browser for the future remote mode. The UI imports no `@tauri-apps/*` package.
- The `.deb` bundle installs a `.desktop` entry, which can be pinned to the dock. During development, a hand-written `.desktop` file points to the release binary. `StartupWMClass` must match the window so the dock groups it correctly.

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
│  │                            #   sessions, harness adapters, compose, predraft, quota
│  ├─ server/                   # axum routes, WebSocket, auth, static UI serving
│  └─ cli/                      # `launchpad` binary: daemon + agent-facing subcommands
├─ desktop/                     # Tauri v2 shell (src-tauri)
├─ web/                         # React app (Vite)
│  └─ src/
│     ├─ app/                   # shell: sidebar, top bar, status bar, routing
│     ├─ features/              # inbox, launch, agents, github, projects, predraft, settings
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

## 6. Backend

### 6.1 Stack

`tokio`, `axum` + `tower-http`, `reqwest` (rustls), `serde`/`serde_json`, `rusqlite` (bundled SQLite) behind a small DB actor, `tracing`, `ts-rs`, `notify` (config file watch), `uuid`, `time`, `notify-rust`. Git operations shell out to the `git` CLI: it uses your SSH configuration as-is and is simpler than libgit2.

### 6.2 Modules (in `crates/core`)

| Module | Responsibility |
|---|---|
| `config` | Load and watch `config.toml`: presets, external commands (editor, terminal), poll intervals |
| `projects` | Registry, discovery in the projects root, repo ↔ project mapping, temporary projects |
| `git` | Status, branches, worktrees, PR ref fetch, diff parsing into files/hunks/lines |
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
POST   /api/projects/temp    POST /api/projects/{id}/promote    DELETE /api/projects/{id}
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

- `state.db` holds data you would not want to lose: projects, sessions and their events, launch drafts, pre-draft PRs, draft comments, local notification overrides. Schema changes are a list of SQL statements applied in order using `PRAGMA user_version`. This takes about 20 lines of code and is not a framework.
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
pending_after_silent_command_seconds = 30   # see 9.1

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

- **Polling:** `GET /notifications?all=true` (read and unread threads; threads marked as done are never returned). Each tick fetches the first page with `If-Modified-Since` and respects the `X-Poll-Interval` header (usually 60 s). A full paginated pass runs at startup and every 30 minutes to remove threads that were marked as done on github.com.
- **Enrichment:** the notifications payload has no participants and no subject state. For new or changed threads, one batched GraphQL query (aliases, about 20 subjects per request) fetches for each subject: state, `isDraft`, merged, author, `participants(first: 8)` with `totalCount`, labels, comment count, `updatedAt`, and for PRs `reviewDecision` and `statusCheckRollup.state`.
- **Reasons:** the `reason` field maps to a badge: `review_requested`, `mention`, `team_mention`, `assign`, `author`, `comment`, `subscribed`, `state_change`, `ci_activity`, `manual`, `security_alert`, …
- **Actions:**

| Action | API |
|---|---|
| Mark as read | `PATCH /notifications/threads/{id}` |
| Done | `DELETE /notifications/threads/{id}` |
| Unsubscribe | `DELETE /notifications/threads/{id}/subscription`, then mark as done (to verify: the web inbox seems to do both) |
| Mark as unread | **No public API.** Checked on 2026-10-02: no REST endpoint and no GraphQL mutation. Implemented as a local override: Launchpad shows the thread as unread until you open it or GitHub reports new activity. github.com still shows it as read. |

- Opening a thread (preview or full view) marks it as read, like github.com. The "Mark as unread" button sets it back.
- All actions are optimistic: the cache and the UI change immediately, the API call follows, and a failure rolls back with a toast.

### 7.3 Issues and pull requests (cached on demand)

- **Detail query:** one GraphQL query per item fetches the header fields, `bodyHTML` and `body`, `timelineItems` (comments, reviews, review threads as read-only, commits, label/assign/rename/reference events, ready-for-review, merged, closed), participants, review requests, latest reviews, assignees, labels, milestone, `closingIssuesReferences`, check contexts (name, conclusion, details URL), `mergeable`, `mergeStateStatus`, `headRefName`, `baseRefName`, `headRefOid`.
- **Freshness:** the cached version renders immediately. A background refresh runs when an opened item is older than 60 s, or when you click Refresh. Before refetching a full timeline, the daemon compares `updatedAt` from the list or notification.
- **Prefetch:** when the inbox loads, details for the top 15 unread threads are prefetched. Selecting a row prefetches the rows just below it, the likely next clicks. A project page fetches its first page of open PRs and issues (30 most recently updated) when opened, and refreshes it if older than 5 minutes. Nothing else is prefetched.
- **Rendering:** GitHub's own `bodyHTML` (exact rendering: mentions, issue references, task lists, suggestion blocks), sanitized with DOMPurify and styled with `github-markdown-css` (light). The raw `body` is kept for quoting and editing.
- **Images:** private repositories embed short-lived signed image URLs. The daemon rewrites image URLs to its `/img` proxy and fetches the bytes at sync time, while the URLs are still valid. Avatars go through the same proxy, so they show instantly after the first load.
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

- When the repository is a registered project: `git fetch origin +refs/pull/{n}/head:refs/launchpad/pr/{n}` (a private ref namespace, so no local branches are created), then `git diff --find-renames origin/{base}...refs/launchpad/pr/{n}`. The three-dot form diffs from the merge base, like GitHub. The output is parsed into files, hunks and lines in Rust and cached by `(base_sha, head_sha)`.
- Otherwise: `GET /repos/{o}/{r}/pulls/{n}/files` (paginated; GitHub truncates very large patches).
- The UI renders diffs with virtualization (TanStack Virtual), highlights code with Shiki in a Web Worker (per file, only when visible), offers unified and split views, and stores "viewed" checkboxes locally.

### 7.5 Rate-limit budget

5,000 REST requests and 5,000 GraphQL points per hour. Notification polling is nearly free thanks to `304` responses. Enrichment costs about 1 point per 20 threads. A PR detail query costs 1 to 5 points. The status bar shows the remaining budget.

## 8. Projects

- A project is a folder in the projects root (setting `projects_root`, default `~/projects`) plus an optional GitHub remote (`owner/repo` parsed from `origin`, SSH or HTTPS form). A project can also be registered from another location by path.
- **Discovery:** "Scan projects root" lists git repositories with a GitHub `origin` (you have 184 folders in `~/projects`; most do not need to be registered). Registered projects can be pinned in the sidebar. Each project has a one-line description (for example "server-side codebase") that is used when the project is given as context.
- **Repo → project mapping:** tells notifications, issues and PRs where to launch an agent and which local clone to use for diffs. For forks, the `upstream` remote is matched too.
- **Temporary projects:**
  - Create: `<projects_root>/tmp-<MMDD>-<slug>`, where the slug comes from the first words of the prompt (for example `tmp-1002-xet-upload-bench`). Optional `git init` (setting). Registered with `is_temp = 1` and listed in a "Temporary" group with their age.
  - Trust: headless modes show no trust prompt (`claude -p` skips the trust dialog; `codex app-server` does not prompt). For "Open in terminal", Codex gets `-c 'projects."<path>".trust_level="trusted"'`. The Claude TUI shows its trust dialog in a git repository whose root is not trusted, even under a trusted `~/projects` and even with `--dangerously-skip-permissions` ([spike 5](spikes/5-claude-trust-prompt.md)). For temporary projects, "Open in terminal" sets `CLAUDE_CODE_SANDBOXED=1` in the command's environment, which skips the dialog. Launchpad never writes `~/.claude.json`.
  - Clean up: one button stops the sessions running there, deletes the folder and unregisters it. The confirmation is part of the button (second click), not a dialog.
  - Promote: pick a name, the folder is renamed (by default the `tmp-<MMDD>-` prefix is dropped) and `is_temp` is cleared. Renaming is refused while a session runs there. Claude stores transcripts in a folder derived from the path (`~/.claude/projects/<encoded-path>/`), so promotion also moves that folder to keep past sessions resumable.

## 9. Agent sessions

### 9.1 Session model

States are the same for every harness:

```
starting ─▶ working ⇄ pending ─(Close)─▶ closed ─(Resume)─▶ working
                        ├─ waiting:    turn finished, needs your input
                        └─ background: idle while a background command runs

other states: in terminal ─(re-attach)─▶ working
              interrupted (daemon restart) ─(resume)─▶ working
              error
```

The agents list has three groups:

- **Running:** the agent is producing output.
- **Pending:** the agent is not doing anything you need to watch, and the session stays here until you close it. Two cases:
  - `waiting`: the turn finished and the agent waits for your input (including questions shown as cards).
  - `background`: the agent only waits for a background command. Claude reports these explicitly (see [9.3](#93-claude-code-adapter)). For Codex, a command that has produced no output for `pending_after_silent_command_seconds` (default 30 s) counts as background. This is a heuristic on purpose; it only changes a badge.
- **Closed:** sessions you closed. Closing stops the harness process; the conversation stays resumable in one click.

A session that becomes pending raises the sidebar badge and sends a desktop notification. Each session stores: Launchpad id (a UUID, also passed to Claude as `--session-id`), harness session or thread id, cwd, project, title, model, effort, cost, token usage, and the launch spec that created it.

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

Raw harness output lines are also stored (size-capped) so the normalizer can be improved and replayed on old sessions.

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
- **Environment:** the adapters remove `CLAUDECODE`, `CLAUDE_CODE_*`, `CLAUDE_PID` and `CLAUDE_EFFORT` from the environment before spawning a harness. A dev daemon started from an agent session would otherwise pass them on, and Claude then turns transcript saving off ("inherited CLAUDE_CODE_CHILD_SESSION marker").
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
- **Typing while the agent works:** `turn/steer {threadId, expectedTurnId, input}`. The steered text appears as a `userMessage` item when Codex reads it (after the running command), in the same turn. The Stop button sends `turn/interrupt {threadId, turnId}`, which ends the turn with `status: "interrupted"`. The running command item gets no `item/completed`, so the adapter closes every open item when a turn ends. Verified in [spike 3](spikes/3-codex-app-server.md), with the full event mapping.
- **Notifications → events:** `item/agentMessage/delta`, `item/started` and `item/completed` (user messages, agent messages with `phase` `commentary` or `final_answer`, command executions with `item/commandExecution/outputDelta`, file changes, MCP tool calls, reasoning summaries), `turn/plan/updated`, `turn/diff/updated`, `thread/tokenUsage/updated` (`total`, `last`, `modelContextWindow`), `turn/completed` (switches to `waiting`), `account/rateLimits/updated` (after every model response).
- **Background commands:** Codex has no explicit signal for them. A `commandExecution` item that is still running without new output for 30 s puts the session in `background` until the item completes or new output arrives.
- **Server requests:** `item/tool/requestUserInput` becomes an `ask_user` card. Approval requests should not happen with `approvalPolicy: "never"`; if one arrives, it is declined automatically and logged.
- **Resume after a restart:** `thread/resume {threadId}`.
- **Open in terminal:** `codex resume <thread-id>` with `-c check_for_update_on_startup=false` and the trust override for the cwd.
- **Protocol types:** `codex app-server generate-json-schema` produces the full schema (104 client methods, 83 notifications in 0.159.3). Launchpad hand-writes Rust types for the subset it uses. After a Codex update, a small dev script regenerates the schema and lists the changes in the methods Launchpad uses. It is run by hand, not as a test.
- **Option for later:** Codex now ships a shared local app-server daemon (`codex app-server daemon`, `codex agents`, `codex --remote unix://…`). Connecting to it instead of a private child process would let Codex threads survive Launchpad restarts and be attached from a terminal TUI. It is marked experimental, so it is not part of v1.

### 9.5 Versions, quotas and updates

- **Versions:** `claude --version` and `codex --version` at startup and every 30 minutes, shown in the status bar.
- **Codex quota:** `account/rateLimits/read` every 2 minutes, plus `account/rateLimits/updated` notifications.
- **Claude quota:** two sources. First, `rate_limit_event` messages from any running session. Second, the endpoint behind Claude Code's `/usage` screen (`GET https://api.anthropic.com/api/oauth/usage`, `Authorization: Bearer <token>`, no beta header), polled every 5 minutes with the OAuth access token read from `~/.claude/.credentials.json` (approved). It returns `five_hour` and `seven_day` with `utilization` (percent) and `resets_at` ([spike 2](spikes/2-claude-usage-endpoint.md)). Launchpad never refreshes that token itself: refresh tokens rotate, so a refresh by Launchpad would log Claude Code out. Before each call it checks `expiresAt` in the same file; when the token has expired, it keeps the last values and waits for Claude Code to refresh it. This endpoint is not documented, so the module is isolated and the UI shows "unknown" when it fails.
- **Codex updates:** `codex update` runs at daemon start and every 6 hours. You use the standalone install (`~/.codex/packages/standalone`, currently 0.159.3 with 0.160.0 available). Running sessions keep the binary they started with. Headless modes never show the update prompt, and "Open in terminal" passes `-c check_for_update_on_startup=false`.
- **Claude updates:** the native installer updates Claude Code by itself. Launchpad only shows the version.

### 9.6 Session view (UI)

- **Assistant text:** markdown (markdown-it, Shiki for code blocks), rendered incrementally. Finished blocks are memoized; only the last block re-renders while streaming.
- **Tool calls:** collapsed to one line (`Bash · pytest tests/test_hf_api.py · 12 s · exit 0`). Consecutive calls are grouped ("Ran 6 commands, read 4 files"). File changes show a small inline diff. Click to expand.
- **Copy:** every message keeps its raw markdown. "Copy" puts the exact markdown on the clipboard, without the line breaks and padding that terminals add. Code blocks have their own copy button. Tool output copies as raw text.
- **Input:** a message box under each session, with a Send button. `Enter` also sends and `Shift+Enter` adds a new line, like in the terminal. A Stop button in the session header interrupts the agent. TUI-only slash commands point to "Open in terminal".
- **Header:** tokens (input, cached, output), context window usage, cost estimate (Claude reports `total_cost_usd`; Codex reports tokens only), elapsed time.

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
2. Launchpad runs `git fetch`, creates branch `lp/<slug>` from `origin/<default branch>` in the worktree `<projects_root>/.worktrees/<project>/<slug>`, creates the pre-draft record, and starts the agent in the worktree with this instruction appended to the prompt:

   > You are working in a dedicated git worktree on branch `lp/<slug>`. Commit your work on this branch. Do not push and do not open a pull request on GitHub. When you are done, set the title and description with `launchpad pr update --title "…" --body-file <file>`.

3. `launchpad pr create|update|show` finds the pre-draft PR from the current worktree and calls the daemon. `create` also works from any branch in any session, so an agent (or you) can turn existing local work into a pre-draft PR.
4. A shared skill at `~/.agents/skills/launchpad/SKILL.md` (Claude reads it through your existing `~/.claude/skills` symlinks; Codex reads `~/.agents/skills`) explains when and how to use `launchpad pr`, the expected PR description format, and later `launchpad suggest`. Launchpad links it from the repository at startup, so it always matches the code.
5. Review in the PR view: description, commits, and files changed computed locally (`git diff origin/main...lp/<slug>`). A banner shows when the base branch moved ("main is 3 commits ahead").
6. **Feedback loop:** write notes (general, or attached to a file). "Send feedback to agent" resumes the same session with the notes as a message. The agent amends the branch and you review again.
7. **Publish:** choose the remote branch name (default `<github-user>/<slug>`), Launchpad runs `git push -u origin lp/<slug>:<name>` then `POST /repos/{o}/{r}/pulls` with `draft: true` or `false`, the title and the body. The record now links to `#N`. The worktree is kept or removed (setting).
8. **Discard:** removes the worktree, deletes the branch and archives the record.

## 11. Agent-written suggestions (later phase)

`launchpad suggest comment|review --repo <owner/repo> --number <n> --body-file <file> [--event approve|request-changes]` creates `draft_comment` rows. They appear in the issue or PR timeline as teal "Suggested by Claude" cards with Edit, Post and Discard buttons. Posting uses the normal mutation endpoints. Suggestions attached to specific lines wait until inline comments are supported.

## 12. Frontend

- **Libraries:** React 19 + TypeScript + Vite, with pnpm as the package manager; TanStack Router (typed routes); TanStack Query (cache, background revalidation, optimistic mutations; WebSocket events call `invalidateQueries` or `setQueryData`); Zustand for UI state (layouts, composer draft, selection); Tailwind CSS v4 with Radix primitives (shadcn-style components copied into the repo); TanStack Virtual (lists, diffs, long sessions); Shiki in a Web Worker; markdown-it; DOMPurify; github-markdown-css.
- **Theme:** light only. One set of CSS variables, no dark tokens.
- **Layout:** left sidebar (Inbox, Agents, Projects, pinned projects, temporary projects, Settings; collapses to icons), main area, composer drawer on the right, status bar at the bottom.
- **Harness marks:** the official Claude and Codex logos (SVG files in `web/src/assets/`) everywhere a harness appears: radial launch buttons (logo only, no label), session lists, pane headers, status bar.
- **Speed tactics:** render from cache; prefetch on selection; keep list views mounted so scroll position and selection survive navigation; optimistic actions; virtualized lists; every frequent action one click away.
- **Click only:** no keyboard shortcuts (see principle 5). Frequent actions are visible buttons where they are used: hover actions on notification rows, a Launch button on every notification, issue, PR and project, layout buttons in the agents view, Refresh and Open on GitHub on every cached object. Icon-only buttons have a hover title. Context is added to the composer with buttons, or with `@` and `#` suggestions in the prompt (see [9.7](#97-prompt-composition)).
- **WebKitGTK:** the Tauri webview on Linux is WebKit, not Chromium. Phase 0 measures performance with a 5,000-line diff and a 2,000-event session. If the window renders blank with some GPU drivers, `WEBKIT_DISABLE_DMABUF_RENDERER=1` is the usual workaround. Because the UI is a plain web app, it can be developed in Chrome against the dev daemon, and switching the shell to Chromium (`--app=` mode) stays possible if WebKit is too slow.

## 13. Desktop integration

- Desktop notifications are sent by the daemon, so they work when the window is closed. Examples: "Claude is waiting in huggingface_hub", "Review requested: huggingface_hub#3420". Clicking one focuses the window on the item (`launchpad-desktop --open <route>`).
- External apps are chosen in Settings → General (stored in `config.toml`, see [6.5](#65-configuration-file)): the terminal used by "Open in terminal" (Terminator by default; Ghostty, GNOME Terminal or a custom command), the editor (`code` by default; Cursor is also installed) and the browser (`xdg-open`).

## 14. Security

- The daemon can start agents that run any command. Its API must be treated like remote code execution.
- It binds `127.0.0.1` only. At first start it writes a random 256-bit token to `~/.local/share/launchpad/token` (mode 0600). The desktop shell opens `/auth?t=<token>`, which sets an `HttpOnly`, `SameSite=Strict` cookie. The CLI sends `Authorization: Bearer <token>`.
- Requests whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>` are rejected (DNS rebinding). WebSocket upgrades and mutations with a foreign `Origin` are rejected (CSRF). No CORS headers.
- The GitHub token stays in daemon memory. Logs redact tokens.
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

## 17. Decisions log

Confirmed on 2026-10-02:

1. **Headless sessions.** Launchpad renders sessions itself. Losing TUI-only features is fine; "Open in terminal" is the escape hatch. No embedded terminal for now.
2. **Background daemon.** The systemd user service with a thin Tauri window is the target architecture.
3. **Pending agents.** A session is pending when its turn finished and it waits for input, or when it only waits for a background command. It stays pending until you close it. Kept simple (see [9.1](#91-session-model)).
4. **Pre-draft PRs.** Launchpad creates the branch and worktree under `<projects_root>/.worktrees/` and links a `launchpad` skill into `~/.agents/skills`.
5. **Temporary folders.** `<projects_root>/tmp-<MMDD>-<slug>`. The projects root is configurable, default `~/projects`.
6. **Claude quota.** Launchpad may read the OAuth access token in `~/.claude/.credentials.json` to call the usage endpoint, never refreshing it.
7. **Harness icons.** Launch buttons and harness marks use the Claude and Codex logos, without text labels.
8. **Click-only UI.** No keyboard shortcuts, at least at first. Two typing behaviors only: `@` and `#` suggestions in the composer prompt (added on 2026-10-02, see [9.7](#97-prompt-composition)), and `Enter` to send in a session's message box. Shortcuts may be added later if they are really missed.
9. **Terminal is a setting.** "Open in terminal" uses the terminal chosen in Settings: Terminator by default, Ghostty, GNOME Terminal, or a custom command.
10. **Minimal tests.** Move fast and break things. Test only small, specific parts that clearly benefit from it. No mocks, no tests that need a server, and the whole run takes a few seconds at most. Details in [5.2](#52-testing).

Phase 0 spikes, on 2026-10-02 (notes in [docs/spikes/](spikes/)):

11. **Claude adapter flags.** Sessions add `--replay-user-messages` and `--permission-prompt-tool stdio` (needed for AskUserQuestion cards). A resume uses `--resume <id>` instead of `--session-id`. Harness processes get an environment without the `CLAUDECODE` and `CLAUDE_CODE_*` variables. A message typed during a turn joins the running turn. The Agent SDK sidecar fallback is not needed.
12. **Claude quota endpoint.** `/api/oauth/usage` works with the read-only token. Launchpad checks `expiresAt` and never calls with an expired token.
13. **Codex adapter.** `codex app-server` works as planned (steer, interrupt, resume after restart). The adapter closes open items when a turn ends.
14. **Claude TUI trust.** "Open in terminal" sets `CLAUDE_CODE_SANDBOXED=1` for temporary projects, so the trust dialog does not appear. Launchpad does not write `~/.claude.json`.

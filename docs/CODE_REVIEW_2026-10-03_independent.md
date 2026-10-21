# Launchpad: independent full review (2026-10-03)

Scope: everything built at `847320f` (Phases 0 to 6, plus the reverted Phase 7), compared with `initial.md`, `README.md`, `AGENTS.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, the spike notes and the skill. This review was made without reading the earlier `docs/CODE_REVIEW_2026-10-03.md`; section 14 compares the two afterwards and merges the findings it had that this review missed.

This file is not committed. Note that any untracked file in the checkout blocks the production updater (see OPS-1).

## Contents

1. [Method](#1-method)
2. [Summary](#2-summary)
3. [Quality gates and runtime probes](#3-quality-gates-and-runtime-probes)
4. [Plan versus build](#4-plan-versus-build)
5. [Documentation accuracy](#5-documentation-accuracy)
6. [Security](#6-security)
7. [Bugs and correctness, by area](#7-bugs-and-correctness-by-area)
8. [AGENTS.md rule compliance](#8-agentsmd-rule-compliance)
9. [Performance, storage and scalability](#9-performance-storage-and-scalability)
10. [Code quality and maintainability](#10-code-quality-and-maintainability)
11. [Repository and process hygiene](#11-repository-and-process-hygiene)
12. [What went wrong](#12-what-went-wrong)
13. [Recommended fix order](#13-recommended-fix-order)
14. [Comparison with the earlier review](#14-comparison-with-the-earlier-review)

## 1. Method

- Read the five documents in full, then every source file: `crates/core` (35 files), `crates/server` (11), `crates/cli`, `desktop/src-tauri`, `packaging/`, `.github/workflows/ci.yml`, `skills/launchpad/SKILL.md` and all of `web/src` (6 parallel reviewers, each reading its area line by line, then cross-checked).
- Ran the CI gates locally: pinned Clippy, `cargo test`, UI type checks, `cargo audit`, `pnpm audit`.
- Probed the running dev daemon (port 7318) with read-only and harmless requests: authentication, `Host` and `Origin` checks, the code viewer path checks, security headers, the `/auth` redirect.
- Read the production `state.db` in read-only mode for size statistics, and checked the state of the `launchpad-update` systemd unit.
- Confirmed git behaviors (index writes, push and upstream failures, diff headers) with throwaway repositories in `/tmp`. Checked the Codex 0.160.0 app-server schema and the installed Claude Code 2.1.288 binary for protocol details.
- Nothing was published to GitHub and no daemon was restarted. The only repository files written are this one and an addendum appended to the end of the earlier review (its existing content is unchanged).

Severity scale for a single-user local app:

| Severity | Meaning |
|---|---|
| Critical | Likely data loss or compromise in normal use. None found. |
| High | Breaks a core workflow or blocks the owner in realistic conditions, or puts production data at risk. |
| Medium | Wrong behavior in plausible situations, a hardening gap with real exposure, or a noticeable performance problem. |
| Low | Edge case, small bug, defense in depth. |
| Info | Observation, no action strictly needed. |

Findings marked **(verified)** were reproduced or re-read directly during this review; the others come from line-by-line reading by a reviewer, with the exact location given.

## 2. Summary

The project is in good shape for its stage. Phases 0 to 6 are built and match the roadmap closely; the Phase 7 revert is complete and clean. The code is consistent, the security model (localhost, token, cookie, `Host`/`Origin` checks, code viewer root and path checks, image proxy allowlist, DOMPurify) holds against every probe, and no path was found that lets GitHub content or agent output run script in the authenticated origin. All gates pass: strict Clippy, 11 Rust tests, UI type checks, and the dependency audits (two transitive Tauri/GTK advisories only).

The weak points are recovery paths and long-running operation, not the happy path:

- several states can become permanently stuck (open questions, pre-draft publication, updater status, inbox sync, install reservation);
- processes started by the daemon die with it, and a second daemon can modify live production data;
- global locks are held across network and git operations;
- the inbox and item caches do much more GitHub and database work than needed;
- storage grows without limit;
- the architecture document has drifted from the code in many places (API list, schema, modules, frontend libraries, presets, test counts).

### High-severity findings

| ID | Area | Finding |
|---|---|---|
| OPS-1 | Updater | Any untracked file blocks the production updater forever; it is blocked right now by the earlier review file **(verified)**. The update status can also stay `running` forever. |
| SES-1 | Sessions | `open_question` is never cleared on resume, cancel or process exit: after a daemon restart during a question, every typed message fails **(verified)**. |
| PD-1 | Pre-drafts | Publish pushes, then fails at `--set-upstream-to` in single-branch or shallow clones; the record is stuck for good **(verified)**. |
| PD-2 | Pre-drafts | Once `pushed_head` is saved, a push that always fails (`@{-1}`, submodule recursion, no push rights) has no way out: retry fails, Discard is refused, turns are frozen. |
| SRV-1 | Daemon | `launchpad daemon` runs migrations, wipes `cache.db`, marks live sessions `interrupted` and starts the scheduler before it binds the port: a second prod daemon damages the live one **(verified)**. |
| OPS-2 | Packaging | Terminals, browsers, editors and windows started by the daemon live in its systemd cgroup (`KillMode=control-group`) and are killed by every restart or update; the updater ignores `in_terminal` sessions **(verified)**. |
| GH-1 | Inbox | One failing thread subscription request aborts the whole notification sync before anything is saved; a permanently failing thread stops the inbox from updating **(verified)**. |

In total the review lists 175 findings (7 high, 59 medium, 109 low or info) and 36 documentation corrections. 25 of the findings and 5 of the documentation corrections come from the earlier review and were merged after it was read (section 14).

## 3. Quality gates and runtime probes

| Check | Result |
|---|---|
| `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo +1.93.1 test --locked` | Pass, 11 tests (code 3, compose 3, github 1, git 1, items 1, schedules 1, sessions 1) |
| `(cd web && pnpm typecheck)` | Pass |
| `cargo audit` | 0 vulnerabilities; 2 warnings in transitive Tauri/GTK crates: `proc-macro-error` unmaintained (RUSTSEC-2024-0370), `glib 0.18.5` unsound `VariantStrIter` (RUSTSEC-2024-0429) |
| `pnpm audit` | Fails against the configured private registry (HTTP 405); against `registry.npmjs.org`: no known vulnerabilities |
| CI on GitHub | Never ran: the repository has no remote |

Runtime probes against the dev daemon (all **verified**):

| Probe | Result |
|---|---|
| No token | 401 |
| Wrong `Host` with a valid token | 403 |
| POST with `Origin: http://evil.com`, `null`, or `http://localhost:3000` | 403 |
| `/api/code/file` with `../../../etc/passwd`, `/etc/passwd`, `./README.md`, `web/../README.md` | Rejected |
| `/api/code/files` with `root=/etc` or a project subfolder | Rejected |
| `rev=--output=/tmp/x` | Rejected, no file written |
| `/auth?t=<token>&next=/%5Cexample.com` | `Location: /\example.com`, an open redirect (needs the token) |
| Headers of `/` | No `Content-Security-Policy`, no `X-Frame-Options`/`frame-ancestors` |
| File modes | `token` 0600; data folder 0775; `state.db` 0644; `config.toml` 0664 (parents `~/.local` and `~/.config` are 0700, home 0750) |

## 4. Plan versus build

### 4.1 Phases

| Phase | Built | Gaps against the plan and the progress notes |
|---|---|---|
| 0 Foundations | Yes | CI never ran (no remote). The desktop shell is 242 lines, not "roughly 100". `install.sh` refuses (instead of warning) when agents run. The spike-planned schema diff script for Codex does not exist. |
| 1 Agents | Yes | Two default presets, not three (draft PR became the Pre-draft target, by design, but ROADMAP and ARCHITECTURE still say three). Codex sessions have no cost (no data from Codex). The header shows context tokens and cost, not input/cached/output. Images can be attached to messages but not to the initial launch. No "TUI-only slash command → Open in terminal" hint. Session code blocks have no Shiki colors. |
| 2 Inbox | Yes | The inbox Refresh button is almost always a no-op (GH-4). Restore from a known snapshot is API only (no UI). Review and check summaries show in the preview, not in rows. "Review requested" desktop notifications (ARCHITECTURE 13) do not exist. Enrichment does not fetch labels or comment counts. |
| 2b Code viewer | Yes | No `D` letter for deleted files. Linked paths in tool calls only for file tools. At most 150 refs per kind (undocumented). |
| 3 Issues and PRs | Yes | Files changed errors instead of falling back to the API when no remote matches (ITM-3). Project lists use `gh_repo` only, never `upstream`. Labels are a free-text list saved with PUT (ITM-6). Checks capped at 100. "A Launch button on every issue and PR" exists only through multi-select. |
| 4 Pre-drafts | Yes | "Settings can remove a clean worktree afterward" is automatic removal at publish time. Publication can get stuck (PD-1, PD-2). |
| 5 Workspace | Yes | No functional gap found. |
| 6 Suggestions | Yes | Line-specific suggestions deferred (documented). Rejected posts are locked as uncertain (SUG-1). |
| 7 | Reverted | Revert verified complete: trees of `crates`, `web`, `desktop`, `packaging`, `skills`, `.github`, `Cargo.toml`, `Cargo.lock` are identical between `9fdcb0a` and `847320f`. Two small improvements made by `7b670af` were lost with it (Markdown link errors shown as toasts; titles saved through the rename path). `web/node_modules/@xterm/*` is still installed locally (untracked). |

### 4.2 The owner's original wishes (`initial.md`)

| Wish | Status | Note |
|---|---|---|
| Tauri dock app, light theme, Linux only | Done | |
| Future remote hosting | Deferred | Phase 7 candidate |
| Faster than github.com, from a local cache | Done | Views remount on navigation (no kept-mounted lists) |
| Notifications: list, reason, participants, repo filter, read/unread, Done/Unsubscribe/Unread, age | Done | Unread is local only (no GitHub API); reason is text, not a badge |
| Register projects | Done | |
| One-click launch in the right folder | Done (by design: composer and prompt) | |
| Codex: always "no" to update prompts, background update | Done | |
| Temporary folder, always trust, 1-click cleanup and promote | Done | Cleanup needs a second click (by design) |
| Presets: editable, concatenated, exactly the three listed | Changed | "draft PR" replaced by the Pre-draft target; the composer hides any preset with id `draft-pr`, so the owner's existing one in `config.toml` silently disappears from the composer but stays in Settings (UI-21) |
| Context projects with a branch | Done | Local branches only |
| Issues, PRs and comment links as context | Partial | `#pullrequestreview-` anchors are dropped |
| Model and effort, default = the agent's last selection | Partial | Defaults come from Launchpad's latest launch (decision 23), which overrides the agent's own default once a model was picked |
| Launch from anywhere (notification, multi-selected issues/PRs, project) | Done | |
| Radial launcher | Replaced | Built, then replaced by a harness toggle (decisions 7 and 23) |
| Agents panel: running, past, pending | Done | Done/Running/Scheduled/Archived |
| Streamed I/O, typing into the session | Done | |
| Cost and token usage | Partial | Codex has no cost; token totals are not shown |
| Copyable messages with exact formatting | Mostly | User messages and reasoning have no Copy button |
| Side by side and 4-pane grid | Done | |
| Versions, quotas and reset times always visible | Done | |
| Issue/PR view with comments, checks, diff, review, merge, close, won't fix, draft/ready | Done | |
| Regular pull into a local DB, Refresh button | Partial | Only notifications poll; inbox Refresh throttled (GH-4) |
| Local git for diffs | Done | No API fallback when no remote matches (ITM-3) |
| Agent-written comments and reviews as editable suggestions | Done | |
| Pre-draft PRs | Done | |
| Project lists with pre-drafts in another color, cache on demand | Done | Upstream repository not listed |
| Refresh and Open in browser everywhere | Partial | Inbox rows need two clicks to open GitHub |

## 5. Documentation accuracy

The README, AGENTS.md commands and CI description are accurate, with the small exceptions listed below. ARCHITECTURE.md has drifted the most: several sections still describe the first design draft.

| ID | Location | Problem | Suggested correction |
|---|---|---|---|
| DOC-1 | ROADMAP overview table | Phases 1–6 say "In progress"; README, AGENTS and ARCHITECTURE say "built, awaiting daily-use checks". Phase 7 says "Next" while it is deferred; its heading says "hardening and options". | Use "Built, awaiting acceptance" and "Deferred"; rename the Phase 7 heading. |
| DOC-2 | ARCHITECTURE 4.3, 9.8 | "A later phase removes this limitation" (restart survival); 9.8 does not mention the reverted attempt. | Point to the Phase 7 candidate list and decision 44. |
| DOC-3 | ARCHITECTURE 1 | "and, later, agent-written comments" | Phase 6 is built; drop "later". |
| DOC-4 | ARCHITECTURE 4.2 | Shell of "roughly 100 lines" (242). Falls back to a direct spawn "if the daemon is down" (also when health is late, see SRV-1). Navigation "cancelled" outside the origin (bypassable, SEC-4). | Update. |
| DOC-5 | ARCHITECTURE 5.1, 6.1, 12 | Radix and shadcn components (none used; `Popover.tsx` is custom). `components/ (Radix based)`, `lib/` holds markdown and diff (they are in `components/Markdown.tsx` and `FilesChanged.tsx`/`items.rs`), `features/predraft` (it is `predrafts`), no `stores/`, `suggestions/` or `dev/`. The sidebar "collapses to icons" (fixed 200 px). "Keep list views mounted" (single `<Outlet/>`, scroll restored from stores). tower-http `trace` feature enabled but unused. | Regenerate from the tree. |
| DOC-6 | ARCHITECTURE 6.2 | Modules `sync`, `quota`, `predraft`, and `git` doing worktrees, PR fetch and diff parsing. Real: `inbox`, `status`, `predrafts`; worktrees in `predrafts`/`code`, PR fetch and diff split in `items`, hunks parsed in the UI. 18 real modules are not listed. | Replace the table. |
| DOC-7 | ARCHITECTURE 6.3 | The API list is the first draft: `/api/notifications/*` is `/api/inbox/*` (`action` in the body); `/api/gh/.../items|pulls` is `/api/items/{o}/{r}/{n}[/{action}]` and `/api/repo-items/{o}/{r}`; diff is a POST action; `/api/refresh` and `POST /api/projects/temp` do not exist; `/stop` is `/close`; `/img?u=` is `?url=`; about 30 routes are undocumented (config, log, drafts, defaults, rename, git, git-diff, task-output, history, search, sync, preview, reply, resolve, suggest, terminals, update, install-lock, open, predraft resolve/refresh, suggestions, `/auth`). WebSocket topics differ (no `gh` topic; real: `session/{id}`, sessions, projects, config, inbox, status, predrafts, suggestions, schedules), and the client does not send its last `seq` over the socket: it calls `GET /events?after=`. | Regenerate from the routers. |
| DOC-8 | ARCHITECTURE 6.4 | Cache tables `gh_notification`, `gh_item`, `gh_timeline`, `gh_diff`, `gh_etag`, `image_cache` are created (`db.rs:150-159`) but never used; the real cache is `gh_response` plus `inbox_thread`, `inbox_subscription`, `inbox_preview`; images are files. State: `notification_override` is `inbox_override` plus `inbox_active`; `predraft` and `draft_comment` are `(id, json)`; `scheduled_launch` and several session columns are missing. Three tables bypass the `user_version` list (`db.rs:136-140`). The event kind table lists `usage`, `rate_limit` and `ask_user`, while stored kinds include `message_delivered`, `turn_end`, `notice`, `background_task`. | Update; drop the dead tables. |
| DOC-9 | ARCHITECTURE 6.5 | Example omits `remove_published_worktree`; the browser is not in Settings although 13 says so. | Add the key; correct section 13. |
| DOC-10 | ARCHITECTURE 7.1–7.4 | Token read "when the daemon starts" (read lazily). Pages 2+ "complete paginated pass" (conditional). Enrichment fetches labels and comment count (it does not). `updatedAt` compared "from the list or notification" (separate GraphQL request). Prefetch "when the inbox loads" (after every poll, GH-3). Diff "parsed into files, hunks and lines in Rust" (Rust splits files only). The label mutation is missing from the mutation table. | Update. |
| DOC-11 | ARCHITECTURE 8 | Discovery lists repositories "with a GitHub origin" (it lists every git repo). "Optional `git init` (setting)" does not exist. "Renaming is refused while a session runs there" ignores terminal sessions (PRJ-2). | Update. |
| DOC-12 | ARCHITECTURE 8.1, 14 | Files read with `git show` (it is `cat-file`). "Every file is untracked" without commits (staged files get no letter). Paths "without `.`" (`a/./b` accepted; harmless). | Update. |
| DOC-13 | ARCHITECTURE 9.1 | Orphan sentence "States are the same for every harness:"; "three groups" (four, with Scheduled). | Fix. |
| DOC-14 | ARCHITECTURE 9.1.1, AGENTS | The git panel "only runs read-only commands": `git diff` rewrites the index (CODE-2). | Reword or fix the code. |
| DOC-15 | ARCHITECTURE 9.2 | A `Harness` trait that does not exist; `LiveSession` signatures differ. | Describe the real module functions. |
| DOC-16 | ARCHITECTURE 9.3 | `LAUNCHPAD_ENV`/`LAUNCHPAD_SESSION_ID` "for harness processes": the shared Codex app-server and terminal handoffs get neither (prompts carry the values). "Close stdin (the process exits), then run the terminal": no wait (SES-3). | Update. |
| DOC-17 | ARCHITECTURE 9.4, 16 | `turn/diff/updated` handled (it is not); a schema diff dev script (missing); terminal handoff also passes `--dangerously-bypass-approvals-and-sandbox` (undocumented); "running sessions keep their binary" (the shared app-server keeps it for new sessions too). | Update or build the script. |
| DOC-18 | ARCHITECTURE 9.5 | `codex update` "at daemon start" (20 s after). | Minor. |
| DOC-19 | ARCHITECTURE 9.6 | Shiki for session code blocks (none); finished blocks memoized (the whole message is re-parsed per delta); header with input/cached/output tokens (context only). | Update or implement. |
| DOC-20 | ARCHITECTURE 9.7, ROADMAP 1, sketches | "The three initial ones" including draft-pr (two defaults). Old `LaunchSpec` shape (real targets: `Project`, `Temp`, `Predraft`, `ExistingPredraft`). The example uses `~/` paths while the text says absolute. | Update. |
| DOC-21 | ARCHITECTURE 9.7.1 | Dependencies "only on existing sessions or queued records": scheduled records in any status (Failed, Starting) are accepted. | Code or doc. |
| DOC-22 | ARCHITECTURE 10.3 | Quoted pre-draft prompt differs from `predrafts.rs:264`. Step 7 "sets the local branch upstream" fails in single-branch clones (PD-1). Step 8 "removes a clean worktree" deletes ignored files (PD-5). | Update with the fixes. |
| DOC-23 | ARCHITECTURE 13 | "Review requested" desktop notifications; browser chosen in Settings. Neither exists. | Mark as not built. |
| DOC-24 | ARCHITECTURE 14 | "Logs redact tokens": no redaction code exists (it holds only because nothing logs tokens, and signed image URLs with JWTs are logged, SEC-12). Token "written at first start by the daemon": the CLI can create it too (SEC-6). | Update. |
| DOC-25 | ARCHITECTURE 17 | Decision 29 under "Continuous integration", 33–34 under "Code viewer implementation", 42 under "Phase 5". | Regroup. |
| DOC-26 | ARCHITECTURE 24, decision 8, AGENTS, ROADMAP "Not planned", sketches | "Text fields only type text": four fields use Enter/Escape and Popover has a global Escape (UI-8). | Remove the handlers or document them. |
| DOC-27 | ROADMAP progress notes | "five", "nine", "ten" tests (11 now); README's test list omits the diff parser, schedule and UTF-8 tests. | Write "the existing Rust tests". |
| DOC-28 | ROADMAP Phase 0 | "warn if agents are running": `install.sh` refuses unless confirmed or `--force`. | Update. |
| DOC-29 | README, AGENTS, ARCHITECTURE 4.2 | "Waits for a clean committed main": untracked files also block (OPS-1). README CI section reads as if CI runs (no remote). `launchpad-desktop --quit` "stops it" (opens a window when none runs, OPS-9). | Update. |
| DOC-30 | AGENTS.md | Blank lines split the Product rules list; `packaging/` description omits `update.sh`, `install-icons.sh`, `generate-icons.py`. | Fix. |
| DOC-31 | Code comments | `codex.rs:134` "background-command heuristic", `ProjectPage.tsx:2` "Issues and PRs arrive in Phase 3", `InboxView.tsx:151` "without making GitHub requests", `terminal.rs:97` "The terminal must outlive the daemon", `sessionStore.ts` "sessions shown on screen", `gen/EventBody.ts` new-file diff comment. | Update. |
| DOC-32 | ARCHITECTURE 9.7 ("shown in the composer before launch") | The preview is not the exact prompt: pre-draft and suggestion instructions are appended at launch, and the temporary or worktree cwd is resolved afterward (earlier review). | Say what the preview omits, or preview the final prompt. |
| DOC-33 | ARCHITECTURE 2, 8.1 ("one code renderer") | `CodeLines` and `FilesChanged` have separate line renderers and language mapping; only the worker is shared (earlier review). | Correct the claim, or merge them if useful. |
| DOC-34 | AGENTS.md (code viewer) | "Every `/api/code/*` request goes through `code::open`": `sources` does not (metadata only, CODE-10). | Note the exception. |
| DOC-35 | AGENTS.md, ARCHITECTURE 5 | "The find bar also has a Find button": the bar has previous/next controls, and the Find button is in the toolbar (earlier review, not re-verified). | Align the wording. |
| DOC-36 | ARCHITECTURE 9.1, 16 | Raw lines "so the normalizer can be improved and replayed on old sessions": no replay or renormalization tool exists, and the 20,000-line cap makes long sessions incomplete. | Describe the real diagnostic value. |

## 6. Security

The threat model (ARCHITECTURE 14) treats the API as remote code execution. The core controls hold. The findings are hardening gaps; none is directly exploitable from the internet today.

### Medium

- **SEC-1. No Content-Security-Policy or anti-framing header on the UI.** `crates/server/src/ui.rs:18-29` (verified by probe); `tauri.conf.json` has `"csp": null`. The UI renders GitHub `bodyHTML` (DOMPurify) and Markdown; a future sanitizer mistake would be RCE because same-origin script can `POST /api/sessions`. Pages on any `localhost:*` port are same-site, so they receive the `SameSite=Strict` cookie and could frame the UI for clickjacking. `index.html` has no inline script, so a strict policy is easy: `default-src 'self'; script-src 'self'; worker-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`, plus `nosniff`.
- **SEC-2. The dev profile trusts every page served on port 5173.** `crates/server/src/auth.rs:29-33`. Port 5173 is Vite's default, so any other project's dev server is an allowed origin for the dev daemon: it gets the dev cookie (same-site), can open `/ws` and read all events, and can send body-less POSTs (resume, interrupt, sync, suggestion post to real GitHub). Fix: have the Vite proxy rewrite `Origin` to the daemon and drop the 5173 origins.
### Low

- **SEC-3. Data folder and databases rely on parent folder permissions.** Verified: data folder 0775, `state.db` 0644, `config.toml` 0664, created with default modes (`paths.rs` `ensure_dirs`). `state.db` holds full agent conversations (tool output often contains secrets), `cache.db` holds private repository content. On this machine `~/.local` and `~/.config` are 0700 and there is a single local user, so there is no actual exposure today; the protection is accidental. Fix: create the data and config folders with 0700.
- **SEC-4. Desktop navigation check uses a string prefix.** `desktop/src-tauri/src/main.rs:211-218`: `url.as_str().starts_with("http://127.0.0.1:7317")` accepts `http://127.0.0.1:7317@evil.com/` (userinfo; verified with the WHATWG parser), and `http://tauri.localhost/...` is also treated as internal. A link in GitHub content could load a foreign page inside the trusted window (no IPC, but phishing). Fix: compare scheme, host and port, and reject userinfo.
- **SEC-5. Open redirect and panic in `/auth`.** `auth.rs:100-102`: `next=/\evil.com` passes (verified), and a decoded CR/LF makes `Redirect::to` panic. Both need a valid token. Fix: reject `\` and control characters.
- **SEC-6. Token file creation.** `crates/core/src/paths.rs:93-115`: `token.tmp` is written with umask permissions, then chmodded; two first-time creators race; the CLI also creates a token when the file is missing, so `launchpad url` can print an address that the running daemon rejects. Fix: `OpenOptions::mode(0o600).create_new(true)`, and make the CLI read only.
- **SEC-7. Token in URLs.** `launchpad url` and `/auth?t=` put the 256-bit token in browser history (and Chrome sync). A one-time nonce exchange would avoid it.
- **SEC-8. Untrusted GitHub titles go into prompts for agents in yolo mode.** `crates/core/src/compose.rs:55-63`: only `"` is replaced; branch names and paths in the suggested `git -C … show` commands are not shell-quoted (`compose.rs:30,73`). Prompt injection is inherent to the product, but control characters and length should be stripped and titles labeled as data.
- **SEC-9. Remote images in Markdown load directly.** `web/src/components/Markdown.tsx:34-40`, inbox previews (`InboxView.tsx:117-119`): images in raw GitHub Markdown and agent output bypass GitHub's camo proxy (IP and read-time tracking; an exfiltration channel for a prompt-injected agent's output, though agents already have network access). Route them through `/img` or render them as links.
- **SEC-10. `/api/open-editor` accepts any string.** `crates/server/src/api.rs:360-372`: no absolute-path check and no `--` before the path, so a path starting with `-` becomes an editor option (`--install-extension=…`). Not reachable from the current UI.
- **SEC-11. reqwest honors `HTTP_PROXY` for 127.0.0.1 in the CLI (plausible).** `crates/cli/src/main.rs`: the bearer token could go to a configured proxy. Use `.no_proxy()`.
- **SEC-12. Signed image URLs are logged.** `crates/server/src/error.rs:28-31` logs every `/img` error with the full URL, including private-image JWTs.
- **SEC-13. `.git/config` is readable through the code viewer.** It is inside the root and belongs to the owner, but it may contain credentials in HTTPS remote URLs. Consider hiding `.git/`.

### Info

- Agents run as the same user: they can read the daemon token and use `gh` directly. "Nothing reaches GitHub without a click" is a UI convention, not an enforced boundary. The same applies to `update-install-lock` and starting a production update through the API.
- Unauthenticated `/api/health` exposes the profile and PID.
- The status poller reads `~/.claude/.credentials.json` at a fixed path and ignores `CLAUDE_CONFIG_DIR`.
- `glib 0.18.5` (RUSTSEC-2024-0429, unsound `VariantStrIter`) comes through Tauri/GTK/WebKitGTK; reachability from Launchpad is not established. The earlier review rates it Medium (R21); here it is tracked as an upstream dependency to bump when Tauri moves to glib 0.20.
- An existing token file with wrong permissions is accepted as is; its mode is never corrected (earlier review, R17).

### Verified correct

Token: 256-bit CSPRNG, 0600 after rename, constant-time comparison, separate prod/dev cookie names, `HttpOnly; SameSite=Strict; Path=/`. Only `/api/health` and `/auth` bypass authentication; the guard wraps the UI fallback too. Exact `Host` allowlist (DNS rebinding). `Origin` checked on mutations and WebSocket upgrades (`null` and other ports rejected); no CORS headers; no state-changing GET route. Body limit raised only for `/input`; image data URLs validated. `/api/open` accepts http and https only. Editor and terminal launches use argv lists without a shell. `/img`: HTTPS on 443, no userinfo, fixed GitHub host allowlist, each redirect revalidated (at most 3), raster types only (no SVG), 5 MB limit while streaming, sandbox CSP and `nosniff`, no GitHub token. `/api/code/raw` has the same headers. GitHub token only sent to `api.github.com`; never logged. GraphQL variables are JSON-escaped (no injection). DOMPurify forbids `style`, `form`, `iframe`; markdown-it uses `html: false` and the default `validateLink`; label colors go through React style objects. The Claude OAuth token is only read, `expiresAt` is checked, it is never refreshed. Code viewer: canonical root check including linked worktree back-pointers, path checks, symlinks cannot escape, `--end-of-options` revision resolution, literal pathspecs, `-e` patterns, `--no-ext-diff`; fork PR heads are never checked out, so PR-supplied filters and hooks never run. History import validates ids as UUIDs. Temporary-project cleanup only deletes folders directly in the canonical projects root and does not follow symlinks.

## 7. Bugs and correctness, by area

### 7.1 Agent sessions and harness adapters

**High**

- **SES-1. Open questions are never cleared, so a session rejects every message or stays Running.** (verified) `crates/core/src/sessions.rs:126` keeps `open_question` on load; `send` (558-578), `resume` (619-639) and `process_exited` (692-715) never reset it; only `answer`, `close` and `open_terminal` do. The UI sends typed text to `/answer` whenever `open_question && live` (`SessionPane.tsx:73-76,315`). Claude's `control_cancel_request` and Codex's `serverRequest/resolved` are ignored (`claude.rs:359`, `codex.rs:382`), and both adapters skip status changes while a question is pending (`claude.rs:306-308,370`, `codex.rs:369-371`). Scenario: an agent asks a question, the updater restarts the daemon (Waiting counts as idle), the owner replies: the first message resumes the session, every later message goes to `/answer` with a dead request id and fails. A cancelled question instead leaves the session Working forever, blocking the updater, publication and dependents. Fix: handle the two cancel messages, clear the adapter maps and `open_question` on turn end, load, exit and reattach, and fall back to `send` for an unknown request id.

**Medium**

- **SES-2. Archive does not stop the old Claude process from changing the status.** `claude.rs:109-124,181-194`, `sessions.rs:604-616,812-830`: the reader keeps handling `result` (Waiting) and `init` (Working) after `stop()`, and `Sink::set_status` does not check the instance. An archived session can jump back to Done, or stay Working forever when SIGTERM lands after `init`. Fix: ignore events from a detached instance.
- **SES-3. One session can have two live processes.** `sessions.rs:595-601` (`detach` does not wait for exit), `619-663`; `claude.rs:181-194`. Archive then Resume within 5 s runs two `claude` processes on the same transcript; Archive of an `in_terminal` session then Resume starts a headless process while the TUI runs (`tui_running` only checked for imports); a double "Open in terminal" opens two TUIs. Breaks "a session never has two live processes".
- **SES-4. A background task end shows a false Done.** `claude.rs:406-414`: an empty `background_tasks_changed` sets Waiting about 90 ms before Claude's self-started turn (spike timeline). Effects: a spurious "waiting" notification per task, and a dependent scheduled launch can start too early. Breaks "Never infer Done from a command being silent". Fix: stay Background until the next `result`, with a fallback timer.
- **SES-5. A failed send leaves the session Running forever.** `sessions.rs:574-577`: the status is set to Working before `live.send`, and is not reverted on error (Codex `turn/start` timeout or app-server restart). Blocks the updater, publication and dependents.
- **SES-6. The Codex router can drop protocol messages.** `codex_rpc.rs:46,122-126` uses a lossy `broadcast(8192)`; the single router awaits two database writes per message (`codex.rs:135-161`); lag is only logged (`codex.rs:157`). During a database stall (session search, large import, `task_output`), a chatty command can overflow the channel and lose `turn/completed` (session stuck Working), `item/completed` or `item/tool/requestUserInput` (agent blocked without a card). Fix: per-thread unbounded queues, no raw storage of output deltas, `thread/read` on lag.
- **SES-7. Invalid replies to some Codex server requests.** `codex.rs:191-203`, checked against the 0.160.0 schema: legacy `execCommandApproval`/`applyPatchApproval` expect `denied`/`abort`, not `decline`; `item/permissions/requestApproval`, `mcpServer/elicitation/request` and `item/tool/call` get `{}` while fields are required; a `requestUserInput` for an unknown thread gets `{}`. An MCP elicitation can stall the turn. Fix: JSON-RPC errors or the correct deny values.
- **SES-8. The install guard is held across slow operations.** `sessions.rs:177-181,386,559,585,620,643`, `predrafts.rs:143-160,224-229,450,464`: `git fetch`, `/user`, `worktree add` (with LFS smudge), Codex `thread/start|resume`, `turn/start` (60 s timeouts), push and GitHub calls run while the global `installing` mutex is held. A slow fetch or a hung app-server freezes sending, answering and resuming for every session, and the scheduler. Fix: hold it only for a check-and-register step plus an in-flight counter.
- **SES-25. Stopping a Codex turn marks a still-running command Done.** (from the earlier review, R01; re-verified) `codex.rs:355-375`: at `turn/completed` the adapter writes synthetic "Interrupted" results for open tool items and sets Waiting, while spike 3 shows that `turn/interrupt` does not stop a running command (its real `item/completed` arrives later). After Stop, the session counts as Done while the command still changes files: dependents can start, and a pre-draft looks idle for publication. Keep the session Background until the open command items complete.
- **SES-26. Answers are consumed before they are delivered.** (from the earlier review, R12; Claude part re-verified) `claude.rs:162` removes the question from the map before writing the answer, and Codex removes the request before responding; a write failure leaves an unanswerable question ("already answered"). Answers are zipped with questions without checking the count. In `EventRow.tsx` (AskCard), picking the last single-select option can submit a mixed single/multi-select set before Send answer, and the multi-select Send button lacks the live/completeness checks of the option buttons.
- **SES-27. Event and snapshot persistence is not one ordered, atomic operation.** (from the earlier review, R02; groups SES-9 to SES-11) Besides the races below, `Sink::emit` inserts the event and updates `last_seq` in two statements without a transaction, and logs a failed insert but still broadcasts the event, so the UI can show an event that was never stored. Serialize each session's mutations, numbering, persistence and publication through one writer.
- **SES-28. Imported history is a lossy snapshot.** (from the earlier review, R31) `history.rs` imports image messages with `images: []` (image-only instructions become empty), stores `Usage::default()`, handles only a subset of native item types, samples Claude metadata from the start of the transcript, and resolves the project with `projects::for_path`, which misses linked worktrees outside the project folder (no project link, no Files/context navigation). Resume still works through the native id and cwd. Label imports as partial snapshots.

**Low**

- **SES-9. Event numbers can collide after a restart.** `sessions.rs:239-274` writes `last_seq` from a stale snapshot while `Sink::emit` (745-760) updates it separately; after a restart the next insert can hit the primary key, the event is not stored and is broadcast with a duplicate number. Fix: compute `max(seq)` on load or stop writing `last_seq` in `persist`.
- **SES-10. Events can be published out of order.** `sessions.rs:745-765` assigns the number under the lock but broadcasts after the database write; together with UI-1, an event can be dropped.
- **SES-11. `update()` can save an older snapshot.** `sessions.rs:277-287` writes the whole row from a clone; a late title write can restore Working in the database while memory says Waiting (a restart then shows Interrupted instead of Done).
- **SES-12. One invalid UTF-8 line stops a reader.** `claude.rs:95-116`, `codex_rpc.rs:102-112` use `lines()`; the loop ends and the child blocks on a full pipe. Use `read_until` and `from_utf8_lossy`.
- **SES-13. SIGTERM by PID 5 s after Archive, without checking for exit** (`claude.rs:185-192`): PID reuse risk; child commands may survive.
- **SES-14. Presence.** `sessions.rs:230-237` expires presence after 10 minutes while the UI only sends it on focus and pane changes, so watching a long turn still notifies; one global value for all windows; any socket close clears it (`ws.rs:19`).
- **SES-15. `resume()` ignores the current state** (`sessions.rs:619-639`): it sets Waiting even when a process is attached and working (false Done and notification).
- **SES-16. `answer()` sets Working after writing the answer** (`sessions.rs:584-593`): a fast `result` can be overwritten, leaving Working. Set it before writing.
- **SES-17. Archive during a launch is undone** (`sessions.rs:604-616`): `close` takes no lock.
- **SES-18. A launch error after the session row exists** (`sessions.rs:469-481`) marks the scheduled entry Failed while the session stays Starting; Start now then deletes the entry without launching; temporary folders or pre-draft worktrees are left behind.
- **SES-19. A wedged Codex app-server is never restarted** (`codex_rpc.rs:60-82,153-165`); a hang in `initialize` leaks the process; `get()` holds the mutex up to 60 s. On app-server exit, all threads are cleared, possibly including one just registered on the new process (`codex.rs:150-156`).
- **SES-20. Resume may change the model.** Codex `thread/resume` does not send the model (`codex.rs:51-52`); imported Claude sessions resume with the API model id and can lose `[1m]` (`history.rs:224`).
- **SES-21. `tui_running` detection is narrow** (`terminal.rs:117-130`): misses `-r`, `-c` and `/resume` inside a TUI.
- **SES-22. Environment hygiene.** Stripping every `CLAUDE_CODE_*` (`shellenv.rs:55-63`) also removes variables the owner sets on purpose (`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS`), contradicting "same variables as in a terminal"; systemd variables (`JOURNAL_STREAM`, `INVOCATION_ID`, `RUST_LOG`) leak into agents, so a dev daemon started by a prod agent logs without timestamps (`cli main.rs:202`); `GIT_DIR`, `GIT_INDEX_FILE`, `GIT_CONFIG_*` would also be inherited by every git call.
- **SES-23. The login-shell probe** (`shellenv.rs:70-79`) runs `$SHELL -l -i` without `setsid`; under a terminal (dev daemon) it can take the terminal's foreground group; on timeout only the shell is killed.
- **SES-29. Message delivery outcomes are not tracked.** (from the earlier review, R11) Codex falls back from `turn/steer` to `turn/start` on any error, not only on "turn ended" (`codex.rs`); `codex_rpc.rs` requests time out after 60 s without reconciliation, so an accepted but unacknowledged message can be sent twice by a retry. A stored `UserMessage` is shown as sent even when delivery failed (see SES-5).
- **SES-30. Codex stop drops the completion route.** (from the earlier review, R46) `CodexSession::stop` ignores interrupt and unsubscribe failures and removes the thread from the router while the shared app-server keeps running, so work that survives the interrupt loses its completion events.
- **SES-31. Model lists are not pushed to the UI.** (from the earlier review, R42) `harness/mod.rs` refreshes `ModelCache` at startup and every 30 minutes without an event; the composer query has no timer and window-focus refetch is disabled, so the picker can keep an empty or old list.
- **SES-24. Info.** Stale "background-command heuristic" comment and dead Background→Working branch in `codex.rs`; `codex update` has no timeout; the Codex effort fallback list includes `max`; title generation with Claude runs the owner's hooks; Claude imports keep raw JSON as tool summaries, produce no `FileChange` events and include meta user messages; `thread/read includeTurns` is deprecated for paginated threads; `load` adds no "interrupted" notice; `PR_SET_PDEATHSIG` is correct today only because no spawn happens on a blocking thread (fragile), and there is no `getppid()` check after `prctl`.

### 7.2 Scheduler

- **SCH-1 (Medium). A second daemon can claim the same launch.** The claim at `schedules.rs:163` is a SELECT then an UPDATE without a transaction; combined with SRV-1, two processes can start the same agent. Use `UPDATE … WHERE status='queued'` and check the changed row count.
- **SCH-2 (Low). Scheduled launch shown twice while starting.** `schedules.rs:241-265` publishes the session before `finish()` deletes the schedule; the Agents view shows the id in Scheduled and Running with a duplicate React key (`AgentsView.tsx:139,148-154,191`). Filter schedules whose id is already a session.
- **SCH-3 (Low). Dependencies accept scheduled records in any status** (Failed, Starting), not only queued ones (DOC-21).
- **SCH-4 (Low). False Done from SES-4 and SES-25** can start a dependent too early.
- **SCH-5 (Info). Readiness is one observation.** (from the earlier review) Dependencies are checked at one tick; claim and launch are later steps, so a predecessor can be resumed in between. This matches the documented "same check" wording; decide whether "after Done" should be rechecked under the start reservation.
- Verified correct: claims serialized by the database actor, crash recovery without automatic retry, cancelled or missing dependencies never Done, no cycles, quota deadlines frozen and requiring a reading under 15 minutes old, ready entries queued during installation, scheduled id becomes the session id, temporary folders created only at launch.

### 7.3 Projects, temporary folders and terminal handoff

- **PRJ-1 (Medium). Leftover temporary folders from automated checks in the real projects root.** (verified) The dev profile uses the same `projects_root` (`~/projects`) as prod, so dev checks created `~/projects/tmp-1002-reply-ready-only-do`, `-2`, `-3` and `tmp-1002-sleep-30minutes-before-starting`. Give the dev profile its own default projects root, or a temp prefix, and clean these up.
- **PRJ-2 (Medium). Promote and cleanup do not reserve the folder.** `running_in`/`close_in` (`sessions.rs:666-689`) skip `in_terminal`; `delete_project` and `promote` (`api.rs:89-117`) check then act without holding `allow_work`, so a launch or message can enter the folder between the check and the rename or deletion (the earlier review rates this High, R47). Cleanup calls `remove_dir_all` without waiting for the stopped processes to exit.
- **PRJ-7 (Medium). Removing a project orphans its local PRs and worktrees.** (from the earlier review, R29) `delete_project` unregisters a regular project without checking dependent pre-drafts; their detail, publication and discard need the project row, and re-registering creates a new id. Deleting a temporary git project removes the common git folder that linked worktrees stored elsewhere still need.
- **PRJ-8 (Medium). Promotion has no recovery.** (from the earlier review, R30) `projects.rs` renames the folder, moves Claude transcripts (failure only logged), then updates the project row and session cwds, without rollback; linked worktree administrative paths are not repaired. A failure after the rename leaves records pointing at a missing folder.
- **PRJ-3 (Low). Claude transcripts silently not moved on promote** when the destination folder exists (`projects.rs:241-244`), so old sessions cannot be resumed.
- **PRJ-4 (Low). Terminal template quoting.** `{cwd}` inside a shell-string custom template is not quoted, and the Codex `-c projects."<cwd>"…` override breaks with `"` or `\` in the path (`terminal.rs:58,91`).
- **PRJ-5 (Low). `github_repo` misses `https://user@github.com/...` and `insteadOf` aliases** (`git.rs:52-66`); Publish then says "origin must be a GitHub repository".
- **PRJ-6 (Info). Discovery lists every git repository**, not only those with a GitHub origin (DOC-11).

### 7.4 GitHub client and inbox

**High**

- **GH-1. One thread's subscription error aborts the whole sync.** (verified) `crates/core/src/inbox.rs:235`: `app.github.subscription(&t.id).await?` inside the loop over about 240 threads; threads are saved only later (266-275). A 502, a 30 s timeout or a permanent 403 (SAML org, lost access) fails every pass at the same thread, and the inbox never updates again. Fix: treat a per-thread error as unknown, keep the cached value, continue.

**Medium**

- **GH-2. A failed sync consumes the page-1 change.** `github.rs:194-198` stores every response immediately; `inbox.rs:205` stops on a page-1 304. After a failure later in the pass, the next poll gets 304 and does nothing; new notifications wait up to 30 minutes. Fix: reset `full_at` or delete the stored page-1 response on failure.
- **GH-3. Detail prefetch runs after every poll.** `inbox.rs:281-307`: up to 15 `items::detail` calls every 60 s, each a GraphQL check plus image proxying: about 900 GraphQL points per hour (18% of the budget) with an idle inbox, and overlapping tasks. Prefetch only new or changed threads.
- **GH-4. The inbox Refresh button is almost always a no-op.** `inbox.rs:172-174`: a manual sync obeys the 60 s window that every poll resets, with no feedback.
- **GH-5. Each full pass checks and enriches the whole `all=true` history while holding the inbox lock.** `inbox.rs:224-247,298-301`: about 240 sequential subscription GETs and one database write each, for about 4 displayed threads; actions and preview refreshes wait (all row buttons disabled, `InboxView.tsx:170`). Threads marked Done locally are absent from `list()`, so they are re-enriched every pass, and their enrichment is discarded; Release/Commit threads never get `state` and are refetched every pass.
- **GH-6. GraphQL partial data is cached as complete.** `github.rs:236-240` drops `errors` when `data` is not null. A null `timelineItems` stores an incomplete conversation as fresh (`items.rs:81-97,107-128`); `repository: null` is cached for 5 minutes as `{prs:null, issues:null}` and crashes `RepoItems.tsx:115-116` (`[...null]`).
- **GH-7. GitHub's error messages are discarded.** `github.rs:184-187` never reads the body, so the UI shows "422 Unprocessable Entity" instead of "Can not approve your own pull request" or "Head branch was modified"; draft/ready drops GraphQL errors (`items.rs:238`). Contradicts decision 37 ("API failures remain visible").

**Low**

- **GH-8.** The 30-minute unconditional pass can be skipped indefinitely: `full_at` resets on any page-1 change (`inbox.rs:276-278`).
- **GH-9.** Local read/unread overrides are written back into `inbox_thread.json` by `enrich` (`inbox.rs:281-288,402-417`).
- **GH-10.** A half-done Unsubscribe (PUT ok, DELETE failed) is rolled back and reported as a failure while the thread is already ignored on GitHub (`inbox.rs:468-506`).
- **GH-11.** `error_for_status().unwrap_err()` (`github.rs:186`) panics on an unfollowed 3xx and would stop the poller task.
- **GH-12.** Rate limits: no backoff for a 403 secondary limit without `Retry-After`; `Retry-After` recorded only with `x-ratelimit-*` headers (`github.rs:156-172`); a revoked token reruns `gh auth token` on every request; 404 subscription responses are not cached (`github.rs:194`).
- **GH-13.** `gh_suggest` and `resolve_link`/`parse_link` skip `items::validate` (`api.rs:267-273`, `github.rs:247,356-393`).
- **GH-14.** Inbox preview renders raw Markdown with `html: false`, so PR templates show `<!-- -->` and `<details>` as text; prefetch failures are logged at debug level only (`inbox.rs:305`).

### 7.5 Issues, pull requests, diffs and images

**Medium**

- **ITM-1. Files changed fails offline and costs a request per open.** `items.rs:419`: `diff()` calls `detail(…, false)`, whose network check (`:56`) fails before the diff cache (`:428`) is consulted.
- **ITM-2. `parse_diff` gets file names wrong.** (verified structure) `items.rs:529-546`: git adds a trailing tab to `---`/`+++` names that contain spaces, so names become `with space.txt\t`; a path containing ` b/` is split at the wrong place (`a b/c` → `c`); content lines starting with `++ b/` or `++ /dev/null` are taken as headers; `diff.noprefix`/`mnemonicPrefix` would break all names; `procs::output` trims stdout and drops a final blank context line. Effects: View file opens a missing path, file notes attach to the wrong path. Quoted paths are decoded as JSON strings, which does not cover git's octal escapes for control and non-ASCII bytes (earlier review, R08). Fix: take names from `--raw -z`, pass `--src-prefix=a/ --dst-prefix=b/`, parse metadata only before the first hunk, do not trim. AGENTS.md names the diff parser as a good test candidate: this is where a test pays off.
- **ITM-3. No API fallback when no remote matches.** `items.rs:434-440`: `.context("missing mapped remote")?` sits outside the error-tolerant block, and the remote comparison is case-sensitive while the project lookup is not (`projects.rs:75`). Notification routes use the canonical case, so `git@github.com:wauplin/launchpad.git` vs `Wauplin/launchpad` gives no diff.
- **ITM-4. The image cache is keyed by the signed URL and never evicted.** `items.rs:286`: `private-user-images` URLs carry short-lived JWTs; each detail check (every poll with GH-3) downloads the same image again under a new key. Key signed hosts by host and path; add eviction.
- **ITM-5. Image downloads block detail refreshes.** `proxy_images` (`items.rs:377-387`) fetches sequentially, 15 s timeout each, inside the request, retrying permanent failures every time; it delays details, draft/ready and suggestion posting (which holds `predraft_lock`).
- **ITM-6. Saving labels with PUT removes labels added by others.** `items.rs:195-199`, `ItemView.tsx:257-303`: the field is filled once with `useState` and not refreshed when the chips update; a bot's `needs-triage` added after load is removed on Save; labels beyond 50 are dropped. Use add/remove per label.
- **ITM-15. The client diff identity omits the base.** (from the earlier review, R23; re-verified) `FilesChanged.tsx:279` keys PR and local diffs by head only, with an infinite stale time, and item Refresh does not invalidate `pr-diff`. When the base advances or the PR is retargeted with the same head, the old diff stays on screen.
- **ITM-16. View file on a deleted file uses the wrong base.** (from the earlier review, R24; re-verified) `items.rs:515` returns `baseRefOid` while the local diff is three-dot (old side = merge base). When the base branch changed or deleted the same file, View file shows different content or fails. Return the merge base (the pre-draft path already does).
- **ITM-17. Invalidation does not mean a GitHub refresh.** (from the earlier review, R25) `ItemView.tsx` and `RepoItems.tsx` decide to POST a refresh from effects keyed on `fetched_at`; GET returns the cached object. After closing or merging an item, the project list can keep it as open; a fresh detail opened before CI ends keeps old checks until manual Refresh.
- **ITM-18. Null GitHub authors and reviewers crash item views.** (from the earlier review, R56; re-verified) `ItemView.tsx:320` (`r.author.login`), `:328` (`r.requestedReviewer.login`), `RepoItems.tsx:53` (`i.author.login`) dereference fields that GitHub's schema allows to be null (deleted accounts, "ghost"). One such item hides the whole PR view or project list behind an error.

**Low**

- **ITM-7.** The diff can belong to a newer head than the one displayed (`items.rs:419`), while viewed marks and review `commit_id` use the displayed head (`FilesChanged.tsx:279-281,329`, `ItemView.tsx:726`). Send the expected head to `/diff`.
- **ITM-8.** Proxied images inside `<a href>` and in-page anchors (`#user-content-fn-1`) open the daemon URL in the external browser, which shows a 401 page (`items.rs:388-415` replaces every occurrence; `lib.tsx:52-57`, `Markdown.tsx:71-76`).
- **ITM-9.** Draft/ready refetches the whole item (`detail(force=true)`) just to get its node id (`items.rs:224-236`).
- **ITM-10.** Timelines fetch every event type (Subscribed, Mentioned…) as empty cards; checks silently capped at `contexts(first:100)` (`items.rs:50`).
- **ITM-11.** Cache keys depend on letter case (`item:{repo}:{n}`), so the same item can be cached and fetched twice.
- **ITM-12.** Fork projects share `refs/launchpad/pr/<n>` between origin and upstream PRs, and these refs are never pruned (`items.rs:441`).
- **ITM-13.** Concurrent image fetches rewrite the cache file non-atomically and a new `reqwest::Client` is built per miss (`items.rs:287-296,337-338`).
- **ITM-14.** Two diffs lack `--no-textconv` (`code.rs:481`, `items.rs:455-462`): line marks can mismatch and textconv programs run on PR blobs if configured.

### 7.6 Agent-written suggestions

- **SUG-1 (Medium). A clearly rejected or never-sent post is locked as "uncertain" forever.** `suggestions.rs:239-260`: every error sets `uncertain`, including the local rate-limit gate, a failing `gh auth token` and explicit 4xx rejections (approving one's own PR). Edit and Post are then refused; Discard is the only way, and the card shows rendered Markdown, so copying loses the source. Revert to `draft` when nothing was sent or a 4xx came back.
- **SUG-2 (Low). Posting holds `predraft_lock` during network calls** (`suggestions.rs:177-266`), blocking pre-draft launches, notes and publication.
- **SUG-3 (Low). Posting claims can be lost on power failure**: WAL with `synchronous=NORMAL` (`db.rs:53-56`) does not fsync at commit, which weakens "never post twice" after a crash.
- **SUG-4 (Info).** Discarding a posting or uncertain suggestion is one click with a text warning (`Suggestions.tsx:59`). Manual comments, reviews and replies (`items.rs::mutate`, `inbox.rs` reply) have no duplicate protection after a lost response: a successful post whose response was lost looks failed, and a retry posts twice (the earlier review rates this Medium, R50). Showing "check GitHub before retrying" after a transport error would be enough.
- Verified correct: 64 KB body limit, 40-hex `--head`, claim persisted before the mutation, posting/uncertain never posted again, claims survive restart, forced refresh checks open PR and head, `commit_id` sent, local PR notes saved with the posted marker in one transaction.

### 7.7 Pre-draft PRs

**High**

- **PD-1. Publish pushes, then never creates the PR in single-branch or shallow clones.** (verified) `predrafts.rs:561-570`: after `git push`, `git branch --set-upstream-to=origin/<name>` fails because the push does not create `refs/remotes/origin/<name>` when `remote.origin.fetch` maps only `main` ("the requested upstream branch … does not exist"). `pushed_head` is already saved (`:548-549`); a retry fails at the same step; Discard is refused; agent turns are frozen. Fix: make the upstream step best-effort and run it after PR creation, or write `branch.<b>.remote/merge` with `git config`.
- **PD-2. A push that always fails locks the record for good.** `predrafts.rs:473-560,454-457,517-521,118-126`. Confirmed triggers: `@{-1}` passes `git check-ref-format --branch` but is pushed raw ("invalid refspec"); `submodule.recurse=true` in the owner's global config makes push recurse and fail when a submodule commit is not on its remote. Plausible: no push rights on `origin`, rulesets on branch names, a `user` branch conflicting with `user/<slug>`. Fix: validate `refs/heads/<name>` with plain `check-ref-format`, check prefix conflicts, push with `-c submodule.recurse=false --recurse-submodules=check`, and add "Abandon publication" when `ls-remote` shows the branch absent.

**Medium**

- **PD-12. Publication can push to a different repository than the PR targets.** (from the earlier review, R06, rated High there; parser re-verified) `git.rs:22-50` reads only `remote.<name>.url` from the config file, while `git push origin` honors `pushurl`, `insteadOf`/`pushInsteadOf` and includes. With `url` pointing to repository A and `pushurl` to B, the reviewed branch goes to B and the PR is created in A. Resolve the effective URLs with `git remote get-url --push origin` and show the destination in the confirmation.
- **PD-3. The publication freeze and `idle()` can be bypassed.** The earlier review (R04, High) reproduced a further path: a launch with target `Project` into a pre-draft worktree registered as a normal project skips `check_session_work`, and sessions in a subfolder of the worktree evade the exact-string `idle()` and freeze checks. In addition: `answer` (`sessions.rs:584-593`) never calls `check_session_work`, and a session with an open question counts as idle (`predrafts.rs:279-289`): answering resumes a turn in a frozen worktree. CLI-created pre-drafts: the skill tells the agent to switch to the new folder, but its session cwd stays the original checkout, so `idle()`, the freeze and the status never see it; Publish or Discard can run while it commits.
- **PD-4. Locks held across network and long git operations with no timeout** (`sessions.rs:386,403`, `predrafts.rs:450-461,464-612`): no `GIT_TERMINAL_PROMPT=0`, no SSH `BatchMode`; a stalled SSH connection freezes all agent input (see SES-8).
- **PD-5. Discard deletes ignored files without warning.** `predrafts.rs:423-434`: the clean check ignores ignored files, and `git worktree remove` deletes them (`.env`, `target/`, agent outputs). Add `--ignored` to the check or list them in the confirmation.

**Low**

- **PD-6.** Mutating git runs inline in the request handler with `kill_on_drop(true)` (`procs.rs:19`): a page reload during Discard, Publish or a pre-draft launch can kill `worktree remove`, `push` or `worktree add` half-way. Run mutations in a detached task.
- **PD-7.** Discard fails forever when the worktree folder was deleted by hand (`branch -D` refuses; `predrafts.rs:422-446`). Run `git worktree prune` first.
- **PD-8.** Broken records (missing branch, renamed default branch) show only an error and no Discard button (`predrafts.rs:323-342`, `PredraftView.tsx:72`).
- **PD-9.** Two agents can share one pre-draft worktree: an existing-pre-draft launch only refuses when another session there is running (`sessions.rs:427-431`).
- **PD-10.** Lost update window: the record is saved with `session_id` after `create()` released the lock (`sessions.rs:403-413`).
- **PD-11.** `predrafts.rs:373` runs `git status` without `GIT_OPTIONAL_LOCKS=0`; `initial_base` is stored but never used.
- **PD-13.** (from the earlier review, R07, rated High there) The remote-branch absence check (`ls-remote`) and the push are separate; a branch created in between at an ancestor of the reviewed head is fast-forwarded instead of refused. Very unlikely for a single user, but `--force-with-lease=refs/heads/<name>:` (expect absent) makes creation atomic at no cost. The UI message also says "pushed" when the checkpoint only records an attempted push.

### 7.8 Code viewer and per-session git panel

- **CODE-1 (Low). Git panel in a subfolder cwd.** Porcelain paths are relative to the repository root but `git diff -- <path>` is relative to the cwd, so diffs are empty and untracked reads fail (`workspace.rs:31-35,86-107`). Run git in `rev-parse --show-toplevel`.
- **CODE-2 (Medium). Working-tree `git diff` rewrites the index, even with `GIT_OPTIONAL_LOCKS=0`.** Confirmed on git 2.34.1 after a stat-only change; while it runs it holds `index.lock`, and an agent's `git add` fails ("index.lock: File exists"). Callers: the git panel every 5 s (`workspace.rs:83-102`, no `GIT_OPTIONAL_LOCKS` at all), and the code viewer's live reload on agent `file_change` events (`code.rs:340-345,477,481-486`). Use a temporary index copy (`GIT_INDEX_FILE`) for working-tree diffs.
- **CODE-3 (Low). File reads.** Size check then full read is not atomic (a growing log can exceed the limit); a FIFO passes the `is_dir` check and blocks a thread forever; the path is canonicalized and later opened by name, so a concurrent symlink swap can redirect the read outside the root (earlier review R20, rated Medium) (`code.rs:411-419`). Require `is_file()` on the opened descriptor and read with `take(limit+1)`.
- **CODE-4 (Low). `search()` may stall** when git writes over 64 KB to stderr, which is read only after stdout EOF (`code.rs:642-669`).
- **CODE-5 (Low).** `commit()` splits remote names on `/`; `--contains` can be slow; `sources()` reads all refs before the 150 cap; no letter for staged files in a repository without commits; `main_worktree` resolves a relative `gitdir` against the daemon cwd (breaks with git 2.48+ relative worktrees).
- **CODE-6 (Low). The README reopens after closing all tabs** (`CodeView.tsx:89-94`): the effect reruns on each file-list refetch.
- **CODE-7 (Low). Regex find can freeze the UI.** (from the earlier review, R33) `code/lib.ts` runs the find regex synchronously over the whole file during render; a pattern with nested repetition on a long line blocks the webview. Only the owner's own input triggers it.
- **CODE-8 (Low). A failed highlight worker leaves later requests pending.** (from the earlier review, R44) `code/highlight.ts` resolves current requests on error but keeps the failed worker; new requests are posted to it and never resolve.
- **CODE-9 (Low). Add to agent drops the source root for an existing session.** (from the earlier review, R38) `CodeView.tsx` sends `path:lines (at rev)` without the root to a session's message box; sessions of the same project can run in different worktrees, so the agent may read the wrong checkout. Use the composer's `FileRef` rendering.
- **CODE-10 (Info).** The non-git directory walk stops at 200,000 files without a truncation flag; `/api/code/sources` does not go through `code::open` (it only returns metadata of a registered project), unlike the AGENTS.md wording "every `/api/code/*` request".
- Verified correct: see section 6. The `-z` grep parser, the 1,000-hit cap, the 30 s timeout, `submodule.recurse=false` for grep, line marks, binary detection and size limits match the docs.

### 7.9 Daemon, database, configuration and CLI

**High**

- **SRV-1. A second daemon modifies live prod data before it notices the port is taken.** (verified) `crates/cli/src/main.rs:160-165` calls `App::start` (`crates/core/src/lib.rs:60-96`) before `serve` binds. `App::start` runs state.db migrations, deletes `cache.db` on a schema-hash change, marks every running session `interrupted` (`sessions.load`), fails in-flight scheduled claims, starts the scheduler (first tick immediate) and the inbox poller. Scenarios: an agent working on this repository runs `cargo run -p launchpad -- daemon` without `LAUNCHPAD_ENV=dev` (prod is the default); or the desktop shell, after `systemctl --user start`, waits only 10 s for health and then also spawns `launchpad daemon` directly (`desktop main.rs:72-104`). Fix: take an exclusive `flock` on `data_dir/daemon.lock` and bind the listener first; spawn directly only when the unit does not exist; fail on unknown `LAUNCHPAD_ENV` values (they silently mean prod, `paths.rs:16-21`).

**Medium**

- **SRV-2. Polled endpoints load whole session histories on the single state.db thread.** `api.rs:230-249` (`task_output`, polled every 3 s per running background task row, `EventRow.tsx:253-257`) loads and parses every event including base64 images; `api.rs:462-471` (search) runs `json_extract` over all events. Both stall event persistence for every agent (and feed SES-6).
- **SRV-3. A panicking database job disables that database for good** (`db.rs:21-32`): later calls return "database thread stopped" while the process stays up, so systemd never restarts it. Catch the panic or exit.

**Low**

- **SRV-4. Config file handling.** Write-to-temp-and-rename replaces a symlinked `config.toml` (dotfiles), and the watch misses edits to a symlink target; concurrent saves share `config.toml.tmp`; `zoom` validated only on save; a file that failed to parse is overwritten with defaults on the next UI save (`config.rs:192-200,221-253`).
- **SRV-5. Startup fails if the skill link fails** (`lib.rs:86`, `link_skill()?`).
- **SRV-6. CLI error handling.** `error_for_status()` drops the server's `{"error": …}` (`cli main.rs:128-135,166-177`); `pr show` outside a worktree prints a bare 500; the resolve endpoint returns 500 instead of 404; text 403 responses give "error decoding response body".
- **SRV-7. Framework rejections are plain text** (axum JSON/Path/Query), while `web/src/lib/api.ts:19-20` parses JSON and shows a parse error instead of the reason.
- **SRV-8. Child processes are not reaped**: `notify.rs:44-49` and the desktop's `xdg-open`/daemon spawns use `std::process::Command` without `wait` (zombies in long-lived processes).
- **SRV-9. The 28,000,000-character input limit** allows about 21 MB while the message says 20 MB (`api.rs:179-180`).
- **SRV-10. Unbounded reads.** (from the earlier review, R45) `sessions::events` returns all remaining events in one array; `procs::output` awaits commands without a shared timeout or output cap.

### 7.10 Desktop shell, packaging, updater and CI

**High**

- **OPS-1. The updater waits forever on untracked files, and its status can stay "running" forever.** (verified) `packaging/update.sh:32` loops while `git status --porcelain` is not empty, which includes untracked files. At the time of this review, `launchpad-update.service` had been looping on "Waiting for committed main" for more than two hours because of the untracked `docs/CODE_REVIEW_2026-10-03.md` (this file blocks it too). In addition, the TERM/INT traps (`:25-27`) release the reservation but write no final status, and SIGKILL, logout without linger or reboot leave `update-status` at `running`; the UI then disables Update and shows "Updating…" forever (`api.rs:289-296`). The idle wait (`:74-84`) has no timeout or cancel. Fix: `--untracked-files=no` (or show "untracked files present"), write a failed status in the traps, report `failed` when the unit is not active, add Cancel and a timeout.
- **OPS-2. Apps started by the daemon are killed by every restart.** (verified) `terminal.rs:95-105` uses `setsid`, which does not leave the cgroup; `packaging/launchpad.service` has `KillMode=control-group`. Terminals, `xdg-open` browsers, editors and `launchpad-desktop` windows started by notification clicks (`notify.rs:44-49`) are in the daemon's cgroup. If Terminator was first started by "Open in terminal", every later Terminator window belongs to it; Update kills the `claude --resume` TUI and all those windows. `prepare_install` checks only `is_running()`, which excludes `in_terminal` (`sessions.rs:160-171`, `model.rs:78-80`), so decision 33 ("never interrupts active agents") does not hold. Fix: `systemd-run --user --scope --collect` (or `gio launch`) for external apps, and count `in_terminal` sessions with a running TUI as busy.

**Medium**

- **OPS-3. The service may start before the graphical session (plausible).** `packaging/launchpad.service`: `After=network-online.target` has no effect in the user manager, and nothing orders the unit after `graphical-session.target`. The environment is captured once at startup; without `DISPLAY`/`WAYLAND_DISPLAY`, every browser, editor, terminal and notification click fails for the whole day. Add `After=graphical-session.target` and `PartOf=graphical-session.target`, or read `systemctl --user show-environment` before GUI spawns.
- **OPS-12. The manual installer does not reserve production and can inspect the wrong profile.** (from the earlier review, R34) `packaging/install.sh:38` probes prod health, then runs `launchpad status` with the caller's `LAUNCHPAD_ENV`; from a shell with `LAUNCHPAD_ENV=dev` it counts dev sessions before restarting prod. It also does not take `update-install-lock`, so a session can start between its last check and the restart.
- **OPS-4. The install reservation never expires.** `sessions.rs:160-171`: if `update.sh` is killed after reserving, all new work is refused until the daemon restarts. Any authenticated client (agents included) can reserve.

**Low**

- **OPS-5. `update.sh` details.** Lock fd 9 is inherited by build children (a long-lived child such as an sccache server keeps the lock, and later updates exit 0 silently); `cargo` runs without `--locked`; `check_revision` cannot detect a tree changed and restored during the build (build from `git worktree` or `git archive` instead).
- **OPS-6. `install.sh` checks.** If `curl` is missing, `check_running` reports 0 sessions and restarts without warning (plausible); the 10 s health wait is shorter than a possible startup (shell probe up to 10 s plus load); its health check can be answered by an orphan daemon (SRV-1).
- **OPS-7. Build script reruns.** `rerun-if-changed` on `.git/index` rebuilds core and dependents on every `git add`; a packed `refs/heads/main` may rerun it on every build (`crates/core/build.rs:13-17`).
- **OPS-8. The Update button stays hidden** when the running commit is not an ancestor of main (`api.rs:284-288`).
- **OPS-9. Desktop shell.** `launchpad-desktop --quit` opens a window when no instance runs (`main.rs:177-190`); the warm shell stays on the fallback page with no error when the token is unreadable (`main.rs:199-204,230`).
- **OPS-10. CI.** Actions pinned to major tags, not SHAs; never run (no remote); artifacts embed the runner's path as `LAUNCHPAD_SOURCE`, so self-update cannot work from them. No `rust-toolchain` file, so local `cargo` is not pinned to 1.93.1.
- **OPS-11. Icons.** `packaging/launchpad.svg` (1.4 MB) is a base64 copy of the 1024 px PNG; `launchpad.webp` is unused; `install.sh` installs 4 sizes while `install-icons.sh` installs 10.

### 7.11 Web UI

**Medium**

- **UI-1. Session events are dropped while the log loads.** `web/src/lib/sessionStore.ts:31-47,62-66`, `SessionPane.tsx:157-162`: a WS event that shows a gap calls `load()`, which returns immediately while a load is in progress, so the event is lost until the next gap. Opening a session as its turn ends can hide the final answer. Fix: remember the gap and reload in `finally`.
- **UI-2. Live command output freezes at 8,000 characters.** (verified) `sessionStore.ts:57` trims output to the last 8,000 characters, and the row version is `c.live?.length` (`rows.ts:123`), so the memo (`EventRow.tsx:316`) stops re-rendering once the length stays at 8,000. The same version ignores the result content: a synthetic "Interrupted" result later replaced by the real result (SES-25) keeps the same version, so the row keeps showing "Interrupted" (earlier review, R09). Use a revision counter that changes with any content.
- **UI-3. The conversation virtualizer has no `getItemKey`.** `SessionPane.tsx:174,294-299`: size caches are keyed by index while rows move (only the latest plan event makes a row), so rows can overlap or leave gaps in Codex sessions with plan updates (plausible, not reproduced).
- **UI-4. Moving the caret with the mention list open corrupts the prompt.** `PromptField.tsx:82-106,116-139`: the mention state is recomputed only on change and click; after ArrowLeft, Enter inserts the item and keeps the old text (`fix @moon-landing x @moo`). Breaks "ignoring it never changes what you typed".
- **UI-5. The build-id auto-reload discards unsent work.** `web/src/lib/ws.ts:63-66`: message box text and images, inbox quick replies (in memory) and the last 400 ms of composer edits are lost; the updater installs exactly when agents are Done, which is when the owner types. Persist drafts or show a "Reload to update" button.
- **UI-6. The custom terminal command field swallows spaces.** (verified) `SettingsView.tsx:144-148`: `split(" ").filter(Boolean)` on each keystroke removes a trailing space, so the command cannot be typed normally.
- **UI-7. The project page session list is not virtualized** (`ProjectPage.tsx:74-88`) and grows without limit.
- **UI-29. A failed first load leaves a conversation on Loading forever.** (from the earlier review, R53) `SessionPane.tsx` creates the log and calls `load()` only when no log exists; `sessionStore.ts` records no error, so after a transient API failure the pane shows Loading with no Retry until a reconnect or a reload.
- **UI-30. Launch defaults are rebuilt from the current session status.** (from the earlier review, R36; re-verified) `composer.ts:60,106` picks the latest session with `last_seq > 0 && status !== "error"`: after a reload, a launch that succeeded and later failed is skipped and an older spec becomes the default. AGENTS.md says "latest successfully launched spec". Persist the last successful launch spec explicitly.
- **UI-31. Composer draft saves fail silently.** (from the earlier review, R37) `composer.ts` debounces daemon saves by 400 ms and swallows errors; `restore()` marks the draft loaded even when the read failed. Typing while the daemon is down loses the draft without notice (see also UI-5).
- **UI-8. Click-only violations.** The GitHub link field (Enter adds, Escape closes, `Composer.tsx:109-112`), the project description field (Enter saves, Escape cancels, implicit save on blur without a Save button, `ProjectsView.tsx:36-39`), "Add a folder by path" (Enter adds, `ProjectsView.tsx:242`), and a global Escape handler in `Popover.tsx:27-29`.

**Low**

- **UI-9. Other long lists not virtualized**: discover list (149+ repositories, `ProjectsView.tsx:181-196`), project registry (`:233-235`), branch popover (`Composer.tsx:66-68`), worktrees in the source picker (`CodeView.tsx:227-248`), expanded EditRow diff (`EventRow.tsx:151-157`), worktrees on the project page and pre-draft notes (`PredraftView.tsx`, earlier review R32).
- **UI-10. Memoization defeated by inline callbacks**: `AgentsView.tsx:157`, `RepoItems.tsx:277`, `DelaySelector.tsx:78,89-90` (re-renders every second even with Start = Now), `ItemView.tsx:142`.
- **UI-11. The inbox re-renders on every scroll event and keystroke** (`InboxView.tsx:131,167`: whole-store subscription plus scroll written to the store).
- **UI-12. Session logs are never evicted** (`sessionStore.ts:29-51,89-93`): every opened log stays in memory, receives deltas and is refetched on every reconnect.
- **UI-13. `localStorage` grows without limit and quota errors are not handled**: `diff-viewed` (never pruned, `JSON.parse` without try/catch, `FilesChanged.tsx:287-289,340-349`), empty `github-reply:*` keys (`ItemView.tsx:689`), `predraft-draft:*` not removed on discard (`PredraftView.tsx:58`), code viewer sources and expanded folders (`code/store.ts:30-46`), pins of deleted sessions (`layout.ts:36`).
- **UI-14. `start()` overwrites an open draft's harness, model and effort** (`composer.ts:76-87`), when adding code lines or clicking Launch elsewhere while the composer is open.
- **UI-15. "Review with agent" and "Launch agent" replace the unsent prompt text** and keep unrelated chips (`ItemView.tsx:611-614`, `PredraftView.tsx:84`).
- **UI-16. Fuzzy linkify turns file names into links**: `setup.py`, `main.rs`, `README.md` become `http://setup.py` because `.py`, `.rs`, `.md`, `.sh` are country TLDs (`Markdown.tsx:13`). Set `fuzzyLink: false`.
- **UI-17. The message box loses focus after each send** (`SessionPane.tsx:104`: disabled while busy).
- **UI-18. Answering a question with images sends the text twice** (`SessionPane.tsx:73-76`).
- **UI-19. Recent file chips show `path:null`** (`Composer.tsx:191`).
- **UI-20. The tool-output copy button has no title** (`EventRow.tsx:100-102`), the only icon-only button without one.
- **UI-21. The composer hides any preset with id `draft-pr`** (`Composer.tsx:389`). The production `config.toml` still contains it, so the owner's preset silently disappears from the composer while Settings still shows and edits it.
- **UI-22. Add to agent offers interrupted, errored and in-terminal sessions** (`CodeView.tsx:694` filters only `closed`; 8.1 says "not archived").
- **UI-23. Any inbox action disables every row's buttons** (`InboxView.tsx:170`), including the automatic mark-read on selection.
- **UI-24. Silent failures**: project page GitHub and Editor buttons have no `.catch` (`ProjectPage.tsx:35,44`); Markdown link clicks ignore `/api/open` errors (`Markdown.tsx:75`).
- **UI-25. The local PR diff is refetched twice on every session status change** (`ws.ts:89-90`, `FilesChanged.tsx:279-280`, `PredraftView.tsx:24`).
- **UI-26. Smaller issues**: item Controls remount on `updatedAt` and lose typed reviewers/labels (`ItemView.tsx:745`); Settings keeps a stale draft after an outside `config.toml` change and overwrites it on Save (`SettingsView.tsx:58-60`); `restore()` races an early Launch click (`composer.ts:103-113`); `FileMeta` splits a 5 MB file twice per render (`CodeView.tsx:402,416`); `String(e)` adds "Error:" prefixes; the first `hello` build id is trusted (`ws.ts:63-67`); `Number(number)` without validation requests `/NaN` (`ItemView.tsx:504`); the review event type is not persisted with the unsent review (`ItemView.tsx:525`, decision 38).
- **UI-27. One-click actions with lasting effects**: Close PR, Close issue, Reopen, Draft/Ready, Save labels (`ItemView.tsx:290-303,435-487`), inbox Unsubscribe (`InboxView.tsx:63`), and Archive on a Running session next to the X (`SessionPane.tsx:254-261`). Merge already needs a second click; these deserve the same.
- **UI-28. Info**: dead `EmptyView` (`Shell.tsx:32`) and unused icon symbols (`grip`, `bell-off`, `mail`, `xcircle`, `okcircle`, `pr-local`); hand-written `Comment` type shadows `gen/Comment`; `UpdateStatus`, `Detail` and `Diff` are hand-written instead of generated; no ESLint, so hook dependencies are unchecked.
- Verified correct: WebSocket backoff (500 ms to 5 s), resync and reconnect reload, build-id reload; query keys consistent; GitHub mutations never retried; optimistic inbox updates with rollback; stale times match the docs; polling stops on unmount; search aborts; Shiki only for the visible file; local datetime conversion correct; image limits match the server; Copy puts the exact Markdown on the clipboard; all 22 virtualizers use `useFlushSync: false`; no `dark:` or `prefers-color-scheme`; zoom math has no `getBoundingClientRect`/`clientX` dependencies.

## 8. AGENTS.md rule compliance

| Rule | Status | Evidence |
|---|---|---|
| Work on `main` only, no other branches or worktrees | Violated (leftover) | Branch `code-viewer` (merged) and worktree `.claude/worktrees/code-viewer` still exist |
| Finished work committed with docs updated | Mostly | Docs drift (section 5) shows doc updates did not keep up with ARCHITECTURE |
| Agent groups: Done = waiting for input, never inferred from silence | Violated in one path | SES-4 (background task end) |
| Groups collapsible, counts visible, collapse persisted, hidden rows not virtualized | Compliant | `AgentsView.tsx:143-153`, `layout.ts:20,56` |
| Sidebar counts and project markers use the same predicates | Compliant | `lib/sessionStatus.ts` matches `model.rs:78-83` |
| Header X only closes the panel; Archive separate | Compliant | `SessionPane.tsx:254-261`, `layout.ts:67` |
| Scheduled never in Running or Done | Mostly | Transient duplicate during launch (SCH-2) |
| Cancelled or missing launches never Done; no automatic retry after a claim | Compliant | `schedules.rs` |
| Launch defaults from the latest successful launch | Compliant, with UI-14 | `composer.ts:59-79` |
| Five zoom positions, 120% default, virtualization at every position | Compliant | `lib/zoom.ts`, `styles.css:38` |
| Light theme only | Compliant | No `dark:` or `prefers-color-scheme` |
| Click-only, only the listed keyboard exceptions | Violated | UI-8 |
| Virtualize every long list, memoized rows, `useFlushSync: false`, fixed heights | Partial | UI-7, UI-9, UI-10, UI-3 |
| Linux only | Compliant | No `target_os` |
| No backward compatibility, simple `state.db` schema changes | Partial | Three tables bypass the `user_version` list; six dead cache tables |
| GitHub content stays local until the owner clicks | Compliant (UI convention) | Agents can still use `gh` directly (SEC info) |
| Notifications: `all=true` is not the inbox; 404 subscription is not unsubscribe | Compliant | `inbox.rs` |
| Never refresh the Claude OAuth token | Compliant | `status.rs` |
| Automated checks must not create real side effects | Partially violated | Leftover temporary folders in `~/projects` (PRJ-1) |
| Tests: minimal, fast, no mocks | Compliant | 11 tests, under a second; the diff parser now deserves one (ITM-2) |

## 9. Performance, storage and scalability

- **PERF-1 (Medium). `state.db` grows without limit.** (verified) Production `state.db` is 68 MB after 25 sessions: `session_raw` 69,895 rows and 46 MB (lines up to 200,000 characters, 20,000 lines per session), `session_event` 3,808 rows and 12.8 MB (`tool_result` 9 MB). At ten sessions a day this is about 25 MB per day. Images are stored inline in events. Nothing is ever pruned. Add retention for raw lines of archived sessions (or move them to compressed files) and store images outside the event JSON.
- **PERF-2 (Medium). GitHub budget and image traffic** from GH-3, GH-5 and ITM-4/5: about 18% of the GraphQL budget with an idle inbox, 240 subscription requests per full pass, repeated image downloads.
- **PERF-3 (Medium). The single state.db thread is a bottleneck**: SRV-2 (task output and search over all events), history import in one large transaction, and every agent's event writes go through it; this also causes SES-6.
- **PERF-4 (Low). History listing** clones every session per candidate and rescans all transcripts per page (`history.rs:140-148,169-174`); the 100-line metadata limit does not bound bytes.
- **PERF-5 (Low). Unbounded caches**: `gh_response` `diff:` rows per head pair, `inbox_preview`, `inbox_active`, posted suggestions (all returned by `GET /api/suggestions`), the images folder (not cleared with `cache.db`), `refs/launchpad/pr/*`.
- **PERF-6 (Low). Front end**: UI-9 to UI-13 and UI-25; streaming messages are re-parsed in full on every delta (no block memoization, contrary to 9.6).

## 10. Code quality and maintainability

- The Rust code is consistent and readable; error handling uses `anyhow` with context; no `unsafe` outside `pre_exec`/`prctl`. Some files mix formatting styles (dense one-line blocks in `inbox.rs` such as `tx.execute(...)?;}tx.commit()?;`), which suggests `cargo fmt` is not run; adding `cargo fmt --check` to CI would keep agent-written code uniform.
- Large files carry several responsibilities: `sessions.rs` (921 lines: registry, state machine, persistence, sink, launch, install guard), `CodeView.tsx` (738), `ItemView.tsx` (752). Splitting the install guard and the sink out of `sessions.rs` would make the locking rules (SES-8) easier to see.
- Global locks: the `installing` mutex and `predraft_lock` serve as general serialization points; they are held across network I/O in several places (SES-8, PD-4, SUG-2). A small "in-flight work" counter plus short critical sections would remove most of the stalls.
- Shared types: most API types are generated with ts-rs, but `UpdateStatus`, the pre-draft `Detail`, `Diff` and GitHub payload types are hand-written and can drift.
- Dead code: six cache tables, `EmptyView`, unused icon symbols, `initial_base`, the Codex Background→Working branch, `launchpad.webp`.
- No ESLint or `eslint-plugin-react-hooks`: several stale-closure and dependency issues (UI-26) would be flagged automatically.
- Test coverage matches the policy. The best candidates for the next small tests are `parse_diff` (ITM-2), the mention parser (UI-4, but there is no frontend test setup), and the Claude/Codex event normalizers fed with recorded lines (SES-1, SES-4), which ARCHITECTURE 5.2 already names.

## 11. Repository and process hygiene

- No git remote: CI has never run, and nothing is backed up off the machine. Objects are loose (12.5 MiB, never packed).
- No backup or export of `state.db`, which holds conversations, pre-draft notes, suggestions and inbox triage state. With no remote either, the code and the data live on one disk. A periodic `sqlite3 .backup` copy would be enough.
- Leftover branch `code-viewer` and worktree `.claude/worktrees/code-viewer` (AGENTS.md says to work on `main` only).
- Leftover temporary folders from dev checks in `~/projects` (PRJ-1); `web/node_modules/@xterm/*` and an empty `/run/user/1000/launchpad-dev/` from the reverted Phase 7.
- `.gitignore` only ignores `/.claude/worktrees/`; a future `.claude/settings.local.json` would appear untracked and block the updater (OPS-1).
- About 4.4 MB of icon binaries (including a 1.4 MB SVG that embeds a PNG) are tracked; acceptable but avoidable.
- No secrets in history; generated folders (`web/dist`, `target`, `desktop/src-tauri/gen`) ignored; `web/src/gen` tracked by design and up to date.
- The production updater is waiting on an untracked review document right now (OPS-1). Either commit, move or ignore such files (`.git/info/exclude`), or fix `update.sh`.

## 12. What went wrong

1. **Features were stacked faster than they were accepted.** Phases 1 to 6 were all built on 2026-10-02 and 2026-10-03, and none has passed its "Done when" check (a week, a morning or two weeks of daily use). Each phase was built on phases that had not been used for real, so lifecycle problems that daily use would have exposed early (a restart during a question, a stuck updater, a failing sync) accumulated across six phases.
2. **Verification covered the happy paths.** The progress notes list many detailed checks, but nearly all are success paths in a fresh dev daemon. Every High finding is a recovery path: after a restart, a failed push, a failed request, a second process, a killed updater. None of those was exercised.
3. **Phase 7 was implemented without a scope.** Three independent extensions (session workers, embedded terminal, remote mode) went into one commit and had to be reverted. The revert is clean, but two small improvements it carried were lost with it.
4. **Global locks and strings stand in for ownership rules.** The install guard and `predraft_lock` became general serialization points and are held across network and git I/O. Exact `cwd` string equality stands for worktree identity, `Waiting` for "nothing runs", a head-only key for diff identity. These shortcuts cause most medium findings (PD-3, SES-8, SES-25, ITM-15, PRJ-2).
5. **Documentation was updated by appending.** Each phase added a progress paragraph and a decision, but the design sections written before Phase 0 (API, schema, modules, frontend stack, presets) were never rewritten, so ARCHITECTURE describes in many places a system that was not built. The ROADMAP status column was not updated to the "built, awaiting acceptance" state used everywhere else.
6. **Automated checks leaked into the owner's environment.** The dev profile shares `~/projects`, so checks left temporary folders there; an untracked review file has blocked the production updater for hours; a merged `code-viewer` branch and its worktree remain.
7. **No remote, no CI run, no backup.** The CI workflow was written but never ran, and code and data live on a single disk.

## 13. Recommended fix order

1. **Unblock and harden the updater** (OPS-1, OPS-4): ignore untracked files, final status in traps, Cancel, reservation expiry.
2. **Daemon single instance** (SRV-1, SCH-1): lock and bind before `App::start`; no direct spawn after a systemd start; reject unknown profiles.
3. **Session recovery** (SES-1, SES-5, SES-2, SES-3, SES-4, SES-25, SES-26, SES-27): clear `open_question` correctly, revert status on send failure, ignore events from detached instances, wait for process exit, keep Background until `result` or until interrupted commands finish, keep questions until the answer is written, one ordered writer per session.
4. **Pre-draft publication** (PD-1, PD-2, PD-12, PD-5, PD-3, PD-13): best-effort upstream step, stricter branch validation, effective push URL check, "Abandon publication", ignored files warning, one freeze/idle policy for every entry path (launch, send, answer, resume, terminal).
5. **Inbox robustness** (GH-1, GH-2, GH-4, GH-5, GH-3): per-thread error tolerance, page-1 reset on failure, a working Refresh, enrich only displayed threads, prefetch only changes.
6. **External apps outside the daemon cgroup** (OPS-2, OPS-3), and count TUI sessions as busy.
7. **Security hardening** (SEC-1, SEC-2, SEC-3, SEC-4, SEC-6): CSP and `frame-ancestors`, drop the 5173 origins, 0700 folders, exact navigation check, safe token creation.
8. **Diff and items** (ITM-2 with a test, ITM-18, ITM-3, ITM-1, ITM-15, ITM-16, ITM-17, GH-6, GH-7, ITM-6, CODE-2).
9. **Project lifecycle** (PRJ-2, PRJ-7, PRJ-8): reserve the folder during cleanup and promotion, refuse removal while local PRs depend on the project, make promotion recoverable.
10. **Locks and database load** (SES-8, PD-4, SRV-2, SES-6, PERF-1): short critical sections, SQL filters, raw line retention.
11. **UI correctness** (UI-1, UI-2, UI-29, UI-30, UI-4, UI-5, UI-31, UI-6, UI-8, UI-3, UI-7) and the remaining low findings.
12. **Documentation**: rewrite ARCHITECTURE 5.1, 6.2, 6.3, 6.4, 9.2, 12 and 13 from the code; fix the preset count, test counts and ROADMAP statuses (section 5).
13. **Process**: add a remote so CI runs, delete the `code-viewer` branch and worktree, remove leftover temporary folders, give the dev profile its own projects root.

## 14. Comparison with the earlier review

The earlier review (`docs/CODE_REVIEW_2026-10-03.md`, 53 active findings: 8 high, 37 medium, 8 low) was read only after sections 1 to 12 were written. It concentrates on execution ownership, ordered durability, worktree reservations and publication safety, and it reproduced several issues in an isolated daemon (duplicate daemon, frozen worktree launch, push races, diff parser, memo versions). This review covers the same core and adds the inbox sync, the updater and packaging, process cgroups, the full documentation drift, UI details, performance and storage.

Its findings that this review had missed were checked against the code and merged into the sections above, marked "from the earlier review": SES-25 (R01), SES-26 (R12), SES-27 (R02), SES-28 (R31), SES-29 (R11), SES-30 (R46), SES-31 (R42), SCH-5, PRJ-7 (R29), PRJ-8 (R30), ITM-15 (R23), ITM-16 (R24), ITM-17 (R25), ITM-18 (R56), PD-12 (R06), PD-13 (R07), CODE-7 (R33), CODE-8 (R44), CODE-9 (R38), CODE-10, SRV-10 (R45), OPS-12 (R34), UI-29 (R53), UI-30 (R36), UI-31 (R37), DOC-32 to DOC-36, plus details added to PD-3 (R04), ITM-2 (R08), UI-2 (R09), CODE-3 (R20), PRJ-2 (R47), SUG-4 (R50) and the security info notes (R17, R21).

### 14.1 Mapping of the earlier findings

| Earlier | Here | Severity earlier → here | Note |
|---|---|---|---|
| R01 | SES-25 | High → Medium | Needs an explicit Stop during a long Codex command |
| R02 | SES-27, SES-9–11 | High → Medium | Same root cause; no crash reproduced |
| R04 | PD-3 | High → Medium | Same paths, plus the CLI-created pre-draft path found here |
| R06 | PD-12 | High → Medium | Needs a `pushurl` or `insteadOf` configuration |
| R07 | PD-13 | High → Low | Needs another process creating the same branch within seconds |
| R08 | ITM-2 | Medium → Medium | Agree; also the ` b/` split and prefix settings |
| R09 | UI-2 | Medium → Medium | Agree |
| R10 | UI-1 | Medium → Medium | Agree |
| R11 | SES-5, SES-29 | Medium → Medium | Agree |
| R12 | SES-26 | Medium → Medium | Agree |
| R13 | SES-15 | Medium → Low | Needs a stale client or a direct API call |
| R14 | SES-6 | Medium → Medium | Agree |
| R17 | SEC-3, SEC-6 | Medium → Low | Parent folders are 0700 on this machine, single user |
| R18 | SEC-4 | Medium → Low | No IPC exposed; phishing only |
| R19 | ITM-14 | Medium → Low | Needs a textconv driver in the owner's git config |
| R20 | CODE-3 | Medium → Low | Needs a concurrent local symlink swap |
| R21 | Section 6 info | Medium → Info | Reachability not established; upstream bump |
| R22 | GH-6 | Medium → Medium | Agree; also the RepoItems crash found here |
| R23 | ITM-15 | Medium → Medium | Agree |
| R24 | ITM-16 | Medium → Medium | Agree |
| R25 | ITM-17 | Medium → Medium | Agree |
| R26 | ITM-4, ITM-5 | Medium → Medium | Agree; signed-URL cache keys found here |
| R27 | GH-5 | Medium → Medium | Agree; GH-1 (sync abort) is the more serious sibling |
| R28 | GH-10 | Medium → Low | Agree on the bug |
| R29 | PRJ-7 | Medium → Medium | Agree |
| R30 | PRJ-8, PRJ-3 | Medium → Medium | Agree |
| R31 | SES-28 | Medium → Medium | Agree |
| R32 | UI-7, UI-9 | Medium → Medium/Low | Agree |
| R33 | CODE-7 | Medium → Low | Owner's own input only |
| R34 | OPS-12 | Medium → Medium | Agree |
| R35 | OPS-1 | Medium → High | Combined with the untracked-file wait, which is blocking the updater now |
| R36 | UI-30 | Medium → Medium | Agree |
| R37 | UI-31, UI-5 | Medium → Medium | Agree |
| R38 | CODE-9 | Medium → Low | Agree on the bug |
| R39 | SRV-4 | Low → Low | Agree |
| R40 | UI-26 | Low → Low | Agree |
| R41 | UI-8 | Low → Medium | Two more violations found here (GitHub link field, Popover Escape) |
| R42 | SES-31 | Low → Low | Agree |
| R43 | OPS-9 | Low → Low | Agree |
| R44 | CODE-8 | Low → Low | Agree |
| R45 | SRV-2, SRV-10, UI-12, SES-8, PD-4 | Medium → Medium | Agree |
| R46 | SES-2, SES-3, SES-30 | High → Medium | Agree on the bugs |
| R47 | PRJ-2 | High → Medium | Agree on the bug |
| R48 | SES-1 | Medium → High | A routine restart triggers it: the updater treats a session with an open question as idle |
| R49 | PD-2 | Medium → High | Concrete triggers confirmed here (`@{-1}`, submodule recursion), plus PD-1 |
| R50 | SUG-4 | Medium → Info | Needs a lost response and a manual retry |
| R51 | SRV-1 | High → High | Agree; reproduced by the earlier review |
| R52 | ITM-7 | Medium → Low | Agree on the bug |
| R53 | UI-29 | Medium → Medium | Agree |
| R54 | DOC-19, section 4.2 | Low → Low | Agree |
| R55 | ITM-6 | Medium → Medium | Agree |
| R56 | ITM-18 | Medium → Medium | Agree |
| R57 | SES-14 | Low → Low | Agree |

### 14.2 Found here and absent from the earlier review

- **High:** OPS-1 (untracked files block the updater; the earlier review considered this wait "appropriate", but it has blocked the production update since 06:22), OPS-2 (daemon-started terminals, browsers, editors and windows die with every restart; the updater ignores terminal sessions), GH-1 (one failing subscription request stops the whole inbox sync), PD-1 (`--set-upstream-to` failure after a successful push in single-branch or shallow clones).
- **Medium:** SEC-1 (CSP and `frame-ancestors`, mentioned there only as a general defense), SEC-2 (dev daemon trusts any page on port 5173), SES-4 (false Done at the end of a Claude background task), SES-7 (invalid replies to Codex server requests), SES-8 (install guard held across fetch, push and Codex RPCs), SCH-1 (non-transactional claim), PRJ-1 (temporary folders left in `~/projects`), GH-2, GH-3, GH-4, GH-7 (inbox page-1 change lost after a failure, prefetch on every poll, Refresh no-op, GitHub error messages discarded), ITM-1, ITM-3, ITM-4 (diff offline failure, no API fallback when no remote matches, signed-URL image keys), SUG-1 (rejected posts locked as uncertain), PD-5 (Discard deletes ignored files), CODE-2 (`git diff` rewrites the index and breaks an agent's `git add`), SRV-2 (session search full scan), SRV-3 (a panicking DB job disables the database), OPS-3 (service ordering before the graphical session), OPS-4 (reservation never expires), UI-3, UI-4, UI-5, UI-6 (virtualizer keys, mention caret corruption, reload losing unsent work, terminal field), PERF-1 (measured `state.db` growth) and the documentation drift DOC-5 to DOC-8 and DOC-13 to DOC-23 (the earlier review states that the documents "agree again", but ARCHITECTURE 5.1, 6.2, 6.3, 6.4, 9.2, 12 and 13 describe a different system).
- **Low and info:** SEC-5, SEC-7 to SEC-13; SES-12, SES-16 to SES-23; SCH-2; PRJ-4, PRJ-5; GH-8, GH-9, GH-11 to GH-14; ITM-8 to ITM-13; SUG-2, SUG-3; PD-6 to PD-10; CODE-1, CODE-4 to CODE-6; SRV-4 to SRV-9; OPS-5 to OPS-11; UI-10 to UI-28; and the repository hygiene items in section 11 (no remote, leftover branch and worktree, `@xterm` in `node_modules`, `.gitignore` coverage, icon sizes, no `state.db` backup).

### 14.3 Points where the two reviews differ in judgment

- **Documentation:** the earlier review checked mainly the Phase 7 wording and concluded that the documents agree. This review finds large drift in ARCHITECTURE outside Phase 7 (section 5).
- **Severity calibration:** the earlier review rates by worst possible consequence; this review also weighs how likely the trigger is for one user on one machine (section 14.1). Both lists are useful: the earlier one for invariants to restore, this one for what to fix first.
- **Updater wait on untracked files:** described there as appropriate; treated here as a defect, because untracked notes, logs or editor files should not block an update, and the block gives no visible reason.
- **Roadmap status:** the earlier review keeps "In progress" for Phases 1–6; this review only asks that ROADMAP, README, AGENTS and ARCHITECTURE use the same wording.

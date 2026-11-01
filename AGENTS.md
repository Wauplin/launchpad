# Launchpad: rules for coding agents

Launchpad is a personal desktop app (Linux first, macOS and Windows supported) that launches and drives coding agents (Claude Code, Codex, OpenCode) and replaces the parts of the GitHub UI used every day, with a local cache. It has one user. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) before changing the design.

## Where things are

- `crates/core`: domain and services. `crates/server`: axum API, WebSocket, auth, embedded UI. `crates/cli`: the `launchpad` binary. `web/`: React app. `desktop/`: Tauri shell. `extension/`: Chrome extension. `deploy/hf-space/`: Hugging Face Space connector. `packaging/`: installers. `skills/launchpad/SKILL.md`: the agent skill for the CLI.
- `docs/`: user documentation by feature, plus `architecture.md`, `development.md`, `harness-adapters.md` and `glossary.md`. `initial.md`: the original project description, in the owner's words.

## Commands

- Build the UI: `cd web && pnpm install && pnpm build` (Node 24 from `web/.nvmrc`, pnpm through Corepack).
- Dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon` (port 7318, data in `~/.local/share/launchpad-dev`). A debug daemon reads `web/dist` from disk. For an isolated second daemon, set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Sign a browser in to the dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- url` (`pair` prints a QR code). Headless screenshots: `google-chrome --headless=new --screenshot=out.png --virtual-time-budget=5000 "<url>"` after pairing; they do not wait for Web Workers, so syntax colors need a real DevTools-driven wait.
- Demo mode: `/agents?demo=1` on any running UI (`?demo=essentials:layout`, `?demo=full:<chapter>`, `?demo=phone`). No pairing or real data needed.
- Dev window: `cargo build -p launchpad-desktop && LAUNCHPAD_ENV=dev ./target/debug/launchpad-desktop [--open <route>] [--workspace <id or name>] [--new-window]`.
- Regenerate TypeScript types after changing a Rust type shared with the UI: `cargo run -p launchpad -- gen-types`.
- Lint (CI): `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings` and `(cd web && pnpm typecheck)`. Install the toolchain with `rustup toolchain install 1.93.1 --profile minimal --component clippy`.
- Tests: `cargo test`, and `(cd web && pnpm test:<name>)` for attachments, code, composer, drafts, markdown, operations, proposals, sessions, terminals and tour (Node 24, all run in CI). Targeted Rust checks: `cargo test -p launchpad-core capacity` (model capacity retries), `cargo test -p launchpad-core trace` (trace backup).
- Chrome extension: `cd extension && pnpm install --frozen-lockfile && pnpm build` after installing `web/` dependencies; see `extension/README.md`.
- Icons: keep `packaging/icons/source.png` unchanged; regenerate exports with `python3 packaging/generate-icons.py`; `packaging/install-icons.sh` updates installed icons without a restart.
- Read-only context CLI for agents: `launchpad folders|sessions|profiles|presets|inbox|schedules|proposals list [--workspace <id or name>]`, `launchpad sessions show <id>`; `launchpad propose …` stores an inert Ask proposal. Local PR CLI: `launchpad pr create|update|show|list`.
- CI (`.github/workflows/ci.yml`): lint and debug tests on main, pull requests and tags; release builds only on tags. Linux releases build on Ubuntu 22.04 (verify a downloaded binary locally before changing that baseline); build the UI before the release daemon so it embeds the assets. `.github/workflows/hf-space.yml` publishes the Space image on main pushes. Keep CI commands and this file in sync.
- Production install/update: `packaging/install.sh` (Linux/macOS, `--headless`, `--prebuilt`) and `packaging/install.ps1` (Windows). They restart the production daemon: only run them when the owner asks.

## Safety rules

- **Never stop or restart the production daemon (port 7317)** unless the owner asks: it runs the agents, maybe including you. Restarting a daemon interrupts its running turns and ends its terminals.
- **Nothing reaches GitHub without an owner click.** Automated checks must not post comments or reviews, request reviewers, change labels, state or draft status, merge real items, publish Local PRs, post real suggestions, or mute real threads. Local unread markers, viewed markers and unsent drafts are safe. Use throwaway local bare remotes for publication checks.
- Agent-written comments and reviews go through `launchpad suggest comment|review` (reviews need `--head <inspected commit>`; Local PRs use `--predraft <id>`). The CLI never publishes.
- Automated checks must close the terminals they open, never type into harness TUIs, pause or delete test crons, cancel test follow-ups and scheduled launches, discard test worktrees (use throwaway repositories), close throwaway browsers and servers, cancel test HF jobs and remove test SSH hosts.
- Never run setup fixes that change the real machine's configuration, never enable trace backup against a real account, and use **Send test notification** only on throwaway Web Push subscriptions.
- Never refresh the Claude OAuth token; only read it. Never log in, refresh tokens or copy credentials to hosts on the owner's behalf.
- Never retry an uncertain operation automatically after a crash (launch/send claims, quota continuations, scheduled launches, Ask proposals, merges, browser actions, GitHub posts). Show the uncertainty and let the owner check.

## Design invariants

### General

- Single user: no backward compatibility, no versioning, no migration framework. `cache.db` can be deleted and rebuilt; keep `state.db` schema changes simple.
- Linux (Ubuntu) is primary; gate OS-specific APIs and keep native release builds and tests in CI.
- Phase 7 extensions need an owner-approved scope. The daemon owns harnesses and terminals; there are no session workers.
- Click only: every action is a visible button. Keyboard shortcuts are limited to `Ctrl+F` in Files, `Ctrl+T`/`Cmd+T` for a new terminal, terminal clipboard and clear keys, normal text-editing keys, `Enter`/`Shift+Enter` in the session message box and the find field, and `@`, `/`, `#` suggestions in prompts.
- Editor integration is removed: no editor buttons, settings or endpoints.
- Presets are user-defined only: keep the config default and first-start template empty.

### Harnesses and sessions

- `crates/core/src/harness/mod.rs` is the registration list and common contract. Each adapter owns native startup/resume, discovery, monitors, environment markers, terminal commands and title hooks. Keep harness identity branches out of core, server and React; UI metadata comes from `GET /api/harnesses`. See `docs/harness-adapters.md`.
- Adapters start processes with `procs::child_command(&app.env.on(spec.host), …)` and the native `spec.cwd`, so they work on SSH hosts unchanged. Treat `exec::is_lost` exits as Interrupted.
- Session states are shared by all harnesses. Done means waiting for input; Running includes starting, working and background commands; never infer Done from a silent command. Stopped is distinct from Interrupted and Failed.
- Needs you = an open question, or Done/Failed/Interrupted and unseen (`session.seen_at < status_since`). `Session::needs_you()` (Rust) and `needsYou`/`isUnseen` (`web/src/lib/sessionStatus.ts`) are the only predicates; every count, badge and list uses them. A session becomes seen only after a focused, visible, recently used window showed it for 1.5 s; notification suppression uses the same rule.
- Subagents share the root session's durable sequence; `Sink::child` must never change root usage, model, identity, status or previews. Include child turns in idle checks and root Stop.
- Codex background commands keep the session Running (Background) until real completion; publication and merge idle checks must include commands that outlive a turn.
- Permission modes (YOLO default, Ask for approval, Auto) use native mechanisms only; never implement Auto by blindly accepting requests. Typed replies cannot grant permissions.
- The initial launch prompt gets `compose::append_initial_summary_instruction`; follow-up messages stay verbatim. Keep the typed text first in `render_prompt`.
- Agent trace context is referenced by `launchpad trace <id> --through <seq>`, never inlined. Missing traces fail visibly.
- Scheduled launches and crons claim before launching (crons advance `next_run_at` first) and never catch up: runs more than one hour late or with unmet conditions are skipped. All start conditions must be met (AND). Quota conditions need a fresh reading. Dependencies use `Session::dependency_done()` / `dependencyDone`.
- Attachments upload once (`upload:<mime>;<sha256>` references); non-image files go to `<data_dir>/attachments/<session-id>/`, never into the project checkout.
- Model capacity retries: only terminal capacity/overload failures, five-minute persisted deadline, at most five per session, cancelled by Resume, input, Stop or Archive.
- Immediate Launch and session input use UUID `Idempotency-Key` headers and durable `browser_operation` claims. Keep authentication before retry middleware.
- Notifications (desktop and Web Push) only for completed turns, failures, unexpected interruptions, automatic quota/cost stops and automatic resumption. Push payloads never contain code or messages.

### Profiles, quota and costs

- Profiles (`[[profiles]]` in config.toml) are the only way to run an agent; there are no built-in profiles. Sessions store the profile ID, never environment values. Missing IDs or changed harnesses fail visibly on resume.
- Each subscription login (harness + `account_env` values) has its own quota monitor; quota checks use the session's login.
- Allow paid usage defaults to false and is never remembered. Exhausted quota holds input or pauses sessions into Quota paused; automatic continuation needs fresh confirmed capacity in both windows. Never infer availability from a reset time alone. A null reset means the window has not started. Owner messages sent during a quota pause are queued in the durable pending continuation (never sent to the harness directly) and delivered in order with the continuation turn; Stop and Archive discard them.
- Cost limits and rolling spend limits apply only to harnesses that report USD cost; reaching one refuses input, interrupts best effort and never resumes automatically. Claude adds only increases of its per-process `total_cost_usd`.
- Terminal TUIs are outside quota protection.
- When the Claude token has expired or the direct usage call fails (except 429), read usage through Claude Code's `get_usage` stream-json control request.

### Folders, worktrees and Local PRs

- Use the glossary words in the UI: Folder (code: project), Local PR (code: predraft), Suggestion, Files (code viewer). See `docs/glossary.md`.
- New drafts in Git folders run in a Launchpad-owned worktree (`<projects_root>/.worktrees/<project-id>/`, branch `lp/<slug>-<session8>`); never use a harness's own worktree feature. Merges wait for idle agents and a clean worktree, rebase, then `git merge --ff-only`. Never stash, reset, clean, force or commit other changes. Conflicts go back to the agent. Take the session action lock before `worktree_lock`, and release both before messaging an agent.
- Cleanup removes only clean worktrees of idle agents and keeps the branch; Resume restores the folder.
- Local PR publication and cleanup require idle agents, a clean worktree and no terminal in that worktree. Merge locally never pushes.
- HTML previews in Files are static, sanitized and sandboxed with no permissions. Keep the highlighting size/line limits shared with the viewer and terminate stalled highlight workers.
- Every `/api/code/*` request goes through `code::open`, which accepts only a registered folder or one of its worktrees. Code API paths use forward slashes.
- Folder remote metadata is derived from local Git configuration; broadcast only when it changes.

### SSH hosts

- A host folder is `<host>:<absolute path>` in every path field. Run commands through `procs::command` and file operations through `hostfs`; never use `std::fs` or `tokio::process` directly on project, session or worktree paths. Re-host Git output paths with `hosts::loc::rehost`.
- Unreachable is a host state, never a session status. Preflight is read-only; only owner clicks run install or sign-in commands, in a visible terminal.
- Test with a throwaway host (local Docker sshd or `hf jobs run --ssh`) and an isolated daemon.

### Remote access and devices

- `[server] bind` defaults to `127.0.0.1`. `public_origin` (explicit) or Tailscale discovery (`auto_tailnet`) defines the exact allowed Host/Origin. Never infer allowed origins or Secure cookies from forwarded headers. Startup discovery is read-only; never adopt Funnel routes or another daemon's port.
- Owner-clicked Tailscale setup runs only a fixed `tailscale serve` root proxy for this daemon; it never runs arbitrary input or overwrites routes.
- Pairing links are single-use and expire after five minutes. Reverse pairing binds approval to a separate tab secret; never exchange the short code alone for a credential. Every paired browser has full access.
- With a remote origin, `[commands]` is file-only.
- Only the desktop shell uses `/api/open` and `/api/window`; browser tabs open links and windows locally.
- Session lists and bus events carry `Session::light()` (no attachment bytes, prompt cut at 4,000 bytes); fetch `GET /api/sessions/{id}` for the full record. Sessions open tail-first.
- The Web Push service worker never caches `/api`, `/ws` or `/auth`, and never registers in the desktop shell.

### GitHub

- Detail and list GETs return cached content; POST refreshes. Label/reviewer pickers read `cache.db` only.
- `all=true` notifications include historical Done threads; keep inbox membership in `state.db` and never promise exact parity with the web inbox. A missing thread subscription (404) is not evidence of unsubscribe.

### Ask Launchpad

- The assistant writes inert proposals only, with a restricted token allowed only `GET /api/context/*`, `GET /api/sessions/{id}/trace`, `GET /api/status`, `GET|POST /api/proposals` and `POST /api/launch/preview`. Every start needs an owner click; paid usage is forced off.
- Assistant threads (`Target::Assistant`) are hidden from normal session lists, counts and remembered launch defaults.

### Terminals

- The daemon owns PTYs; output stays in memory (1 MB each) and goes over one WebSocket per viewer, never the global bus. The PTY size follows the owner viewer. Keep visited xterm views mounted.
- Shells open only in a registered folder, one of its worktrees or a session folder.
- Shell snippets in agent messages run only on an owner click (Run in terminal), never automatically, and never from repository or GitHub Markdown.

### Trace backup and sync

- Off by default. Adapters export native traces unchanged (`Harness::native_trace`); never synthesize native traces from normalized events. Launchpad metadata goes in `launchpad.json` and `launchpad-events-*.jsonl`.
- Never log in, change bucket visibility or delete remote files. Imported traces are read-only replicas: locally owned session IDs always win, and replicas are never re-exported.

### Hugging Face Space

- The control plane talks to the shell only through `/_space/status`, `/_space/connections`, `/_space/login` and `POST /_space/action/<name>`. Updates need an owner click, idle agents and a verified checkpoint; edited Dockerfiles are never overwritten. See `deploy/hf-space/README.md`.
- `~/projects` is a bucket volume (`projects/`); never archive or upload it. Everything else is latest-only under `space/` (A/B slots, one manifest, one pre-update point): never add per-run generations, and the connector never deletes bucket files. Format 1 (`backups/`, `credentials/`) is read once to migrate. Python checks: `cd deploy/hf-space && python -m unittest discover -s tests`.

## UI rules

- Build pages from `web/src/components/ui.tsx`: `PageHeader`, `Tabs`, `FilterChip`, `ConfirmButton` (every destructive or GitHub action; never `window.confirm`), `Loading`/`Empty`/`ErrorState` with `humanError`, `Badge`/`Token`, `SettingRow`. Use semantic color tokens and sprite icons; text sizes 11/12/13/15/18 px. One primary button per region.
- Virtualize every long list, memoize row components, use `useFlushSync: false` and prefer fixed row heights (WebKitGTK performance).
- Keep heavy imports out of `router.tsx`, `main.tsx`, both shells and the sidebar; other routes load lazily.
- The phone shell (`web/src/mobile/`) reuses feature components with `usePhone()`; never fork features. Keep 44 px touch targets. Keep the desktop layout unchanged at every zoom position (100–150%, default 120%, per device).
- Theme, zoom and layout are device-local (browser storage), never in config.toml.
- Workspaces: sessions, schedules, crons and inbox threads belong to a workspace only through their folder or repository. Every narrowed list uses the `useScope()` predicates.
- Demo mode (`web/src/demo/`) answers every request in the browser; never add a demo path that calls the real daemon or writes browser storage. A new major feature updates its tour chapter (`web/src/onboarding/chapters/`), demo fixtures and handlers, and keeps `data-tour` anchors (`pnpm test:tour` checks them).
- Setup checklist: detect first, never redo; fixes run only fixed commands in visible terminals after an owner click; never overwrite an existing Git identity, credential helper, SSH key or harness login, never run package managers automatically.
- The Chrome extension panel is its own small UI: never import the app's Composer, SessionPane, router or phone shell into it; reuse only `lib/` helpers, pure store helpers, generated types and Markdown. Browser grants bind a device, document and URL, expire after 30 idle minutes and end on Stop/Archive. Consequential browser actions are always reviewed. Keep the extension ID/key and the daemon origin allowlist in sync.

## Git workflow

These rules apply unless the owner says otherwise for a given task.

- Work on the `main` branch, in this checkout. Do not create other branches, worktrees or pull requests for this repository.
- When a piece of work is complete, commit it on `main`. Do not leave finished work uncommitted.
- Before committing, update the documentation in the same commit: `README.md` (features, getting started), the matching page in `docs/` when a user-facing feature changes, `docs/architecture.md` when the design changes, and this file for new commands, rules or conventions. Keep documents short: describe behavior, not implementation history.
- Use a short commit message that says what changed.

## Testing

This is a personal project: move fast.

- Keep tests to the minimum. Add a test only for a small, specific part that clearly benefits from it, typically a pure function with tricky edge cases.
- No mocks of HTTP APIs, harness processes, git or the database. No tests that need a server, the network, GitHub, a running agent or the daemon.
- The whole test run must take a few seconds. Delete or rewrite slow or flaky tests.
- Check changes by building and running the app in the dev environment.

## Writing style

- Markdown: never hard-wrap lines. Write each paragraph or list item on one line.
- Use plain technical American English that non-native speakers can read easily. Be concise.

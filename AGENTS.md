# Launchpad: rules for coding agents

Launchpad is a personal desktop app (Linux first, macOS and Windows supported) that launches and drives coding agents (Claude Code, Codex, OpenCode) and replaces the parts of the GitHub UI used every day, with a local cache. It has one user. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) before changing the design.

## Where things are

- `crates/core`: domain and services. `crates/server`: axum API, WebSocket, auth, embedded UI. `crates/cli`: the `launchpad` binary. `web/`: React app. `desktop/`: Tauri shell. `extension/`: Chrome extension. `deploy/hf-space/`: Hugging Face Space connector. `packaging/`: installers. `skills/launchpad/SKILL.md`: the agent skill for the CLI.
- `docs/`: user documentation by feature, plus `architecture.md`, `development.md`, `harness-adapters.md` and `glossary.md`. `initial.md`: the original project description, in the owner's words.

## Commands

- Build the UI: `cd web && pnpm install && pnpm build` (Node 24 from `web/.nvmrc`, pnpm through Corepack).
- Dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon [--role home|runner|all]` (port 7318, data in `~/.local/share/launchpad-dev`; a runner uses `launchpad-runner-dev`). A debug daemon reads `web/dist` from disk. For an isolated second daemon, set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Runners: `launchpad daemon --role runner` (own folders), paired with `launchpad connect <Home address>` and approved in Home's Settings → Home and runners. Test pairs use two scratch `HOME`s and ports; remove test runners afterwards. Contained sessions need `bwrap` (`sudo apt install bubblewrap`) and a harness login in the vault (Settings → Credentials and tools). `packaging/local-vm.sh <Home address>` creates a Local VM runner with Multipass.
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
- Each subscription login has its own quota monitor; quota checks use the session's login. On Linux the logins are Home's vault logins (`quota::vault_key`): Home monitors them and runners receive the readings.
- Allow paid usage defaults to false and is never remembered. Exhausted quota holds input or pauses sessions into Quota paused; automatic continuation needs fresh confirmed capacity in both windows. Never infer availability from a reset time alone. A null reset means the window has not started. Owner messages sent during a quota pause are queued in the durable pending continuation (never sent to the harness directly) and delivered in order with the continuation turn; Stop and Archive discard them.
- Cost limits and rolling spend limits apply only to harnesses that report USD cost; reaching one refuses input, interrupts best effort and never resumes automatically. Claude adds only increases of its per-process `total_cost_usd`.
- Terminal TUIs are outside quota protection.
- When the Claude token has expired or the direct usage call fails (except 429), read usage through Claude Code's `get_usage` stream-json control request.

### Folders, worktrees and Local PRs

- Use the glossary words in the UI: Folder (code: project), Local PR (code: predraft), Suggestion, Files (code viewer). See `docs/glossary.md`.
- Git folders are Home records served by Home Git (`home_git.rs`): a bare mirror per folder (`<data_dir>/git/<id>.git`) that follows the folder's remote for every branch except Launchpad branches (`lp/…`, `lp-scratch/…`), which only runners push (the `pre-receive` hook refuses everything else). Only Home writes base branches, through Merge or Publish. Every previous tip of a Launchpad branch is kept as `refs/kept/<time>/<branch>` for 30 days, with Restore. Runners and agents never hold a Git credential for Home: runner clones use the runner-local endpoint `/git/<id>.git` (per-session tokens in per-worktree config), which forwards to Home's `/api/git/` with the runner credential; Home refuses a session that now runs elsewhere.
- Each session gets fresh worktrees (`repo_cache.rs` clone in `<data_dir>/repos/<id>`, worktrees in `<data_dir>/worktrees/<id>/`, branch `lp/<slug>-<session8>`); never use a harness's own worktree feature, and never work in the owner's checkouts. The launch request only records the worktree (`worktrees::plan`); the background start task (`prepare`) has Home refresh its mirror, fetches and adds the worktree before setup and the harness start. Never put network calls or the harness start in the launch request. **When done** (`WorktreeOptions.when_done`, remembered per folder) is keep, merge or Local PR.
- The runner pushes each worktree's branch to Home at every turn end (`--force-with-lease`, so a Restore on Home is never overwritten) and snapshots running worktrees to `refs/lp-snapshots/<id>` every five minutes. A folder is removed only after Home confirmed the branch; uncommitted files go to `lp-scratch/…` first. Resume clones it again. Take the session action lock before `worktree_lock`, and release both before messaging an agent.
- Merges run on Home (`home_git::merge`): fast-forward of the base branch to the reviewed commit, then a best-effort push to the folder's remote, never forced. A base that moved goes back to the agent for a rebase. Discard deletes the branch on Home; its tip stays restorable.
- Local first, then follow GitHub: **Push to PR** (`WorktreeOptions.pull_request`) has Home fetch `pull/N/head` into the session's branch and follow it (`git_follow`): after each push from the runner, Home pushes the branch to the PR, never forced. Published Local PRs are followed the same way.
- Plain and asset folders (`plain.rs`) are stored by Home: a tree in `state.db`, contents in `<data_dir>/blobs/sha256/` (uploaded by Home backups), every overwrite and delete in `plain_history` for 30 days. One session holds a store's lock; runners sync down before a session starts and up at every turn end and at Archive. Runners call `/api/plain/` with their credential. Never write a store outside `plain::commit`.
- Local PRs live on Home (`predrafts.rs`): a record for a Launchpad branch, reviewed from the mirror. Publish pushes exactly the reviewed commit from the mirror. Local PRs opened by one session form a change set (Publish all links the PRs). A runner forwards every `/api/predrafts` request to Home, except `create`/`resolve` by worktree, which push the branch first.
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

- The assistant writes inert proposals only, with a restricted token allowed only `GET /api/context/*`, `GET /api/sessions/{id}/trace`, `GET /api/status`, `GET|POST /api/proposals` and `POST /api/launch/preview`. Every start needs an owner click; paid usage is forced off. Preset proposals create or update presets in config.toml and never delete them.
- Assistant threads (`Target::Assistant`) are hidden from normal session lists, counts and remembered launch defaults.

### Terminals

- The daemon owns PTYs; output stays in memory (1 MB each) and goes over one WebSocket per viewer, never the global bus. The PTY size follows the owner viewer. Keep visited xterm views mounted.
- Shells open only in a registered folder, one of its worktrees or a session folder.
- Shell snippets in agent messages run only on an owner click (Run in terminal), never automatically, and never from repository or GitHub Markdown.

### Trace backup and sync

- Off by default. Adapters export native traces unchanged (`Harness::native_trace`); never synthesize native traces from normalized events. Launchpad metadata goes in `launchpad.json` and `launchpad-events-*.jsonl`.
- Never log in, change bucket visibility or delete remote files. Imported traces are read-only replicas: locally owned session IDs always win, and replicas are never re-exported.

### Home and runners

- Home (`--role home`) runs no agents or terminals; runners run them and replicate records and events through `home_outbox` (same transaction as their copy, dropped only after Home's `Ack`). Never apply runner items outside `Sessions::apply_replica`/`projects::apply_replica`, and keep the generation check.
- Home's relay serves records, events, images and traces itself and forwards every other session or runner-folder request to its runner. Workspaces, Git folders, Home Git and Local PRs are Home's: runners keep copies of workspaces and Git folders (snapshots over the tunnel) and forward their changes (`home_client::home_owned`); Home accepts only `home_client::home_accepts` paths from a runner. A launch in a Git folder goes to any available runner.
- Home backups add one Git bundle per mirror (Launchpad branches and kept tips minus what the remote has; a full bundle without a remote); a restored Home applies it when it recreates the mirror.
- Home backups: content-addressed objects, the manifest written last, restore only from a complete manifest whose hashes match. Pruning under `home/` is allowed (last day, newest kept); never touch other prefixes. Test backups with a local folder destination, never a real bucket.

### Credentials and the jail

- Every agent on Linux runs contained (`contain::apply` in `Sessions::attach`): a bubblewrap jail, or a Local VM (`[runner] boundary = "vm"`). Never add a full-machine mode, never pass the machine's credentials, logins, Git config or SSH keys into a session, and keep `jail::KEEP_ENV` free of secrets. Title requests and harness TUIs use the session's lease and jail.
- Secrets live only in Home's vault (`vault.rs`) and in the runner proxy's memory. No API returns them; the lease goes only through a runner's own tunnel (`FromRunner::Call`) or in process on an `all` daemon. Agents see `vault::placeholder` values. Never write a real token into a session's files.
- The proxy adds tokens only on hosts of the lease and never judges requests; other traffic is a plain tunnel. Log every request that received a token. Harness-specific login setup belongs to `Harness::contained_login`.
- Leases end on Stop, Archive and Revoke, and after an hour without Home. Home refuses leases of a session that runs elsewhere. Only Home refreshes Codex logins.
- Test contained sessions with scratch daemons and copies of tokens read without refreshing them; never refresh the owner's Claude token, and check a Codex login's `last_refresh` before copying it into a test vault.

### Hugging Face Space

- The control plane talks to the shell only through `/_space/status`, `/_space/connections`, `/_space/login` and `POST /_space/action/<name>`. Updates need an owner click and a verified checkpoint; they stop running agents first and the next boot resumes them once (claimed in `space/update-resumed.json` before resuming, never more than one hour late). Edited Dockerfiles are never overwritten. See `deploy/hf-space/README.md`.
- The Space bucket defaults to `<namespace>/launchpad`, like trace sync elsewhere (bucket `launchpad`, prefix `traces`): `projects/` and `traces/` sit at the root for any machine, Space-only state stays under `space/`. `~/projects` is a bucket volume (`projects/`); never archive or upload it. Everything else is latest-only under `space/` (A/B slots, one manifest, one pre-update point): never add per-run generations, and the connector never deletes bucket files. Format 1 (`backups/`, `credentials/`) is read once to migrate. Python checks: `cd deploy/hf-space && python -m unittest discover -s tests`.

## UI rules

- Build pages from `web/src/components/ui.tsx`: `PageHeader`, `Tabs`, `FilterChip`, `ConfirmButton` (every destructive or GitHub action; never `window.confirm`), `Loading`/`Empty`/`ErrorState` with `humanError`, `Badge`/`Token`, `SettingRow`. Use semantic color tokens and sprite icons; text sizes 11/12/13/15/18 px. One primary button per region.
- Virtualize every long list, memoize row components, use `useFlushSync: false` and prefer fixed row heights (WebKitGTK performance).
- Keep heavy imports out of `router.tsx`, `main.tsx`, both shells and the sidebar; other routes load lazily.
- The phone shell (`web/src/mobile/`) reuses feature components with `usePhone()`; never fork features. Keep 44 px touch targets. Keep the desktop layout unchanged at every zoom position (100–150%, default 120%, per device).
- Theme, zoom and layout are device-local (browser storage), never in config.toml.
- Workspaces: every folder belongs to exactly one workspace (`project.workspace_id`); Everything is a view only. Adding, cloning or creating a folder requires a workspace; a repository in two workspaces is two folders. Sessions, schedules, crons and inbox threads belong to a workspace only through their folder or repository. Every narrowed list uses the `useScope()` predicates.
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

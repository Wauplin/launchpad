# Launchpad: rules for coding agents

Launchpad is a personal desktop app (Linux first, macOS and Windows supported) that launches and drives coding agents (Claude Code, Codex, OpenCode) and replaces the parts of the GitHub UI used every day, with a local cache. It has one user. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) before changing the design.

## Where things are

- `crates/core`: domain and services. `crates/server`: axum API, WebSocket, auth, embedded UI. `crates/cli`: the `launchpad` binary (and the `build.rs` that stamps the commit). `web/`: React app. `desktop/`: Tauri shell. `extension/`: Chrome extension. `deploy/hf-space/`: Hugging Face Space connector. `packaging/`: installers. `skills/launchpad/SKILL.md`: the agent skill for the CLI.
- `docs/`: user documentation by feature, plus `architecture.md`, `development.md`, `harness-adapters.md` and `glossary.md`. `initial.md`: the original project description, in the owner's words.

## Commands

- UI: `cd web && pnpm install && pnpm build` (Node 24 from `web/.nvmrc`, pnpm through Corepack). A debug daemon reads `web/dist` from disk.
- Dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon [--role home|runner|all]` (port 7318, data in `~/.local/share/launchpad-dev`; a runner uses `launchpad-runner-dev`). For an isolated second daemon, set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Sign a browser in to the dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- url` (`pair` prints a QR code). Headless screenshots: `google-chrome --headless=new --screenshot=out.png --virtual-time-budget=5000 "<url>"` after pairing (they do not wait for Web Workers, so syntax colors need a DevTools-driven wait).
- Demo mode: `/agents?demo=1` on any running UI (`?demo=essentials:layout`, `?demo=full:<chapter>`, `?demo=phone`), no pairing or real data needed.
- Dev window: `cargo build -p launchpad-desktop && LAUNCHPAD_ENV=dev ./target/debug/launchpad-desktop [--open <route>] [--workspace <id or name>] [--new-window]`.
- Runners: `launchpad daemon --role runner` (own folders), paired with `launchpad connect <Home address>` and approved in Home's Settings → Home and runners. Test pairs use two scratch `HOME`s and ports; remove test runners afterwards. Contained sessions need `bwrap` and a harness login in the vault. `packaging/local-vm.sh <Home address>` creates a Local VM runner with Multipass.
- Home-only Space POC: see `deploy/hf-home/README.md` (runners use port 7320 and their own folders; never use the production installer for it). Checks: `cd deploy/hf-home && python -m unittest discover -s tests`.
- After changing a Rust type shared with the UI: `cargo run -p launchpad -- gen-types`.
- Lint (CI): `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings`, `(cd web && pnpm typecheck)` and `(cd web && pnpm lint:hooks)` (toolchain: `rustup toolchain install 1.93.1 --profile minimal --component clippy`).
- Tests (CI): `cargo test`, and `(cd web && pnpm test:<name>)` for attachments, code, composer, drafts, markdown, operations, proposals, sessions, terminals and tour (Node 24).
- Chrome extension: `cd extension && pnpm install --frozen-lockfile && pnpm build` after installing `web/` dependencies; see `extension/README.md`.
- Icons: keep `packaging/icons/source.png` unchanged; regenerate with `python3 packaging/generate-icons.py`; `packaging/install-icons.sh` updates installed icons.
- Agent CLI: read-only context `launchpad folders|bots|sessions|profiles|presets|inbox|schedules|proposals list [--workspace …]`, `launchpad sessions show <id>`; `launchpad propose …` (inert proposal); `launchpad api <method> <path> [--body-file]` (the API at admin access); `launchpad pr create|update|show|list` (Local PRs); `launchpad send <channel>|telegram --body-file <file> [--title …] --session-id <id>`.
- CI (`.github/workflows/ci.yml`): lint and debug tests on main, pull requests and tags; release builds only on tags, Linux on Ubuntu 22.04 (verify a downloaded binary before changing that baseline); build the UI before the release daemon so it embeds the assets. `.github/workflows/hf-space.yml` publishes the Space image on main pushes. Keep CI and this file in sync.
- Production install/update: `packaging/install.sh` (Linux/macOS, `--headless`, `--prebuilt`) and `packaging/install.ps1` (Windows). They restart the production daemon: only run them when the owner asks.

## Checks to run (keep iteration fast)

Run only what your change can break; CI runs everything.

- Docs only: nothing.
- Web only: `(cd web && pnpm typecheck)`, `pnpm lint:hooks` if you touched hooks, and the matching `pnpm test:<name>`. Do not run cargo.
- Rust, while working: `cargo check --all-targets` (about 20 s after a change in `core`). Run `cargo test <filter>` only when you changed code that has tests (the whole run takes about one second once built).
- Rust, once before committing: `cargo clippy --all-targets -- -D warnings` (about 1.5 min after a change in `core`), plus `cargo test` if you changed tested code.
- Run cargo from the repository root without `-p`: selecting one crate resolves different dependency features and rebuilds every dependency. Do not run release builds or `cargo clean`.
- Contained sessions share compiled dependencies through `sccache` when it is installed on the runner (`cargo install sccache --locked`).

## Safety rules

- **Never stop or restart the production daemon (port 7317)** unless the owner asks: it runs the agents, maybe including you. Restarting a daemon interrupts its running turns and ends its terminals.
- **Nothing reaches GitHub without an owner click.** Automated checks never post comments or reviews, request reviewers, change labels, state or draft status, merge real items, publish Local PRs, post real suggestions or mute real threads. Local unread and viewed markers and unsent drafts are safe. Use throwaway local bare remotes for publication checks.
- Agent-written comments and reviews go through `launchpad suggest comment|review` (reviews need `--head <inspected commit>`; Local PRs use `--predraft <id>`). The CLI never publishes.
- Automated checks clean up after themselves: close the terminals they open, never type into harness TUIs, pause or delete test routines, cancel test follow-ups, scheduled launches and HF jobs, discard test worktrees (use throwaway repositories), close throwaway browsers and servers, remove test SSH hosts.
- Never run setup fixes that change the real machine's configuration, and never enable trace backup against a real account.
- Never refresh the Claude OAuth token; only read it. Never log in, refresh tokens or copy credentials to hosts on the owner's behalf.
- Never retry an uncertain operation automatically after a crash (launch/send claims, quota continuations, scheduled launches, Ask proposals, merges, browser actions, GitHub posts). Show the uncertainty and let the owner check.

## Design invariants

### General

- Single user: no backward compatibility, versioning or migration framework. `cache.db` can be deleted and rebuilt; keep `state.db` schema changes simple.
- Linux (Ubuntu) is primary; gate OS-specific APIs and keep native release builds and tests in CI.
- Phase 7 extensions need an owner-approved scope. The daemon owns harnesses and terminals; there are no session workers.
- Click only: every action is a visible button. Keyboard shortcuts are limited to `Ctrl+F` in Files, `Ctrl+T`/`Cmd+T` for a new terminal, terminal clipboard and clear keys, normal text-editing keys, `Enter`/`Shift+Enter` in the session message box and the find field, and `@`, `/`, `#` suggestions in prompts.
- No editor integration (buttons, settings or endpoints).
- Presets are user-defined only: keep the config default and first-start template empty.

### Harnesses and sessions

- `crates/core/src/harness/mod.rs` is the registration list and common contract; each adapter owns startup/resume, discovery, monitors, markers, terminal commands and title hooks. No harness identity branches in core, server or React (UI metadata: `GET /api/harnesses`). See `docs/harness-adapters.md`.
- Adapters start processes with `procs::child_command(&app.env.on(spec.host), …)` and the native `spec.cwd` (works on SSH hosts). `exec::is_lost` exits are Interrupted.
- Shared session states: Done means waiting for input; Running includes starting, working and background commands (including Codex background commands until real completion); never infer Done from a silent command. Stopped differs from Interrupted and Failed. Idle checks for publication, merge and root Stop include child turns and commands that outlive a turn.
- Needs you = an open question, or Done/Failed/Interrupted and unseen. `Session::needs_you()` and `needsYou`/`isUnseen` (`web/src/lib/sessionStatus.ts`) are the only predicates. A session becomes seen only after a focused, visible, recently used window showed it for 1.5 s (notification suppression uses the same rule).
- `Sink::child` (subagents) never changes root usage, model, identity, status or previews.
- Permission modes (YOLO default, Ask for approval, Auto) use native mechanisms only; never implement Auto by accepting requests blindly. Typed replies cannot grant permissions.
- Only the initial prompt gets `compose::append_initial_summary_instruction`; follow-ups stay verbatim, typed text first in `render_prompt`. Trace context is referenced (`launchpad trace <id> --through <seq>`), never inlined; missing traces fail visibly.
- Schedules and routines (code: crons) claim before launching (advance `next_run_at` or the GitHub watermark, write the history entry first) and never catch up: runs over one hour late or with unmet conditions (all must hold; quota needs a fresh reading; dependencies use `dependency_done`) are skipped. Claimed but not started at shutdown becomes Failed, never retried.
- Routine triggers: schedule, new Inbox activity (one run per check), another agent's turn ending Done; a routine's own runs never trigger routines. Bot routines message the bot's thread (`crons::bot_message`) and skip while it works; other routines launch their spec. Pausing a bot pauses its routines and hooks, never the owner's messages. A final `no_update` message (`crons::is_no_update`) never notifies and is marked seen only if nothing was unseen when the run started.
- Attachments upload once (`upload:<mime>;<sha256>`); non-image files go to `<data_dir>/attachments/<session-id>/`, never into the checkout.
- Capacity retries: terminal capacity/overload failures only, persisted five-minute deadline, at most five per session, cancelled by Resume, input, Stop or Archive.
- Launch and session input use `Idempotency-Key` headers and durable `browser_operation` claims; keep authentication before retry middleware.
- Notifications (desktop, Telegram) only for completed turns, failures, unexpected interruptions, automatic quota/cost stops and automatic resumption, never with code or messages. No Web Push.

### Profiles, quota and costs

- Profiles (`[[profiles]]` in config.toml) are the only way to run an agent; none are built in. Sessions store the profile ID, never environment values; missing IDs or changed harnesses fail visibly on resume.
- One quota monitor per subscription login, checks use the session's login. On Linux the logins are Home's vault logins (`quota::vault_key`); runners receive the readings. When the Claude token expired or the usage call fails (except 429), use Claude Code's `get_usage` control request.
- Allow paid usage defaults to false and is never remembered. Exhausted quota holds input or pauses into Quota paused; continuing needs fresh confirmed capacity in both windows, never a reset time alone (null reset = window not started). Owner messages during a pause wait in the durable pending continuation and are delivered in order with it; Stop and Archive discard them.
- Cost and rolling spend limits apply only to harnesses reporting USD; reaching one refuses input, interrupts best effort and never resumes automatically. Claude adds only increases of its per-process `total_cost_usd`. Terminal TUIs are outside quota protection.

### Folders, worktrees and Local PRs

The Git and storage design is in `docs/architecture.md`; these rules must hold.

- Use the glossary words in the UI: Folder (code: project), Local PR (code: predraft), Suggestion, Files (code viewer). See `docs/glossary.md`.
- Home Git (`home_git.rs`): a bare mirror per Git folder that follows the remote, except Launchpad branches (`lp/…`, `lp-scratch/…`), which only runners push (`pre-receive` refuses the rest). Only Home writes base branches, through Merge or Publish. Previous tips stay 30 days under `refs/kept/` with Restore. Runners and agents never hold a Git credential for Home (runner-local `/git/<id>.git` with per-session tokens); Home refuses a session that now runs elsewhere.
- Temporary folders (`Target::Temp`) live in `<data_dir>/tmp/`, registered `is_temp`; `projects::after_archive` deletes one when its last session is archived. Keep as folder (`projects::promote`) moves and renames it into the folders root; never keep temporary folders by default.
- Each session gets fresh worktrees (`<data_dir>/worktrees/<id>/`, branch `lp/<slug>-<session8>`); never use a harness's own worktree feature or the owner's checkouts. The launch request only records the worktree (`worktrees::plan`); network calls, setup and the harness start happen in the background start task (`prepare`). **When done** is keep, merge or Local PR.
- Runners push each branch to Home at every turn end (`--force-with-lease`, so a Restore is never overwritten) and snapshot running worktrees every five minutes. A folder is removed only after Home confirmed the branch; uncommitted files go to `lp-scratch/…` first. Take the session action lock before `worktree_lock`, and release both before messaging an agent.
- Merges run on Home (`home_git::merge`): push the reviewed commit to the remote base branch (never forced), then fast-forward Home's; a refused push merges nothing and stays visible; a moved base goes back to the agent for a rebase. Push to PR and published Local PRs follow GitHub (`git_follow`), never forced. Publish pushes exactly the reviewed commit; one session's Local PRs form a change set.
- Plain and asset folders (`plain.rs`): stored by Home, history 30 days in `plain_history`, one session holds a store's lock; never write a store outside `plain::commit`. Unused asset folders go to the trash after 14 days (`plain::clean_assets`), with Restore.
- Files reads Git folders from a browse copy (`home_git::browse_copy`), never the bare mirror. Every `/api/code/*` request goes through `code::open` (registered folders and their worktrees only); code API paths use forward slashes. HTML previews are static, sanitized and sandboxed with no permissions; keep highlighting limits shared with the viewer and terminate stalled highlight workers.
- Folder remote metadata is derived from local Git configuration; broadcast only when it changes.

### SSH hosts

- A host folder is `<host>:<absolute path>` in every path field. Run commands through `procs::command` and file operations through `hostfs`; never `std::fs` or `tokio::process` directly on project, session or worktree paths. Re-host Git output paths with `hosts::loc::rehost`.
- Unreachable is a host state, never a session status. Preflight is read-only; only owner clicks run install or sign-in commands, in a visible terminal.
- Test with a throwaway host (local Docker sshd or `hf jobs run --ssh`) and an isolated daemon.

### Remote access and devices

- `[server] bind` defaults to `127.0.0.1`. `public_origin` (explicit) or Tailscale discovery (`auto_tailnet`) defines the exact allowed Host/Origin. Never infer allowed origins or Secure cookies from forwarded headers. Startup discovery is read-only; never adopt Funnel routes or another daemon's port.
- Owner-clicked Tailscale setup runs only a fixed `tailscale serve` root proxy for this daemon; it never runs arbitrary input or overwrites routes.
- Pairing links are single-use and expire after five minutes. Reverse pairing binds approval to a separate tab secret; never exchange the short code alone for a credential. Every paired browser has full access.
- With a remote origin, `[commands]` is file-only.
- Only the desktop shell uses `/api/open` and `/api/window`; browser tabs open links and windows locally.
- Session lists and bus events carry `Session::light()` (no attachment bytes, prompt cut at 4,000 bytes); fetch `GET /api/sessions/{id}` for the full record. Sessions open tail-first.

### GitHub

- Detail and list GETs return cached content; POST refreshes. Label/reviewer pickers read `cache.db` only.
- `all=true` notifications include historical Done threads; keep inbox membership in `state.db` and never promise exact parity with the web inbox. A missing thread subscription (404) is not evidence of unsubscribe.

### Bots

- A bot is files on Home (`BOT.md`, `bot.toml`, store `bot-<id>`) plus one thread: the sessions whose `LaunchSpec.bot` names it. `bot.toml` is the only source of its settings (profile, model, folder, permission mode, proposal mode, Launchpad CLI access); no profile means the first subscription profile (`bots::profile_of`). Only the owner and sessions with admin Launchpad CLI access write either file. Its folder is optional: without one, each session runs in a temporary folder and adds worktrees with `launchpad work add-project`.
- A new session of the thread starts only on the first message, a profile or folder change, an archived current session or the owner's New session click; it references the previous trace (`launchpad trace`), never inlines it. Harness compaction handles a full context.
- Bot sessions are normal sessions (counted once, in Agents); the sidebar shows only a state dot per bot. Their notes go in `bots/<slug>/` of the knowledge base. Creating a bot (form or proposal) starts nothing; bot proposals never auto-start.

### Channels and the Outbox

- Channels live on Home (`channels.rs`, `telegram.rs`, `hooks.rs`); runners forward `/api/channels`, `/api/outbox` and `/api/suggestions`. Webhook URLs and the Telegram token are `channel-…` vault entries: never listed, lent or returned by an API.
- Messages to other people (webhooks, GitHub suggestions) wait in the Outbox for an owner click; a send is claimed (`sending`) first, never retried, and `uncertain` if incomplete. Automated checks never send real Outbox messages or call Telegram with a real token.
- The owner's own Telegram chat (bound with a single-use code; other chats ignored) gets messages directly. Updates are claimed (offset saved) before handling. Telegram never enables paid usage.
- Incoming hooks (`POST /hooks/<id>`) need `X-Launchpad-Hook-Secret` (SHA-256 stored, constant-time compare), one message per 10 seconds, an HF token on a private Space; the body is a fenced, size-limited block marked as outside data.

### Ask Launchpad and proposals

- Ask Launchpad is a normal bot with no special code path: Home creates it once per workspace (`ask_bot::seed`, recorded in the `seeded` table) with its own plain folder; the owner may edit, replace or delete it, and Home never recreates it. Never add Ask-only branches to sessions, crons, hooks, proposals or the UI.
- The Launchpad CLI is a tool of Home's registry (`vault::LAUNCHPAD_TOOL`), added once and granted to every workspace, with three access levels (`vault::LaunchpadAccess`): **read** (`/api/status`, `/api/context/*`, `GET /api/proposals`), **propose** (plus `POST /api/proposals`; the default for bots and every other agent) and **admin** (plus Launchpad's own records through `launchpad api`: bots, routines, schedules, workspaces, proposal accept/dismiss, launches and session messages, MCP tool switches). Bots set it with `launchpad` in `bot.toml`; Ask Launchpad is created with admin. Home puts the level in the lease (`Lease.launchpad`); `agent_auth` enforces it on the session CLI token, and `proposals::cli_reference` tells the session what it may do. A session without a lease on its machine gets propose. Admin never reaches GitHub, the Outbox, channels, credentials, devices, terminals, hosts, runners, setup fixes, backups, Burst, config.toml or paid usage (`agent_auth::admin_allows`); keep that list closed.
- Proposals are inert. Every start needs an owner click, except one-time launches, messages, schedules and presets of a bot whose `bot.toml` says `proposals = "auto"`, and what an admin session does directly; proposed crons, routines, bots, tools and BOT.md changes always wait. Paid usage is forced off. A session proposes routines and BOT.md changes only for bots of its own workspace. Preset proposals create or update presets and never delete them.

### Terminals

- The daemon owns PTYs; output stays in memory (1 MB each) and goes over one WebSocket per viewer, never the global bus. The PTY size follows the owner viewer. Keep visited xterm views mounted.
- Shells open only in a registered folder, one of its worktrees or a session folder.
- Shell snippets in agent messages run only on an owner click (Run in terminal), never automatically, and never from repository or GitHub Markdown.

### Trace backup and sync

- Off by default. Adapters export native traces unchanged (`Harness::native_trace`); never synthesize native traces from normalized events. Launchpad metadata goes in `launchpad.json` and `launchpad-events-*.jsonl`.
- Never log in, change bucket visibility or delete remote files. Imported traces are read-only replicas: locally owned session IDs always win, and replicas are never re-exported.

### Home and runners

The replication, relay, move and Burst design is in `docs/architecture.md`; these rules must hold.

- Home (`--role home`) runs no agents or terminals. Runners replicate through `home_outbox` (same transaction, dropped only after Home's `Ack`); apply runner items only through `Sessions::apply_replica`/`projects::apply_replica`, and keep the generation check.
- Workspaces, Git folders, Home Git and Local PRs are Home's: runners keep copies and forward changes (`home_client::home_owned`); Home accepts only `home_client::home_accepts` paths from a runner, and only runner-authenticated paths in tunnel exchanges (`exchange::home`). The tunnel protocol is the same whichever side dialed (`server::wire`).
- Seen and unread are recorded on Home. Archive of a session whose runner is away closes the record on Home (`runners::archive_away`) and is delivered once when the runner reconnects.
- Runner updates (`self_update.rs`) start only on an owner click, wait until no session of the runner is running, verify Home's build (SHA-256, OS, architecture) and restart in place. Test them only with scratch daemons and copies of the binary, never on the owner's runners.
- Moves (`handoff.rs`) happen only between turns: Home increases the generation and names the new runner first; the old runner's final handoff pushes branches and saves leftovers to `lp-scratch/…`. Runners remove an archived session's folder only after Home confirmed the unchanged native files (and trace backup uploaded them when it is on); a start restores them from Home. Contained harnesses keep no prompt history (`Harness::contained_history`). Adapters own `native_trace` and `restore_native`; never synthesize a conversation from events.
- Home backups: content-addressed objects, manifest written last, restore only from a complete manifest with matching hashes. Prune only under `home/`. Test with a local folder destination, never a real bucket.
- Burst (`burst.rs`): the runner secret is derived, never stored, and a Burst lease carries no secret (`token_relay`). One job per session; stop it only after the final handoff and a newer complete Home backup. Starts claimed before a Home restart are cancelled, never retried. Respect `[home.burst]`. Test with a scratch Home and scratch bucket (`LAUNCHPAD_BURST_IMAGE`, `_COMMAND`, `_VOLUMES`) and cancel every test job.

### Knowledge base

- The knowledge base (`kb.rs`, store `kb-<workspace id>`) is the only agent memory. Only Home writes it, through `kb::apply` (one writer at a time, author and line counts in `plain_history.meta`); never commit a `kb-` store with `plain::commit`. Writes name the content they started from; a changed base keeps both versions (`.concurrent-<id>` sibling), never a silent overwrite.
- Every contained session gets its workspace's mirror read-only (`$LAUNCHPAD_KB`) and the harness's own memory off through `Harness::contained_memory`. Agents write only with `launchpad kb` (session CLI token, `/api/kb`); Home takes the author from its own session record, never from the runner.
- The gardener is a normal launch the owner starts (or repeats as a routine) from the Knowledge page; memory import reads a runner's harness memories only on an owner click.

### Credentials and the jail

- Every agent on Linux runs contained (`contain::apply`): a bubblewrap jail or a Local VM (`[runner] boundary = "vm"`). No full-machine mode; never pass the machine's credentials, logins, Git config or SSH keys into a session; keep `jail::KEEP_ENV` free of secrets. Title requests and harness TUIs use the session's lease and jail.
- Secrets live only in Home's vault (`vault.rs`) and the runner proxy's memory; no API returns them, and agents see `vault::placeholder` values only. The lease travels only over a runner's own tunnel or in process. Never write a real token into a session's files.
- The proxy adds tokens only on the lease's hosts, never judges requests, and logs every request that received a token. Login setup belongs to `Harness::contained_login`, MCP config to `Harness::contained_mcp` (pointing at the runner proxy; the gateway checks the lease, hides tools that are off and logs each `tools/call`).
- Leases end on Stop, Archive and Revoke, and after an hour without Home; Home refuses leases of a session that runs elsewhere. Only Home refreshes Codex logins.
- Test contained sessions with scratch daemons and token copies read without refreshing; check a Codex login's `last_refresh` before copying it into a test vault.

### Hugging Face Space

- The control plane talks to the shell only through `/_space/status`, `/_space/connections`, `/_space/login` and `POST /_space/action/<name>`. Updates need an owner click and a verified checkpoint; they stop running agents and the next boot resumes them once (claimed before resuming, never more than one hour late). Edited Dockerfiles are never overwritten. See `deploy/hf-space/README.md`.
- Bucket `<namespace>/launchpad`: `projects/` (a volume: never archive or upload it) and `traces/` at the root, Space state latest-only under `space/` (no per-run generations). The connector never deletes bucket files. Checks: `cd deploy/hf-space && python -m unittest discover -s tests`.

## UI rules

- Build pages from `web/src/components/ui.tsx`: `PageHeader`, `Tabs`, `FilterChip`, `ConfirmButton` (every destructive or GitHub action; never `window.confirm`), `Loading`/`Empty`/`ErrorState` with `humanError`, `Badge`/`Token`, `SettingRow`. Use semantic color tokens and sprite icons; text sizes 11/12/13/15/18 px. One primary button per region.
- Virtualize every long list, memoize row components, use `useFlushSync: false` and prefer fixed row heights (WebKitGTK performance).
- Keep heavy imports out of `router.tsx`, `main.tsx`, both shells and the sidebar; other routes load lazily.
- The phone shell (`web/src/mobile/`) reuses feature components with `usePhone()`; never fork features. Keep 44 px touch targets. Keep the desktop layout unchanged at every zoom position (100–150%, default 120%, per device).
- Theme, zoom and layout are device-local (browser storage), never in config.toml.
- Workspaces: every folder belongs to exactly one workspace (`project.workspace_id`); Everything is a view only. Adding, cloning or creating a folder requires a workspace; a repository in two workspaces is two folders. Sessions, schedules, crons and inbox threads belong to a workspace only through their folder or repository. Every narrowed list uses the `useScope()` predicates.
- Demo mode (`web/src/demo/`) answers every request in the browser; never call the real daemon or write browser storage from it. A new major feature updates its tour chapter (`web/src/onboarding/chapters/`), demo fixtures and handlers, and keeps `data-tour` anchors (`pnpm test:tour` checks them).
- Setup checklist: detect first, never redo; fixes run only fixed commands in visible terminals after an owner click; never overwrite an existing Git identity, credential helper, SSH key or harness login, never run package managers automatically.
- The Chrome extension panel is its own small UI: never import the app's Composer, SessionPane, router or phone shell; reuse only `lib/` helpers, pure store helpers, generated types and Markdown. Browser grants bind a device, document and URL, expire after 30 idle minutes and end on Stop/Archive. Consequential browser actions are always reviewed. Keep the extension ID/key and the daemon origin allowlist in sync.

## Git workflow

These rules apply unless the owner says otherwise for a given task.

- Work on `main`, in this checkout. Do not create other branches, worktrees or pull requests for this repository.
- Commit finished work on `main` with a short message that says what changed.
- Update the documentation in the same commit: `README.md` (features, getting started), the matching `docs/` page when a user-facing feature changes, `docs/architecture.md` when the design changes, and this file for new commands, rules or conventions. Keep documents short: describe behavior, not implementation history.

## Testing

This is a personal project: move fast.

- Add a test only for a small, specific part that clearly benefits from it, typically a pure function with tricky edge cases.
- No mocks of HTTP APIs, harness processes, git or the database. No tests that need a server, the network, GitHub, a running agent, the daemon or a full `App`.
- The whole test run must take a few seconds; no test may take more than a second. Delete slow or flaky tests.
- Check changes by building and running the app in the dev environment.

## Writing style

- Markdown: never hard-wrap lines. Write each paragraph or list item on one line.
- Use plain technical American English that non-native speakers can read easily. Be concise.

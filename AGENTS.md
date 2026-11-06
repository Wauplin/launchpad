# Launchpad: rules for coding agents

Launchpad is a personal desktop app (Linux first, macOS and Windows supported) that launches and drives coding agents (Claude Code, Codex, OpenCode) and replaces the parts of the GitHub UI used every day, with a local cache. It has one user. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) before changing the design.

## Where things are

- Rust crates (layout and dependencies in [docs/architecture.md](docs/architecture.md#code-layout)): `crates/core` (domain and services, the harness contract, the action list `actions.rs` and the decision list `today.rs`), `crates/harness` (adapters), `crates/channels` (channels, Telegram, hooks), `crates/github` (issue and PR views, Inbox, Suggestions), `crates/automation` (routines, scheduled launches, proposals, the activity list `activity.rs`), `crates/setup` (setup checklist, clone, onboarding), `crates/http` (API error type and route state, session token checks), `crates/api` (JSON routes that need only the `App`), `crates/server` (auth, runner tunnel and relay, WebSockets, embedded UI, activity recording `activity.rs`) and `crates/cli` (the `launchpad` binary, `extensions.rs` that connects the crates, and the `build.rs` that stamps the commit).
- `web/`: React app (feature folders in `web/src/features/`, for example `chat/` for the bot chat and the Ask panel `AskPanel.tsx`; the top-bar field is `web/src/app/CommandField.tsx`). `desktop/`: Tauri shell. `extension/`: Chrome extension. `deploy/hf-space/`: Hugging Face Space connector. `packaging/`: installers. `skills/launchpad/SKILL.md`: the agent skill for the CLI.
- `.claude/skills/implement-review-docs/`: the skill for plans with several steps (implementer, reviewer and docs subagents per step) and `prose-check.py` (proselint and reading ease of added Markdown lines).
- `docs/`: user documentation by feature, plus `architecture.md`, `development.md`, `harness-adapters.md` and `glossary.md`. `initial.md`: the original project description, in the owner's words.

## Commands

- UI: `cd web && pnpm install && pnpm build` (Node 24 from `web/.nvmrc`, pnpm through Corepack). A debug daemon reads `web/dist` from disk.
- Dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon [--role home|runner|all]` (port 7318, data in `~/.local/share/launchpad-dev`; a runner uses `launchpad-runner-dev`). For an isolated second daemon, set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Sign a browser in to the dev daemon: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- url` (`pair` prints a QR code).
- Screenshots: `node web/scripts/shots.mjs [--demo] --views 100,120,150,phone [--out <dir>] <page>...` (pages `/path` or `name=/path`; clicks with `--pages <file.json>`). It serves this checkout's `web/dist` (rebuilt when stale) and forwards the API to the dev daemon, signs in once by itself, and reports sideways scrolling and page errors. Never write another screenshot script or start a server for it. See `docs/development.md#screenshots`.
- Demo mode: `/?demo=1` on any running UI (`?demo=essentials:layout`, `?demo=full:<chapter>`, `?demo=phone`), no pairing or real data needed.
- Dev window: `cargo build -p launchpad-desktop && LAUNCHPAD_ENV=dev ./target/debug/launchpad-desktop [--open <route>] [--workspace <id or name>] [--new-window]`.
- Runners: `launchpad daemon --role runner` (own folders), paired with `launchpad connect <Home address>` and approved in Home's Settings → Home and runners. Test pairs use two scratch `HOME`s and ports; remove test runners afterwards. Contained sessions need `bwrap` and a harness login in the vault. `packaging/local-vm.sh <Home address>` creates a Local VM runner with Multipass.
- Home-only Space POC: see `deploy/hf-home/README.md`. `deploy/hf-home/deploy.sh [--release <tag> | --source [<ref>]] [--space <name>]` installs or updates a Space with the hf CLI (latest release by default); test it only with a throwaway `--space` and delete that Space and its bucket afterwards (runners use port 7320 and their own folders; never use the production installer for it). Checks: `cd deploy/hf-home && python -m unittest discover -s tests`.
- After changing a Rust type shared with the UI: `cargo run -p launchpad -- gen-types`.
- Lint (CI): `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings`, `(cd web && pnpm typecheck)` and `(cd web && pnpm lint:hooks)` (toolchain: `rustup toolchain install 1.93.1 --profile minimal --component clippy`).
- Tests (CI): `cargo test`, and `(cd web && pnpm test:<name>)` for attachments, chat, code, composer, drafts, markdown, operations, proposals, sessions, terminals and tour (Node 24).
- Chrome extension (on hold: not built or checked in CI, not shipped in release archives; do not change or fix it until the owner resumes it): `cd extension && pnpm install --frozen-lockfile && pnpm build` after installing `web/` dependencies; see `extension/README.md`.
- Icons: keep `packaging/icons/source.png` unchanged; regenerate with `python3 packaging/generate-icons.py`; `packaging/install-icons.sh` updates installed icons.
- Network capture: `launchpad netcap --self-test` checks the supervisor on this machine; `launchpad netcap --port <capture port> --key <proxy key> -- <program>` runs a program under it; Settings → Home and runners → **Check network** probes a runner's sandbox.
- Agent CLI: read-only context `launchpad folders|bots|sessions|profiles|presets|inbox|schedules|proposals list [--workspace …]`, `launchpad sessions show <id>`, `launchpad today`, `launchpad activity list`; `launchpad propose …` (inert proposal; `propose ready` prepares an owner click); `launchpad do <group>.<verb> <ids…> [--<field> <value>] [--body-file <file>]` and the short verb `launchpad <group> <verb> …` (the admin actions of the action list); `launchpad api <method> <path> [--body-file]` (the API at admin access); `launchpad pr create|update|show|list` (Local PRs); `launchpad send <channel>|telegram --body-file <file> [--title …] --session-id <id>`.
- CI (`.github/workflows/ci.yml`): lint and debug tests on main, pull requests and tags; Linux only (no macOS or Windows jobs for now); release builds only on tags, on Ubuntu 22.04 (verify a downloaded binary before changing that baseline); build the UI before the release daemon so it embeds the assets. The release build is `.github/workflows/build.yml`, called by `ci.yml` on tags and by `.github/workflows/release.yml` (manual: tags the commit `vYYYY.MM.DD.N`, UTC, and creates the GitHub release with the package). `.github/workflows/hf-space.yml` publishes the Space image on main pushes. Keep CI and this file in sync.
- Production install/update: `packaging/install.sh` (Linux/macOS, `--headless`, `--prebuilt`) and `packaging/install.ps1` (Windows). They restart the production daemon: only run them when the owner asks.

## Checks to run (keep iteration fast)

Run only what your change can break, and rebuild only what you changed; CI runs everything.

- Docs only: nothing.
- Web only: `(cd web && pnpm typecheck)`, `pnpm lint:hooks` if you touched hooks, and the matching `pnpm test:<name>`. Do not run cargo.
- Rust: Cargo rebuilds only the crate you changed and the crates that depend on it. After a change in a feature crate, `api` or `server`, `cargo check --all-targets` takes 1–2 s, Clippy 2–4 s and `cargo test` 3–5 s. After a change in `core`, every crate rebuilds: about 9 s for `cargo check` and 20 s for Clippy or `cargo test`. Keep code out of `core` when the core does not call it.
- Rust, while working: `cargo check --all-targets`. Run `cargo test <filter>` only when you changed code that has tests; tests sit next to the code in each crate.
- Rust, once before committing: `cargo clippy --all-targets -- -D warnings`, plus `cargo test` if you changed tested code.
- Run checks and tests from the repository root without `-p`, even for one crate: selecting one crate resolves different dependency features and recompiles dozens of dependencies. Do not run release builds or `cargo clean`.
- Skip builds your change cannot affect: `gen-types` only when a type shared with the UI changed, `pnpm build` only when a running daemon must show a `web/` change, the desktop shell only for `desktop/` changes, the extension only for `extension/` changes.
- Contained sessions share compiled dependencies through `sccache` when it is installed on the runner (`cargo install sccache --locked`).

## Safety rules

- **Never stop or restart the production daemon (port 7317)** unless the owner asks: it runs the agents, maybe including you. Restarting a daemon interrupts its running turns and ends its terminals.
- **Nothing reaches GitHub without an owner click.** Automated checks never post comments or reviews, request reviewers, change labels, state or draft status, merge real items, publish Local PRs, post real suggestions or mute real threads. Local unread and viewed markers and unsent drafts are safe. Use throwaway local bare remotes for publication checks.
- Agent-written comments and reviews go through `launchpad suggest comment|review` (reviews need `--head <inspected commit>`; Local PRs use `--predraft <id>`). The CLI never publishes.
- Automated checks clean up after themselves: close the terminals they open, never type into harness TUIs, pause or delete test routines, cancel test follow-ups, scheduled launches and HF jobs, discard test worktrees (use throwaway repositories), close throwaway browsers and servers.
- Never run setup fixes that change the real machine's configuration, and never write trace copies or backups to a real bucket (use a local folder destination).
- Never refresh the Claude OAuth token; only read it. Never log in, refresh tokens or copy credentials to hosts on the owner's behalf.
- Never retry an uncertain operation automatically after a crash (launch/send claims, quota continuations, scheduled launches, Ask proposals, merges, browser actions, GitHub posts). Show the uncertainty and let the owner check.

## Design invariants

### General

- Single user: no backward compatibility, versioning or migration framework. `cache.db` can be deleted and rebuilt; keep `state.db` schema changes simple.
- Linux (Ubuntu) is primary; gate OS-specific APIs. CI currently runs tests and release builds on Linux only.
- Crate layering: `launchpad-core` depends on no other Launchpad crate; the feature crates (`harness`, `channels`, `github`, `automation`, `setup`) depend only on the core, never on each other; `server` never depends on `api` or a feature crate. The core reaches the crates above it only through `harness::install` and `ext::Extensions`, both installed by `crates/cli`. Put new code in the highest crate that can hold it; move code into the core only when the core calls it.
- Phase 7 extensions need an owner-approved scope. The daemon owns harnesses and terminals; there are no session workers.
- Click only: every action is a visible button. Keyboard shortcuts are limited to `Ctrl+F` in Files, `Ctrl+T`/`Cmd+T` for a new terminal, terminal clipboard and clear keys, normal text-editing keys, `Enter`/`Shift+Enter` in the session message box, the top-bar field (Enter sends with the primary button, Shift+Enter adds a line) and the find field, and `@`, `/`, `#` suggestions in prompts.
- No editor integration (buttons, settings or endpoints).
- Presets are user-defined only: keep the config default and first-start template empty.

### Harnesses and sessions

- `crates/core/src/harness/mod.rs` is the common contract and `crates/harness/src/lib.rs` the registration list; each adapter owns startup/resume, discovery, monitors, markers, terminal commands and title hooks. No harness identity branches in core, server or React (UI metadata: `GET /api/harnesses`). See `docs/harness-adapters.md`.
- Adapters start processes with `procs::child_command(&app.env, …)` and `spec.cwd`. `exec::is_lost` exits are Interrupted.
- Shared session states: Done means waiting for input; Running includes starting, working and background commands (including Codex background commands until real completion); never infer Done from a silent command. Stopped differs from Interrupted and Failed. Idle checks for publication, merge and root Stop include child turns and commands that outlive a turn.
- Needs you = an open question, or Done/Failed/Interrupted and unseen. `Session::needs_you()` and `needsYou`/`isUnseen` (`web/src/lib/sessionStatus.ts`) are the only predicates. A session becomes seen only after a focused, visible, recently used window showed it for 1.5 s (notification suppression uses the same rule); a bot page counts as showing its current session.
- Today is the decision list: Home computes it once for every window (`today::compute`, the core's items plus `ext::Extensions::today` for the feature crates), serves it at `GET /api/today` (Home-owned; runners forward it) and announces changes with the `today`/`changed` bus event; session updates recompute only when the set of sessions that need you changed. Every attention count (sidebar, rail, phone, window title, app badge) reads only this list; agents join it through `Session::needs_you()`. Each item names its main action from the action list. An item whose place resolves to no workspace is not listed (repositories without a folder never show); only shared things show in every workspace (outdated runners, tool or preset proposals without a workspace, Outbox messages without a session). A Merge from a Today card needs the session Done, its runner present and a safe branch; the card stays until the merge completed.
- `Sink::child` (subagents) never changes root usage, model, identity, status or previews.
- Permission modes (YOLO default, Ask for approval, Auto) use native mechanisms only; never implement Auto by accepting requests blindly. Typed replies cannot grant permissions.
- Only the initial prompt gets `compose::append_initial_summary_instruction`; follow-ups stay verbatim, typed text first in `render_prompt`. Trace context is referenced (`launchpad trace <id> --through <seq>`), never inlined; missing traces fail visibly.
- Schedules and routines (code: crons) claim before launching (advance `next_run_at` or the GitHub watermark, write the history entry first) and never catch up: runs over one hour late or with unmet conditions (all must hold; quota needs a fresh reading; dependencies use `dependency_done`) are skipped. Claimed but not started at shutdown becomes Failed, never retried.
- Routine triggers: schedule, new Inbox activity (one run per check), another agent's turn ending Done; a routine's own runs never trigger routines. Bot routines message the bot's thread (`crons::bot_message`) and skip while it works; other routines launch their spec. Pausing a bot pauses its routines and hooks, never the owner's messages. A final `no_update` message (`routines::is_no_update`) never notifies and is marked seen only if nothing was unseen when the run started.
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
- Temporary folders (`Target::Temp`) live in `<data_dir>/tmp/`, registered `is_temp`, with their files in Home's store `tmp-<folder id>` (synced like a plain folder, same folder ID on every runner). `projects::after_archive` removes the local copy and the record (hidden from the UI) when its last session is archived and Home holds every file; Resume recreates it from Home; Home drops the store 30 days after the last Archive (`plain::clean_temp`). Keep as folder (`projects::promote`) moves and renames it into the folders root; never keep temporary folders by default.
- Each session gets fresh worktrees (`<data_dir>/worktrees/<id>/`, branch `lp/<slug>-<session8>`); never use a harness's own worktree feature or the owner's checkouts. The launch request only records the worktree (`worktrees::plan`); network calls, setup and the harness start happen in the background start task (`prepare`). **When done** is keep, merge or Local PR.
- Runners push each branch to Home at every turn end (`--force-with-lease`, so a Restore is never overwritten) and snapshot running worktrees every five minutes. A folder is removed only after Home confirmed the branch; uncommitted files go to `lp-scratch/…` first. Take the session action lock before `worktree_lock`, and release both before messaging an agent.
- Merges run on Home (`home_git::merge`): push the reviewed commit to the remote base branch (never forced), then fast-forward Home's; a refused push merges nothing and stays visible; a moved base goes back to the agent for a rebase. Push to PR and published Local PRs follow GitHub (`git_follow`), never forced. Publish pushes exactly the reviewed commit; one session's Local PRs form a change set.
- Plain and asset folders (`plain.rs`): stored by Home, history 30 days in `plain_history`, one session holds a store's lock; never write a store outside `plain::commit`. Unused asset folders go to the trash after 14 days (`plain::clean_assets`), with Restore.
- Files reads Git folders from a browse copy (`home_git::browse_copy`), never the bare mirror. Every `/api/code/*` request goes through `code::open` (registered folders and their worktrees only); code API paths use forward slashes. HTML previews are static, sanitized and sandboxed with no permissions; keep highlighting limits shared with the viewer and terminate stalled highlight workers.
- Folder remote metadata is derived from local Git configuration; broadcast only when it changes.

### Other machines

- Agents run on this computer or on runners; there are no SSH hosts. Another machine joins as a runner that pairs itself with Home (`launchpad connect`), never through a connection Home opens into it.

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
- A new session of the thread starts only on the first message, a profile or folder change, an archived current session or the owner's New session checkbox; it references the previous trace (`launchpad trace`), never inlines it. Harness compaction handles a full context.
- The bot page is a chat over the thread, built from session events by `chatRows` (`web/src/features/chat/`) with no Ask-only code; the full trace stays in Agents (Open trace).
- Bot sessions are normal sessions (counted once, in Today); the sidebar shows only a state dot per bot. Their notes go in `bots/<slug>/` of the knowledge base. Creating a bot (form or proposal) starts nothing; bot proposals never auto-start.

### Channels and the Outbox

- Channels live on Home (`crates/channels`: `channels.rs`, `telegram.rs`, `hooks.rs`); runners forward `/api/channels`, `/api/outbox` and `/api/suggestions`. Webhook URLs and the Telegram token are `channel-…` vault entries: never listed, lent or returned by an API.
- Messages to other people (webhooks, GitHub suggestions) wait in the Outbox for an owner click (shown in Today → Send and Today → GitHub; "Outbox" stays the concept and code name, there is no Outbox page); a send is claimed (`sending`) first, never retried, and `uncertain` if incomplete. Automated checks never send real Outbox messages or call Telegram with a real token.
- The owner's own Telegram chat (bound with a single-use code; other chats ignored) gets messages directly. Updates are claimed (offset saved) before handling. Telegram never enables paid usage. Notifications of finished, failed and interrupted sessions link to their Today card (`/?card=session:<id>`); every link carries `ws=` of its workspace. No buttons in Telegram.
- Incoming hooks (`POST /hooks/<id>`) need `X-Launchpad-Hook-Secret` (SHA-256 stored, constant-time compare), one message per 10 seconds, an HF token on a private Space; the body is a fenced, size-limited block marked as outside data.

### Ask Launchpad and proposals

- Ask Launchpad is a normal bot with no special code path: Home creates it once per workspace (`ask_bot::seed`, recorded in the `seeded` table) with its own plain folder; the owner may edit, replace or delete it, and Home never recreates it. Never add Ask-only branches to sessions, crons, hooks, proposals or the UI. The top-bar Ask and the Ask panel send to `askTarget` (`web/src/lib/bots.ts`: the oldest admin bot of the workspace, else its oldest bot) or the bot the owner picks.
- The Launchpad CLI is a tool of Home's registry (`vault::LAUNCHPAD_TOOL`), added once and granted to every workspace, with three access levels (`vault::LaunchpadAccess`): **read** (`/api/status`, `/api/context/*`, `GET /api/proposals`), **propose** (plus `POST /api/proposals`; the default for bots and every other agent) and **admin** (plus Launchpad's own records: exactly the `admin` actions of the action list, `crates/core/src/actions.rs`, through `launchpad do`, short verbs or `launchpad api`: bots, routines, schedules, workspaces, folder settings (`folder.set`, never the setup command), shared settings (`settings.set`), proposal start and dismiss, launches and session messages, MCP tool switches; GETs outside a closed list; it never reads `GET /api/config`). `agent_auth::admin_allows` matches routes exactly (no trailing slash, no empty segment), and an admin request body may hold only the action's input fields. Accepting a card runs the action its kind maps to (`actions::for_card`), so an admin session may accept a card only when that action is an admin action; tool cards wait for the owner. Home records each direct admin action in the activity list (`launchpad_automation::activity`) with the values before and its undo call; the bot chat shows it as a Changed line with Undo. Bots set the level with `launchpad` in `bot.toml`; Ask Launchpad is created with admin. Home puts the level in the lease (`Lease.launchpad`); `agent_auth` enforces it on the session CLI token, and `launchpad_cli::cli_reference` tells the session what it may do. A session without a lease on its machine gets propose. Admin never reaches GitHub, the Outbox, channels, credentials, devices, terminals, hosts, runners, setup fixes, backups, Burst, config.toml (except the closed shared-settings list of `settings.set` through `/api/settings/shared`: presets, notifications, GitHub poll interval, worktrees kept), folder setup commands or paid usage (`agent_auth::admin_allows`); keep that list closed. Every config.toml write, the owner's or an agent's, goes through `ConfigHandle::update` (read, change and save under one lock).
- Proposals are inert. Every start needs an owner click, except one-time launches, messages and schedules of a bot whose `bot.toml` says `proposals = "auto"`, and what an admin session does directly; proposed crons, routines, presets, bots, tools and BOT.md changes always wait (an admin session may accept preset cards). Ready cards (`ProposalBody::Ready`, actions with `ready: true`: merge, publish, post, send, run CI, Inbox done) prepare an owner action: they run only from their own button, with the action's call and confirmation, then `POST /api/proposals/{id}/ran`; accepting never runs them, and Start all and auto mode skip them. Paid usage is forced off. A session proposes routines and BOT.md changes only for bots of its own workspace. Preset proposals create or update presets and never delete them.

### Terminals

- The daemon owns PTYs; output stays in memory (1 MB each) and goes over one WebSocket per viewer, never the global bus. The PTY size follows the owner viewer. Keep visited xterm views mounted.
- Shells open only in a registered folder, one of its worktrees or a session folder.
- Shell snippets in agent messages run only on an owner click (Run in terminal), never automatically, and never from repository or GitHub Markdown.

### Trace copies

- Home copies each session's native files from its latest handoff, unchanged, after each backup (`trace_copies.rs`): `<workspace>/traces/<folder>/<YYYYMMDD_HHMM>_<id>/`, one file per trace plus `launchpad.json`. Adapters export native traces unchanged (`Harness::native_trace`); never synthesize native traces from normalized events. Home never reads the copies back.
- There is no peer trace sync between instances. Never log in or change bucket visibility; only a renamed workspace or folder deletes the old copies.

### Home and runners

The replication, relay, move and Burst design is in `docs/architecture.md`; these rules must hold.

- Home (`--role home`) runs no agents or terminals. Runners replicate through `home_outbox` (same transaction, dropped only after Home's `Ack`); apply runner items only through `Sessions::apply_replica`/`projects::apply_replica`, and keep the generation check.
- Workspaces, Git folders, Home Git and Local PRs are Home's: runners keep copies and forward changes (`home_client::home_owned`); Home accepts only `home_client::home_accepts` paths from a runner, and only runner-authenticated paths in tunnel exchanges (`exchange::home`). The tunnel protocol is the same whichever side dialed (`server::wire`).
- Seen and unread are recorded on Home. Archive of a session whose runner is away closes the record on Home (`runners::archive_away`) and is delivered once when the runner reconnects.
- Runner updates (`self_update.rs`) start only on an owner click, wait until no session of the runner is running, verify Home's build (SHA-256, OS, architecture) and restart in place. Test them only with scratch daemons and copies of the binary, never on the owner's runners.
- Moves (`handoff.rs`) happen only between turns: Home increases the generation and names the new runner first; the old runner's final handoff pushes branches and saves leftovers to `lp-scratch/…`. Runners remove an archived session's folder only after Home confirmed the unchanged native files; a start restores them from Home. Contained harnesses keep no prompt history (`Harness::contained_history`), never at the cost of the native conversation. Home keeps handoff files in its blob store, never in `state.db`. Adapters own `native_trace` and `restore_native`; never synthesize a conversation from events.
- Home backups: content-addressed objects, manifest written last, restore only from a complete manifest with matching hashes. Prune only under `home/`. Test with a local folder destination, never a real bucket.
- Burst (`burst.rs`): the runner secret is derived, never stored, and a Burst lease carries no secret (`token_relay`). One job per session; stop it only after the final handoff and a newer complete Home backup. Starts claimed before a Home restart are cancelled, never retried. Respect `[home.burst]`. Test with a scratch Home and scratch bucket (`LAUNCHPAD_BURST_IMAGE`, `_COMMAND`, `_VOLUMES`) and cancel every test job.

### Knowledge base

- The knowledge base (`kb.rs`, store `kb-<workspace id>`) is the only agent memory. Only Home writes it, through `kb::apply` (one writer at a time, author and line counts in `plain_history.meta`); never commit a `kb-` store with `plain::commit`. Writes name the content they started from; a changed base keeps both versions (`.concurrent-<id>` sibling), never a silent overwrite.
- Every contained session gets its workspace's mirror read-only (`$LAUNCHPAD_KB`) and the harness's own memory off through `Harness::contained_memory`. Agents write only with `launchpad kb` (session CLI token, `/api/kb`); Home takes the author from its own session record, never from the runner.
- Memory stays small: every instruction that tells agents about the knowledge base (`kb::instructions`, the seeded `AGENTS.md`, `bots::instruction`, Ask Launchpad's `BOT.md`, `skills/launchpad/SKILL.md`) asks for a few durable findings and what the owner asks to remember, never a log of sessions, runs or results.
- The gardener is a normal launch the owner starts (or repeats as a routine) from the Knowledge page; memory import reads a runner's harness memories only on an owner click.

### Credentials and the jail

- Every agent on Linux runs contained (`contain::apply`): a bubblewrap jail or a Local VM (`[runner] boundary = "vm"`). No full-machine mode; never pass the machine's credentials, logins, Git config or SSH keys into a session; keep `jail::KEEP_ENV` free of secrets. Title requests and harness TUIs use the session's lease and jail.
- Network capture (`netcap.rs`, supervisor in `crates/cli/src/netcap.rs`): every contained process runs under `launchpad netcap` through `Jail::command`, in the jail, on a Local VM and on Burst (`Jail.bwrap` false there). An agent must reach the same addresses with the same credentials on all three; runners also reach their LAN and tailnet, Burst connections leave from Home to public addresses only (`token_relay::tcp`). Keep the policy in `netcap` (loopback direct, UDP to port 53 only, same IPv4/IPv6 rules); never fix tools one by one. A machine where `netcap::check` fails runs no agents. No shared agent image or toolchain uniformity; Docker stays VM-only (its containers are outside the capture).
- Secrets live only in Home's vault (`vault.rs`) and the runner proxy's memory; no API returns them, and agents see `vault::placeholder` values only. The lease travels only over a runner's own tunnel or in process. Never write a real token into a session's files.
- The proxy adds tokens only on the lease's hosts, never judges requests, and logs every request that received a token. Login setup belongs to `Harness::contained_login`, MCP config to `Harness::contained_mcp` (pointing at the runner proxy; the gateway checks the lease, hides tools that are off and logs each `tools/call`).
- Default tools (`tool_defaults.rs`): a grant decides where a tool or MCP server may be used; the default tools (workspace, then folder, then bot `[tools]`, most specific wins, on when unset) and the launch's `tools_off`/`tools_on` decide whether a session gets it. Home resolves them only in `LeaseRequest::off`; never turn on an item that is not granted.
- Leases end on Stop, Archive and Revoke, and after an hour without Home; Home refuses leases of a session that runs elsewhere. Only Home refreshes Codex logins.
- Test contained sessions with scratch daemons and token copies read without refreshing; check a Codex login's `last_refresh` before copying it into a test vault.

### Hugging Face Space

- The control plane talks to the shell only through `/_space/status`, `/_space/connections`, `/_space/login` and `POST /_space/action/<name>`. Updates need an owner click and a verified checkpoint; they stop running agents and the next boot resumes them once (claimed before resuming, never more than one hour late). Edited Dockerfiles are never overwritten. See `deploy/hf-space/README.md`.
- Bucket `<namespace>/launchpad`: `projects/` (a volume: never archive or upload it) and `traces/` at the root, Space state latest-only under `space/` (no per-run generations). The connector never deletes bucket files. Checks: `cd deploy/hf-space && python -m unittest discover -s tests`.

## UI rules

- Every action a button, a card, the CLI or the admin rules name is one entry of the action list (`crates/core/src/actions.rs`: route, input, confirmation, effect, access, card kind, undo). Add new actions there; buttons use `ActionButton` (it confirms when the action has a confirmation) or `runAction` (`web/src/lib/actions.ts`); run `gen-types` after changing the list.
- Build pages from `web/src/components/ui.tsx`: `PageHeader`, `Tabs`, `FilterChip`, `ConfirmButton` (every destructive or GitHub action; never `window.confirm`), `Loading`/`Empty`/`ErrorState` with `humanError`, `Badge`/`Token`, `SettingRow`. Use semantic color tokens and sprite icons; text sizes 11/12/13/15/18 px. One primary button per region.
- One visible action per list row or card; the other row actions use `.lp-row-more` (`web/src/styles.css`): shown on hover and keyboard focus, kept while their menu or confirmation is open, always visible on touch screens.
- Page explanations go in `PageHeader intro` or `SettingsPage intro`: shown on the first visit on this device (`launchpad.intros-seen` in browser storage, never config.toml; the demo keeps them in memory and never writes it), then behind ⓘ next to the title. Keep the meta line for facts.
- Virtualize every long list, memoize row components, use `useFlushSync: false` and prefer fixed row heights (WebKitGTK performance).
- Keep heavy imports out of `router.tsx`, `main.tsx`, both shells and the sidebar; other routes load lazily.
- The phone shell (`web/src/mobile/`) reuses feature components with `usePhone()`; never fork features. Keep 44 px touch targets. Keep the desktop layout unchanged at every zoom position (100–150%, default 120%, per device).
- Phone tabs: Today, Agents, Ask (the chat of the `askTarget` bot, full screen, no back button), Inbox and More (`/more`: the other pages with their state, and the workspace). The workspace sheet lists only workspaces with their counts. Phone Settings is a list in three groups (This device, On the phone, On a computer; the last shows status only). The phone session page has a bottom bar with Reply and the main action, or Merge into the base branch when it is safe. The phone status button warns about connection, Home, runners, hosts, harnesses used by a profile, quota, spending and the GitHub rate limit, and its sheet says why.
- Theme, zoom and layout are device-local (browser storage), never in config.toml.
- Workspaces: every folder belongs to exactly one workspace (`project.workspace_id`). A window shows exactly one workspace, never all of them: the rail (laptop) and the workspace sheet (phone) show other workspaces as counts only. There is no default workspace: the owner names at least one in the setup's Workspaces step (adding or cloning a folder waits for it); Home refuses to delete the last one; runners never create workspaces and wait for Home's. Adding, cloning or creating a folder requires a workspace; a repository in two workspaces is two folders. Sessions, schedules, crons and inbox threads belong to a workspace only through their folder or repository; repositories without a folder belong to no workspace (Settings → GitHub lists them). Every narrowed list, count, `#`/`@` suggestion and message-context picker uses the `useScope()` predicates, except Today, which uses the workspaces Home resolved for each item with the same rules. Settings has two parts: **This workspace** (`/settings/workspace`) and **Launchpad** (shared). Notifications name their workspace and open it.
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

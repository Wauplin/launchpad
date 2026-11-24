# Launchpad: rules for coding agents

Launchpad is a personal web app (a PWA; the daemon and runners on Linux) with one user. It launches and drives coding agents (Claude Code, Codex, OpenCode) on runners and replaces the parts of the GitHub UI used every day. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) before changing the design.

**Keep this file short** (about 15 KB at most): every session loads it. New design invariants go into their area's page in [docs/invariants/](docs/invariants/README.md), not here. Add a line here only for a rule that every change must respect, in one sentence.

## Where things are

- `crates/` (layout in [docs/architecture.md](docs/architecture.md#code-layout)): `core` (domain, harness contract, action list `actions.rs`, Today `today.rs`, exit routes `egress.rs`), `harness` (adapters), `channels` (connections, Telegram, hooks, bot channels), `github` (Suggestions, ledger, extension context, notes), `automation` (routines, schedules, proposals, activity, oracles, folios), `setup`, `http`, `api` (JSON routes), `server` (auth, runner tunnel, WebSockets, embedded UI), `cli` (the `launchpad` binary, `extensions.rs` connects the crates).
- `web/`: React app (`web/src/features/<feature>/`, shared UI in `web/src/components/ui.tsx`, phone shell `web/src/mobile/`, demo `web/src/demo/`). `extension/`: Chrome extension. `desktop/`: Linux desktop companion (own Cargo workspace). `deploy/hf-home/`: Home on a Hugging Face Space. `packaging/`: installers. `skills/launchpad/SKILL.md`: the agent skill for the CLI.
- `docs/`: user documentation by feature, plus `architecture.md`, `development.md`, `harness-adapters.md`, `glossary.md` and `invariants/`. `.claude/skills/implement-review-docs/`: the skill for plans with several steps.

## Commands

Full list with every constraint: [docs/development.md](docs/development.md#commands-in-detail).

- UI: `cd web && pnpm install && pnpm build` (Node 24, pnpm through Corepack). A debug daemon reads `web/dist` from disk.
- Dev Home: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon` (port 7318, runs no agents). Dev runner: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon --role runner` (port 7321). Pair once with `LAUNCHPAD_ENV=dev cargo run -p launchpad -- connect http://127.0.0.1:7318` and approve in Settings → Home and runners. Sign a browser in: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- url`. Isolated daemon: scratch `HOME` and a free `LAUNCHPAD_PORT`.
- Screenshots: only `node web/scripts/shots.mjs [--demo] --views 100,120,150,phone [--out <dir>] <page>...` (agent sessions pass `--daemon http://127.0.0.1:7318`); never write another screenshot script or start a server for it. Demo mode: `/?demo=1` on any running UI.
- After changing a Rust type shared with the UI, or the action list: `cargo run -p launchpad -- gen-types`.
- Lint (CI): `cargo +1.98.0 clippy --workspace --all-targets --locked -- -D warnings`, `(cd web && pnpm typecheck)`, `(cd web && pnpm lint:hooks)`.
- Tests (CI): `cargo test`, `(cd web && pnpm test:<name>)` (attachments, chat, code, composer, drafts, folios, markdown, operations, proposals, review, sessions, terminals, tour, usage), and `cd deploy/hf-home && uv run -q --no-project --with-requirements requirements.txt python -m unittest discover -s tests`.
- Extension: `cd extension && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test:suggestions && pnpm build`. Desktop: `cargo check|clippy --all-targets -- -D warnings|test --manifest-path desktop/Cargo.toml` (needs GTK 4 dev files; for `desktop/` changes run only these).
- CI is `.github/workflows/ci.yml` (Linux only), release builds `build.yml`, releases `release.yml`; keep CI and the docs in sync.
- Production install and update (`packaging/install.sh`) restart the production daemon: only when the owner asks.

## Checks to run (keep iteration fast)

Run only what your change can break; CI runs everything.

- Docs only: nothing.
- Web only: `pnpm typecheck`, `pnpm lint:hooks` if you touched hooks, `pnpm ui-check` after UI changes, the matching `pnpm test:<name>`. No cargo.
- Rust while working: `cargo check --all-targets`; once before committing: `cargo clippy --all-targets -- -D warnings`, plus `cargo test` if you changed tested code. A change in `core` rebuilds every crate (about 20 s), so keep code out of `core` when the core does not call it.
- Run from the repository root without `-p`. No release builds, no `cargo clean`. Skip builds your change cannot affect.
- Agents cannot use `sudo`. Runner tools are listed in `deploy/hf-home/agent-tools.sh`, `deploy/hf-home/install-runner.sh` and `packaging/local-vm.sh`: keep the three in sync with CI's Rust and pnpm versions.

## Safety rules

- **Never stop or restart the production Home (port 7317) or runner (port 7320)** unless the owner asks: the runner runs the agents, maybe including you.
- **Nothing reaches GitHub without an owner click.** Exceptions: the pull request of the owner's own launch with When done = Draft PR or Open PR, a bot's draft publication of its own Local PR, and an agent's own CI approval (`launchpad ci approve`, checked on Home). Automated checks never post, review, label, merge, publish or change GitHub items. Agents write comments and reviews only through `launchpad suggest comment|review|action`; the CLI never publishes. Use throwaway local bare remotes for publication checks.
- Automated checks clean up: close terminals, pause or delete test routines, cancel test schedules, follow-ups and HF jobs, discard test worktrees, close browsers and servers. Never type into harness TUIs.
- Never send real Outbox messages, call Telegram with a real token, post to a real Slack, or create, update or delete a real Space or bucket in checks. Use a scratch Home, a throwaway `--space`, local folder destinations and local fakes.
- Never change the real machine's configuration, refresh the Claude OAuth token, log in or copy credentials on the owner's behalf, or run a real agent CLI, runner or Home update in checks.
- Never retry an uncertain operation automatically after a crash (launches, sends, merges, GitHub posts, scheduled runs): show the uncertainty and let the owner check.

## Cross-cutting invariants

Each line is a rule every change must keep. The detail of each area is in its page; read it before changing that area.

- Single user: no backward compatibility, versioning or migration framework; removed config keys keep loading.
- Linux first; agents and terminals run only on Linux runners (a machine, a Local VM or Burst). Home runs no agents; a runner holds no records of its own; there is no single-process mode.
- Crate layering: `core` depends on no other Launchpad crate; feature crates depend only on `core`; `server` never depends on `api` or a feature crate. Put code in the highest crate that can hold it.
- No harness identity branches in core, server or React: adapters own harness behavior ([harness-adapters.md](docs/harness-adapters.md)).
- Every action is one entry of the action list (`crates/core/src/actions.rs`) and a visible button; no gestures or shortcuts beyond the listed ones ([general](docs/invariants/general.md)).
- Today (`today::compute`) is the only source of every attention count; `Session::needs_you()` and `Session::group()` are the only state predicates.
- Workspaces are a boundary for agents: an agent reads and changes only its own workspace's records, checked on Home.
- Home stamps every sender, author and access level from its own records, never from a request body or a runner.
- Proposals and ready cards are inert until the owner clicks; only the owner grants permissions, and an agent only makes modes stricter.
- Automatic work needs fresh confirmed quota; paid usage is off by default and never remembered.
- Secrets live only in Home's vault and the runner proxy's memory; agents run contained and never see a real token.
- GitHub is read live and never cached; Launchpad has no GitHub item pages, Inbox or poller.
- Claim before acting, never retry an uncertain claim, and never catch up on missed schedules.

## Where each area's invariants live

Before changing an area, read its page (index: [docs/invariants/README.md](docs/invariants/README.md)):

- General rules (layering, click only, presets): [docs/invariants/general.md](docs/invariants/general.md)
- Harnesses, sessions, states, Today, schedules, routines, sleeping, attachments, notifications: [docs/invariants/sessions.md](docs/invariants/sessions.md)
- Profiles, quota and costs: [docs/invariants/quota.md](docs/invariants/quota.md)
- References (links and `lp:` IDs): [docs/invariants/references.md](docs/invariants/references.md)
- Topics: [docs/invariants/topics.md](docs/invariants/topics.md)
- Folders, Home Git, worktrees, Local PRs, merges, plain folders, Files: [docs/invariants/folders.md](docs/invariants/folders.md)
- Remote access and devices (origins, pairing, extension and desktop auth): [docs/invariants/remote-access.md](docs/invariants/remote-access.md)
- GitHub (live reads, extension context, Push to PR, attribution, ledger): [docs/invariants/github.md](docs/invariants/github.md)
- Bots, threads, routing, conversations, senders, Chats: [docs/invariants/bots.md](docs/invariants/bots.md)
- Bot channels and DMs: [docs/invariants/channels.md](docs/invariants/channels.md)
- Connections, Outbox, Telegram, hooks: [docs/invariants/connections.md](docs/invariants/connections.md)
- Ask Launchpad, Launchpad CLI access levels, proposals: [docs/invariants/proposals.md](docs/invariants/proposals.md)
- Oracles and routine gates: [docs/invariants/oracles.md](docs/invariants/oracles.md)
- Folios: [docs/invariants/folios.md](docs/invariants/folios.md)
- Desktop companion: [docs/invariants/desktop.md](docs/invariants/desktop.md)
- Terminals: [docs/invariants/terminals.md](docs/invariants/terminals.md)
- Home and runners (replication, placement, moves, updates, backups, Burst, exit routes, trace copies): [docs/invariants/home-and-runners.md](docs/invariants/home-and-runners.md)
- Knowledge base and bot memory: [docs/invariants/knowledge.md](docs/invariants/knowledge.md)
- Credentials, the jail, daemon Git, network capture, vault, Slack, leases: [docs/invariants/jail.md](docs/invariants/jail.md)
- UI rules in full: [docs/invariants/ui.md](docs/invariants/ui.md); Chrome extension: [docs/invariants/extension.md](docs/invariants/extension.md)

## UI rules

Full rules: [docs/invariants/ui.md](docs/invariants/ui.md).

- Build pages from `web/src/components/ui.tsx` (`PageHeader`, `Tabs`, `ConfirmButton` for destructive or GitHub actions, never `window.confirm`; `Loading`/`Empty`/`ErrorState`, `StatusBadge`, `Count`, `RowActions`, `IconButton`). Buttons run actions through `ActionButton` or `runAction`.
- Sizes are tokens checked by `pnpm ui-check`: text 11/12/13/15/18 px as `text-[Npx]`, corners `rounded-control`/`rounded-card`, spacing-scale gaps, semantic colors only, `max-w-page`/`max-w-text`.
- A row or card shows at most one visible button (its main action); the rest goes in ⋯. One filled button per region.
- States use `StatusBadge` colors: blue running, amber needs you, green Done, red failed or interrupted, grey idle.
- Dates only through `web/src/lib/format.ts` (`ago`, `age`, `when`…); never `toLocaleString`, never seconds.
- Use the glossary words ([docs/glossary.md](docs/glossary.md)): Folder, Local PR, Suggestion, Files, Launch, Delete/Remove.
- Virtualize long lists, memoize rows, keep heavy imports out of `router.tsx`, `main.tsx`, the shells and the sidebar.
- The phone shell reuses feature components with `usePhone()`, never forks them; every overlay owns one history entry (`useBackClose`); keep 44 px touch targets. Theme, zoom and layout are device-local.
- A window shows exactly one workspace; narrowed lists use `useScope()`.
- Demo mode answers every request in the browser; a new major feature updates its tour chapter, demo fixtures and `data-tour` anchors.
- The Chrome extension never imports the app's UI, never acts in GitHub pages beyond Insert and prefill, and talks to Home only from its service worker ([extension](docs/invariants/extension.md)).

## Git workflow

These rules apply unless the owner or the launch says otherwise.

- Work on `main`, in this checkout. Do not create other branches, worktrees or pull requests for this repository.
- Commit finished work with a short message that says what changed.
- Update the documentation in the same commit: `README.md` (features), the matching `docs/` page for a user-facing change, `docs/architecture.md` for a design change, and the area's `docs/invariants/` page for a new or changed rule. Change this file only for a new command or a cross-cutting rule, and keep it short. Describe behavior, not implementation history.

## Testing

This is a personal project: move fast.

- Add a test only for a small, specific part that clearly benefits from it, typically a pure function with tricky edge cases.
- No mocks of HTTP APIs, harness processes, git or the database. No tests that need a server, the network, GitHub, a running agent, the daemon or a full `App`.
- The whole test run takes a few seconds; no test takes more than a second. Delete slow or flaky tests.
- Check changes by building and running the app in the dev environment.

## Writing style

- Markdown: never hard-wrap lines. Write each paragraph or list item on one line.
- Use plain technical American English that non-native speakers can read easily. Be concise.

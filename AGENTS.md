# Launchpad

Launchpad is a personal desktop app for Ubuntu (Tauri) that launches and drives coding agents (Claude Code, Codex) and replaces the parts of the GitHub UI used every day, with a local cache so nothing feels slow. It has one user, on one machine.

Status: Phase 0 done, Phases 1 and 2 built and awaiting daily-use checks (see `docs/ROADMAP.md`). Keep the build and run commands below and in `README.md` up to date.

## Commands

- Build the UI: `cd web && pnpm install && pnpm build` (Node 24 from `web/.nvmrc`, pnpm through Corepack).
- Run the dev daemon: `LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- daemon` (port 7318, data in `~/.local/share/launchpad-dev`). Never stop or restart the prod daemon (port 7317) unless the owner asks: it runs the agents, maybe including you.
- Sign a browser in to the dev daemon: `LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- url`. Headless screenshots work with `google-chrome --headless=new --screenshot=out.png --virtual-time-budget=5000 "<url>"`.
- Regenerate the TypeScript types after changing a Rust type shared with the UI: `cargo run -p launchpad -- gen-types`.
- Type-check the UI: `(cd web && pnpm typecheck)`.
- Tests: `cargo test` (the few unit tests in `crates/core`; the desktop crate is excluded by default).
- Dev window: `cargo build -p launchpad-desktop && LAUNCHPAD_PROFILE=dev ./target/debug/launchpad-desktop [--open <route>]`.
- Install or update the prod app: `packaging/install.sh` (release build, binaries in `~/.local/bin`, systemd user unit, `.desktop` and autostart entries). It restarts the prod daemon, so only run it when the owner asks.
- UI update: Settings → General starts `packaging/update.sh` in a transient systemd user service. It runs UI type checks, Rust tests and release builds, then `packaging/install.sh --prebuilt`. It restarts prod, so do not click it during agent work. The log is `~/.local/share/launchpad/update.log`.
- Icon assets: keep `packaging/icons/source.png` as the unchanged master. Regenerate PNG/WebP/SVG exports with `python3 packaging/generate-icons.py` (requires Pillow). Update installed icons without restarting the app or daemon with `packaging/install-icons.sh`.
- GitHub inbox: authenticate with `gh auth login`; the dev daemon uses the real GitHub account too. Verify replies and unsubscribe controls without posting comments or muting real threads during automated checks. Local unread actions are safe to check.
- UI rule from spike 4 (WebKitGTK): virtualize every long list, memoize row components, use `useFlushSync: false` on virtualizers, prefer fixed row heights.

## Where things are

- `README.md`: what the project is, its current status, and how to build and run it.
- `initial.md`: the original description of the project, in the owner's words.
- `docs/ARCHITECTURE.md`: architecture and the decisions log. Read it before changing the design, and update it when a decision changes.
- `docs/ROADMAP.md`: phases, and what "done" means for each one.
- `docs/spikes/`: notes from the Phase 0 spikes (exact harness behavior, with sample output).
- `crates/core` (domain and services), `crates/server` (axum API, WebSocket, auth, embedded UI), `crates/cli` (`launchpad` binary), `web/` (React app), `desktop/` (Tauri shell), `packaging/` (systemd unit, `.desktop` file, install script).
- `docs/design/sketches.html`: UI sketches (static HTML, open it in a browser). It is also published as a private artifact at https://claude.ai/code/artifact/916895f3-aa0f-4378-96da-b971415d6a5a. After changing the file, republish it to that same URL when the Artifact tool is available.

## Git workflow

These rules apply unless the owner says otherwise for a given task.

- Always work on the `main` branch, in this checkout. Do not create other branches, worktrees or pull requests for this repository.
- When a piece of work is complete, always commit it on `main`. Do not leave finished work uncommitted.
- Before committing, update the documentation so that it matches the new state of the project, and include it in the same commit:
  - `README.md`: status, build and run commands.
  - `AGENTS.md`: new commands, rules or conventions.
  - `docs/ARCHITECTURE.md`: design changes, and the decisions log when a decision is made or changed.
  - `docs/ROADMAP.md`: the Status column and the items done in the current phase.
  - `docs/design/sketches.html`: when the UI direction changes.
- Use a short commit message that says what changed.

## Product rules

- Agent groups: Done means waiting for input; Running includes starting, working and background commands. Never infer Done from a command being silent. Sidebar counts and project markers must use the same predicates.
- Launch defaults come from the latest successfully launched spec, not from harness clicks or unsent edits.
- General zoom has five positions (100%, 110%, 120%, 135%, 150%), with 120% as the default. Scale the entire interface and keep list virtualization working at every position.
- Light theme only. Do not add a dark theme or dark-mode styles.
- Click-only UI: no keyboard shortcuts, except `Ctrl+F`, which opens the find bar in the code viewer. Every action is a visible button (the find bar also has a Find button). Text fields only type text, with three exceptions: in the composer prompt, `@` (projects) and `#` (issues and PRs) at the start of a word open a suggestion list; in the session message box, `Enter` sends, `Shift+Enter` adds a new line, and standard clipboard paste accepts images; in the find field of the code viewer, `Enter` and `Shift+Enter` go to the next and previous match and `Escape` closes the bar. See `docs/ARCHITECTURE.md`, sections 8.1 (code viewer) and 9.7 (mention rules).
- Linux (Ubuntu) only. Do not add code for macOS or Windows.
- Single user: no backward compatibility, no versioning, no data migration framework. The GitHub cache (`cache.db`) can be deleted and rebuilt at any time. Keep schema changes to `state.db` simple.
- Anything Launchpad prepares for GitHub (pre-draft PRs, comments or reviews written by agents) stays local until the owner publishes it with a click.
- GitHub notifications: `all=true` returns historical Done threads too and has no Done/inbox field. Never treat the REST history as the web inbox or promise exact parity. Keep membership in `state.db`; a missing thread subscription (404) is not evidence of unsubscribe.
- Never refresh the Claude OAuth token. Read it only (see `docs/ARCHITECTURE.md`, section 9.5).

## Testing

This is a personal project: move fast and break things.

- Keep tests to the minimum. Most code does not need tests. Do not add tests by default.
- Add a test only for a small, specific part that clearly benefits from it, typically a pure function with tricky edge cases (for example the diff parser or the prompt renderer).
- No mock tests. Do not mock HTTP APIs, harness processes, git or the database.
- No tests that need a server, the network, GitHub, a running agent or the daemon.
- The whole test run must take a few seconds at most. Delete or rewrite a test that becomes slow or flaky.
- Check your changes by building and running the app (dev profile) rather than by writing tests.

## Writing style

- Markdown: never hard-wrap lines. Write each paragraph or list item on one line.
- Use technical American English that non-native speakers can read easily.

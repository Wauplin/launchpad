# Launchpad

Launchpad is a personal desktop app for Ubuntu that launches and drives coding agents (Claude Code and Codex) and replaces the parts of GitHub used every day: notifications, issues, pull requests and reviews. Everything GitHub-related is cached locally, so moving between items never reloads a page. Launchpad also keeps pre-draft PRs: pull requests that exist only on the laptop until they are worth publishing. A read-only code viewer (designed, not built yet) shows the files of projects, worktrees and branches, and turns selected lines into context for an agent.

## Status

Phase 0 (foundations and spikes) is done: the daemon runs as a systemd user service, the window opens from the dock, and the status bar shows the Claude and Codex versions and quotas. Phase 1 (launch and run agents) is built and awaiting its week of daily use. Phase 2 (GitHub inbox) is built and awaiting a morning of notification triage: cached notifications, repo filters, thread summaries, quick replies, triage actions and agent launch from a thread. Agent launches open the composer directly and remember the last launched harness, model and effort. Sessions support pasted and dropped images, AI-generated prompt-based titles, and Done/Running/Closed groups with sidebar activity indicators. The light UI has stronger contrast and defaults to 120% zoom; Settings → General offers five zoom sizes. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Install

```sh
packaging/install.sh            # build in release mode, install for the current user, restart the daemon
```

It installs `~/.local/bin/launchpad` and `~/.local/bin/launchpad-desktop`, the systemd user unit `launchpad.service`, the `Launchpad` application entry with its icon, and an autostart entry that keeps the window loaded in the background. Open Launchpad from the applications grid, then right click its dock icon and choose "Add to Favorites". Closing the window only hides it, so the next click shows it at once; `launchpad-desktop --quit` stops it. Running the script again updates everything; it asks first when agent sessions are open, because restarting the daemon stops them (`--force` skips the question).

Logs: `journalctl --user -u launchpad -f`.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): architecture and decisions log.
- [docs/ROADMAP.md](docs/ROADMAP.md): phases and their progress.
- [docs/design/sketches.html](docs/design/sketches.html): UI sketches (open the file in a browser, or the [published version](https://claude.ai/code/artifact/916895f3-aa0f-4378-96da-b971415d6a5a)).
- [AGENTS.md](AGENTS.md): rules for coding agents working on this repository (`CLAUDE.md` links to it).
- [initial.md](initial.md): the original project description.

## Development

Requirements: Rust 1.93 or later, Node 24 (`nvm use` in `web/`, which has an `.nvmrc`), pnpm 10 (pinned in `web/package.json`; Corepack picks it up when pnpm runs from `web/`), and the Tauri system libraries (`libwebkit2gtk-4.1-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `build-essential`).

```sh
(cd web && pnpm install)
(cd web && pnpm build)                                   # UI into web/dist (served by the daemon)
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- daemon   # dev daemon on 127.0.0.1:7318
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- url      # address that signs a browser in
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- status   # status as JSON
```

- Profiles: `LAUNCHPAD_PROFILE=dev` uses port 7318, `~/.config/launchpad-dev` and `~/.local/share/launchpad-dev`. Without it (prod): port 7317, `~/.config/launchpad`, `~/.local/share/launchpad`.
- A debug build of the daemon reads `web/dist` from disk, so `(cd web && pnpm build)` updates the UI without restarting it. A release build embeds the UI.
- Hot reload: `(cd web && pnpm dev)` serves the UI on http://localhost:5173 and proxies the API to the dev daemon. Sign in once with the address printed by `launchpad url`, with port 5173 instead of 7318.
- GitHub: sign in with `gh auth login` (a token with `repo` or `notifications` scope). The daemon reads the token from `gh auth token`, polls notifications in the background, and keeps cached threads available when offline. GitHub’s public API also returns Done notifications, so Launchpad tracks inbox membership locally and excludes explicitly ignored subscriptions. Its initial inbox contains unread threads; existing read inbox items can be restored from a known GitHub snapshot. Read/Done changes made on github.com cannot be mirrored exactly through this API. Opening an unread thread marks it read on GitHub. Unread is local only; replies post only when you click **Post reply**.
- Types shared with the UI are generated from Rust: `cargo run -p launchpad -- gen-types` (writes `web/src/gen/`).
- Desktop window for the dev daemon: `cargo build -p launchpad-desktop && LAUNCHPAD_PROFILE=dev ./target/debug/launchpad-desktop` (a dev window is not single-instance and quits when closed). `--open <route>` opens a given screen.
- WebKitGTK benchmark: open `/dev/perf?auto=1` in the window; results go to the daemon log ([spike 4](docs/spikes/4-webkitgtk.md)).
- Tests: `cargo test` (a few pure functions: the prompt renderer, the remote and link parsers). It runs in well under a second once built.

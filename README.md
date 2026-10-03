# Launchpad

Launchpad is a personal desktop app for Ubuntu that launches and drives coding agents (Claude Code and Codex) and replaces the parts of GitHub used every day: notifications, issues, pull requests and reviews. Everything GitHub-related is cached locally, so moving between items never reloads a page. Launchpad also keeps pre-draft PRs: pull requests that exist only on the laptop until they are worth publishing. A read-only code viewer shows the files of projects, worktrees and branches, and turns selected lines into context for an agent.

## Status

Phase 0 (foundations and spikes) is done: the daemon runs as a systemd user service, the window opens from the dock, and the status bar shows the Claude and Codex versions and quotas, plus the running Launchpad commit and an Update button when local main has newer commits. Phase 1 (launch and run agents) is built and awaiting its week of daily use. Phase 2 (GitHub inbox) is built and awaiting a morning of notification triage: cached notifications, repo filters, thread summaries, quick replies, triage actions and agent launch from a thread. Agent launches open the composer directly and remember the last launched harness, model and effort. The launcher can start now, after a duration, at a local date and time, when selected running or scheduled agents are all Done, or at a reported Claude/Codex quota reset. Delayed launches persist locally and appear under Scheduled in Agents, with Start now and Cancel controls. Sessions support pasted and dropped images, AI-generated prompt-based titles, and independently collapsible Done/Running/Scheduled/Archived groups with remembered expansion state and sidebar activity indicators. Closing a session panel keeps its state; a separate Archive button archives the resumable conversation. The light UI has stronger contrast and defaults to 120% zoom; Settings → General offers five zoom sizes. Phase 2b (code viewer) is built and awaiting its week of daily use: a project's checkout, worktrees, branches, tags and fetched PR heads, with a file tree, change letters and line bars, tabs, colors, find in file (`Ctrl+F`), search in the project, and selected lines sent to the composer or a session. Phase 3 (issues and pull requests) is built and awaiting daily review and merge checks: full cached conversations, reviews, check links, reviewers, labels, issue state controls, PR draft and merge controls, project issue/PR lists with colored GitHub labels, label filtering and multi-select launch, and virtualized unified/split diffs with viewed markers and full-file navigation. Phase 4 (pre-draft PRs) is built and awaiting two weeks of daily use: dedicated local branches and worktrees, agent-facing CLI commands, teal project PR rows, descriptions, commits and diffs, saved general/file review notes, feedback to the same session, and explicit draft/ready publication with a reviewed commit check. Phase 5 (workspace polish) is built and awaiting daily use: draggable session panes, named saved layouts, visible focus and move controls, persistent pins, manual rename and session/message search, explicit Claude/Codex history import with native resume, a read-only per-session Git panel and Open in editor, and recent composer context chips. Phase 6 (agent-written suggestions) is built and awaiting daily posting checks: local comment/review drafts from the CLI, editable teal cards, a PR Review with agent action, and explicit publication tied to the reviewed commit. Phase 7 is deferred: its session workers, embedded terminal and remote browser mode were reverted on 2026-10-03 because the phase needed a clearer scope. Restarting the daemon interrupts running turns; native Resume and the external terminal handoff remain available. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Install

```sh
packaging/install.sh            # build in release mode, install for the current user, restart the daemon
```

It installs `~/.local/bin/launchpad` and `~/.local/bin/launchpad-desktop`, the systemd user unit `launchpad.service`, the `Launchpad` application entry with its icon, and an autostart entry that keeps the window loaded in the background. Open Launchpad from the applications grid, then right click its dock icon and choose "Add to Favorites". Closing the window only hides it, so the next click shows it at once; `launchpad-desktop --quit` stops it. Running the script again updates everything; it asks first when agent sessions are running, because restarting the daemon interrupts them (`--force` skips the question). Settings → General has **Check and update Launchpad** in the production app: it checks UI types, runs Rust tests, builds both release binaries and the UI, then installs and relaunches. Progress and failures appear there; the full log is in `~/.local/share/launchpad/update.log`. The button uses the local checkout and does not fetch changes from GitHub.

Logs: `journalctl --user -u launchpad -f`.

The app uses the sculpted launch icon, also shown in the top-left corner of the UI. Its master and PNG/WebP exports are in [packaging/icons](packaging/icons/README.md); the SVG wraps the raster artwork. Use `packaging/install-icons.sh` to update only the installed icon without restarting the app or daemon. Regenerate exports with `python3 packaging/generate-icons.py` (requires Pillow).

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): architecture and decisions log.
- [docs/ROADMAP.md](docs/ROADMAP.md): phases and their progress.
- [docs/design/sketches.html](docs/design/sketches.html): UI sketches (open the file in a browser, or the [published version](https://claude.ai/code/artifact/916895f3-aa0f-4378-96da-b971415d6a5a)).
- [AGENTS.md](AGENTS.md): rules for coding agents working on this repository (`CLAUDE.md` links to it).
- [initial.md](initial.md): the original project description.

The status bar’s **Update** button compares the running binary’s embedded commit with local `main` (no remote fetch). Updating waits for a clean committed `main`, builds in the background, then waits until no agents are Running before reinstalling and relaunching. New launches and messages are briefly blocked during installation. The same updater is available in Settings → General, including stage and failure logs. Dev mode shows the commit but cannot start production updates.

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
- Pre-draft PRs: choose a project, then **Run in → Pre-draft PR** in the composer. Launchpad fetches `origin`, creates `lp/<slug>` in `<projects_root>/.worktrees/<project-id>/<slug>`, and launches the agent there. Review it from the project PR list or the session’s **Pre-draft PR** button. **Refresh** fetches the base; **Notes** supports general and file feedback, sent to the same session. **Publish** takes a remote branch name and a draft/ready choice, then requires confirmation. Commit all changes before publishing or discarding. Settings → General can remove clean worktrees after publication; the default keeps them.
- Local PR CLI: `LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- pr list`. From a pre-draft worktree, use the absolute dev binary (`<checkout>/target/debug/launchpad pr show` or `pr update --title "…" --body-file <file>`) with `LAUNCHPAD_PROFILE=dev`. The installed binary uses production by default. `pr create --title "…" --body-file <file>` copies the current committed HEAD into a dedicated worktree; it preserves the original checkout and returns the new folder as `root`. Its review page offers **Launch agent** to link a session. At startup, the daemon links `skills/launchpad` into `~/.agents/skills/launchpad`, preserving any existing real directory.
- Agent suggestions: `launchpad suggest comment --repo owner/repo --number 123 --body-file /path/to/comment.md`; use `suggest review` with `--head <inspected-commit>` and optionally `--event approve|request-changes`. Agents also pass `--session-id` from their prompt for attribution. Use the prompt’s exact binary path and `LAUNCHPAD_PROFILE`. Replace `--repo` and `--number` with `--predraft <id>` for a local PR. These commands only save drafts. Issue/PR conversation cards offer Edit, Post comment/review and Discard; local PR description cards offer Add to local notes. The PR’s **Review with agent** button opens the composer with the displayed head and instructions. A changed PR head blocks a suggested review. An interrupted or failed publication stays visible for inspection and is never automatically retried.
- Types shared with the UI are generated from Rust: `cargo run -p launchpad -- gen-types` (writes `web/src/gen/`).
- Desktop window for the dev daemon: `cargo build -p launchpad-desktop && LAUNCHPAD_PROFILE=dev ./target/debug/launchpad-desktop` (a dev window is not single-instance and quits when closed). `--open <route>` opens a given screen.
- WebKitGTK benchmark: open `/dev/perf?auto=1` in the window; results go to the daemon log ([spike 4](docs/spikes/4-webkitgtk.md)).
- Code viewer: Code on a project page, on a worktree or `/projects/<id>/code?root=<folder>&rev=<revision>&path=<file>&line=<n>`. Files come from disk and git (`/api/code/*`); only registered projects and their git worktrees are readable. Colors come from Shiki in a Web Worker, so a file shows as plain text first.
- Tests: `cargo test` (a few pure functions: the prompt renderer, the remote and link parsers, the code viewer's diff hunk, path and `git grep` parsers). It runs in well under a second once built.

## Continuous integration

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on pushes to `main`, pull requests and manual dispatch in `Wauplin/launchpad`. It has three independent Ubuntu 24.04 jobs: **Lint** runs strict Clippy across the whole workspace (including the desktop shell) and UI type checks; **Tests** runs the existing Rust tests for the default workspace members; **Build artifacts** builds the UI first, then both release binaries so the daemon embeds the UI. Rust is pinned to 1.93.1 for repeatable lint results; Node comes from `web/.nvmrc`, pnpm from `web/package.json`, and dependency installs use the lockfiles.

Successful builds upload `launchpad-linux-x86_64`, retained for seven days. Download it from the workflow run and extract `launchpad-linux-x86_64.tar.gz`: `bin/` contains `launchpad` and `launchpad-desktop` with executable permissions preserved, and `web/dist/` contains the standalone UI build. The binaries target Ubuntu 24.04 x86_64 and require the desktop runtime libraries. CI does not install the app or publish a GitHub release.

Run the same checks locally:

```sh
cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings
(cd web && pnpm install --frozen-lockfile && pnpm typecheck && pnpm build)
cargo +1.93.1 test --locked
cargo +1.93.1 build --release --locked -p launchpad -p launchpad-desktop
```

Install the CI toolchain if needed: `rustup toolchain install 1.93.1 --profile minimal --component clippy`.

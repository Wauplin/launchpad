# Launchpad

Launchpad is a personal desktop app for Ubuntu that launches and drives coding agents (Claude Code and Codex) and replaces the parts of GitHub used every day: notifications, issues, pull requests and reviews. Everything GitHub-related is cached locally, so moving between items never reloads a page. Launchpad also keeps pre-draft PRs: pull requests that exist only on the laptop until they are worth publishing.

## Status

Phase 0 of the roadmap (foundations and spikes) is in progress: the daemon, the API and the app shell work; the desktop window and the packaging are next.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): architecture and decisions log.
- [docs/ROADMAP.md](docs/ROADMAP.md): phases and their progress.
- [docs/design/sketches.html](docs/design/sketches.html): UI sketches (open the file in a browser, or the [published version](https://claude.ai/code/artifact/916895f3-aa0f-4378-96da-b971415d6a5a)).
- [AGENTS.md](AGENTS.md): rules for coding agents working on this repository (`CLAUDE.md` links to it).
- [initial.md](initial.md): the original project description.

## Development

Requirements: Rust 1.93 or later, Node 24 (`nvm use` in `web/`, which has an `.nvmrc`), pnpm 10 (pinned in `web/package.json`, Corepack picks it up), and the Tauri system libraries (`libwebkit2gtk-4.1-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `build-essential`).

```sh
pnpm -C web install
pnpm -C web build                                   # UI into web/dist (served by the daemon)
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- daemon   # dev daemon on 127.0.0.1:7318
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- url      # address that signs a browser in
LAUNCHPAD_PROFILE=dev cargo run -p launchpad -- status   # status as JSON
```

- Profiles: `LAUNCHPAD_PROFILE=dev` uses port 7318, `~/.config/launchpad-dev` and `~/.local/share/launchpad-dev`. Without it (prod): port 7317, `~/.config/launchpad`, `~/.local/share/launchpad`.
- A debug build of the daemon reads `web/dist` from disk, so `pnpm -C web build` updates the UI without restarting it. A release build embeds the UI.
- Hot reload: `pnpm -C web dev` serves the UI on http://localhost:5173 and proxies the API to the dev daemon. Sign in once with the address printed by `launchpad url`, with port 5173 instead of 7318.
- Types shared with the UI are generated from Rust: `cargo run -p launchpad -- gen-types` (writes `web/src/gen/`).
- There are no tests yet.

# Launchpad

Launchpad is a personal desktop app for Ubuntu (Tauri) that launches and drives coding agents (Claude Code, Codex) and replaces the parts of the GitHub UI used every day, with a local cache so nothing feels slow. It has one user, on one machine.

Status: design phase. There is no code yet. When code lands, add the build and run commands to this file and to `README.md` (and the test command, if there are tests).

## Where things are

- `README.md`: what the project is, its current status, and how to build and run it.
- `initial.md`: the original description of the project, in the owner's words.
- `docs/ARCHITECTURE.md`: architecture and the decisions log. Read it before changing the design, and update it when a decision changes.
- `docs/ROADMAP.md`: phases, and what "done" means for each one.
- `docs/design/sketches.html`: UI sketches (static HTML, open it in a browser).

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

- Light theme only. Do not add a dark theme or dark-mode styles.
- Click-only UI: no keyboard shortcuts. Every action is a visible button. Text fields only type text: no character opens a popup or triggers an action (no `@` or `#` autocomplete). The only exception is the session message box, where `Enter` sends and `Shift+Enter` adds a new line.
- Linux (Ubuntu) only. Do not add code for macOS or Windows.
- Single user: no backward compatibility, no versioning, no data migration framework. The GitHub cache (`cache.db`) can be deleted and rebuilt at any time. Keep schema changes to `state.db` simple.
- Anything Launchpad prepares for GitHub (pre-draft PRs, comments or reviews written by agents) stays local until the owner publishes it with a click.
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

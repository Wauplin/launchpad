# Plan: an integrated terminal in Launchpad, replacing external terminals

Status: implemented on 2026-10-10 with the recommended owner decisions (bottom dock, terminals lost on daemon restart, manual Re-attach). Written on 2026-10-10, based on `origin/main` at `d66929d`. Progress is tracked in `docs/ROADMAP.md` (Phase 7a).

## What I found

- **External terminals today:** `crates/core/src/terminal.rs` has a list of terminal command templates, a `[terminal]` section in `config.toml`, Settings → General → Terminal, `GET /api/settings/terminals` and `POST /api/sessions/{id}/open-terminal`. That endpoint stops the headless harness and starts `claude --resume` or `codex resume` in a separate terminal window. The session shows "In terminal" until you click Re-attach.
- **An earlier attempt exists.** Commit `7b670af` built an embedded PTY with xterm.js, and `847320f` reverted it. That attempt:
  - Was Linux-only (raw `libc::openpty`).
  - Was tied to restart-safe session workers.
  - Sent terminal output through the global event bus.
  - Its xterm.js light theme and its snapshot-then-live-output pattern are worth reusing; the rest is not.
- **The global WebSocket is the wrong channel for terminal output.** It sends every event to every window and resets lagging clients. A command with heavy output would trigger resets for the whole UI.
- **Interface zoom is a risk.** The UI scales everything with CSS `zoom` on `<html>`. xterm.js often places the mouse selection wrong under CSS zoom.
- **The code viewer catches `Ctrl+F` for the whole window**, so it would take that key away from a terminal opened on that page.
- **The spike passed.** `portable-pty` 0.9 builds with Rust 1.93.1. It opened a PTY, started `$SHELL -i` in the requested folder, resized it (`stty size` → `40 120`), reported 256 colors and returned the exit code. It also supports ConPTY on Windows, which the reverted code did not.

## Scope

The project rules require a defined scope before any Phase 7 work, so here it is.

- **Need:** use a shell or a harness TUI without leaving Launchpad.
- **Ownership:** the daemon starts and owns the terminal processes, like it owns the harnesses.
- **Failure behavior:** restarting the daemon kills the terminals. Their output is kept in memory only (up to 1 MB per terminal) and nothing goes into `state.db`.
- **Non-goals:**
  - Terminals that survive a daemon restart.
  - tmux-style detach and reattach.
  - Split panes.
  - Remote access.
  - Running Launchpad commands inside a terminal.
- **Acceptance check:** see [Acceptance check](#acceptance-check-dev-daemon-desktop-window).

## Design

### Daemon

**New module `crates/core/src/terminals.rs`** (replaces `terminal.rs`):

- A registry of open terminals. Each record holds `id`, `kind` (`shell` or `tui`), `title`, `cwd`, `project_id`, `session_id`, `alive` and `exit_code`.
- Each terminal is a `portable-pty` child process. A blocking reader thread fills a 1 MB buffer that drops the oldest output first.
- Input goes through a writer, and resize calls `master.resize`.
- Close sends SIGTERM to the process group, then SIGKILL after 5 seconds (on Windows, it kills the ConPTY child).
- A cap of about 12 open terminals.

**Environment:**

- Use the login-shell environment the daemon already loads (`app.env`), plus `TERM=xterm-256color`, `COLORTERM=truecolor` and `LAUNCHPAD_PROFILE`.
- The shell is `$SHELL -l` on Linux and macOS. On Windows it is `pwsh.exe`, or `powershell.exe` if `pwsh` is not installed.

**Harness TUIs:**

- Keep `tui_command`, `project_trust_override` and its unit test.
- "Open in terminal" becomes a `tui` terminal. The session goes to `InTerminal` exactly as today.
- Re-attach can now close the TUI itself (after a confirmation) instead of saying "close it there first".
- Keep `tui_running` for imported conversations that are still open in another client.
- Harness TUIs stay outside quota protection, like external terminals today.

**API:**

- `GET /api/terminals` lists terminals. `POST /api/terminals {cwd}` opens a shell; like `code::open`, it only accepts a registered project, one of its Git worktrees, or the folder of an existing session.
- `POST /api/sessions/{id}/terminal` opens the harness TUI and replaces `open-terminal`.
- `POST /api/terminals/{id}/close` stops a terminal, and `DELETE /api/terminals/{id}` removes an exited one from the list.
- A dedicated WebSocket per terminal, `GET /api/terminals/{id}/ws`:
  - On connect, the daemon sends the buffered output, then streams live output as binary frames.
  - The client sends keystrokes as binary frames and resize requests as JSON text frames.
- The global bus only carries list changes (topic `terminals`: opened, exited, closed). Output never goes through it.
- The new endpoints use the existing auth guard (token cookie, plus the `Host` and `Origin` checks on WebSocket upgrades).

**How terminals interact with existing features:**

- **Pre-draft PRs:** `predrafts::idle`, launching into an existing pre-draft and `check_session_work` count any open terminal whose folder is inside the pre-draft worktree as activity. Publication and cleanup wait until those terminals are closed.
- **Temporary projects:** cleanup (`close_in`) and promotion (`move_cwd`) close the terminals in that folder.
- **Manual updates:** Settings shows a Close all terminals button. `allow_work` serializes new terminals with pre-draft publication and cleanup. The in-app updater and install lock were removed on 2026-10-10.

### UI (`web/src/features/terminals/`)

**`TerminalView`:**

- Uses `@xterm/xterm` 6.0.0 and `@xterm/addon-fit` 0.11.0 with the default DOM renderer. WebGL is unreliable in WebKitGTK.
- Uses the light theme from the reverted attempt and 5,000 lines of scrollback.
- Zoom fix: set `zoom: 1/scale` on the terminal container and multiply the font size by the scale, so xterm.js measures real pixels.

**`TerminalDock`:**

- A bottom panel in `Shell.tsx`, under the main area.
- A tab for each terminal shows its title (project · branch, or the session title), its status (running, or "exited 0") and a Close button.
- Buttons: New terminal (opens in the current context, active workspace terminal folder or first workspace project) and Hide. Copy/Paste/Clear buttons were replaced by normal terminal shortcuts on 2026-10-12.
- Hide keeps the processes running.
- Small, medium and large height buttons instead of a drag handle.
- A Terminal button in the status bar shows the count. Visibility, height and the active tab are saved in the layout store.
- Tabs are filtered by the current workspace through the terminal's project, using `useScope()`. No new ownership field.

**Where you can open a terminal:**

| Place | Button | Folder |
|---|---|---|
| Project page header | Terminal | Project root |
| Each row in the project's Worktrees section | Terminal | That worktree |
| Session header | Terminal (shell) and Open in terminal (harness TUI) | Session folder |
| Pre-draft view | Terminal | Pre-draft worktree |
| Code viewer header | Terminal | The root being viewed |

**Keyboard:**

- Exceptions to the click-only rule: global `Ctrl+T`/`Cmd+T` opens a new shell. The focused terminal handles selection copy (`Ctrl+Shift+C`/`Cmd+C`) and paste (`Ctrl+Shift+V`/`Ctrl+V`/`Cmd+V`, with bracketed paste); `Ctrl+L` and macOS `Cmd+L` send clear/redraw input to the shell. Other keys, including `Ctrl+C`, `Ctrl+D` and `Ctrl+F`, reach the process.
- The code viewer's `Ctrl+F` handler must ignore keys that come from inside a terminal.

### What gets removed

- The terminal templates, `TerminalOption`, `options()`, `open()` and `shell_join` (only if nothing else uses it).
- The `Terminal` config struct and the `[terminal]` default section. An old `[terminal]` section is ignored, because `Config` uses `serde(default)` and does not reject unknown fields, and the next save drops it.
- `/api/settings/terminals`, `useTerminals` and the Terminal row in Settings.
- The generated `Terminal.ts` and `TerminalOption.ts` (regenerate types with `gen-types`).
- `osascript` and `setsid` handoff code.
- `history.rs`: change "open it in the terminal" to the new wording.
- `procs.rs`: update the doc comment.

### Documentation, in the same commit

- **`docs/ARCHITECTURE.md`:**
  - New decision entry; it replaces decisions 1 ("No embedded terminal") and 9 ("Terminal is a setting"), and supersedes the terminal part of 44.
  - Update the table in section 2 and sections 3, 6.5, 9.x and 13.
  - Remove the terminal templates table.
- **`docs/ROADMAP.md`:** move "Embedded terminal mode" out of the Phase 7 candidates, as a scoped item with its own Status.
- **`CLAUDE.md` and `AGENTS.md`:** the keyboard exception, the end of "embedded terminals were reverted", "TUIs in integrated terminals are outside quota protection", and the pre-draft rules for terminals.
- **`README.md`** and **`docs/design/sketches.html`:** add a dock sketch and republish the artifact.

## Implementation order

1. Daemon: `terminals.rs`, the API and the WebSocket. Check them from a small dev page.
2. Switch "Open in terminal" to the integrated terminal, then remove the external terminal code and its config.
3. UI: `TerminalView`, the dock, the entry points, the zoom fix and the `Ctrl+F` guard.
4. Pre-draft and temporary project checks.
5. Documentation, then run `clippy` and `pnpm typecheck` and do the manual checks below.

Rough size: about 450 lines of Rust and about 500 lines of TypeScript. No new tests; the existing `project_trust_override` test stays.

## Acceptance check (dev daemon, desktop window)

- Open a shell from a project, a worktree, a session, a pre-draft and the code viewer. In each one:
  - `pwd` prints the right folder.
  - `vim`, `htop` and colors work.
  - Resizing the window updates `stty size`.
  - Mouse selection lines up at 100% and 150% zoom.
  - `yes | head -n 200000` leaves the rest of the UI responsive.
- Open a Claude TUI and a Codex TUI from a session (with a cheap model, and without posting anything to GitHub), then Re-attach.
- Open the same terminal in two windows, and reload a window: the buffered output reappears.
- With a terminal open:
  - Pre-draft publication is blocked.
- Restarting the dev daemon marks the terminals as gone.
- Repeat on Windows and macOS through the CI builds.

## Decisions for the owner

1. **Placement:** a bottom dock in every view (recommended) or a separate Terminals page.
2. **Restart behavior:** losing terminals when the daemon restarts (recommended) or making them restart-safe. Restart-safe terminals bring back the reverted worker design.
3. **When a harness TUI exits:** keep the session "In terminal" until the owner clicks Re-attach (recommended, same as today) or re-attach it automatically.

# Integrated terminal reliability

Investigation and fixes, 2026-10-13. This is hardening of the existing Phase 7a terminal, not a new terminal architecture. Daily-use acceptance remains open.

## Keep the established components

Launchpad already uses `@xterm/xterm` 6.0.0, the terminal emulator used by [VS Code](https://code.visualstudio.com/docs/terminal/advanced), and `@xterm/addon-fit` 0.11.0. Carriage returns, ANSI cursor movement, colors and alternate-screen modes are interpreted by xterm.js; Launchpad does not implement those operations itself. Its [6.0 release](https://github.com/xtermjs/xterm.js/releases/tag/6.0.0) includes synchronized-output support for modern TUIs.

The Rust daemon uses [`portable-pty` 0.9.0](https://docs.rs/portable-pty/0.9.0/portable_pty/), from the WezTerm project, for native PTYs on Unix and ConPTY on Windows. Output stays as bytes through the dedicated WebSocket and xterm's streaming UTF-8 decoder. Switching to the Node-based PTY stack used by VS Code would add a runtime/native-module boundary and would not address the integration problems found here. Keep the current libraries and improve their lifecycle and transport.

## Findings and changes

| Area | Previous behavior | Current behavior |
| --- | --- | --- |
| Tab, workspace and Hide lifecycle | Only the selected terminal was mounted. Switching or hiding destroyed its emulator and replayed raw history when reopened. | Each visited terminal stays mounted and connected until its record is removed or the window closes. Hidden terminals keep parsing output; showing one fits and focuses it. Zoom updates the font without recreating the emulator. |
| Reset ordering | `reset()` ran immediately while older `write()` operations could still be queued. Older output could then mix with the new snapshot. | CAN + RIS is queued through xterm's parser before the snapshot. It cancels an unfinished escape sequence and resets screens/modes in stream order. |
| Snapshot dimensions | A new renderer replayed output at its own initial dimensions. | The daemon includes the PTY's most recent rows/columns, including after exit. Replay uses those dimensions, then the visible viewer fits and reports its size. This cannot reconstruct earlier resizes. |
| Renderer pressure | Socket delivery fed `write()` without waiting for parsing. Browser/parser queues could grow independently of the server's lag detection. | Each viewer has a 256 KiB credit window for its current epoch (an ordered resync can briefly overlap the old and new windows, up to 512 KiB total). ACKs come from xterm write callbacks. Snapshot output is chunked too. A slow viewer waits for credit while controls remain readable; it never pauses the shared PTY or another viewer. |
| Resync credit | No parser credit protocol. | Every reset starts a new epoch; callbacks for old epochs and oversized ACKs cannot release current credit. |

The credit mechanism follows [xterm's WebSocket flow-control guidance](https://xtermjs.org/docs/guides/flowcontrol/). xterm writes are asynchronous; successful WebSocket delivery does not mean the emulator has consumed the data. This implementation bounds the viewer queue rather than pausing the shared PTY: the existing bounded broadcast channel and snapshot-on-lag behavior still apply.

The investigation did not reproduce a general carriage-return failure. Both a native PTY check and xterm/browser redraw checks preserve and handle `\r` correctly. A particular CLI can still fail for reasons these fixtures do not cover; keep its exact command and whether the corruption appeared live, after switching tabs, or after a reload when reporting a regression.

## Repeatable checks

Run with the pinned Rust toolchain and Node 24:

```sh
cargo +1.93.1 test --locked
cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings
(cd web && pnpm test:terminals && pnpm test:attachments && pnpm build)
```

`web/tests/terminals.test.mjs` uses the matching xterm headless parser. It checks carriage-return/erase-line progress, byte-by-byte UTF-8 and split ANSI sequences, color attributes, alternate-screen restoration, queued resets after a partial CSI or unfinished UTF-8 character, 10,000 progress updates and ACK epochs. CI runs this small focused suite. Rust tests check native Unix PTY control bytes and resizing, and server credit exhaustion/stale ACK rejection. They complement rendering checks; they do not exercise the DOM renderer or Windows ConPTY.

Local validation also uses the actual `TerminalView` with the app CSS in Chrome and WebKitGTK 2.50: progress redraw, Unicode, hidden output, zoom without reconnecting, ordered reset and parser ACKs. An isolated daemon and Chrome exercise a real PTY producing more than 1 MB of alternate-screen progress output, tab switching and Hide, plus a deliberately stalled WebSocket snapshot. No production daemon or real harness TUI is stopped or used for these checks. All seven parser tests, all 19 Rust tests (18 core, one server), the four existing attachment tests, strict pinned-toolchain workspace Clippy and the production UI build pass. Chrome and WebKitGTK 2.50 view checks pass. The live stalled snapshot stopped at exactly 256 KiB, advanced after ACK, and left the daemon responsive; tab/Hide checks kept the existing sockets and screens after more than 1 MB of output. Test terminals were closed and removed. These local results are not a cross-platform performance certification.

## Remaining limits and acceptance

- Reloading a window, opening a second window or recovering a viewer that fell behind still replays only the last 1 MB of **raw bytes**. It is not a serialized screen. Truncation can lose mode/color setup or start inside an escape/UTF-8 sequence; replay at the latest dimensions cannot reproduce historical resizes. Preserving visited emulators removes this recovery path from ordinary tab/workspace/Hide operations, but does not solve complete screen restoration. A daemon-owned terminal state/checkpoint would need a separately scoped design and compatibility tests.
- Two visible viewers with different sizes share one PTY. Since 2026-10-13 the size follows one owner viewer (the first one, then the one that typed or clicked Use this size); other viewers render letterboxed at the PTY size instead of resizing it. Each window still has no independent PTY size.
- Continuous output that exceeds parsing capacity can still cause a snapshot resync. Memory growth at the viewer is bounded; this is not a lossless unlimited output archive or a universal throughput guarantee. Hidden/background browser scheduling may delay parsing and ACKs.
- The default DOM renderer remains selected for WebKitGTK compatibility. Switching to WebGL needs native Linux/webview evidence and a fallback; it is not necessary to support carriage-return progress.
- Unicode grapheme/emoji widths and platform fonts need representative real CLI checks. Native macOS and Windows rendering/ConPTY daily use, and the reported failing command, remain acceptance work.
- The daemon still owns every process; restarting it ends terminals. These changes do not add restart survival.

For daily use, try the originally failing CLI plus a progress-heavy download/package install, a narrow-window multiline progress display and a full-screen program such as `vim` or `htop`. Check redraws while live, switching between two terminals, hiding/showing the dock, zooming/resizing and reloading. Close the terminals afterward. Report reload-only corruption separately from live redraw failures because their recovery mechanisms differ.

# Spike 4: WebKitGTK performance

Date: 2026-10-02. Tauri 2.12 (wry 0.57), WebKitGTK 2.50.4 (Ubuntu 22.04), GNOME 42 on Wayland, Intel Iris Xe (Tiger Lake, 4 cores / 8 threads). Google Chrome 154 for comparison.

**Result: WebKitGTK is good enough. The Tauri shell stays, and the Chromium fallback is not needed now.** On an idle machine, the virtualized diff and session scroll at 60 fps. Under full CPU load (all 8 threads busy, as during a large build), WebKitGTK drops to about 20 fps where Chrome stays at 60 fps. The UI rules below keep that cost low; the check is repeated in Phase 3 with the real diff view. A cold start of the window takes about 0.5 s, so the prod shell stays warm in the background and opens in about 100 ms.

## Method

The hidden page `/dev/perf` (`web/src/features/dev/PerfView.tsx`) builds synthetic data and renders it with the same stack as the app (React 19, Tailwind classes, TanStack Virtual):

- **diff:** 5,000 virtualized diff lines, 20 px each, two line-number columns and 3 to 10 token spans per line (about the DOM of syntax highlighting);
- **session:** 2,000 virtualized events of measured height: user messages, assistant paragraphs with code blocks, one-line tool calls, small inline diffs;
- **plain-diff:** the same 5,000 lines without virtualization, for reference.

`/dev/perf?auto=1` runs the three tests one after the other. Each test scrolls from top to bottom in `requestAnimationFrame` (40 px per frame for the diff, 30 px for the session, for at most 20 s), records the time between frames, and writes the results to the daemon log. Memory is the `VmRSS` of the `WebKitWebProcess`, sampled every second. "Under load" means 8 busy loops (`while :; do :; done`) in another session, like an agent build under the daemon. Commands:

```sh
LAUNCHPAD_PROFILE=dev ./target/release/launchpad-desktop --open "/dev/perf?auto=1"
google-chrome --user-data-dir=/tmp/chrome-perf --app="http://127.0.0.1:7318/auth?t=$(cat ~/.local/share/launchpad-dev/token)&next=%2Fdev%2Fperf%3Fauto%3D1"
journalctl --user -u launchpad | grep perf:      # or the dev daemon output
```

## Results

Frame time p50 / p95 in ms, "janky" = frames over 50 ms. Rows are memoized and `useFlushSync` is off in the WebKitGTK under-load row marked (2); the other rows use the first version of the page.

| Test | Engine | Load | Mount | fps | p50 / p95 | Janky frames |
|---|---|---|---|---|---|---|
| diff | WebKitGTK | idle | 21 ms | 60 | 17 / 18 | 0 |
| session | WebKitGTK | idle | 61 ms | 59 | 17 / 23 | 0 |
| plain-diff | WebKitGTK | idle | 1,859 ms | 60 | 17 / 17 | 0 |
| diff | Chrome | idle | 15 ms | 60 | 16.7 / 16.7 | 0 |
| session | Chrome | idle | 27 ms | 60 | 16.7 / 16.7 | 0 |
| plain-diff | Chrome | idle | 129 ms | 60 | 16.7 / 16.8 | 0 |
| diff | WebKitGTK | 8 threads busy | 86 ms | 15 | 69 / 100 | 256 |
| diff (2) | WebKitGTK | 8 threads busy | 83 ms | 20 | 51 / 70 | 205 |
| session | WebKitGTK | 8 threads busy | 282 ms | 20 | 30 / 113 | 167 |
| session (2) | WebKitGTK | 8 threads busy | 351 ms | 23 | 28 / 92 | 189 |
| plain-diff | WebKitGTK | 8 threads busy | 6,478 ms | 45 | 21 / 34 | 5 |
| diff | Chrome | 8 threads busy | 23 ms | 59 | 16.7 / 16.8 | 1 |
| session | Chrome | 8 threads busy | 60 ms | 59 | 16.7 / 16.8 | 0 |
| plain-diff | Chrome | 8 threads busy | 574 ms | 55 | 16.7 / 33.3 | 2 |

Pure JavaScript (no DOM) is close in both engines, so the JIT is on: `fib(30)` 12 ms in WebKitGTK and 8 ms in Chrome, sorting 300,000 numbers 70 ms and 65 ms, creating 1,000,000 objects 113 ms and 58 ms. The difference is in DOM creation, style and layout.

Other checks under load, none of which helped WebKitGTK:

- Busy loops at `nice 10`, and their whole session at autogroup nice 10: same frame rates. With all 8 hardware threads busy, the CPU runs at a lower clock and WebKit shares a core with a busy thread; priorities cannot fix that. (On this machine, user services also cannot get a `CPUWeight`: the user manager only delegates the `memory` and `pids` controllers.)
- `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` (forced CPU painting): slightly worse (diff 14 fps), so the default already paints with the GPU.

The first run happened during a `cargo build --release` and gave the same picture as the "8 threads busy" rows (diff 8 fps, plain-diff mount 9.7 s).

## Memory

| Process | RSS |
|---|---|
| `launchpad-desktop` (UI process) | 181 MB |
| `WebKitWebProcess`, app shell loaded | 212 MB |
| `WebKitWebProcess`, perf page with the virtualized views | 234 to 303 MB |
| `WebKitWebProcess`, peak with the 5,000-line plain diff mounted | 493 MB |
| `WebKitNetworkProcess` | 57 MB |

## Startup

Release build, warm daemon, measured from the start of the shell process to the second animation frame of the page (logged by the UI):

- Cold start of the shell process: 456 to 535 ms (5 runs). The page itself takes 65 to 137 ms; the rest is GTK, WebKit process startup and the `/auth` redirect.
- Warm shell (window hidden, page loaded), opened by starting `launchpad-desktop` again: 100 to 107 ms (3 runs). The second process hands over to the running one through the single-instance plugin and exits.

The cold start misses the 300 ms goal, so the prod shell keeps one process warm: closing the window hides it, an autostart entry starts `launchpad-desktop --hidden` at login, and `install.sh` starts it after an install.

## Rules for the UI

- Virtualize every long list (sessions, diffs, notification lists). Never mount thousands of rows: 5,000 plain diff lines take 1.9 s to mount in WebKitGTK.
- Memoize row components so a scroll only renders new rows, and create virtualizers with `useFlushSync: false`.
- Use fixed row heights where possible (diff lines); measured heights (session events) cost more.
- Keep overscan small (6 to 10 rows).

## When to switch to Chromium

The fallback stays as planned: a Chrome or Chromium `--app=` window on the same URL, with no change to the UI. Switch if scrolling during agent builds bothers you in daily use, or if the Phase 3 diff view is not smooth on an idle machine.

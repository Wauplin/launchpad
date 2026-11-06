# Development

## Requirements

- Rust 1.93 or newer (CI pins 1.93.1).
- Node 24 (`web/.nvmrc`) and pnpm through Corepack (pinned in `web/package.json`).
- The [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for the desktop shell. On Ubuntu: `libwebkit2gtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev build-essential`.

## Build and run

Use the development environment: it has its own port (7318), configuration and data, so it never touches the production daemon (7317) and the agents it runs.

```sh
(cd web && pnpm install && pnpm build)                # UI into web/dist
LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon    # dev daemon on 127.0.0.1:7318
LAUNCHPAD_ENV=dev cargo run -p launchpad -- url       # a link that signs a browser in
```

- A debug daemon reads `web/dist` from disk: rebuild the UI without restarting the daemon. Release builds embed the UI.
- Hot reload: `(cd web && pnpm dev)` serves the UI on http://localhost:5173 with the API proxied to the dev daemon. Sign in with the `url` link, using port 5173.
- Desktop window: `cargo build -p launchpad-desktop && LAUNCHPAD_ENV=dev ./target/debug/launchpad-desktop`.
- Isolated daemon: set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Demo mode needs no daemon data: open `/?demo=1` on any running UI.

After changing a Rust type shared with the UI, regenerate the TypeScript types:

```sh
cargo run -p launchpad -- gen-types    # writes web/src/gen/
```

## Screenshots

`web/scripts/shots.mjs` takes screenshots of a list of pages at several zoom levels and in the phone layout. It needs no extra package and no server to start: it serves this checkout's `web/dist` from a small server inside the script, and that server forwards API calls to the daemon. It runs `pnpm build` first when `web/src` is newer than `web/dist`.

```sh
node web/scripts/shots.mjs --demo --views 100,120,150,phone / /agents /bots     # demo data, no daemon
node web/scripts/shots.mjs --views 120,phone / settings=/settings/workspace     # real data from the dev daemon (port 7318)
```

- Each file is `<out>/<prefix><view>-<name>.png` (default folder `/tmp/lp-shots`). A page is `/path` or `name=/path`.
- For each screenshot, the script prints a line with the problems it found: sideways scrolling, bar items that leave the screen or overlap, page errors, failed requests. It exits with code 1 when a screenshot has a problem.
- Real data: the first run signs in to the daemon with its token (`~/.local/share/launchpad-dev/token`, or `--token-file`) as the device "Screenshots (headless)". The cookie is kept in `~/.cache/launchpad-shots/` for later runs. The script refuses the production daemon (port 7317). Use `--daemon http://127.0.0.1:<port>` for a scratch daemon.
- Clicks before a screenshot: `--pages <file.json>` with entries like `{"name": "launch", "path": "/", "click": ["text=Launch"]}`. A click is a CSS selector or `text=<button label>`. Entries can also have `eval` (JavaScript) and `wait` (milliseconds).
- Page explanations and the welcome dialog stay closed (`--first-visit` shows them). `--wait 1500` gives syntax colors in Files time to load. `--base http://127.0.0.1:5173` uses a running `pnpm dev` server instead of `web/dist`.
- Several runs can go at the same time: each one uses its own Chrome profile, server port and browser contexts. Run `node web/scripts/shots.mjs --help` for every option.

## Checks

```sh
cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings
cargo test
(cd web && pnpm typecheck)
(cd web && pnpm test:<name>)    # attachments, chat, code, composer, drafts, markdown,
                                # operations, proposals, sessions, terminals, tour
```

Install the pinned toolchain with `rustup toolchain install 1.93.1 --profile minimal --component clippy`.

Testing policy: keep tests few, fast and free of mocks. Add a test only for a small pure function with tricky edge cases (parsers, prompt rendering); no test builds a full `App`, starts processes or takes more than a second. The whole Rust run takes about one second. Check everything else by running the app in the dev environment.

## Fast iteration

- Run only the checks your change can break (see [Checks to run](../AGENTS.md#checks-to-run-keep-iteration-fast) in AGENTS.md); CI runs everything. While working, use `cargo check --all-targets`; run Clippy once before committing.
- The Rust code is split into crates (see [Code layout](architecture.md#code-layout)). Cargo rebuilds only the crate you changed and the crates that depend on it. A change in a feature crate (`harness`, `channels`, `github`, `automation`, `setup`), in `api` or in `server` checks in 1–2 s. A change in `core` rebuilds every crate (about 9 s for `cargo check`, 20 s for Clippy or `cargo test`).
- Run checks and tests from the repository root without `-p`: a single-crate selection resolves different dependency features and recompiles dozens of dependencies.
- Debug builds keep only line tables for Launchpad's crates and no debug info for dependencies, and the libraries have no doctests.
- The commit stamp is computed by `crates/cli/build.rs` and set at startup (`launchpad_core::set_build`): a new commit relinks only the binary. Nothing in the build watches the Git index or `main`.
- Agents work in fresh worktrees, each with its own `target/`. When `sccache` is installed on a runner (`cargo install sccache --locked`), contained sessions use it (`RUSTC_WRAPPER`), with one cache per workspace (`<data_dir>/caches/<workspace>/sccache`) and one server per session (a socket in the session's `/tmp`). Compiled dependencies are then shared by every session of the workspace.

## Continuous integration

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml):

- On pushes to `main`, pull requests and tags: Clippy, UI type checks and UI tests on Linux, and debug tests on Linux.
- On tags only: release builds and tests on Ubuntu 22.04 (Linux x86_64 only; macOS and Windows jobs are paused), with downloadable packages kept 90 days. The build steps live in [`.github/workflows/build.yml`](../.github/workflows/build.yml), shared with releases.

[`.github/workflows/release.yml`](../.github/workflows/release.yml) makes a release when started by hand (Actions → Release → Run workflow, on the branch to release). It tags the branch's latest commit `vYYYY.MM.DD.N` (UTC date; `N` counts that day's releases from 1, so several releases per day are possible), runs the release build and tests, and creates a GitHub release with generated notes and the `launchpad-linux-x86_64.tar.gz` package and its SHA-256 file. Runs wait for each other, so two runs never pick the same tag. The tag it pushes does not start `ci.yml` again.

[`.github/workflows/hf-space.yml`](../.github/workflows/hf-space.yml) builds and publishes the Hugging Face Space image on every push to `main` (see [Hugging Face](hugging-face.md#updates)).

## Other components

- **Chrome extension** (on hold, not built in CI or included in releases): `cd extension && pnpm install --frozen-lockfile && pnpm build` after installing the `web/` dependencies. See [extension/README.md](../extension/README.md).
- **Hugging Face Space**: see [deploy/hf-space/README.md](../deploy/hf-space/README.md).
- **Icons**: `packaging/icons/source.png` is the master. Regenerate exports with `python3 packaging/generate-icons.py`.
- **New harness**: see [Harness adapters](harness-adapters.md).

## Agent runs with several steps

For a plan with several steps, the skill [`.claude/skills/implement-review-docs`](../.claude/skills/implement-review-docs/SKILL.md) runs three fresh subagents per step: an implementer, a reviewer and a docs agent. The docs agent of one step runs at the same time as the implementer of the next step. The skill's `prose-check.py` runs proselint and a reading-ease score on the Markdown lines a change adds.

## Conventions

- [AGENTS.md](../AGENTS.md) has the rules for coding agents, including what automated checks must never do (publish to GitHub, restart the production daemon, change the machine's configuration).
- UI words follow the [glossary](glossary.md).
- Documentation is written in plain technical English, one line per paragraph.

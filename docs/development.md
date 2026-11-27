# Development

## Requirements

- Rust 1.98 or newer (CI pins 1.98.0).
- Node 24 (`web/.nvmrc`) and pnpm through Corepack (pinned in `web/package.json`).
- Linux, with bubblewrap, for the dev runner (agents run only on Linux).

## Build and run

Use the development environment: a dev Home and a dev runner with their own ports, configuration and data, so they never touch the production Home (7317), the production runner (7320) and the agents they run. There is no single-process mode: the dev Home runs no agents, and the dev runner runs them.

```sh
(cd web && pnpm install && pnpm build)                                   # UI into web/dist
LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon                       # dev Home on 127.0.0.1:7318
LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon --role runner         # dev runner on 127.0.0.1:7321
LAUNCHPAD_ENV=dev cargo run -p launchpad -- connect http://127.0.0.1:7318  # pair once
LAUNCHPAD_ENV=dev cargo run -p launchpad -- url                          # a link that signs a browser in to the dev Home
```

- Pair once: `connect` prints a code; approve it in the dev Home's **Settings → Home and runners**. The dev runner keeps the link after restarts. Its data is in `~/.local/share/launchpad-runner-dev`.
- `url` with `LAUNCHPAD_ROLE=runner` signs a browser in to the dev runner's own window (its status page).
- A debug daemon reads `web/dist` from disk: rebuild the UI without restarting the daemon. Release builds embed the UI.
- Hot reload: `(cd web && pnpm dev)` serves the UI on http://localhost:5173 with the API proxied to the dev Home. Sign in with the `url` link, using port 5173.
- Isolated daemon: set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Demo mode needs no daemon data: open `/?demo=1` on any running UI.

Home runs three programs: `monty` (the Monty worker) and `ruff` for oracles, and `slackline` for agents' `launchpad slack` calls. Recall, when it is on, also runs `funes` (`LAUNCHPAD_FUNES`, else `PATH`; built from huggingface/funes at the commit pinned in `release.json`) with model snapshots in the Hugging Face cache that `LAUNCHPAD_FUNES_HF_HOME` names (default `<data>/recall/hf`); without them recall answers that it is not available. A dev Home finds them on its `PATH`, or at the paths in `LAUNCHPAD_MONTY`, `LAUNCHPAD_RUFF` and `LAUNCHPAD_SLACKLINE`. Use the versions pinned in `deploy/hf-home/release.json`:

```sh
uv tool install pydantic-monty-runtime==1.6.0     # prebuilt monty; or: cargo +1.98.0 install monty-runtime --version 1.6.0 --locked
uv tool install ruff==0.17.5
curl -fsSL https://github.com/co42/slackline/releases/download/v1.5.0/slackline-v1.5.0-x86_64-unknown-linux-gnu.tar.gz | tar xz -C ~/.local/bin slackline
```

slackline publishes Linux builds for x86_64 only. On another machine, build it from its repository with `cargo install --git https://github.com/co42/slackline --tag v1.5.0 --locked`.

After changing a Rust type shared with the UI, regenerate the TypeScript types:

```sh
cargo run -p launchpad -- gen-types    # writes web/src/gen/
```

## Screenshots

`web/scripts/shots.mjs` takes screenshots of a list of pages at several zoom levels and in the phone layout. It needs no extra package and no server to start: it serves this checkout's `web/dist` from a small server inside the script, and that server forwards API calls to the daemon. It runs `pnpm build` first when `web/src` is newer than `web/dist`.

```sh
node web/scripts/shots.mjs --demo --views 100,120,150,phone / /agents /bots     # demo data, no daemon
node web/scripts/shots.mjs --views 120,phone / settings=/settings/workspace     # real data from the dev Home (port 7318)
```

- Each file is `<out>/<prefix><view>-<name>.png` (default folder `/tmp/lp-shots`). A page is `/path` or `name=/path`.
- For each screenshot, the script prints a line with the problems it found: sideways scrolling, bar items that leave the screen or overlap, page errors, failed requests. It exits with code 1 when a screenshot has a problem.
- Real data: the first run signs in to the daemon with its token (`~/.local/share/launchpad-dev/token`, or `--token-file`) as the device "Screenshots (headless)". The cookie is kept in `~/.cache/launchpad-shots/` for later runs. The script refuses the production daemon (port 7317). Use `--daemon http://127.0.0.1:<port>` for a scratch daemon.
- Clicks before a screenshot: `--pages <file.json>` with entries like `{"name": "launch", "path": "/", "click": ["text=Launch"]}`. A click is a CSS selector or `text=<button label>`. Entries can also have `eval` (JavaScript) and `wait` (milliseconds).
- In demo mode the first page of a run loses its query string on the laptop (for example `?session=`). Start the run with a plain first page, like `first=/`.
- Page explanations and the welcome dialog stay closed (`--first-visit` shows them). `--wait 1500` gives syntax colors in Files time to load. `--base http://127.0.0.1:5173` uses a running `pnpm dev` server instead of `web/dist`.
- Several runs can go at the same time: each one uses its own Chrome profile, server port and browser contexts. Run `node web/scripts/shots.mjs --help` for every option.

## Checks

```sh
cargo +1.98.0 clippy --workspace --all-targets --locked -- -D warnings
cargo test
(cd web && pnpm typecheck)
(cd web && pnpm lint:hooks)
(cd web && pnpm test:<name>)    # attachments, chat, code, composer, drafts, folios, markdown,
                                # operations, proposals, review, sessions, terminals, tour
(cd web && pnpm ui-check)       # off-scale UI classes (not in CI)
(cd deploy/hf-home && python -m unittest discover -s tests)   # in a venv with requirements.txt
```

Install the pinned toolchain with `rustup toolchain install 1.98.0 --profile minimal --component clippy`.

`pnpm ui-check` runs `web/scripts/ui-check.mjs`. It reads `web/src` (except `gen/`) and lists each class or call that breaks the Sizes and Dates rules of the [UI rules](invariants/ui.md): Tailwind text sizes and text sizes outside 11/12/13/15/18 px, corner radii other than `rounded-control` and `rounded-card`, arbitrary gaps, widths of 700 px or more (use `max-w-page` or `max-w-text`), hex and Tailwind palette colors, forced `size-…!` classes, icon sizes outside 12/14/16, and `toLocale…String` on dates. It prints `file:line rule match` and a count per rule, and exits with 1 when it finds something. The allowlist at the top of the script excuses a few files and matches, each with its reason (for example the date helpers, the terminal themes and the phone icon sizes). It does not read `styles.css`. It needs no dependencies and takes under a second.

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

- On pushes to `main`, pull requests and tags: Clippy, UI type checks and UI tests, debug tests, the `deploy/hf-home` Python tests and the extension's type check and build, on Linux only.
- On every trigger: the `desktop` job checks the desktop companion (`cargo check`, Clippy and tests with `--manifest-path desktop/Cargo.toml --locked`) in an `ubuntu:26.04` container, which provides GTK 4 (`libgtk-4-dev`). It runs beside the other jobs.
- On tags and pushes to `main`: release builds on Ubuntu 22.04 (Linux x86_64). A push to `main` attaches its package to the `main-build` prerelease, named by commit (the 20 newest are kept); `deploy.sh --main` deploys it. The build steps live in [`.github/workflows/build.yml`](../.github/workflows/build.yml), shared with releases.

Release builds keep dependencies in `rust-cache` and cache Launchpad's library compilation with `sccache` in GitHub Actions storage (`RUSTC_WORKSPACE_WRAPPER: sccache`). The UI's `rust-embed` derive sits in the binary crate (`crates/cli`), never in a library: sccache never caches a binary, so the embedded file list is read again on every build, and the libraries, the server included, stay cacheable. A commit that leaves `crates/core`, `crates/server` and `crates/http` unchanged then recompiles only the binary. The action reports cache statistics at the end of the job; compare warm runs when checking the benefit. Each release build and the Tests job upload Cargo's timing report (`cargo-timings-release`, `cargo-timings-tests`: one HTML page with each crate's compile time), kept 14 days. The package is compressed with `pigz` (same gzip format and size as `tar -z`, about half the time).

[`.github/workflows/release.yml`](../.github/workflows/release.yml) makes a release when started by hand (Actions → Release → Run workflow, on the branch to release). It tags the branch's latest commit `vYYYY.MM.DD.N` (UTC date; `N` counts that day's releases from 1, so several releases per day are possible), runs the release build and tests, and creates a GitHub release with generated notes the `launchpad-linux-x86_64.tar.gz` package and its SHA-256 file, and `launchpad-chrome-extension.zip` (a zip of `extension/dist`, also inside the package's `extension/`). Runs wait for each other, so two runs never pick the same tag. The tag it pushes does not start `ci.yml` again.

The desktop companion's package is built by the `desktop` job of `build.yml`, in the same `ubuntu:26.04` container, with read-only repository access. A tag `vYYYY.MM.DD.N` gives the version `YYYY.MM.DD.N` (channel release). A `main` build gives `YYYY.MM.DD.0~main.HHMMSS.g<sha7>` (channel main, from the commit's UTC time), which sorts below that day's release. The `publish-desktop` job then attaches `launchpad-desktop_<version>_amd64.deb` and its `.sha256` to the release, or to `main-build` (keeping the 20 newest packages). The desktop builds (this job and the `desktop` check job of `ci.yml`) read the Hugging Face OAuth client ID from the repository variable `HF_CLIENT_ID`, the one the Chrome extension's builds use: one public Hugging Face OAuth app (no secret, scopes `openid`, `profile` and `read-repos`) serves both, with the redirect URIs `https://bbefehapjjlbeodihifmpgidgjhmcklj.chromiumapp.org/` (Chrome) and `http://127.0.0.1/callback` (the companion's loopback callback, any port). A local build takes `LAUNCHPAD_HF_CLIENT_ID` from its environment. Without a client ID, the companion's sign-in falls back to a pasted Hugging Face token.

## Other components

- **Folios**: the page splitter exists twice, in Rust (`crates/automation/src/folio_split.rs`) and TypeScript (`web/src/lib/folioSplit.ts`), with one fixture folder for both, `web/tests/folio-split/` (`<name>.md` and its expected outline `<name>.json`, read by `pnpm test:folios` and `cargo test shared_fixtures`). Change both together. `FOLIO_FIXTURES_WRITE=1 cargo test shared_fixtures` writes the JSON of a new fixture; check it by hand. The block catalog lives in `folio_catalog.rs`; `gen-types` writes it to `web/src/gen/folioCatalog.ts`. `pnpm build` also builds the folio viewer (`web/folio-viewer/`, `src/viewer/`) into `web/dist/folio-viewer/`, which shared snapshots use. Test sharing with `LAUNCHPAD_FOLIO_SHARE_DIR` on a dev Home, never with a real Space.
- **Chrome extension**: `cd extension && pnpm install --frozen-lockfile && pnpm typecheck && pnpm build` after installing the `web/` dependencies. Set `VITE_HF_CLIENT_ID` in `extension/.env.local` (see `.env.example`) to build with a Hugging Face client ID. Load `extension/dist` unpacked in Chrome and connect it to the dev Home with **Use an address** (`http://127.0.0.1:7318`) and a pairing code. Never connect a test build to your real Space. See [extension/README.md](../extension/README.md).
- **Desktop companion**: `desktop/` is a Cargo workspace of its own (own `Cargo.lock`, excluded from the root workspace), so the root checks never need GTK. It needs the GTK 4 development files: on Ubuntu 26.04, `sudo apt install libgtk-4-dev`. Then run `cargo check --manifest-path desktop/Cargo.toml --all-targets`, `cargo clippy --manifest-path desktop/Cargo.toml --all-targets -- -D warnings` and `cargo test --manifest-path desktop/Cargo.toml`. Agents on runners without root put the packages in a local sysroot instead (see [desktop/README.md](../desktop/README.md)). GNOME shows its notifications, binds its shortcut and draws its dock badge only once `desktop/packaging/dev.launchpad.Desktop.desktop` is installed (for a dev run, copy it to `~/.local/share/applications`). `cargo run --manifest-path desktop/Cargo.toml --example probe` checks pairing, `/api/version` and the event stream against a scratch dev Home, with no desktop or keyring; never point it at a real Home. `--example update_probe` checks the update download and its verification against a local fake release server (`LAUNCHPAD_DESKTOP_UPDATE_API`, debug builds only), never GitHub. To build the package, run `desktop/packaging/build-deb.sh <version>` on Ubuntu 26.04 after a release build with `LAUNCHPAD_DESKTOP_VERSION` and `LAUNCHPAD_DESKTOP_CHANNEL` (`release` or `main`) set like CI. It writes `desktop/dist/launchpad-desktop_<version>_amd64.deb` and its `.sha256`. A build without these variables is channel `dev` and checks for no update. See [Desktop companion](desktop.md).
- **Home in a Hugging Face Space**: see [deploy/hf-home/README.md](../deploy/hf-home/README.md). Test `deploy.sh` only with a throwaway `--space`, and delete that Space and its bucket afterwards.
- **Local VM runner**: `packaging/local-vm.sh <Home address>` creates one with Multipass.
- **Icons**: `packaging/icons/source.png` is the master. Regenerate exports with `python3 packaging/generate-icons.py`.
- **New harness**: see [Harness adapters](harness-adapters.md).

## Agent runs with several steps

For a plan with several steps, the skill [`.claude/skills/implement-review-docs`](../.claude/skills/implement-review-docs/SKILL.md) runs three fresh subagents per step: an implementer, a reviewer and a docs agent. The docs agent of one step runs at the same time as the implementer of the next step. The skill's `prose-check.py` runs proselint and a reading-ease score on the Markdown lines a change adds.

## Conventions

- [AGENTS.md](../AGENTS.md) has the short rules for coding agents, including what automated checks must never do (publish to GitHub, restart the production Home or runner, change the machine's configuration). Keep it short: the design invariants of each area live in [docs/invariants/](invariants/README.md).
- UI words follow the [glossary](glossary.md).
- Documentation is written in plain technical English, one line per paragraph.

## Commands in detail

The full list of commands and their constraints; [AGENTS.md](../AGENTS.md#commands) has the short version.

- UI: `cd web && pnpm install && pnpm build` (also builds the folio viewer into `web/dist/folio-viewer/`; Node 24 from `web/.nvmrc`, pnpm through Corepack). A debug daemon reads `web/dist` from disk.
- Dev setup: a dev Home plus a dev runner, paired once. Home: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon` (port 7318, data in `~/.local/share/launchpad-dev`; it runs no agents). Runner: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon --role runner` (port 7321, data in `~/.local/share/launchpad-runner-dev`). Pair once: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- connect http://127.0.0.1:7318`, then approve the code in the dev Home's Settings → Home and runners; the runner keeps the link. `--role home` is accepted as a no-op; `--role all` is refused (there is no single-process mode). For an isolated second daemon, set `HOME` to a scratch folder and `LAUNCHPAD_PORT` to a free port.
- Sign a browser in to the dev Home: `LAUNCHPAD_ENV=dev cargo run -p launchpad -- url` (`pair` prints a QR code; add `LAUNCHPAD_ROLE=runner` for the dev runner's own window).
- Screenshots: `node web/scripts/shots.mjs [--demo] --views 100,120,150,phone [--out <dir>] <page>...` (pages `/path` or `name=/path`; clicks with `--pages <file.json>`). It serves this checkout's `web/dist` (rebuilt when stale) and forwards the API to the dev daemon (`--daemon`, default port `$LAUNCHPAD_PORT` else 7318; agent sessions inherit `LAUNCHPAD_PORT=7320`, the production runner, so pass `--daemon http://127.0.0.1:7318` there), signs in once by itself, and reports sideways scrolling and page errors. Never write another screenshot script or start a server for it. See `docs/development.md#screenshots`.
- Demo mode: `/?demo=1` on any running UI (`?demo=essentials:layout`, `?demo=full:<chapter>`, `?demo=phone`), no pairing or real data needed.
- Runners: `launchpad daemon --role runner` (own folders), paired with `launchpad connect <Home address>` and approved in Home's Settings → Home and runners. Test pairs use two scratch `HOME`s and ports; remove test runners afterwards. Contained sessions need `bwrap` and a harness login in the vault. `packaging/local-vm.sh <Home address>` creates a Local VM runner with Multipass.
- Home-only Space POC: see `deploy/hf-home/README.md`. `deploy/hf-home/deploy.sh [--release <tag> | --main] [--space <name>]` installs or updates a Space with the hf CLI (latest release by default; `--main` uses CI's package of the newest packaged main commit from the `main-build` prerelease; nothing compiles on the Space builder); test it only with a throwaway `--space` and delete that Space and its bucket afterwards (runners use port 7320 and their own folders; never use the production installer for it). Checks: `cd deploy/hf-home && uv run -q --no-project --with-requirements requirements.txt python -m unittest discover -s tests` (its Python packages, on any runner). `python3 deploy/hf-home/bump_tools.py [--major] [--dry-run]` bumps the pins of `release.json` (Home's image and Burst: the agent CLIs from npm, the oracle binary ruff, and slackline for `launchpad slack`) to the newest release of their major version; the `monty` worker's pin changes only together with the monty crates of `crates/automation/Cargo.toml` (same protocol).
- After changing a Rust type shared with the UI: `cargo run -p launchpad -- gen-types`.
- Oracles and Slack: Home runs the `monty` worker (monty-pool) and `ruff` for oracles and `slackline` for `launchpad slack`, from PATH, or `LAUNCHPAD_MONTY`, `LAUNCHPAD_RUFF` and `LAUNCHPAD_SLACKLINE` (Home's image pins them in `deploy/hf-home/release.json`; see `docs/development.md` for a dev Home). Never give the dev slackline a real token in checks.
- Lint (CI): `cargo +1.98.0 clippy --workspace --all-targets --locked -- -D warnings`, `(cd web && pnpm typecheck)` and `(cd web && pnpm lint:hooks)` (toolchain: `rustup toolchain install 1.98.0 --profile minimal --component clippy`).
- Tests (CI): `cargo test`, and `(cd web && pnpm test:<name>)` for attachments, chat, code, composer, drafts, folios, markdown, operations, proposals, review, sessions, terminals, tour and usage (Node 24), and the `deploy/hf-home` Python tests (`requirements.txt` in a venv). The folio splitter has one fixture folder for both languages, `web/tests/folio-split/` (`<name>.md` and its expected outline `<name>.json`, read by `pnpm test:folios` and `cargo test shared_fixtures`; `FOLIO_FIXTURES_WRITE=1 cargo test shared_fixtures` writes the JSON of a new fixture, check it by hand).
- Chrome extension: `cd extension && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test:suggestions && pnpm build` after installing `web/` dependencies (the Hugging Face OAuth client ID comes from `VITE_HF_CLIENT_ID` in `extension/.env.local`, see `.env.example`; checked and built in CI; release and main builds read the client ID from `vars.HF_CLIENT_ID`; release builds put `launchpad-chrome-extension.zip`, a zip of `extension/dist`, in the package's `extension/` and attach it to the GitHub release); see `extension/README.md`.
- Desktop companion: `cargo check|clippy --all-targets -- -D warnings|test --manifest-path desktop/Cargo.toml` (own `Cargo.lock`, excluded from the root workspace, so root commands never need GTK); it needs the GTK 4 development files (`libgtk-4-dev`, GTK 4.8 or newer; feature flags stay at `v4_8`); see `desktop/README.md` for a local sysroot without root. `cargo run --manifest-path desktop/Cargo.toml --example probe` links to a scratch dev Home through reverse pairing without a desktop or keyring (never a real Home); `--example update_probe` checks the update download and verification against a local fake release server (`LAUNCHPAD_DESKTOP_UPDATE_API`, debug builds only; never GitHub). Package: `desktop/packaging/build-deb.sh <version>` turns a release build into `launchpad-desktop_<version>_amd64.deb` and its `.sha256` (binary, `dev.launchpad.Desktop.desktop` with its actions, hicolor icons from `packaging/icons`; Depends from `dpkg-shlibdeps`, or `--depends` for a sysroot build); the build stamps `LAUNCHPAD_DESKTOP_VERSION` and `LAUNCHPAD_DESKTOP_CHANNEL` (`release`, `main`; a local build is `dev` and checks no update). Installer: `curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/desktop/packaging/install.sh | bash` (`| bash -s -- --release` for the latest release) checks Ubuntu 26.04 amd64, picks the newest `.deb` of `main-build` like the updater (GitHub turns the version's `~` into `.` in asset names), verifies it against GitHub's digest, the `.sha256` and `dpkg-deb -f`, then runs `sudo apt-get install`; it reads the API with `GH_TOKEN`, `GITHUB_TOKEN` or `gh auth token` when set (sent only to api.github.com) and falls back to the release's `expanded_assets` web page when the API refuses; it never uses Python (release JSON through `jq` when it works, else its built-in grep and awk reader, each tested on a sample before any fetch); check it only with `--dry-run` and `LAUNCHPAD_DESKTOP_INSTALL_API` pointing at a local fake release server (never apt or sudo on a runner). For `desktop/` changes run only these checks, never the root ones.
- Icons: keep `packaging/icons/source.png` unchanged; regenerate with `python3 packaging/generate-icons.py`.
- Network capture: `launchpad netcap --self-test` checks the supervisor on this machine; `launchpad netcap --port <capture port> --key <proxy key> -- <program>` runs a program under it; Settings → Home and runners → **Check network** probes a runner's sandbox.
- Agent CLI: read-only context of the session's workspace `launchpad folders|bots|sessions|profiles|presets|schedules|proposals list`, `launchpad egress list` (exit routes, Launchpad-wide), `launchpad sessions show <id>`, `launchpad today`, `launchpad activity list`, `launchpad oracle list|show|history|run|test`, `launchpad folio list|outline|read|check|feedback|items|revs|blocks|rules|data list|data get` (reads) and `create|write|append|patch|add-media|rm|done|reply|resolve|data set|data rm|version` (propose access, own writes); diagnostics at admin access `launchpad health`, `launchpad logs [--session <id>] [--runner <id>|--home|--local] [--since 2h] [--level warn] [--grep …]`, `launchpad failures`, `launchpad runners list`, `launchpad burst runs|job <runner>`, `launchpad home`, `launchpad tools list`, `launchpad connections list`, `launchpad hops` (the hop count for `X-Launchpad-Hops`); `launchpad propose …` (inert proposal; `propose ready` prepares an owner click); `launchpad suggest comment|review|action` and `launchpad note` with action flags (inert suggestions); `launchpad ci approve` (an agent approves a fork's waiting workflow runs at the head it read, checked on Home); `launchpad do <group>.<verb> <ids…> [--<field> <value>] [--body-file <file>]` and the short verb `launchpad <group> <verb> …` (the admin actions of the action list); `launchpad api <method> <path> [--body-file]` (the API at admin access); `launchpad pr create|update|show|list` (Local PRs); `launchpad send <connection>|telegram --body-file <file> [--title …] --session-id <id>`; `launchpad sleep --in 2h|--until <RFC 3339> [--note …]` (end the turn and be woken later in the same session); `launchpad slack --profile <handle> <slackline args>` (Slack through Home with one of the session's Slack profiles); `launchpad conversation read [--since <id>]` (a bot thread's conversation with other bots); `launchpad ref read|resolve <link or lp: ID> [--before N] [--after N]` (a Launchpad reference of the session's workspace as a bounded excerpt).
- CI (`.github/workflows/ci.yml`): lint and debug tests on main, pull requests and tags; Linux only (no macOS or Windows jobs for now); release builds on tags and on main pushes (main pushes attach the package to the `main-build` prerelease, named by commit, keeping 20), on Ubuntu 22.04 (verify a downloaded binary before changing that baseline); build the UI before the release daemon so it embeds the assets. The release build is `.github/workflows/build.yml`, called by `ci.yml` on tags and by `.github/workflows/release.yml` (manual: tags the commit `vYYYY.MM.DD.N`, UTC, and creates the GitHub release with the package and the extension zip). The tests job also runs the `deploy/hf-home` Python tests; the extension job type-checks and builds `extension/`; the desktop job checks, lints and tests `desktop/` in an `ubuntu:26.04` container with `libgtk-4-dev`. `build.yml`'s `desktop` job builds the companion's `.deb` in the same container (version `YYYY.MM.DD.N` from the tag, else `YYYY.MM.DD.0~main.<HHMMSS>.g<commit>`, which sorts below that day's releases; OAuth client ID from `vars.HF_CLIENT_ID`, the Chrome extension's public OAuth app, which also registers the loopback redirect `http://127.0.0.1/callback`; the `desktop` check job of `ci.yml` builds with it too), and `publish-desktop` attaches it and its `.sha256` to the release or to `main-build` (the 20 newest kept), after the package job. Release builds also use `sccache` for every workspace library (`RUSTC_WORKSPACE_WRAPPER: sccache`); the UI's `rust-embed` derive sits in the binary crate (`crates/cli/src/extensions.rs`, handed to the server with `launchpad_server::set_assets`), which sccache never caches, so added and removed UI assets are always seen: keep it out of library crates. Release builds pack with `pigz` and upload Cargo's timing report (`cargo-timings-release`; the Tests job uploads `cargo-timings-tests`). Keep CI and this file in sync.
- Production install/update: `packaging/install.sh` (Linux only, `--headless`, `--prebuilt`) installs a Home (`packaging/launchpad.service` runs `launchpad daemon`, no agents); a machine that runs agents installs a runner (`deploy/hf-home/install-runner.sh` from Home, or `launchpad daemon --role runner` with `launchpad connect`). It restarts the production daemon: only run it when the owner asks.

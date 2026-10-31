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
- Demo mode needs no daemon data: open `/agents?demo=1` on any running UI.

After changing a Rust type shared with the UI, regenerate the TypeScript types:

```sh
cargo run -p launchpad -- gen-types    # writes web/src/gen/
```

## Checks

```sh
cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings
cargo test
(cd web && pnpm typecheck)
(cd web && pnpm test:<name>)    # attachments, code, composer, drafts, markdown, operations,
                                # proposals, sessions, terminals, tour
```

Install the pinned toolchain with `rustup toolchain install 1.93.1 --profile minimal --component clippy`.

Testing policy: keep tests few, fast and free of mocks. Add a test only for a small pure function with tricky edge cases (parsers, prompt rendering). Check everything else by running the app in the dev environment.

## Continuous integration

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml):

- On pushes to `main`, pull requests and tags: Clippy, UI type checks and UI tests on Linux, and debug tests on Linux, macOS and Windows.
- On tags only: release builds and tests on Ubuntu 22.04, macOS 15 (ARM64 and Intel) and Windows 2025, with downloadable packages kept 90 days.

[`.github/workflows/hf-space.yml`](../.github/workflows/hf-space.yml) builds and publishes the Hugging Face Space image on every push to `main` (see [Hugging Face](hugging-face.md#updates)).

## Other components

- **Chrome extension**: `cd extension && pnpm install --frozen-lockfile && pnpm build` after installing the `web/` dependencies. See [extension/README.md](../extension/README.md).
- **Hugging Face Space**: see [deploy/hf-space/README.md](../deploy/hf-space/README.md).
- **Icons**: `packaging/icons/source.png` is the master. Regenerate exports with `python3 packaging/generate-icons.py`.
- **New harness**: see [Harness adapters](harness-adapters.md).

## Conventions

- [AGENTS.md](../AGENTS.md) has the rules for coding agents, including what automated checks must never do (publish to GitHub, restart the production daemon, change the machine's configuration).
- UI words follow the [glossary](glossary.md).
- Documentation is written in plain technical English, one line per paragraph.

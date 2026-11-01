# Launchpad on Hugging Face Spaces

A headless Launchpad in a private Docker Space, with a small control plane in front of it: browser-only setup, a private backup bucket, a separate encrypted credential vault and explicit image updates. Everything Space-specific lives in this folder; Launchpad itself only gained generic hooks (trace prefix, delegated-access presentation, packaged commit).

Status: built and checked locally against a real private Space and bucket. The main update channel is published by CI after merge/push; its first run requires the GHCR package to be public (see below).

## What the owner does

1. Duplicate the template Space. Keep it **Private**. Pick CPU upgrade hardware to keep agents running with the tab closed (the free CPU sleeps after 48 hours without visits).
2. Open the Space and click **Sign in to Hugging Face**. The page runs `hf auth login` and shows the sign-in link and code; after you approve access, click **Save and continue**. Launchpad saves the OAuth sign-in as the `LP_SPACE_MANAGEMENT_TOKEN` secret, creates the private bucket (same name as the Space, or `LP_SPACE_BUCKET`), creates the `LP_SPACE_BACKUP_KEY` encryption secret and restarts the Space once.
3. **Connect your first agent**: Hugging Face (`hf auth login`; open models billed to the HF account), Claude Code, Codex or OpenCode (each runs its own sign-in in a terminal on the page) or a provider API key. Launchpad creates the matching profile automatically. GitHub is offered next, optionally.
4. Use Launchpad. A 36 px bar above the app shows the Space name and backup state; **Space settings** opens accounts, backups, updates and Space actions.

Each step is detected, never redone: a Space that already has its secrets or bucket skips straight ahead. A Space that is public, or a bucket that is public, stops at a page that explains how to fix it in Hub settings.

## Layout

| Path | Owns |
| --- | --- |
| `supervisor/main.py` | The setup stages, `/_space/*` API, action dispatch, the proxy front door |
| `supervisor/management.py` | The Space's own Hub sign-in: refresh and write-back of `LP_SPACE_MANAGEMENT_TOKEN` |
| `supervisor/policy.py` | The one startup visibility decision (the owner accepts protected Spaces for now) |
| `supervisor/runtime.py`, `processes.py` | The Launchpad daemon as the `agent` user, environment allowlist, crash watcher |
| `supervisor/updates.py` | Release checks, image update with expected parent, rollback |
| `gateway/` | Streaming HTTP/WebSocket proxy (`proxy.py`) and the browser shell (`shell.html`, `shell.css`, `shell.js`, xterm copied at build time) |
| `providers/` | Fixed sign-in commands in a PTY (`setup.py`), API-key and HF profiles (`keys.py`), the credential file registry (`roots.py`) |
| `backup/` | Capture and restore (`machine.py`), schema-bound credential splitting (`projection.py`), bucket store (`store.py`), the serialized worker (`worker.py`) |
| `vault/` | AES-256-GCM envelopes (`codec.py`) and planned key rotation (`rotate.py`) |
| `release.json` | Connector version, release channel (manifest URL and image), pinned CLI versions |
| `template/README.md` | The README of the duplicate template Space |

The shell talks to the supervisor only through `GET /_space/status` (with a `stage` field: `template`, `token`, `blocked`, `backups`, `starting`, `connect`, `ready`, `stopped`, `error`, `restarting`), `GET /_space/connections`, the `/_space/login` terminal socket and `POST /_space/action/<name>` with the CSRF token from the status. Keep it that way: a new feature adds an action and a render function.

## Build and run

From the repository root:

```sh
docker build -f deploy/hf-space/Dockerfile --build-arg LAUNCHPAD_REVISION="$(git rev-parse HEAD)" -t launchpad-space:local .
```

The image builds the UI before the daemon, installs the CLI versions from `release.json`, copies xterm from the UI build, records dependency inventories under `/opt/space/sbom/`, and classifies the database schema of the embedded binary (`schema.json`). `tini` forwards signals.

HF supplies `SPACE_ID` and `SPACE_HOST`. Secrets: `LP_SPACE_MANAGEMENT_TOKEN` (the `hf auth login` OAuth access and refresh tokens, as JSON) and `LP_SPACE_BACKUP_KEY`, both created by setup. OAuth access tokens expire after 30 days and each refresh rotates the refresh token: the control plane refreshes one day before expiry and writes the new pair back to the secret (Hub may restart the Space when a secret changes). If the stored sign-in no longer works at startup, the setup page asks to sign in again. Optional variable: `LP_SPACE_BUCKET=namespace/name`. Host and Origin must be exactly `https://SPACE_HOST`.

To try it locally, run the image with `SPACE_ID` of a throwaway private Space you own, `SPACE_HOST=<anything>.hf.space` and a token, put a TLS proxy in front of port 7860 and map the host in the browser (for Chrome: `--host-resolver-rules="MAP <host>:443 127.0.0.1:<tls port>" --ignore-certificate-errors`). Mount `gateway/`, `supervisor/`, `providers/`, `backup/` and `vault/` over `/opt/space/` to iterate without rebuilding.

The control plane runs as root; the daemon, agents, Git and shells run as the `agent` user in `/home/agent` with an allowlisted environment. The Space sign-in runs as root in a private folder under `/run/launchpad-space`; it, the vault key and platform secrets never reach agent processes. Agents can read their own provider credentials: this is not a hostile-code sandbox.

## Backups

- `backups/<generation>/`: credential-free SQLite snapshots, the config projection and a workspace archive, plus `manifest.json`; `backups/latest.json` points at the newest. A manifest may reference unchanged objects of an earlier generation (`path`), so an idle machine uploads nothing and an unchanged workspace is never uploaded twice. All generations are retained.
- `credentials/<generation>/vault.aead`: AES-256-GCM with per-generation HKDF keys, bound to installation, generation and key identity. Credential files are watched and reconciled every 30 seconds; unchanged files are not re-read.
- `traces/`: Launchpad's own trace exporter, enabled once Hugging Face is connected for agents.

State is saved every minute when it changed, the workspace archive every five minutes (and before maintenance), and a final checkpoint runs when HF stops the container. Rebuildable folders are skipped (`node_modules`, `target`, `venv`, `.venv`, build caches, tool temp and log folders, Codex log databases). Symlinks are kept as links. Folders outside `/home/agent` are not backed up. Archives are built in memory: very large workspaces need more RAM and upload time; measure before relying on it.

A backup failure never blocks startup: the bar shows "Backup delayed" and the worker retries. New launches and replies pause when no backup succeeded for ten minutes. A wrong or missing key opens a reconnect notice: files and settings are recovered, sign-ins must be redone, old ciphertext stays in the bucket.

## Releases and updates

Every push to `main` triggers `.github/workflows/hf-space.yml`: build the image, smoke-check isolated startup, push an immutable GHCR image, verify anonymous pulls, then publish `hf-space-release.json` and `hf-space-template.zip` on the `hf-space-channel` artifact branch. No GitHub release or version tag is needed: the exact main commit is the version. Superseded queued builds do not publish, and failed builds leave the previous working channel intact. Manual dispatch on main can retry publication.

`release.json` → `channel` names the public raw manifest URL and official image repository. **Make the `launchpad-hf-space` GHCR package public once** in its package settings; a public GitHub repo does not automatically make packages public. CI refuses to advertise an image that anonymous clients cannot inspect. The workflow needs Contents and Packages write permissions. Optional `HF_SPACE_TEMPLATE` (repository variable) and `HF_SPACE_TEMPLATE_TOKEN` (secret) refresh the public duplicate template after publication; otherwise download the template zip from `hf-space-channel`.

Running Spaces check automatically at startup and roughly every six hours (with jitter), with **Check now** in Space settings. A persistent **Update available · Review** button opens the candidate commit, tool changes and **Update and restart** confirmation. Checks only advertise a different commit; they never apply updates automatically. A failed check preserves the last candidate and reports that the check is unavailable.

Existing source-built Spaces and images using the old release-only URL need a one-time migration: after the first successful channel build, download the template zip and replace the Space Dockerfile with its managed digest-pinned Dockerfile in Hub, then rebuild. Save a current backup and stop agents/terminals first. Source-built Dockerfiles cannot use image rollback and are deliberately refused by the updater. Subsequent updates use the owner-approved flow below.

Updates need a click in Space settings, idle agents and terminals, and a verified clean checkpoint; then the managed Dockerfile is replaced at the expected parent revision. If the new build does not come back within five minutes, the restarting page offers **Roll back the update**, which restores the previous image and its pre-update checkpoint. Edited Dockerfiles are never overwritten.

See the [template README](template/README.md#recover-without-the-app) for recovery from the Hub when the app cannot start.

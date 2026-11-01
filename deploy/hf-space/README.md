# Launchpad on Hugging Face Spaces

A headless Launchpad in a private Docker Space, with a small control plane in front of it: browser-only setup, `~/projects` on a live bucket volume, a latest-only backup of everything else, a separate encrypted credential vault and explicit image updates. Everything Space-specific lives in this folder; Launchpad itself only gained generic hooks (trace prefix, delegated-access presentation, packaged commit).

Status: built and checked locally against a real private Space and bucket. The main update channel is published by CI after merge/push; its first run requires the GHCR package to be public (see below).

## What the owner does

1. Duplicate the template Space. Keep it **Private**. Pick CPU upgrade hardware to keep agents running with the tab closed (the free CPU sleeps after 48 hours without visits).
2. Open the Space. Paste one Hugging Face **write** token and click **Save and continue**. Launchpad saves it as the `LP_SPACE_MANAGEMENT_TOKEN` secret, creates the private bucket (`<namespace>/launchpad`, or `LP_SPACE_BUCKET`), creates the `LP_SPACE_BACKUP_KEY` encryption secret, adds the bucket volume for `/home/agent/projects` (keeping other volumes) and restarts the Space once.
3. **Connect your first agent**: Hugging Face (paste an agent token; open models billed to the HF account), Claude Code, Codex or OpenCode (their own sign-in, in a terminal on the page) or a provider API key. Launchpad creates the matching profile automatically. GitHub is offered next, optionally, then a repository picker that clones into `~/projects` and registers each clone as a folder (recent GitHub notifications first, then recently pushed repositories).
4. Use Launchpad. A 24 px bar above the app shows the Space name and backup state (the template uses `header: mini` to shrink the Hugging Face header, and the ▲ button folds the bar into a small tab); **Space settings** opens accounts, folders (the same repository picker), backups, updates and Space actions.

Each step is detected, never redone: a Space that already has its secrets or bucket skips straight ahead. A Space that is public, or a bucket that is public, stops at a page that explains how to fix it in Hub settings.

## Layout

| Path | Owns |
| --- | --- |
| `supervisor/main.py` | The setup stages, `/_space/*` API, action dispatch, the proxy front door |
| `supervisor/policy.py` | The one startup visibility decision (the owner accepts protected Spaces for now) |
| `supervisor/runtime.py`, `processes.py` | The Launchpad daemon as the `agent` user, environment allowlist, crash watcher |
| `supervisor/updates.py` | Release checks, image update with expected parent, rollback |
| `gateway/` | Streaming HTTP/WebSocket proxy (`proxy.py`) and the browser shell (`shell.html`, `shell.css`, `shell.js`, xterm copied at build time) |
| `providers/` | Fixed sign-in commands in a PTY (`setup.py`), API-key and HF profiles (`keys.py`), the credential file registry (`roots.py`) |
| `backup/` | Capture and restore (`machine.py`), schema-bound credential splitting (`projection.py`), latest-only bucket store and format 1 reader (`store.py`), the serialized worker (`worker.py`) |
| `vault/` | AES-256-GCM envelopes (`codec.py`) and planned key rotation (`rotate.py`) |
| `release.json` | Connector version, release channel (manifest URL and image), pinned CLI versions |
| `template/README.md` | The README of the duplicate template Space |

The shell talks to the supervisor only through `GET /_space/status` (with a `stage` field: `template`, `token`, `blocked`, `backups`, `starting`, `connect`, `ready`, `stopped`, `error`, `restarting`), `GET /_space/connections`, the `/_space/login` terminal socket and `POST /_space/action/<name>` with the CSRF token from the status. Keep it that way: a new feature adds an action and a render function. Long work runs in the background and reports through the status (for example `clones` for the `clone` action), so the action lock is never held for minutes.

## Build and run

From the repository root:

```sh
docker build -f deploy/hf-space/Dockerfile --build-arg LAUNCHPAD_REVISION="$(git rev-parse HEAD)" -t launchpad-space:local .
```

The image builds the UI before the daemon, installs the CLI versions from `release.json`, copies xterm from the UI build, records dependency inventories under `/opt/space/sbom/`, and classifies the database schema of the embedded binary (`schema.json`). `tini` forwards signals.

HF supplies `SPACE_ID` and `SPACE_HOST`. Secrets: `LP_SPACE_MANAGEMENT_TOKEN` and `LP_SPACE_BACKUP_KEY` (both created by setup). Optional variable: `LP_SPACE_BUCKET=namespace/name`. Host and Origin must be exactly `https://SPACE_HOST`.

To try it locally, run the image with `SPACE_ID` of a throwaway private Space you own, `SPACE_HOST=<anything>.hf.space` and a token, put a TLS proxy in front of port 7860 and map the host in the browser (for Chrome: `--host-resolver-rules="MAP <host>:443 127.0.0.1:<tls port>" --ignore-certificate-errors`). Mount `gateway/`, `supervisor/`, `providers/`, `backup/` and `vault/` over `/opt/space/` to iterate without rebuilding.

The control plane runs as root; the daemon, agents, Git and shells run as the `agent` user in `/home/agent` with an allowlisted environment. The management token, the vault key and platform secrets never reach agent processes. Agents can read their own provider credentials: this is not a hostile-code sandbox.

## Backups

Only the latest state is kept. Nothing in the bucket is versioned, and the connector never deletes bucket files.

The bucket defaults to `<namespace>/launchpad`, the same default as Launchpad's trace sync on other machines: `projects/` and `traces/` are at the root so other machines can use them too; the Space's own state is under `space/`.

- `projects/`: a Space bucket volume mounted at `/home/agent/projects` (`set_space_volumes`, subfolder `projects`). Files persist as they are written; there is no snapshot or upload job. Everything there is stored, including dependency and build folders, `.env` files and `.git`, in readable form. Behavior (symlinks, executable bits, locking, speed) is the volume's. Startup refuses to run when the volume is configured but not mounted, or not writable by the `agent` user.
- `space/manifest.json` names the latest objects: credential-free SQLite snapshots (`state/`), the config projection (`config/`), `state/runtime.json`, `credentials/checkpoint.aead` (the credential references of exactly that state) and `home/agents.tar.gz`. A changed object is written to the slot (`space/a/` or `space/b/`) its previous version does not use, then the manifest is replaced, so the manifest never points at a partial object and at most two copies exist. An idle machine uploads nothing.
- `home/agents.tar.gz` holds what agents need to resume outside `~/projects`: `~/.local/share/launchpad` (without databases and tokens), `~/.claude`, `~/.codex`, OpenCode data and config, `~/.agents` and profile folders. Caches, logs, tool temp folders, Codex log databases and rebuildable folders are skipped; credential files go to the vault. It is built in memory every five minutes when it changed (deterministic, so unchanged content is not re-uploaded).
- `space/credentials.aead`: credential files, watched and reconciled every 30 seconds, overwritten in place. Vault objects are AES-256-GCM with HKDF keys bound to installation, seal ID and key identity; the seal ID travels in the object.
- `space/pre-update/`: one restore point written before an image update and replaced by the next one. `space/update.json` records it for rollback.
- `space/next/`: credential copies under a new key during a planned key change; startup uses them when the current copy is under another key.
- `traces/`: Launchpad's own trace exporter (bucket and prefix are locked to these on the Space), enabled once the Hugging Face agent token is connected. Laptops and servers with default trace sync settings write and import here too.

Other folders in `/home/agent` (caches, folders registered outside `~/projects`, shell history) are lost on restart.

A backup failure never blocks startup: the bar shows "Backup delayed" and the worker retries. New launches and replies pause when no backup succeeded for ten minutes. A wrong or missing key opens a reconnect notice: files and settings are recovered, sign-ins must be redone.

### Migrating from format 1

Format 1 wrote a new `backups/<generation>/` (with the whole home folder as `workspaces/home.tar.gz`) and `credentials/<generation>/` on every change and never pruned them. A Space with no projects volume stops at the setup page with **Move projects and restart**, which adds the volume and restarts. The next startup finds no `space/manifest.json`, restores the newest format 1 generation once (its `projects/` files are written into the volume), then saves the first latest-only checkpoint. Later startups never read `backups/` or `credentials/`. After checking that projects, sessions and sign-ins survived a restart, delete the old folders yourself:

```sh
hf buckets rm <namespace>/<bucket>/backups/ --recursive --dry-run
hf buckets rm <namespace>/<bucket>/backups/ --recursive
hf buckets rm <namespace>/<bucket>/credentials/ --recursive
```

## Releases and updates

Every push to `main` triggers `.github/workflows/hf-space.yml`: build the image, smoke-check isolated startup, push an immutable GHCR image, verify anonymous pulls, then publish `hf-space-release.json` and `hf-space-template.zip` on the `hf-space-channel` artifact branch. No GitHub release or version tag is needed: the exact main commit is the version. Superseded queued builds do not publish, and failed builds leave the previous working channel intact. Manual dispatch on main can retry publication.

`release.json` → `channel` names the public raw manifest URL and official image repository. **Make the `launchpad-hf-space` GHCR package public once** in its package settings; a public GitHub repo does not automatically make packages public. CI refuses to advertise an image that anonymous clients cannot inspect. The workflow needs Contents and Packages write permissions. Optional `HF_SPACE_TEMPLATE` (repository variable) and `HF_SPACE_TEMPLATE_TOKEN` (secret) refresh the public duplicate template after publication; otherwise download the template zip from `hf-space-channel`.

Running Spaces check automatically at startup and roughly every six hours (with jitter), with **Check now** in Space settings. The Updates section shows the installed and available commits; **Update…** opens a confirmation listing the running agents, open terminals, tool changes and the steps below. Checks only advertise a different commit; they never apply updates automatically. A failed check preserves the last candidate and reports that the check is unavailable.

Existing source-built Spaces and images using the old release-only URL need a one-time migration: after the first successful channel build, download the template zip and replace the Space Dockerfile with its managed digest-pinned Dockerfile in Hub, then rebuild. Save a current backup and stop agents/terminals first. Source-built Dockerfiles cannot use image rollback and are deliberately refused by the updater. Subsequent updates use the owner-approved flow below.

An owner-confirmed update (`update-plan` then `prepare-update` actions, `supervisor/main.py: update`, helpers in `supervisor/updates.py`) runs in the background and reports `update.progress` in the status:

1. Stop the running agents (`starting`, `working`, `background`) through `POST /api/sessions/{id}/interrupt` and wait up to a minute; agents still running end with the daemon and come back Interrupted. Quota-paused and in-terminal sessions block the update.
2. Stop the daemon (open terminals end) and save a verified clean checkpoint, copied to `space/pre-update/`.
3. Write `space/update.json` (previous image, checkpoint, a `request` ID and the stopped agents), then replace the managed Dockerfile at the expected parent revision. Edited Dockerfiles are never overwritten.
4. On the next boot (new version, or the previous one after a rollback), `Updates.finish` claims the request in `space/update-resumed.json` before anything else, then resumes each stopped or interrupted agent with `POST /api/sessions/{id}/resume {"continue_turn": true}`. The claim makes it run once; a boot more than one hour after the update reports the agents instead of resuming them. The result (`update.result`) lists resumed, already finished and failed agents.

If a step fails before the commit, the daemon restarts on the current version and the same agents resume at once. An uncertain commit stops at the error page with everything stopped. If the new build does not come back within five minutes, the restarting page offers **Roll back the update**, which restores the previous image and its pre-update Launchpad state (`projects/` is live and is not rolled back).

See the [template README](template/README.md#recover-without-the-app) for recovery from the Hub when the app cannot start.

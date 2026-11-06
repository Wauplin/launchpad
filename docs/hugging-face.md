# Hugging Face

Launchpad integrates with Hugging Face in three ways:

- **Open models**: run Claude Code on open models through Hugging Face Inference Providers.
- **Traces**: Home copies the original conversation of every agent to its bucket, one file per trace.
- **Spaces**: run a complete Launchpad in a private Hugging Face Space, usable from any browser.

## Open models through Inference Providers

In **Settings → Profiles → New profile**, choose **Another model provider**, then **Claude Code · Hugging Face**. The profile points Claude Code to `https://router.huggingface.co` and lists the available tool-capable models (Kimi, GLM, Qwen, DeepSeek and others), with prices, context size and throughput per provider. Usage is billed to your Hugging Face account.

The token comes from `HF_TOKEN`, or from the token saved by `hf auth login`. OpenCode profiles can use the same router.

## Traces in the Home bucket

Home copies the original conversation of every agent to its backup destination (on a hosted Home, the private bucket mounted at `/backups`), one file per trace, next to the backups. The bucket view on Hugging Face opens these files in its trace viewer.

```text
<workspace>/traces/<folder>/<YYYYMMDD_HHMM>_<session id>/
├── <native trace files>   # Claude Code JSONL, Codex rollout-*.jsonl or OpenCode export, unchanged
└── launchpad.json         # session record: workspace and folder names, launch settings, prompt, usage, status, timestamps
```

- The start time is in UTC, so the sessions of a folder sort by start time. Bot sessions without a folder are under `bot <slug>`.
- A runner sends the conversation of a session to Home at the end of every turn (the handoff). After each backup, Home uploads the files that changed since the last pass. A failed upload is retried after the next backup and shows in **Settings → Hugging Face**.
- Renaming a workspace or folder moves the copies of its sessions. Archiving a session never deletes them.
- Home never reads the copies back: `state.db` in the backups stays the source. Settings → Hugging Face shows the layout and any copy error.
- Traces can contain sensitive data: prompts, tool output and file paths. Keep the bucket private.

## Run Launchpad in a Space

A private Docker Space runs a headless Launchpad with a small control plane in front of it: browser-only setup, `~/projects` kept live in a private bucket, encrypted credential backups and explicit updates.

```mermaid
flowchart LR
    B[Your browser] -->|HF sign-in, private Space| S
    subgraph S[Private Docker Space]
        C["Control plane<br/>setup, backups, updates"] --> L["launchpad daemon<br/>agents, terminals"]
    end
    C -->|"projects/ (volume), space/ (latest state)"| K[("Private bucket<br/>&lt;account&gt;/launchpad")]
```

### Setup

1. Create a private Docker Space from the template: duplicate the published template Space, or use `hf-space-template.zip` from the [`hf-space-channel`](https://github.com/Wauplin/launchpad/tree/hf-space-channel) branch. Keep it **Private**: everyone who can open the Space controls your agents.
2. Choose paid CPU hardware and disable sleep if agents must run while the tab is closed. Free CPU Spaces sleep and lose running processes.
3. Open the Space and sign in to Hugging Face: the page runs `hf auth login` and shows a link and a code. Launchpad saves the sign-in as a Space secret (refreshed automatically), creates the private `<your account>/launchpad` bucket (if missing), an encryption key and the `~/projects` volume, then restarts once (about a minute).
4. **Connect your first agent**: your Hugging Face account (open models), Claude Code, Codex or OpenCode (their sign-in runs in a terminal on the page), or an API key. GitHub is offered next.
5. **Clone your repositories** (optional): tick repositories to clone into `~/projects`. Each becomes a folder in Launchpad. When GitHub is connected, repositories with recent notifications come first, then your recently pushed ones. You can also type `owner/name`.

A bar above the app shows the Space name and the backup state. **Space settings** opens accounts, folders (the same repository picker), backups, updates and Space actions.

### Backups

- `~/projects` is a Space volume on the bucket's `projects/` folder: files are written to the bucket as you work, with no snapshots. Everything in it is stored, including `node_modules`, `target` and `.env` files, in readable form.
- Launchpad's state, settings and agent history (`~/.claude`, `~/.codex`, OpenCode, `~/.agents`) are saved every minute when they changed, as one latest copy; caches and logs are skipped. A final save runs when the Space stops.
- Credentials are encrypted (AES-256-GCM) with a key that only the control plane can read.
- Keep folders in `~/projects`: other folders and other files in the home folder are lost on restart.
- If no backup succeeded for ten minutes, new launches and replies pause.
- Spaces set up before the projects volume show **Move projects and restart** once. After the restart, the old backup is restored a last time and its projects are written into the volume. The old `backups/` and `credentials/` folders are no longer used; you can delete them once the new setup works.

### Updates

Every commit on `main` that passes the image build becomes a version. The Space checks for updates at startup and about every six hours; **Check now** in Space settings checks immediately. The Updates section shows the installed and available versions, with a link to the changes.

**Update…** opens a confirmation that lists what will happen: the running agents are stopped (like the Stop button), open terminals close, a backup is saved and verified, the Space switches to the new version and restarts, and the stopped agents resume in their own sessions with “Continue the unfinished turn.”. Space settings shows each step while it runs, then the result: the new version, the resumed agents and any agent that could not resume (open it and click **Resume**).

- If a step fails before the restart, the current version keeps running and the stopped agents resume on it.
- If the new version does not start within five minutes, **Roll back** restores the previous image and its pre-update backup; agents that were not resumed yet resume on the previous version.
- Agents resume only once, and not when the Space comes back more than one hour after the update.
- Sessions paused for quota or open in a terminal block the update until you stop or close them.

Developer details (layout, build, recovery without the app) are in [`deploy/hf-space/README.md`](../deploy/hf-space/README.md).

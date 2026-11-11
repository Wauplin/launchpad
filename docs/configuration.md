# Configuration

Most settings are editable in **Settings** and saved as soon as they change. Settings has two parts: **Workspace** (the workspace of the window: its name, folders, bots, Telegram, granted tools and MCP servers with their defaults, Delete) and **Launchpad** (shared by every workspace). On a laptop, opening Settings replaces the sidebar with the list of Settings pages; **Back to Launchpad** at its top returns to the page you were on. The search field at the top of that list finds a page by its name or a keyword. Settings that apply to the whole daemon are written to `config.toml`. Settings that apply to one device (zoom, theme, layout) stay in that browser.

## Files and folders

| | Home | Runner | Development Home / runner (`LAUNCHPAD_ENV=dev`) |
| --- | --- | --- | --- |
| Port | `127.0.0.1:7317` | `127.0.0.1:7320` | `7318` / `7321` |
| Configuration | `~/.config/launchpad/config.toml` | `~/.config/launchpad-runner/config.toml` | `~/.config/launchpad-dev/`, `~/.config/launchpad-runner-dev/` |
| Data | `~/.local/share/launchpad/` | `~/.local/share/launchpad-runner/` | `~/.local/share/launchpad-dev/`, `~/.local/share/launchpad-runner-dev/` |

Home's data folder contains:

- `state.db`: your data (sessions, events, drafts, Local PRs, suggestions, devices, the vault). Home backs it up when `[home] backup` is set.
- `cache.db`: the GitHub cache. Delete it at any time; it is rebuilt.
- `uploads/`: files attached to prompts, drafts, scheduled launches and routines.
- `git/`: the Home Git mirrors. `blobs/`: plain-folder, knowledge base and handoff files, by hash.

A runner's data folder holds copies and work in progress: task worktrees in `worktrees/<id>/`, cloned from `repos/<folder id>/`; plain and temporary folders; each session's harness home in `sessions/<id>/`; and `attachments/<session id>/`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `LAUNCHPAD_ENV=dev` | Use the development port, configuration and data folders |
| `LAUNCHPAD_PORT` | Use another port (for an isolated second daemon, together with another `HOME`) |
| `LAUNCHPAD_TOKEN` | Credential for the CLI (defaults to the daemon's token file) |
| `LAUNCHPAD_ROLE=runner` | Point CLI commands at the runner's folders and port |
| `LAUNCHPAD_HOME_BACKUP` | Home: the backup to restore on a first boot, before `config.toml` exists |

## config.toml

The file is created on first start. Missing keys use these defaults:

```toml
presets = []                        # your own prompt instructions, managed in Settings → Presets

[server]
bind = "127.0.0.1"                  # local listener (browsers, CLI)
auto_tailnet = true                 # also listen on a discovered Tailscale IP
# public_origin = "https://launchpad.example.com"  # see remote-access.md; restart after changes

[github]
poll_interval_seconds = 60          # GitHub's X-Poll-Interval wins when larger

[worktrees]
keep = 15                           # clean task worktrees of idle agents kept on each runner (0: keep all); Home sends it to runners

[home]
backup = ""                         # Home only: hf://buckets/<you>/launchpad or a folder; empty: no backup
git_name = ""                       # Home only: the author of agents' commits on every runner
git_email = ""
time_zone = ""                      # IANA zone (Europe/Paris) of routines saved without a browser; empty: the server's

[runner]
boundary = "jail"                   # runner only: jail (bubblewrap), or vm inside a dedicated Local VM, where agents may use Docker
```

Credentials and tools live in Home's database, never in `config.toml`: edit them in Settings → Credentials and tools. A profile's `login` names a harness login of Home's vault (default: the harness's first login).

Each `[[presets]]` entry has `id`, `label`, `text` and two independent availability flags: `bots` (defaults to `false`) and `agent` (defaults to `true`). Enable either or both in Settings → Presets. Agent availability covers the composer, steering and Send later. Presets are Home's: a runner reads them from Home when it starts an agent, so a launch that names presets fails visibly while Home cannot answer.

A bot with admin access to the Launchpad CLI can change a few shared settings when you ask it in chat: presets, the GitHub poll interval and the number of worktrees kept. Each change shows in the chat as a receipt on the turn's steps line; open the steps line to see the value before and after. Every other setting stays yours.

The daemon watches the file and applies most changes live. Changes to `[server]` need a daemon restart.

### Profiles

Profiles are `[[profiles]]` entries. Settings → Profiles edits them; templates fill in the usual values.

```toml
[[profiles]]
id = "claude-max"
name = "Claude · Max"
harness = "claude"                  # claude, codex or opencode
billing = "subscription"            # or pay_as_you_go
model = "opus"

[[profiles]]
id = "claude-work"
name = "Claude · Work account"
harness = "claude"
billing = "subscription"
login = "<vault login id>"          # a second subscription login in Home's vault

[[profiles]]
id = "claude-hf"
name = "Claude · Hugging Face"
harness = "claude"
billing = "pay_as_you_go"
models_url = "https://router.huggingface.co/v1/models"
spend_limits = [{ usd = 20, hours = 24 }]
env = { ANTHROPIC_BASE_URL = "https://router.huggingface.co", ANTHROPIC_AUTH_TOKEN = "${HF_TOKEN}" }
```

Values can use `~/`, `${NAME}` (from your login shell) and `${LAUNCHPAD_MODEL}` (the session's model). On Unix, the file is saved with mode 0600.

## CLI

The `launchpad` binary is the daemon and the CLI. Select the daemon with `LAUNCHPAD_ENV`.

| Command | Purpose |
| --- | --- |
| `launchpad daemon [--role runner]` | Run Home (the default: records, no agents; it relays terminals to runners), or a runner, which runs agents in its own folders (`~/.local/share/launchpad-runner`, `~/.config/launchpad-runner`). Runners run only on Linux. `--role home` is accepted; there is no role that does both |
| `launchpad connect <Home address> [--name <name>]` | Pair this machine as a runner of a Home (Linux only): prints a code to approve in Home's Settings → Home and runners |
| `launchpad status` | Print the daemon status as JSON |
| `launchpad url` | Print a link that signs a browser in (not on Burst or Local VM runners, which keep no token on disk) |
| `launchpad pair` | Print a five-minute pairing link and QR code |
| `launchpad pr create\|update\|show\|list` | Manage Local PRs (used by agents; an agent updates only its own) |
| `launchpad suggest comment\|review` | Save an agent-written comment or review as a suggestion; never posts |
| `launchpad send <channel>\|telegram --body-file <file> --session-id <id>` | Draft a message for an outgoing channel (it waits in the Outbox), or send one to the Telegram chat of the session's workspace ([Channels](channels.md#telegram)) |
| `launchpad work finish\|add-project` | Request a worktree merge (only when the launch chose Merge) or open a Local PR, or add another Git folder of the workspace to a task (used by agents) |
| `launchpad trace <session> [--through <seq>]` | Print the stored trace of a session |
| `launchpad folders\|sessions\|profiles\|presets\|inbox\|schedules\|proposals list` | Read Launchpad state as JSON (an agent reads only its own workspace) |
| `launchpad today` | Print what waits for the owner (the [Today](today.md) list) as JSON |
| `launchpad activity list` | List what sessions with admin access changed directly, with the values before |
| `launchpad propose launch\|message\|schedule\|cron\|status\|preset\|tool\|bot\|routine\|bot-files\|bot-instructions\|ready` | Store a proposal (sessions with propose or admin access to the Launchpad CLI tool); the owner starts it, except one-time starts of a bot in auto mode. `bot-files` proposes a bot's `BOT.md` and routine files in one card (`routine` and `bot-instructions` become such cards). `ready` prepares an owner click (merge, publish, post, send, run CI, Inbox done) |
| `launchpad api <GET\|POST\|PUT\|PATCH\|DELETE> <path> [--body-file <file.json>]` | Call Launchpad's API directly: sessions with admin access only, limited to Launchpad's own records of the session's workspace and the shared settings |

Agents receive the exact binary path and environment in their prompt. The [launchpad skill](../skills/launchpad/SKILL.md) documents these commands for them; the daemon links it into `~/.agents/skills/launchpad` at startup.

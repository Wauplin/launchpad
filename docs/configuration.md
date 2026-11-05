# Configuration

Most settings are editable in **Settings** and saved as soon as they change. Settings that apply to the whole daemon are written to `config.toml`. Settings that apply to one device (zoom, theme, layout) stay in that browser.

## Files and folders

| | Production | Development (`LAUNCHPAD_ENV=dev`) |
| --- | --- | --- |
| Port | `127.0.0.1:7317` | `127.0.0.1:7318` |
| Configuration | `~/.config/launchpad/config.toml` | `~/.config/launchpad-dev/config.toml` |
| Data | `~/.local/share/launchpad/` | `~/.local/share/launchpad-dev/` |

The data folder contains:

- `state.db`: your data (sessions, events, drafts, Local PRs, suggestions, devices). Back it up.
- `cache.db`: the GitHub cache. Delete it at any time; it is rebuilt.
- `uploads/`, `attachments/`: files attached to prompts.

Task worktrees live in `<data folder>/worktrees/<folder id>/`, cloned from `<data folder>/repos/<folder id>/`. Home keeps its Git mirrors in `<data folder>/git/`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `LAUNCHPAD_ENV=dev` | Use the development port, configuration and data folders |
| `LAUNCHPAD_PORT` | Use another port (for an isolated second daemon, together with another `HOME`) |
| `LAUNCHPAD_TOKEN` | Credential for the CLI (defaults to the daemon's token file) |
| `LAUNCHPAD_ROLE=runner` | Point CLI commands at a runner's folders |
| `LAUNCHPAD_HOME_BACKUP` | Home: the backup to restore on a first boot, before `config.toml` exists |

## config.toml

The file is created on first start. Missing keys use these defaults:

```toml
projects_root = "~/projects"        # the folders root: discovery, kept temporary folders
remove_published_worktree = false   # keep Local PR worktrees after publication
presets = []                        # your own prompt instructions, managed in Settings → Presets

[server]
bind = "127.0.0.1"                  # local desktop and CLI listener
auto_tailnet = true                 # also listen on a discovered Tailscale IP
# public_origin = "https://launchpad.example.com"  # see remote-access.md; restart after changes

[commands]
browser = "xdg-open"                # opens external links

[github]
poll_interval_seconds = 60          # GitHub's X-Poll-Interval wins when larger

[trace_backup]
enabled = false                     # Hugging Face trace sync
bucket = "launchpad"                  # <your account>/launchpad, shared with a Launchpad Space
prefix = "traces"

[agents]
notify = true                       # desktop notifications

[worktrees]
keep = 15                           # clean task worktrees of idle agents kept on disk (0: keep all)

[home]
backup = ""                         # Home only: hf://buckets/<you>/launchpad or a folder; empty: no backup
git_name = ""                       # Home only: the author of agents' commits on every runner
git_email = ""
time_zone = ""                      # IANA zone (Europe/Paris) of routines saved without a browser; empty: the server's

[runner]
boundary = "jail"                   # jail (bubblewrap) or vm: only inside a dedicated Local VM, where agents may use Docker
```

Credentials and tools live in Home's database, never in `config.toml`: edit them in Settings → Credentials and tools. A profile's `login` names a harness login of Home's vault (default: the harness's first login).

Each `[[presets]]` entry has `id`, `label`, `text` and two independent availability flags: `ask_launchpad` (defaults to `false`) and `agent` (defaults to `true`). Enable either or both in Settings → Presets. Agent availability covers the composer, steering and Send later.

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
env = { CLAUDE_CONFIG_DIR = "~/.claude-work" }   # a second subscription login

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
| `launchpad daemon [--role home\|runner\|all]` | Run the daemon. `home` keeps records and runs no agents or terminals; `runner` runs agents in its own folders (`~/.local/share/launchpad-runner`, `~/.config/launchpad-runner`); `all` (default) does both in one process |
| `launchpad connect <Home address> [--name <name>]` | Pair this machine as a runner of a Home: prints a code to approve in Home's Settings → Home and runners |
| `launchpad status` | Print the daemon status as JSON |
| `launchpad url` | Print a link that signs a browser in |
| `launchpad pair` | Print a five-minute pairing link and QR code |
| `launchpad pr create\|update\|show\|list` | Manage Local PRs (used by agents) |
| `launchpad suggest comment\|review` | Save an agent-written comment or review as a suggestion; never posts |
| `launchpad send <channel>\|telegram --body-file <file> --session-id <id>` | Draft a message for an outgoing channel (it waits in the Outbox), or send one to the owner's Telegram chat ([Channels](channels.md)) |
| `launchpad work finish\|add-project` | Request a worktree merge or open a Local PR, or add another folder to a task (used by agents) |
| `launchpad trace <session> [--through <seq>]` | Print the stored trace of a session |
| `launchpad folders\|sessions\|profiles\|presets\|inbox\|schedules\|proposals list` | Read Launchpad state as JSON |
| `launchpad propose launch\|message\|schedule\|cron\|status\|preset\|tool\|bot\|routine` | Store a proposal (sessions with propose or admin access to the Launchpad CLI tool); the owner starts it, except one-time starts of a bot in auto mode |
| `launchpad api <GET\|POST\|PUT\|PATCH\|DELETE> <path> [--body-file <file.json>]` | Call Launchpad's API directly: sessions with admin access only, limited to Launchpad's own records |

Agents receive the exact binary path and environment in their prompt. The [launchpad skill](../skills/launchpad/SKILL.md) documents these commands for them; the daemon links it into `~/.agents/skills/launchpad` at startup.

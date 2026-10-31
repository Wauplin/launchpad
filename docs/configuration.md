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
- `vapid.json`: Web Push keys.

Task worktrees live in `<folders root>/.worktrees/<folder id>/`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `LAUNCHPAD_ENV=dev` | Use the development port, configuration and data folders |
| `LAUNCHPAD_PORT` | Use another port (for an isolated second daemon, together with another `HOME`) |
| `LAUNCHPAD_TOKEN` | Credential for the CLI (defaults to the daemon's token file) |

## config.toml

The file is created on first start. Missing keys use these defaults:

```toml
projects_root = "~/projects"        # the folders root: discovery, scratch folders, worktrees
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
bucket = "launchpad-traces"

[agents]
notify = true                       # desktop and Web Push notifications

[worktrees]
keep = 15                           # clean task worktrees of idle agents kept on disk (0: keep all)
```

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
| `launchpad daemon` | Run the daemon |
| `launchpad status` | Print the daemon status as JSON |
| `launchpad url` | Print a link that signs a browser in |
| `launchpad pair` | Print a five-minute pairing link and QR code |
| `launchpad pr create\|update\|show\|list` | Manage Local PRs (used by agents) |
| `launchpad suggest comment\|review` | Save an agent-written comment or review as a suggestion; never posts |
| `launchpad work finish\|add-project` | Request a worktree merge, or add another folder to a task (used by agents) |
| `launchpad trace <session> [--through <seq>]` | Print the stored trace of a session |
| `launchpad folders\|sessions\|profiles\|presets\|inbox\|schedules\|proposals list` | Read Launchpad state as JSON |
| `launchpad propose launch\|message\|schedule\|cron\|status` | Store an Ask Launchpad proposal; never starts anything |

Agents receive the exact binary path and environment in their prompt. The [launchpad skill](../skills/launchpad/SKILL.md) documents these commands for them; the daemon links it into `~/.agents/skills/launchpad` at startup.

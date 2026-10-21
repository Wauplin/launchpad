# Spike 5: Claude TUI trust prompt

Date: 2026-10-02. Claude Code 2.1.287.

**Result: the dialog can appear, and it can be skipped.** `claude --resume` in the TUI shows the trust dialog when the folder is not trusted, even with `--dangerously-skip-permissions`. The environment variable `CLAUDE_CODE_SANDBOXED=1` skips it.

## Method

No manual terminal was needed. A Python script:

1. created a session in the folder with `claude -p --session-id <uuid> --model haiku --effort low "Reply with exactly: ok"`,
2. opened `claude --resume <uuid> --model haiku [flags]` in a pseudo-terminal (`pty.fork()`), read the screen for 12 seconds without typing anything (so no model call), and searched the text for "trust",
3. killed the TUI.

## Results

| Case | Folder | Flags or environment | Trust dialog |
|---|---|---|---|
| A | `~/projects/tmp-1002-spike5` (plain folder) | none | no |
| B1 | `/tmp/…/s5-untrusted-a` | none | **yes** |
| B2 | `/tmp/…/s5-untrusted-b` | `--dangerously-skip-permissions` | **yes** |
| C | `~/projects/tmp-1002-spike5-git` (after `git init`) | `--dangerously-skip-permissions` | **yes** |
| D | same as C | `CLAUDE_CODE_SANDBOXED=1` | no |

The dialog text:

```
Accessing workspace:
/tmp/claude-1000/…/s5-untrusted-a
Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open source project, or work from your team). If not, take a moment to review what's in this folder first.
Claude Code'll be able to read, edit, and execute files here.
❯ No, exit
  Yes, I trust this folder
Enter to confirm · Esc to cancel
```

## How trust works (read from the 2.1.287 binary)

- Trust is stored in `~/.claude.json` as `projects["<path>"].hasTrustDialogAccepted`. Here, `~/projects` itself is trusted.
- For a plain folder, Claude walks up the parent folders, so anything under `~/projects` is trusted (case A).
- **For a git repository, only the repository root is checked**, so a `git init` temporary folder is not trusted even under `~/projects` (case C).
- `claude -p` never shows the dialog and does not record trust.
- `CLAUDE_CODE_SANDBOXED` makes the trust checks return "trusted". Its other references in the binary are lists of known environment variables; no other behavior was found.

## Decision

- "Open in terminal" sets `CLAUDE_CODE_SANDBOXED=1` in the environment of the `claude --resume` command, only for temporary projects created by Launchpad. Registered projects are your own clones and keep the normal behavior (most are trusted already).
- Launchpad does not write `~/.claude.json`: Claude Code rewrites that file often, and a concurrent write could lose data.
- The variable is not documented. If a later version ignores it, the only effect is that the dialog shows again and you click "Yes" once.

## Other finding: environment variables leak into child sessions

The spike ran inside a Claude Code session, and the TUI showed `⚠ Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION marker`. A daemon started from an agent (for example the dev environment started by an agent working on Launchpad) passes `CLAUDECODE`, `CLAUDE_CODE_*`, `CLAUDE_PID` and `CLAUDE_EFFORT` to every harness it starts. The adapters remove these variables before spawning `claude` or `codex` (except the ones Launchpad sets on purpose, such as `CLAUDE_CODE_SANDBOXED`).

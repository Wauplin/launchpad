# Implementation and acceptance

Source: [technical report revision 3](https://wauplin-launchpad-spaces-technical-report.static.hf.space). The owner's 2026-10-31 follow-up accepts protected Spaces as private until the Hub API distinguishes them (`supervisor/policy.py`).

## Review of 2026-10-31

A second pass rebuilt the browser shell and fixed backend defects found by running the image against a real private Space and bucket.

- Shell: replaced the minified one-file UI (several buttons threw on click; destructive actions ran on one click) with readable `shell.html`/`shell.css`/`shell.js` using Launchpad's design tokens and theme. Setup is a stepper driven by a server `stage`: one pasted write token saves itself as a Space secret, creates the bucket and the encryption key and restarts once; the restarting page reconnects by itself. "Connect your first agent" offers Hugging Face, Claude Code, Codex, OpenCode or an API key; a successful sign-in creates its profile automatically, then GitHub is offered. Sign-ins run in a real terminal (xterm over `/_space/login`) so interactive prompts (OpenCode provider picker, gh questions, Claude code paste) work, with an Open sign-in page button and the one-time code extracted. Consequential actions need a second click. A 36 px bar shows the Space and backup state; Space settings holds accounts, backups, updates and Space actions.
- Gateway: cross-site page loads (the Hub embeds Spaces in an iframe on huggingface.co) were rejected; navigations are now allowed, other cross-site requests still are. Errors are logged to the Space logs and shown in plain words, without Hub request IDs.
- Startup: a failed first backup no longer blocks startup. A one-shot rollback restore is acknowledged immediately so later restarts do not repeat it. A stale SQLite WAL left beside a restored database corrupted it; restores now remove WAL/SHM/journal files, and a damaged tool database is skipped instead of failing every backup.
- Backups: symlinks (such as `CLAUDE.md -> AGENTS.md`, venv interpreters, dangling links) no longer stop every checkpoint. Text that mentions the vault marker no longer breaks restore. Unchanged objects and vaults are referenced, not re-uploaded, and an unchanged machine writes nothing (the archive is deterministic). Workspaces are archived every five minutes as specified, plus on Back up now, before maintenance and when the container stops. Tool temp folders and Codex log databases are excluded (an empty workspace went from 50 MB to 0.4 MB). Credential files are re-read only when they change; Git history classification is cached by refs and ignores committed templates like `.env.example`. The schema fingerprint compares tables and columns, not CREATE text. Rotation re-encrypts vaults and writes manifests that reference existing objects.
- Proxy mutations no longer wait on one global lock (a slow request blocked Stop); maintenance drains in-flight requests instead.
- The release channel (manifest URL, image) moved to `release.json`.

## Checked locally (2026-10-31)

Real image, real private throwaway Space and bucket on the owner's account, TLS in front of the gateway and Chrome mapped to the `hf.space` host. Screens were checked at desktop and phone width, light and dark.

- Token page → pasted token → secrets `LP_SPACE_MANAGEMENT_TOKEN` and `LP_SPACE_BACKUP_KEY` created on the Space, restart requested, restarting page shown.
- Startup with secrets → restore → connect page; Claude sign-in terminal showed the link, accepted typed input (invalid code rejected), survived a page reload and was cancelled. Invalid HF token rejected with a clear message. A connection list, backup, update and Space sections render.
- Backups: idle Back up now creates no generation (0.1 s); a changed workspace creates one referencing unchanged objects; restore on an empty disk recovered a Git repository, symlinks and files from manifests written in both the old and new formats; `docker stop` saved a final checkpoint.
- Daemon killed → "Launchpad stopped" page → Restart Launchpad → app back.
- Template stage page.
- `pnpm typecheck` and `pnpm test:tour` pass.

No real provider sign-in was completed, no HF agent token was connected (trace export was not exercised), and no update or rollback ran. The throwaway Space and bucket were deleted afterwards.

## Still required before publishing

- Make the release channel public: `Wauplin/launchpad` is private, so the manifest and GHCR image in `release.json` cannot be read by duplicated Spaces. Either publish the repository and its GHCR package or point `channel` elsewhere, then cut a release.
- On a real Space: Hub iframe and direct URL, provider sign-ins to completion, HF agent token and trace export to `traces/`, secret-change restarts, update and rollback, a 24–72 hour soak on paid CPU with sleep off.
- Large workspaces: archives are built in memory and uploaded whole; measure RAM and upload time with a representative repository.
- Not in scope: multiple replicas, remote use of the Chrome extension, outbound SSH on port 22, discovering secrets in arbitrary files, an offline recovery key.

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

## Deployed check (2026-10-31)

Deployed privately as `Wauplin/launchpad` (built from source). Fixed after owner feedback: the GitHub step of the finish page never showed its terminal (and a second click failed); Space settings stopped updating while the sign-in terminal had focus. gh's two default prompts are now answered automatically, so GitHub sign-in shows only Open sign-in page and the one-time code (checked live, then cancelled). Setup pages and Space settings have a Text size control (100–150%) that shares `launchpad-zoom` with the app, which follows live.

## Main-commit update channel (2026-10-31)

The public repository now supplies a main-commit channel instead of waiting for GitHub releases. Every main push builds and smoke-checks the runtime; CI verifies anonymous GHCR access before publishing the immutable image manifest and duplicate-template zip on `hf-space-channel`. Running managed Spaces check at startup and about every six hours, and show a separate update prompt even when a backup warning is present. Owners review the commit and confirm before maintenance. Source-built Spaces get a template download and one-time migration instructions instead of an unusable update button.

Checked for this change: eight isolated Python updater regressions; manifest/template generation and first/repeated publication scripts against a throwaway local bare remote; workflow YAML and shell syntax; Chrome fixture for the visible prompt, review drawer, second-click confirmation and exact image request; strict workspace Clippy, all 130 Rust tests, UI typecheck/build and four tour guardrails. Tests use local fixtures and mocked Hub mutations; no real Space was updated or restarted, and the new Docker image/Actions workflow has not been built or executed in the cloud by this task.

## Deployment refresh (2026-10-31)

Updated the private `Wauplin/launchpad` Space from source revision `79025f9c8a2211944659c5e37c488004a64cca7e`, at Hub commit `8d5fc7c24807a984ae134e466c5c0854da3b8365`. Before uploading, the running Space passed the updater's idle checks (no busy agents, live terminals or sign-in), and a full backup completed without errors (`7b604f2b4962493daa3585db9ee8acb4`).

The exact source revision built successfully in Docker locally. Its isolated, network-disabled startup passed the health check, schema/assets checks and all six pinned CLI version checks. All eight Python updater regressions passed. The smoke-test container was removed afterwards. The Hub build also succeeded. The live shell reached `ready`, both the shell and `/api/status` reported the exact revision, and the existing waiting session, six projects, profile context and connections endpoints were recovered successfully. A post-startup backup completed without errors or pending credentials. The new channel URL is installed, but checks correctly report it unavailable until publication.

GitHub Actions currently refuses to start jobs because of the account's payment/spending limit (reported on [CI run 41217995704](https://github.com/Wauplin/launchpad/actions/runs/41217995704)). The immutable image and `hf-space-channel` therefore remain unpublished. This refresh preserves the source-build deployment; enabling in-app updates still requires a successful channel publication and the one-time managed Dockerfile migration described in the deployment README. Automatic checks do not automatically apply updates: the owner confirms them.

## Still required before publishing

- First channel publication: push main to trigger CI (or dispatch on main), make the `launchpad-hf-space` GHCR package public, and rerun if its anonymous-read check failed. Configure `HF_SPACE_TEMPLATE` and `HF_SPACE_TEMPLATE_TOKEN` for automatic duplicate-template refresh. Migrate the existing source-built Space to the resulting managed Dockerfile once; no GitHub release is required.
- On a real Space: Hub iframe and direct URL, provider sign-ins to completion, HF agent token and trace export to `traces/`, secret-change restarts, update and rollback, a 24–72 hour soak on paid CPU with sleep off.
- Large workspaces: archives are built in memory and uploaded whole; measure RAM and upload time with a representative repository.
- Not in scope: multiple replicas, remote use of the Chrome extension, outbound SSH on port 22, discovering secrets in arbitrary files, an offline recovery key.

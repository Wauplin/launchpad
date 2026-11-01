---
title: Launchpad
emoji: 🚀
colorFrom: green
colorTo: blue
sdk: docker
app_port: 7860
pinned: false
---

# Your Launchpad workspace

A personal coding-agent machine you can open from any browser. All visitors admitted by the Space have full Launchpad access. This is one shared machine, not a multi-user service.

## Start here

1. Keep this Space **Private**: everyone who can open it controls your agents and their sign-ins.
2. Open the app and paste a Hugging Face **write** token. Launchpad saves it as a Space secret, creates a private backup bucket, an encryption key and a volume that keeps `/home/agent/projects` in the bucket, then restarts the Space once (about a minute).
3. Connect your first agent: your Hugging Face account (open models through Inference Providers), Claude Code, Codex, OpenCode or an API key. Then optionally GitHub.

To use an existing bucket, set the Variable `LP_SPACE_BUCKET=namespace/name` first. Launchpad uses `projects/` (the live projects volume), `space/` (the latest state, credentials encrypted) and `traces/` in that bucket; you can browse files and native agent traces on the Hub.

## Keep it running

CPU Basic can sleep and disk is ephemeral across rebuilds. Choose paid CPU and disable sleep for unattended sessions; consult current HF pricing and account eligibility in Settings. Running processes and PTYs never survive a rebuild. `/home/agent/projects` lives in the bucket, so files and Git state persist as they are written. Launchpad state, native resume state and settings are saved every minute (latest copy only), with credentials encrypted separately. Backup and sign-in states are shown in Space settings. Keep projects inside `/home/agent/projects` so they are recoverable. Use Git HTTPS by default; outbound port 22 may be blocked.

## Update

Launchpad checks the main update channel at startup and approximately every six hours. Every new main commit becomes a version after its runtime build passes; no GitHub release is required. The bar shows **Update available · Review** when a newer version is ready, and **Check now** in Space settings checks immediately. Updates require an explicit click, idle agents/terminals and a verified clean checkpoint. The managed Dockerfile contains only an immutable runtime image reference. Keep custom files beside it; edited Dockerfiles are never overwritten automatically. The update review includes separate Launchpad and connector versions plus pinned tool versions.

Restart, rebuild, update, restore and credential reconnection are separate actions. Stop active work before controlled maintenance. External Hub changes can still interrupt work.

## Recover without the app

If an update does not come back within five minutes, the restarting page offers **Roll back image & data** when it can read the update record. If the control plane itself cannot boot, open `space/update.json` in your private bucket: it records the prior `image` and `checkpoint`. Revert the Space's Dockerfile to that image and append:

```dockerfile
ENV LP_SPACE_RESTORE_CHECKPOINT=<checkpoint from update.json>
ENV LP_SPACE_RESTORE_REQUEST=<a fresh UUID without hyphens>
```

Commit in Hub to rebuild. This restores the pre-update Launchpad state from `space/pre-update/` and loses state written after it; `projects/` is live and is not rolled back. (An older `backups/update.json` with a `generation` uses `LP_SPACE_RESTORE_GENERATION` instead.) Restore requests are acknowledged once; ordinary later restarts use the latest verified checkpoint. Never run an older image against a newer migrated database by guesswork.

If the encryption key is lost, retain the private bucket and set a new base64-encoded 32-byte random `LP_SPACE_BACKUP_KEY` Secret in Hub. Startup recovers ordinary data and offers **Start reconnecting tools**. Reauthorize credentials; the old ciphertext is replaced by the next backup. Ordinary backups and traces remain readable without the old key. Do not reset the whole bucket. Incidental secrets in code, prompts and traces remain protected by bucket access, not by the credential vault.

Management and encryption secrets belong only to the control plane. Never paste them into an agent prompt or commit them to this repository. Removing a collaborator or changing visibility is managed by HF; the connector checks metadata only at startup. Run one Space per bucket; replicas and overlapping writers are unsupported.

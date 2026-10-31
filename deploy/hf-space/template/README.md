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
2. Open the app and paste a Hugging Face **write** token. Launchpad saves it as a Space secret, creates a private backup bucket and an encryption key, then restarts the Space once (about a minute).
3. Connect your first agent: your Hugging Face account (open models through Inference Providers), Claude Code, Codex, OpenCode or an API key. Then optionally GitHub.

To use an existing bucket, set the Variable `LP_SPACE_BUCKET=namespace/name` first. Backups use `backups/`, `credentials/` and `traces/` in that bucket; you can browse files and native agent traces on the Hub.

## Keep it running

CPU Basic can sleep and disk is ephemeral across rebuilds. Choose paid CPU and disable sleep for unattended sessions; consult current HF pricing and account eligibility in Settings. Running processes and PTYs never survive a rebuild. Backups preserve files, Git state, native resume state and settings, with credentials encrypted separately. Backup and sign-in states are shown in Space settings. Keep projects inside `/home/agent` so they are recoverable. Use Git HTTPS by default; outbound port 22 may be blocked.

## Update

Space settings checks the upstream release manifest approximately every six hours. Updates require an explicit click, idle agents/terminals and a verified clean checkpoint. The managed Dockerfile contains only an immutable runtime image reference. Keep custom files beside it; edited Dockerfiles are never overwritten automatically. Releases include separate Launchpad and connector versions plus pinned tool versions.

Restart, rebuild, update, restore and credential reconnection are separate actions. Stop active work before controlled maintenance. External Hub changes can still interrupt work.

## Recover without the app

If an update does not come back within five minutes, the restarting page offers **Roll back image & data** when it can read the update record. If the control plane itself cannot boot, open `backups/update.json` in your private bucket: it records the prior `image` and `generation`. Revert the Space's Dockerfile to that image and append:

```dockerfile
ENV LP_SPACE_RESTORE_GENERATION=<generation from update.json>
ENV LP_SPACE_RESTORE_REQUEST=<a fresh UUID without hyphens>
```

Commit in Hub to rebuild. This restores the matching pre-update data and loses writes made after that checkpoint. Restore requests are acknowledged once; ordinary later restarts use the latest verified checkpoint. Never run an older image against a newer migrated database by guesswork.

If the encryption key is lost, retain the private bucket and set a new base64-encoded 32-byte random `LP_SPACE_BACKUP_KEY` Secret in Hub. Startup recovers ordinary data and offers **Start reconnecting tools**. Reauthorize credentials; old ciphertext is retained. Ordinary backups and traces remain readable without the old key. Do not reset the whole bucket. Incidental secrets in code, prompts and traces remain protected by bucket access, not by the credential vault.

Management and encryption secrets belong only to the control plane. Never paste them into an agent prompt or commit them to this repository. Removing a collaborator or changing visibility is managed by HF; the connector checks metadata only at startup. Run one Space per bucket; replicas and overlapping writers are unsupported.

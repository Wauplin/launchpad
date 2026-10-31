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

1. Make this Space **Private**. The owner's temporary connector policy also accepts Protected; protected apps may be publicly accessible.
2. In Settings → Secrets, add `LP_SPACE_MANAGEMENT_TOKEN`: an HF token with access to this Space, its settings/secrets and your backup bucket. Keep this separate from the HF token your agents will use.
3. Open the app. It offers to create a private backup bucket (defaults to the same namespace/name as this Space) and protect credentials with a new `LP_SPACE_BACKUP_KEY` Secret. These owner-clicked steps restart the Space. Existing bucket visibility is never changed.
4. **Connect Hugging Face** creates an HF inference profile from your agent token (Inference Providers permission required; usage is billed through HF), so another chat subscription is optional. Or connect Codex, Claude Code or OpenCode using its authorization instructions. Create its profile, then open Launchpad to add or clone a folder. GitHub and the other tools can be connected later in Space controls.

For an existing bucket, set Variable `LP_SPACE_BUCKET=namespace/name` before starting. Choose a dedicated private bucket per installation. Backups use `backups/`, `credentials/` and `traces/`. Connect a separate HF agent token to enable the existing trace exporter. You can browse native trace files directly in the private bucket.

## Keep it running

CPU Basic can sleep and disk is ephemeral across rebuilds. Choose paid CPU and disable sleep for unattended sessions; consult current HF pricing and account eligibility in Settings. Running processes and PTYs never survive a rebuild. Backups preserve files, Git state, native resume state and settings, with credentials encrypted separately. Recovery and credential statuses are shown independently in Space controls. Keep projects inside `/home/agent` so they are recoverable. Use Git HTTPS by default; outbound port 22 may be blocked.

## Update

Space controls checks the upstream release manifest approximately every six hours. Updates require an explicit click, idle agents/terminals and a verified clean checkpoint. The managed Dockerfile contains only an immutable runtime image reference. Keep custom files beside it; edited Dockerfiles are never overwritten automatically. Releases include separate Launchpad and connector versions plus pinned tool versions.

Restart, rebuild, update, restore and credential reconnection are separate actions. Stop active work before controlled maintenance. External Hub changes can still interrupt work.

## Recover without the app

If an update cannot boot, Space controls remains available and offers **Roll back image & data** when it can read the update record. If the control plane itself cannot boot, open `backups/update.json` in your private bucket: it records the prior `image` and `generation`. Revert the Space's Dockerfile to that image and append:

```dockerfile
ENV LP_SPACE_RESTORE_GENERATION=<generation from update.json>
ENV LP_SPACE_RESTORE_REQUEST=<a fresh UUID without hyphens>
```

Commit in Hub to rebuild. This restores the matching pre-update data and loses writes made after that checkpoint. Restore requests are acknowledged once; ordinary later restarts use the latest verified checkpoint. Never run an older image against a newer migrated database by guesswork.

If the encryption key is lost, retain the private bucket and set a new base64-encoded 32-byte random `LP_SPACE_BACKUP_KEY` Secret in Hub. Startup recovers ordinary data and offers **Start reconnecting tools**. Reauthorize credentials; old ciphertext is retained. Ordinary backups and traces remain readable without the old key. Do not reset the whole bucket. Incidental secrets in code, prompts and traces remain protected by bucket access, not by the credential vault.

Management and encryption secrets belong only to the control plane. Never paste them into an agent prompt or commit them to this repository. Removing a collaborator or changing visibility is managed by HF; the connector checks metadata only at startup. Run one Space per bucket; replicas and overlapping writers are unsupported.

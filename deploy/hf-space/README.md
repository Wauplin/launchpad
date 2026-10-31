# Launchpad on Hugging Face Spaces

A headless Launchpad runtime with a small Space control plane: browser-only tool setup, a private recovery bucket, a separate credential vault and explicit image updates. The runtime builds locally; cloud deployment, live provider authorization and the daily-use soak still require owner acceptance. Nothing in this branch has been pushed or deployed.

## User path

Use the public duplicate template when the maintainer publishes it, or upload the `hf-space-template.zip` attached to a runtime release into a Docker Space. The template contains only the README and an immutable image reference, so a duplicate does not vendor Launchpad source or a second package manager.

1. Choose Private visibility. Add `LP_SPACE_MANAGEMENT_TOKEN` in HF Secrets with access to this Space, its secrets/settings and the chosen bucket. All admitted visitors are full operators of this one machine.
2. Open the app. If needed, click **Create private backup bucket**, then **Protect my backups**. The default bucket is the same namespace/name as the Space. These explicit actions may restart it. Existing bucket visibility is never changed. A copy-once manual key path is available when the token cannot set Space Secrets.
3. In **Space controls**, **Connect Hugging Face** can create the core-provided HF inference profile using your agent token (enable Inference Providers permission; usage is billed through HF). Or connect one coding tool through its pinned CLI's authorization flow or a masked API-key form. Create its profile. Connect GitHub for repository/review features and a separate HF agent token for the existing trace exporter. Git HTTPS setup and SSH key generation/import are separate clicks.
4. Use Launchpad normally. Space controls collapses to a 44 px bar with recovery status; on phones its drawer fills the viewport. Tool setup, recovery, versions and maintenance remain available there.

Choose paid CPU with sleep disabled for unattended work; the Hub's current pricing and eligibility apply. Basic hardware can sleep and a rebuild loses live processes. SSH on port 22 and remote use of the local Chrome extension are not promised.

### Temporary visibility policy

The owner's explicit decision on 2026-10-31 accepts HF's `private: true` for both private and protected Spaces. Protected apps may be publicly accessible. Public/unknown Spaces and public/unknown buckets remain rejected. `supervisor/policy.py` owns this one decision so it can be tightened when the Hub exposes a reliable distinction. Checks happen once at startup; no visibility polling, per-upload policy checks, replica leases or automatic visibility changes.

## Build and run

From the repository root:

```sh
docker build -f deploy/hf-space/Dockerfile --build-arg LAUNCHPAD_REVISION="$(git rev-parse HEAD)" -t launchpad-space:local .
```

The image builds the UI before the daemon, uses pinned Node/Rust base digests, installs exact provider CLI versions from `release.json`, verifies the GitHub CLI checksum, records dependency inventories under `/opt/space/sbom/`, and classifies the database schema produced by that binary. No desktop, systemd or host installer runs. `tini` forwards signals and reaps children.

HF supplies `SPACE_ID` and `SPACE_HOST`. Required Secret: `LP_SPACE_MANAGEMENT_TOKEN`. Optional Secret: `LP_SPACE_BACKUP_KEY` (base64 of 32 random bytes; setup creates it). Optional Variable: `LP_SPACE_BUCKET=namespace/name`; otherwise the Space ID is used. Change destinations through Hub Variables and restart. The exact origin is `https://SPACE_HOST`; Host/Origin checks intentionally reject direct localhost browser access.

The image explicitly selects `USER root`: a disposable private HF Docker probe confirmed UID 0 and successful demotion to UID 1000 on CPU Basic, then was deleted. The control plane runs as root inside the container. The daemon, providers, Git and shells run as the separate `agent` user in `/home/agent` with an allowlisted environment. Supervisor files are root-only under `/run/launchpad-space`. Management credentials, vault keys and platform secrets are excluded from daemon/agent environments. Agent tools legitimately can read their own live provider credentials; this is not a hostile-code sandbox.

## Recovery contract

- `backups/<generation>/`: consistent, credential-free SQLite snapshots, config projections and recoverable workspace/native files. Manifest and read-back verification complete before the latest pointer advances. All completed generations are retained.
- `credentials/<generation>/vault.aead`: AES-256-GCM with per-generation HKDF keys, fresh nonces and installation/schema/kind/generation/key identity bound as AAD. Credential changes are observed with file events plus five-second reconciliation, including logout deletions. The newest complete credential generation is restored coherently.
- `traces/`: Launchpad's existing native exporter and sidecars, with its existing minute-based cadence. Connect an HF agent token separately. Trace upload status is distinct from full recovery and credential acknowledgment.

State is snapshotted with SQLite's online backup API, projected and VACUUMed before upload. Schema fingerprints bind projection to the embedded binary; unclassified raw credential columns fail the build/projection. Profile environments and registered provider/SSH/Git credential files are sealed. Git histories containing registered credential filenames are sealed too; ordinary Git histories remain readable. Build caches, CLI temporary/debug directories, sockets, internal bearer tokens, browser registrations and push registrations are excluded. Native SQLite state is also snapshotted online. Archives never extract arbitrary paths, devices, hard links or escaping symlinks; stable local paths and executable modes survive restore.

Keep local folders and profile credential roots under `/home/agent`. Unregistered secrets in arbitrary code, prompts, attachments or tool output are the report's accepted plaintext tradeoff; private bucket access is their confidentiality boundary. Histories deliberately containing credential files depend on the vault key. Ordinary live snapshots are crash-consistent, not transactional Git snapshots. Clean maintenance checkpoints stop the daemon and its agent children and refuse unfinished Git operations. Snapshot staging currently uses memory and full-file uploads; measure CPU/RAM, upload time and recovery loss/time on representative repositories before unattended use.

A wrong/missing/corrupt credential generation opens reconnect recovery without replacing old ciphertext. Ordinary data is recovered; explicitly start a new generation and reauthorize tools if necessary. Restart first to retry transient transfer failures. No offline recovery key is required. Planned key rotation re-encrypts and verifies retained generations before saving a new Space Secret; interrupted/uncertain rotations retain a durable inspection record and are not blindly retried.

New browser launches/replies pause when ordinary recovery falls over ten minutes behind. Existing turns and the daemon's internal scheduling remain independent. Trace export retains its existing best-effort behavior.

## Releases and maintenance

`.github/workflows/hf-space.yml` builds and smoke-checks the image on manual dispatch; publishing a GitHub Release additionally publishes its immutable GHCR image, release manifest and template zip. Optional repository variable `HF_SPACE_TEMPLATE` and secret `HF_SPACE_TEMPLATE_TOKEN` refresh a public HF duplicate template automatically, including a display-only landing page with a Duplicate button. The copied template-ID variable identifies only the original Space; duplicates enter normal setup. Without them, the zip remains available as a release asset. No task-time publication occurs.

`release.json` is the single native CLI version source; keep its HF/uv entries aligned with their pinned Python requirements (the build checks this). Rust/UI changes stay in Launchpad. Provider pins, login flows, credential roots, recovery formats, HF policy, shell and image/update orchestration stay here. Bump the adapter version independently when its contract changes. The generic core integration is limited to trace prefixes/managed destinations, explicit delegated-access presentation metadata and packaged commit identity. Native authentication and Ask's restricted bearer remain unchanged.

Release checks run at startup and about every six hours with jitter/ETags. Candidates show the exact image, Launchpad revision, connector version, tools and release notes. Updates serialize admission, require idle agents/terminals and a verified clean checkpoint, and patch only a recognized managed Dockerfile with the expected parent revision. User-edited Dockerfiles require manual review. Restart, update, rollback, restore and key rotation are separate controls. No in-container binary auto-upgrades run.

See the [template's recovery instructions](template/README.md#recover-without-the-app) for image-plus-data rollback from Hub when the UI cannot boot, and [acceptance evidence](docs/acceptance.md) for performed checks and live deployment work still outstanding.

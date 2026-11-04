# Home-only Hugging Face Space (POC)

This deployment runs `launchpad daemon --role home`, and includes Claude Code, Codex, OpenCode, GitHub CLI and Hugging Face CLI for discovery, usage and quota checks. Tool versions are pinned by `deploy/hf-space/release.json`; the build verifies the executables and checks the GitHub CLI archive against its published checksum. Agent turns and general-purpose terminals still run on paired runners. It is separate from the older combined-daemon Space connector. Use a **private** Docker Space and a **private** bucket mounted read-write at `/backups`. The free `cpu-basic` tier is enough for a trial; it can sleep after inactivity.

The browser signs in through Hugging Face. On first start, Home shows a setup page with three required steps; Home opens only when all three are done, and the page comes back if one of them stops working. Each step runs a fixed login only after a click: the page shows the sign-in link (and the code to enter, for device logins) and updates when you approve it.

1. **Hugging Face**: Home's own HF login, verified against the private backup bucket. Right after it, Home creates a random 32-byte key, saves it as the `LP_HOME_KEY` Space secret with that login, and restarts once. If the login cannot add Space secrets, the page shows the key and asks you to add the secret yourself. Keep a copy of the key: without it, the stored credentials cannot be read.
2. **GitHub**: `gh auth login --web` for the Inbox, issues, pull requests, private clones and publication. A `GH_TOKEN` Space secret replaces this login.
3. **First profile**: sign in to Claude Code (Home runs `claude setup-token`; you open the link, approve, and paste the code back on the page) or to Codex (ChatGPT device login). The login goes into Home's vault and a profile uses it. A profile restored without its login (for example, created before the vault) gets the new login.

**Encryption.** With `LP_HOME_KEY`, only credentials are encrypted (AES-256-GCM): the vault secrets inside each backup copy of `state.db`, and Home's own HF and GitHub logins (`home/credentials/sealed/`, decrypted only to the container's local disk). Everything else (database rows, configuration, Git bundles, plain-folder contents, manifests) stays as plain files under `home/`, so you can browse the bucket and Xet can deduplicate it. Backups from the earlier fully encrypted layout (`home/sealed/`) are restored once, then deleted after the next complete backup.

**Settings → GitHub** links back to `/_home/setup`. Logins survive rebuilds and are never given to agents. `LP_HOME_BUCKET` selects the bucket for verification (default: `<Space ID>-bucket`). The gateway injects a local daemon credential only inside the container, checks the exact Space Host and browser Origin, and blocks local setup, browser pairing, push and desktop endpoints. Native runner pairing, the tunnel, Git and plain-folder traffic keep their own runner authentication. Startup refuses an unmounted bucket or anonymously readable Space metadata. No laptop credentials or Hub management token are installed in this image.

Home backup is mandatory: the gateway refuses config writes that disable or redirect it. The optional trace-sync controls are replaced with required Home backup status. The daemon writes database snapshots (vault secrets encrypted), configuration, Git bundles and plain-folder contents to `/backups/home/`, then writes the complete manifest last. On an empty boot it restores the newest manifest and verifies object hashes. The gateway recovers the configuration first and reapplies the fixed Space origin. Keep the bucket private anyway and grant no agent tool token access to this bucket or Home's Space.

## Deploy

From a clean committed checkout of the Home/runner branch:

```sh
hf buckets create Wauplin/launchpad-home-bucket --private
hf repos create Wauplin/launchpad-home --type space --sdk docker --private --flavor cpu-basic -v hf://buckets/Wauplin/launchpad-home-bucket:/backups
```

Export the committed repository to a temporary directory, copy `deploy/hf-home/Dockerfile` to its root, set its `ARG LAUNCHPAD_REVISION` default to the full source commit, and replace the root README with this Space card:

```yaml
---
title: Launchpad Home
emoji: 🏠
colorFrom: blue
colorTo: green
sdk: docker
app_port: 7860
header: mini
---
```

Upload the directory with `hf upload Wauplin/launchpad-home <directory> --type space`. Keep `.git`, build outputs, `node_modules` and credentials out of the upload. The image builds the UI before embedding it in the release daemon, limits Rust build jobs and installs tools after compilation to keep peak build memory lower. On a memory-constrained Space builder, deploy the executable exported from this locally built Linux image: replace the root Dockerfile’s build stages with a `COPY` of that artifact, verify its pinned SHA-256 before making it executable, and record its runtime and UI source commits separately. Keep this source Dockerfile for reproducible builds. Home and runners must report the same runtime source commit. A packaging or UI-only update can retain `LAUNCHPAD_REVISION` from the deployed runtime when the Rust sources are unchanged, keeping paired runners compatible; record the deployment commit separately in the Space card. Do not use the old Space updater or `packaging/install.sh` for this POC.

## Try it alongside an existing laptop install

Use the already prepared checkout at `/home/wauplin/projects/.worktrees/launchpad-h3`. The commands below build in that checkout and run a foreground runner on port **7320**, with `launchpad-runner-dev` data/config and a dev browser cookie. Runners do not run the daemon's Codex auto-updater. Harness discovery uses a separate empty home under the runner data folder, so it does not open or refresh the old install's logins. They do not install a service, replace `~/.local/bin/launchpad`, or use the existing daemon on port 7317.

1. Build that exact commit: build the UI with Node 24 (`cd web && pnpm build`), then run `cargo build --locked -p launchpad` from the checkout root. Check bubblewrap with `bwrap --ro-bind / / --dev /dev --proc /proc --unshare-pid true`. If it fails, fix bubblewrap/user-namespace support yourself before launching an agent; never set the runner boundary to `vm` on your laptop.
2. Start the runner in a terminal, from that checkout:

   ```sh
   LAUNCHPAD_ENV=dev LAUNCHPAD_PORT=7320 ./target/debug/launchpad daemon --role runner
   ```

3. In a second terminal, from the same checkout, request pairing:

   ```sh
   LAUNCHPAD_ENV=dev LAUNCHPAD_PORT=7320 ./target/debug/launchpad connect https://wauplin-launchpad-home.hf.space --name Laptop-POC
   ```

   This reads the laptop's existing HF sign-in, or `HF_TOKEN`, solely to enter the private Space. The runner credential saved after approval is different. No new HF login is performed by these commands.

4. Open [Home](https://huggingface.co/spaces/Wauplin/launchpad-home), signed in as Wauplin. In **Settings → Home and runners**, compare the pairing code, click **Approve**, and wait for the runner to show connected with the same build. If needed, the local runner UI is opened with `LAUNCHPAD_ENV=dev LAUNCHPAD_ROLE=runner LAUNCHPAD_PORT=7320 ./target/debug/launchpad url`.
5. On Home, add a workspace and a fresh plain folder or a throwaway public Git repository. Set your Git name/email in **Settings → Credentials and tools**. If you skipped the profile step during setup, add a harness login there (a Claude setup token or a separate Codex login) and create a profile that uses it. Do not copy the Codex login used by the old install: Home refreshes its copy, which can invalidate the old one. Do not import your production database or working folders yet.
6. Profiles and vault login IDs sync to runners. The corresponding harness CLI must already be installed on the laptop; this POC mounts it read-only. Grant tool credentials only when needed, with scopes excluding Home and the backup bucket.
7. Launch a small task with **Runner → Laptop-POC**. Confirm that the trace reaches Home, a committed change appears on Home's branch or Local PR, and **Back up now** completes in Settings → Home and runners. Keep the first work local.

GitHub access on Home is separate from tool credentials leased to agents. It comes from the setup page's GitHub step, or from a fine-grained `GH_TOKEN` secret you add in the Home Space's Hub settings (adding a secret restarts Home). The image configures Git's GitHub credential helper to use `gh`; no GitHub login is preinstalled. Publication still needs your click and was not exercised against a real repository in the deployment checks.

Stop the foreground runner with Ctrl+C when finished. Home remains deployed; the old laptop install keeps running. Build updates require rebuilding both Home and the runner from the same source commit.

## Trial limits

This is enough to exercise Home → laptop launch, follow-up, trace replication, Home Git, Local PRs and backups. It is not a migration-ready replacement. Remote Files/terminal relay, a complete laptop cache, recovery epochs/history-gap markers, session moves, Knowledge, Bots and Burst are unfinished. A crash can lose up to a minute of events acknowledged after the last backup; restoring an older backup does not yet reconcile that gap automatically. Keep the runner's native traces until migration and recovery are verified. SSH host sessions are not jailed. Local VM provisioning is unverified.

Jailed processes use session CLI credentials bound to the session generation. They may read context, create inert suggestions/proposals and Local PR descriptions, and finish or extend their own work; machine settings, terminals, file APIs and publication routes require an owner window. This does not make token scopes or untrusted repository instructions safe: start with disposable work and limited credentials.

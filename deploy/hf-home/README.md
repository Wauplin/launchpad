# Home-only Hugging Face Space (POC)

This deployment runs Home (`launchpad daemon`), and includes Claude Code, Codex, OpenCode, GitHub CLI and Hugging Face CLI for discovery, usage and quota checks. Tool versions are pinned by `deploy/hf-home/release.json` (`python3 deploy/hf-home/bump_tools.py` bumps the agent CLIs to the newest release of their major version; Burst jobs use the same pins, while paired runners update their own CLIs); the build verifies the executables and checks the GitHub CLI archive against its published checksum. Home runs no agents or terminals: agents run on paired runners (else on Burst), and terminals and Continue in TUI open from Home's window on the runner of the agent or folder, relayed through that runner's tunnel. Use a **private** Docker Space and a **private** bucket mounted read-write at `/backups`. The free `cpu-basic` tier is enough for a trial; it can sleep after inactivity.

The browser signs in through Hugging Face. On first start, Home shows a setup page that goes from an empty Home to a first agent. The required steps take 10 to 15 minutes, mostly sign-ins. Home opens when the required steps (1 to 7) are done; the optional steps never keep it closed, and **Skip for now** marks them skipped. The page comes back if a required step stops working. **Settings → Onboarding** links to it at any time, and **Open** next to a finished step shows that step again to review or change it. The setup page, Home's **Setup** page and the sidebar's **Finish setup · N left** (required steps only) read the same list of steps from `/_home/status`. Sign-ins and restarts start only on a click: the page shows the sign-in link (and the code to enter, for device logins) and updates when you approve it.

1. **Hugging Face**: Home's own HF login, checked against the private backup bucket.
2. **Credential key**: Home generates a strong key, or you type your own (or the key of an earlier Home). The page never displays a generated key: you copy it or download it as a file, then confirm you saved it. The page then explains why Home must restart, and **Save key and restart Home** saves the key as the `LP_HOME_KEY` Space secret and restarts. If the HF login cannot add Space secrets, the page asks you to add the secret yourself. When the bucket holds credentials of an earlier Home, the page tests the key against them first and says "key does not match" instead of saving a wrong one; a wrong `LP_HOME_KEY` secret at boot shows the same message on every page, and Home does not start until you fix the secret.
3. **GitHub**: `gh auth login --web` for the Inbox, issues, pull requests, private clones and publication. A `GH_TOKEN` Space secret replaces this login. The step is done only while GitHub accepts the token (checked again every minute while the page is open): a revoked or expired login shows as refused, with **Sign in to GitHub** again, but Home stays open.
4. **Git identity**: the name and email of agents' commits, suggested from the GitHub account (its noreply address).
5. **Agent profile**: sign in to Claude Code (Home runs `claude setup-token`; you open the link, approve, and paste the code back; if Claude Code neither accepts nor rejects the code within 90 seconds, the page shows what it says) or to Codex (ChatGPT device login). The login goes into Home's vault and a profile uses it. You can sign in to both agents: each login becomes its own profile. Then choose each profile's default model and effort.
6. **Workspaces**: name and create at least one workspace (Launchpad has no default workspace); add more now or later. The step stays open until **Continue**.
7. **First folder**: clone repositories from GitHub into a workspace you choose. Cloning is possible only once a workspace exists. Click several in a row: they clone one after another.
8. **Where agents run** (recommended): copy the install command, run it on a Linux machine, and approve the request on the page: a waiting request opens this step with the runner's code and **Approve**, also when the step is done or skipped. The step stays done once a runner is paired, also while it is offline. Without a runner, agents run on Burst.
9. **First agent** (recommended): Home asks an agent “What is this project about?” on the new folder and runner, then opens the session.

**Encryption.** Home derives an AES-256-GCM key from `LP_HOME_KEY` (scrypt). Only credentials are encrypted: the vault secrets inside each backup copy of `state.db` (harness logins and tokens), and Home's own HF and GitHub logins (`home/credentials/sealed/`, decrypted only to the container's local disk). Everything else (database rows, configuration, Git bundles, plain-folder contents, manifests) stays as plain files under `home/`, so you can browse the bucket and Xet can deduplicate it. Without the key, the stored credentials cannot be read: keep the copy you saved during setup. When Home cannot reach Hugging Face to verify its login at boot, it checks again with backoff (and while the setup page is open) instead of staying locked.

**Settings → GitHub** runs the same GitHub sign-in when Home is not signed in (open the link, enter the code), and links back to `/_home/setup`. **Settings → Credentials and tools** adds Claude Code and Codex logins the same way. Logins survive rebuilds and are never given to agents. `LP_HOME_BUCKET` selects the bucket for verification (default: `<Space ID>-bucket`). The gateway injects a local daemon credential only inside the container, checks the exact Space Host and browser Origin, and blocks browser pairing and device endpoints (the browser signs in through Hugging Face). Native runner pairing, the tunnel, Git and plain-folder traffic keep their own runner authentication. Startup refuses an unmounted bucket or anonymously readable Space metadata. No laptop credentials or Hub management token are installed in this image.

Home backup is mandatory: the gateway refuses config writes that disable or redirect it. Settings → Hugging Face shows the required Home backup status. The daemon writes database snapshots (vault secrets encrypted), configuration, Git bundles and plain-folder contents to `/backups/home/`, then writes the complete manifest last. On an empty boot it restores the newest manifest and verifies object hashes. The gateway recovers the configuration first and reapplies the fixed Space origin. Keep the bucket private anyway and grant no agent tool token access to this bucket or Home's Space.

## Deploy

`deploy.sh` installs or updates a Home Space with the hf CLI:

```sh
curl -fsSL https://raw.githubusercontent.com/Wauplin/launchpad/main/deploy/hf-home/deploy.sh | bash
```

It checks `curl`, `python3` and the hf CLI (recent enough for buckets and Space volumes) and your Hugging Face sign-in. It then creates or checks the private bucket `<Space ID>-bucket` and the private Docker Space (default `<your user>/launchpad-home`, `cpu-basic`, the bucket mounted read-write at `/backups`). It refuses a public bucket or Space, other volumes, or a Space Dockerfile it did not write (`--force` replaces it). It uploads the Dockerfile and the Space card (and deletes other files of the Space), waits until the Space runs the uploaded commit, and checks that `/_home/status` answers. Options (after `bash -s --`):

- `--release <tag>`: deploy this release (default: the latest one).
- `--main`: deploy the latest `main`. CI builds the package of every push to `main` and attaches it to the `main-build` prerelease as `launchpad-linux-x86_64-<commit>.tar.gz` (the 20 newest commits). The script takes the newest `main` commit that has one (CI needs about 15 minutes after a push) and the Space uses it with `Dockerfile.release`, like a release. Nothing is compiled on the Space builder.
- `--space <[namespace/]name>`, `--bucket <namespace/name>`: other names. A bucket that is not `<Space ID>-bucket` is set as the `LP_HOME_BUCKET` Space variable.
- `--no-wait`: return after the upload.

Run it again to update. The manual steps below do the same.

### Updates from Home

Home updates its own Space. Every hour it reads the Space's Dockerfile to know what it installs (a release, or a `main` commit with `--main`) and checks GitHub for a newer one on the same channel. A newer one shows in Today (Fix) and in **Settings → Home and runners → Home updates**, which also has **Check for updates**. **Update Home** commits the newer Dockerfile and Space card to the Space with Home's own Hugging Face login (it needs write access to the Space; else update with `deploy.sh`). The Space rebuilds and restarts; Home is unavailable for a few minutes, and agents keep running on their runners. With **Then update the runners** (on by default), the new Home asks each connected runner with another build to update itself once none of its agents is in a turn, and does the same for runners that connect in the next 24 hours. A bot with admin access to the Launchpad CLI (such as Ask Launchpad) can do the same when you ask: `launchpad home check-update`, then `launchpad home update` (the newest of the deployed channel; `--channel release|main` switches channel, `--version` names the exact one). The call returns once the Hub accepted the commit, before the restart; the agent keeps running on its runner. It never updates the runners: that stays your click. Other agents can prepare **Update Home** as a ready card. A runner of an older protocol cannot connect to the new Home: run the install command again on it. Home never replaces a Dockerfile that `deploy.sh` did not write or a Space built from source.

To deploy by hand, create the bucket and the Space:

```sh
hf buckets create Wauplin/launchpad-home-bucket --private
hf repos create Wauplin/launchpad-home --type space --sdk docker --private --flavor cpu-basic -v hf://buckets/Wauplin/launchpad-home-bucket:/backups
```

### From a release (recommended)

Upload `deploy/hf-home/Dockerfile.release` as the Space's `Dockerfile`, with the Space card below as `README.md`. The image downloads the release archive of the tag in `ARG LAUNCHPAD_RELEASE`, checks it against the release's `.sha256` file, and takes the deploy files (gateway, setup page, runner installer, tool versions) from the source of the commit named in the archive. For a main commit, set `LAUNCHPAD_RELEASE=main-build` and `LAUNCHPAD_PACKAGE=launchpad-linux-x86_64-<commit>`. Nothing is compiled on the Space builder. To update Home, edit the `LAUNCHPAD_RELEASE` line in the Space's Dockerfile on the Hub to a newer tag: the Space rebuilds and restarts. Runners update with one click in Settings → Home and runners, or with the install command.

### From source

From a clean committed checkout, export the committed repository to a temporary directory, copy `deploy/hf-home/Dockerfile` to its root, set its `ARG LAUNCHPAD_REVISION` default to the full source commit, and replace the root README with this Space card:

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

## Add a runner

Settings → Home and runners (and step 8 of the setup page) shows the install command for this Home:

```sh
(t="${HF_TOKEN:-$(cat "${HF_HOME:-$HOME/.cache/huggingface}/token" 2>/dev/null)}"; [ -n "$t" ] || { echo "No Hugging Face sign-in: run 'hf auth login' first." >&2; exit 1; }; s="$(curl -sSL -w "%{http_code}" -H "Authorization: Bearer $t" https://<your Space host>/_home/runner/install.sh)" || { echo "Could not reach Home." >&2; exit 1; }; c="${s: -3}"; s="${s%???}"; if [ "$c" = 401 ]; then echo "Home refused your Hugging Face sign-in (401): run 'hf auth login' with an account that can open this Space." >&2; exit 1; fi; [ "$c" = 200 ] || { printf "Download failed (HTTP %s): %.300s\n" "$c" "$s" >&2; exit 1; }; bash -c "$s")
```

Run `hf auth login` on the machine first, or set `HF_TOKEN`: the HF token passes the private Space's sign-in. The command works in bash and zsh, reads `HF_TOKEN`, else the `hf auth login` token in `$HF_HOME` (default `~/.cache/huggingface`), keeps it out of your shell, and prints Home's error message when the download fails (with the sign-in hint on a 401). The setup page and Settings → Home and runners show the same command, given by Home's gateway. The script, served by Home, does the following:

1. It checks the machine: Linux x86_64 (on a Mac or an ARM machine, use Burst instead), glibc 2.34 or newer, `curl`, Git, a working bubblewrap jail and a Hugging Face sign-in. It names the agent CLIs it finds. When a check fails, it prints the fix (for example the AppArmor profile Ubuntu 24.04 needs for bubblewrap) and stops. It never uses `sudo`.
2. It downloads Home's own `launchpad` binary (`/_home/runner/launchpad`), checks its SHA-256, and installs it in `~/.local/lib/launchpad-runner/`, with a `launchpad-runner` command in `~/.local/bin`. Home and the runner always run the same build. Another Launchpad install on the machine is left alone: the runner has its own folders and port (7320).
3. It starts the runner as the `launchpad-runner.service` systemd user service (without systemd, it prints the command to run) and enables lingering for your user (`loginctl enable-linger`), so the runner keeps running after you log out. When lingering cannot be enabled, it prints a note and continues. The service gets `HF_TOKEN` or the token file path in `~/.config/launchpad-runner/env` when the token is not in the default place.
4. It runs `launchpad connect`, which prints a code. Approve the request with the same code on the setup page or in Settings → Home and runners. A runner that is already paired with this address skips this step only while Home still accepts its credential: when Home was recreated at the same address, or the runner was removed in Settings, it asks to pair again.

Run the same command again to update a runner after Home is updated. The agent CLI of each profile must be installed on the runner; this POC mounts it read-only. Grant tool credentials only when needed, with scopes that exclude Home and the backup bucket.

GitHub access on Home is separate from tool credentials leased to agents. It comes from the setup page's GitHub step, or from a fine-grained `GH_TOKEN` secret you add in the Home Space's Hub settings (adding a secret restarts Home). The image configures Git's GitHub credential helper to use `gh`; no GitHub login is preinstalled. Publication still needs your click and was not exercised against a real repository in the deployment checks.

Stop a runner with `systemctl --user stop launchpad-runner`. Remove it on Home to unpair it.

## Trial limits

This is enough to exercise Home → runner launch, follow-up, terminals, trace replication, Home Git, Local PRs and backups. It is not a migration-ready replacement. Recovery epochs and history-gap markers are unfinished. When no runner is online, Home runs sessions on Burst (HF Jobs in Home's namespace from this Space's image, with Home's Hugging Face login; on by default with a 20 USD monthly cap, Settings → Home and runners); sessions move between runners between turns. Each workspace has a knowledge base (Knowledge in the sidebar) and its bots (Bots in the sidebar: one thread each with its routines, instructions in `BOT.md`, notes in the knowledge base); after each backup, Home writes readable copies of both under `<workspace>/kb/` and `<workspace>/bots/` in the bucket, and the original conversation files of every session (one file per trace, with `launchpad.json`) under `<workspace>/traces/<folder>/<YYYYMMDD_HHMM>_<session id>/`. A crash can lose up to a minute of events acknowledged after the last backup; restoring an older backup does not yet reconcile that gap automatically. Keep the runner's native traces until migration and recovery are verified. Local VM provisioning is unverified.

Jailed processes use session CLI credentials bound to the session generation. They may read context, create inert suggestions/proposals and Local PR descriptions, and finish or extend their own work; machine settings, terminals, file APIs and publication routes require an owner window. This does not make token scopes or untrusted repository instructions safe: start with disposable work and limited credentials.

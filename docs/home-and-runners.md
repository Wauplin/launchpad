# Home and runners

Work in progress: this implements the [Home and runners plan](https://wauplin-launchpad-bots-research.static.hf.space/index.html) (H1 to H5). The single-process mode (`launchpad daemon`, role `all`) is Home and a runner in one process: it uses Home Git too.

- **Home** (`launchpad daemon --role home`) keeps every record and runs no agents or terminals: its windows show no Terminal buttons (open terminals from a window of the runner).
- **Runners** (`launchpad daemon --role runner`) run agents. A runner works in its own folders (`~/.local/share/launchpad-runner`, `~/.config/launchpad-runner`) and port (7320), and never reuses a single-process install's data.

For a Home in a private Space, see the [Home-only deployment](../deploy/hf-home/README.md).

## Pair a runner

On a Home in a Space, open **Settings → Home and runners**: the Runners card lists the runners and requests waiting for approval, and **Add a runner** shows the steps and the install command (one line, with Copy). The **This computer** card sets the folders root and the command that opens links. Run `hf auth login` on a Linux x86_64 machine (or set `HF_TOKEN`), then run the command there: without a Hugging Face sign-in, it says so and stops. It checks the machine, installs Home's own build next to any other Launchpad, starts the runner as a user service that keeps running after you log out, then asks for approval. On a Mac or an ARM machine, use Burst. Run it again to update the runner. See [Add a runner](../deploy/hf-home/README.md#add-a-runner).

On another Home, pair by hand:

1. Start the runner daemon on the machine: `launchpad daemon --role runner`.
2. Run `launchpad connect <Home address>` there (with `LAUNCHPAD_ROLE=runner`). It prints a code such as `34Z-K9Z` and waits. For a Home in a private Hugging Face Space, the runner sends your HF token (`HF_TOKEN` or the `hf auth login` token) to pass the Space's sign-in.
3. On Home, open **Settings → Home and runners**, check that the code matches and click **Approve**. Requests expire after ten minutes.

The runner keeps one outbound WebSocket to Home (no inbound port) and reconnects on its own. **Remove** on Home disconnects a runner; it must pair again.

## Work started from Home

Every folder is stored by Home, so a launch on Home (from the phone, another browser, a schedule) goes to the first runner that takes launches, whatever the folder: the runner clones a Git folder from Home Git and syncs a plain folder on first use. Adding a folder on a runner sends it to Home (a Git checkout becomes a Git folder of Home, other folders a plain folder); no folder belongs to one runner's disk. Only a new agent in an existing temporary folder goes to the runner that has it now; **Keep as folder** makes it a folder of Home. **New agent here** and **Run in** an existing worktree go to the runner of that session, where its folders are; when that session's Burst job has stopped, the session first moves to a runner that takes launches (its worktrees come back from their branches on Home Git). Follow-ups, Stop, answers and the other session actions on Home go to the session's runner; Home serves the record, the events and the trace itself. Worktree actions (Merge, Discard, Create Local PR, Restore folder, Fix with agent) go to the runner of the worktree's session too, and Review → Agent branches lists the worktrees of the connected runners. When a session's runner is offline or its Burst job has stopped, Home answers from the branches it holds and the session's last handoff: Changes shows the committed branch, Review → Agent branches lists it, and **Merge** and **Create Local PR** run on Home. Uncommitted files and commits that were not pushed stay on that machine; Discard, Restore folder and Fix with agent wait for the runner (or a message that continues the session). When Git cannot be read, the session's primary button is **Review work**, never Archive. When that runner is offline, paused or refuses launches from other devices, Home says so and starts nothing. Ask Launchpad threads, accepted proposals, crons, scheduled launches and bot messages started on Home go to a runner the same way, sent once and never retried.

Profiles (including vault login IDs), workspaces and Git folders belong to Home: a runner shows Home's and sends its changes to Home, so they need Home to be reachable. A runner never creates a workspace: until Home answers, its window waits for Home's workspaces. **Clone from GitHub** registers the repository on Home, from any window; it is the first tab of **Add folders**, and the only way to add a Git folder on Home itself. **Add folder → Browse** on a runner's window (it warns that Home backs the folder up to its cloud bucket) registers a local checkout's remote on Home (a checkout without a remote is imported into Home Git); the runner then works in its own clones, never in your checkout. Each runner keeps a local log of the launches that came from elsewhere (Settings → This runner).

## Burst

When a launch from Home finds no runner that takes it (your laptop is off or asleep), Home starts it on **Burst**: an HF Job in Home's namespace, from Home's own Space image, so it runs the same build. Choose **Burst** in the composer's **Runner** option to use it on purpose. Crons, scheduled launches, bots and Ask Launchpad use it the same way when no runner is online. A message to a session whose runner is offline (yours, a bot's, a routine's, a hook's or Telegram's) moves the session to Burst first, then sends the message there.

- **No token in the job.** Home dials the job's runner through the HF Jobs proxy (the job cannot reach a private Home). The job holds no credential: its proxy sends every request that needs a token through the tunnel, and Home adds the token, sends the request and records it in the session's access log. Home Git and plain folders go through the tunnel too. Agents run without a jail inside the job: the job is the boundary. They run under the same [network capture](#network) as everywhere, and all their connections leave from Home. The job's runner cannot be read or attached to by its agents, and they never receive its credential.
- **Stopped when done.** One job runs one session. When its turn ends, the runner pushes the branches, saves uncommitted files to a scratch branch and sends the conversation to Home. After Home's next complete backup, Home cancels the job. Your next message starts a new job, which takes over the session and resumes its conversation (about 30 seconds); a message sent while the job is stopping does the same. If that job fails, the session shows Failed with the reason, and the message was not sent. A job without Home for 10 minutes stops on its own. A job that waits for your answer to a question keeps running, and the session says so after an hour. A job that cannot hand its session off (for example, a merge in conflict) says why in the session and stops after two hours; files not committed there are lost.
- **Cost.** Settings → Home and runners shows this month's Burst cost, the cap (20 USD by default) and recent runs, with **Stop**. At the cap, Home starts no new job. The hardware is `cpu-basic` by default (0.01 USD per hour).

Burst needs Home to run in a Hugging Face Space, with its Hugging Face login, and an account that can run HF Jobs: a PRO account or credits (a Team or Enterprise plan, or credits, for an organization). Home checks it, and the setup page and Settings say when Burst is not available and why.

## MCP servers

Settings → Credentials and tools → **MCP servers**: add a remote MCP server (Streamable HTTP) with its address and token. Home reads its tools. Every tool starts off, also tools that a later **Refresh tools** finds: turn on each tool agents may use (the server's read only and destructive labels help, but they are hints, not guarantees). Choose where the server is granted when you save: the current workspace by default, chosen workspaces or every workspace. Which sessions get it by default is set by the [default tools](#default-tools).

Agents on every runner, Burst included, reach a granted server at a local address on their runner proxy. Each request goes through the tunnel to Home's gateway, which checks the session's lease, hides and refuses the tools that are off, adds the token and forwards the request. The token never leaves Home. Each tool call appears in the session's access log. A tool that is on acts directly with the server's token.

## Default tools

A grant decides where a tool or an MCP server can be used. The default tools decide which of the granted ones a new session gets without a click. There are three levels, and the most specific one wins:

- **Workspace:** Settings → This workspace → **Tools and MCP servers**, On or Off for each granted item. Without a choice, an item is on.
- **Folder:** the folder's Settings tab → **Default tools**: Inherit (the workspace's choice), On or Off.
- **Bot:** the bot's Settings tab → **Default tools** (`[tools]` in `bot.toml`, for example `gh = false` or `"mcp:gmail" = true`), over the workspace and the bot's folder.

The composer's **Tools** option starts from these defaults for the chosen folder and switches any granted item on or off for one launch. Home applies the defaults each time it builds a session's lease, so a changed default applies at the session's next start or resume. An admin session can set the workspace and folder defaults with `launchpad do tools.defaults workspace|folder <id> --tools '{"gh": false}'`.

## Notifications and routines

Home sends the phone's notifications through Telegram for every session, wherever it runs: a runner forwards the notification of a session that finishes, fails or is interrupted to Home, which sends it to your Telegram chat, while your laptop keeps its own desktop notification. Routines and the morning brief run on Home's schedule, on a runner when one takes the run, otherwise on Burst ([Routines](bots.md#routines)).

## Move a session

Between turns, a session can move to another runner: in its Summary, **Runs on** → **Move to…** lists the connected runners and Burst. The old runner pushes the branches, saves uncommitted files to a scratch branch and removes its folders; the new one clones the worktrees again from Home Git on the next message, and the harness resumes the same conversation (Claude Code and Codex). The move increases the session's generation, so the old runner can no longer push its branches or get a lease. Home records the runner of each turn end it receives, so the Summary's **Turns ran on** shows which machine ran which turns.

Every runner sends the conversation of a session to Home at the end of each turn, when it changed. So a session can also move while its runner is offline (a sleeping laptop): Move to Burst continues it from the last turn's branches and conversation; files that were not committed stay on that machine.

A session in a temporary folder (a bot without a folder, such as Ask Launchpad) keeps it when it moves: Home holds the folder's files as of the last turn end, and the new runner puts them back in a folder with the same name.

Home is the place that keeps conversations. Agents save their conversation files normally, in the session's own folder. About an hour after a session is archived, its runner sends that conversation to Home one last time. When Home has confirmed it, the runner deletes the session's folder (`<data_dir>/sessions/<id>`). Resume gets the conversation back from Home before the agent starts.

## Sessions on Home

Every session a runner runs is saved on Home: its record and every event, in order. The runner writes them to a local outbox in the same transaction as its own copy and drops them only after Home has saved them. While Home is unreachable, sessions keep running and accept your messages on the runner; the outbox is sent when the tunnel is back. Home never marks a runner's session Interrupted when Home itself restarts.

While a runner is disconnected (a sleeping laptop), Home shows "<runner> offline" next to its sessions' status: the status is the last one Home received. This is a runner state, never a session status. Seen and unread are kept on Home, so a finished session leaves Today even while its runner sleeps. **Archive** works too: Home closes the session at once, and the runner archives it (and stops it, if it still runs) when it reconnects. When you **Remove** a runner, its sessions that were running show Interrupted, and Archive closes them on Home.

Each session has a generation number, which grows when a session moves to another runner. Home ignores records and events of an older generation.

## Home Git

Home keeps a mirror of every Git folder: every branch of the folder's remote, plus Launchpad branches (`lp/…`), which only agents push. Runners clone from Home, fetching most objects directly from the remote first.

- Every session starts in a fresh worktree of its branch, cloned before the agent starts. At the end of every turn the runner pushes the branch to Home, and every five minutes it snapshots running worktrees. A worktree is removed only after Home confirmed its branch; uncommitted files are saved to an `lp-scratch/…` branch first, kept 30 days.
- `origin` in an agent's worktree is Home Git, through its runner: agents never hold a credential for it. A runner that lost a session to another runner can no longer push its branches.
- Only Home changes base branches, through **Merge** (a push to the folder's remote, then a fast-forward on Home; a refused push merges nothing) and **Publish** (the reviewed commit). A base branch that moved goes back to the agent for a rebase.
- Home keeps every previous tip of a Launchpad branch for 30 days, also after force pushes and deletes. The folder page lists Launchpad branches and their previous tips, with **Restore**.
- Local PRs live on Home: diff, review, Merge and Publish work from the phone while the laptop sleeps. Local PRs opened by one agent form a change set, published together.
- Sessions that continue a GitHub pull request push to Home; Home pushes the branch to the pull request after each push, never forced. Published Local PRs work the same way.

## Files on Home

Home keeps each Git folder as a bare mirror. For **Files**, it keeps a read-only copy of the default branch next to it, which borrows the mirror's objects and lists every branch as `origin/<branch>`. The copy is refreshed when Files opens, at most every 30 seconds. Plain and temporary folders get a read-only copy of the files Home holds. Files of a session that runs on a connected runner (its worktrees, plain copy or temporary folder) are read on that runner, as they are now. When that runner is offline or its Burst job has stopped, Home shows a worktree from its branch on Home Git, and a plain or temporary folder from the files of the last turn.

## Plain and asset folders

A **plain folder** has no Git: Home stores its files (Add folders → **New plain folder**, or add a folder without Git from Browse on a runner, which copies its files once). Runners work in a synced copy: it comes down from Home before a session starts and goes back at the end of every turn. One agent works in a plain folder at a time; another launch fails until the first agent is archived. Changes compares with the start of the session. Files shows the files Home holds (the state at the end of the last turn), read-only. Home keeps every overwritten version and every deleted file for 30 days: the folder page lists them under **Versions and trash**, with **Restore**.

A plain folder can become a Git folder: on its folder page, open **Settings** and click **Turn into a Git folder**. Its current files become the first commit of `main` in a new Home Git folder without a remote; agents then work in worktrees and branches.

An **asset folder** holds large or generated files of a Git folder (datasets, media, model files) outside Git. An agent runs `launchpad assets create <path> --session-id <id>` in its worktree: the path is mapped in `.launchpad/assets.toml` (committed), kept out of Git, and synced like a plain folder, so later sessions on that branch get the same files. Assets are shared, not pinned per commit.

The folder page of a Git folder lists its asset folders with their size. An asset folder that no branch, kept tip or knowledge base note names, and that nobody changed for 14 days, goes to the trash: its files are removed and **Restore** puts them back for 30 days.

Contents are stored by hash and included in Home backups.

## Backups

Home backs up its database, `config.toml` and a Git bundle of each mirror's Launchpad branches and kept tips (only what the folder's remote does not have) to `[home] backup` (`hf://buckets/<you>/launchpad` or a folder) every minute when something changed, and before a planned stop. Objects are stored by content hash under `home/blobs/sha256/` and never overwritten; each backup is a manifest under `home/backup/<time>/`, written last, so a backup is complete only when its manifest exists. Settings → Home and runners shows the last complete backup and has **Back up now**. Backups older than a day are deleted, except the newest, together with the objects no kept backup needs. After each backup, Home writes readable copies of each workspace's knowledge base and bots (`<workspace>/kb/…`, `<workspace>/bots/<bot>/…`) and removes the copies of deleted files; restores never use them.

When Home starts on an empty data folder, it restores the newest complete backup before it opens the database, and each mirror gets its bundle back when it is cloned again (on a first boot without `config.toml`, `LAUNCHPAD_HOME_BACKUP` names the destination). Bundles include snapshot refs and remain in later backups until their mirrors are opened again. Runners then reconnect with their existing credentials and resend what Home has not acknowledged. After an unplanned restart, up to a minute of events acknowledged since the backup can be missing.

## Credentials and tools

Home’s image includes Claude Code, Codex, OpenCode, `gh` and `hf` for discovery, usage and quota checks. A hosted Home opens on a setup page that leads from an empty Home to a first agent: Hugging Face sign-in, the credential key (generated or your own, saved by you, then kept as the `LP_HOME_KEY` Space secret after a restart you confirm), GitHub sign-in, the Git identity, a first profile (Claude Code or Codex subscription sign-in, stored in the vault, with its default model and effort), a workspace with a first cloned folder, then the optional steps: a runner (done once paired, also while it is offline) and a first agent. Home opens when the required steps are done; optional steps never keep it closed. The GitHub step counts only while GitHub accepts the token; if it is later revoked or expires, the step and Settings → GitHub show it as not signed in (Home stays open) and **Sign in to GitHub** in Settings runs the same sign-in as the setup page. **Settings → Onboarding** opens the setup page again at any time, and **Open** next to a finished step shows it again to review or change it (sign in again, edit the Git identity, choose another model, clone more folders, approve a runner). Each sign-in and restart starts only on a click. Credentials in the bucket (vault secrets in backups, Home's own logins) are encrypted; other data stays as plain files you can browse. Agents never get Home's logins. Its backup is mandatory, with status in Settings → Hugging Face. After each backup, Home also copies every agent's original conversation files to the bucket, one file per trace, under `<workspace>/traces/` ([Hugging Face](hugging-face.md#traces-in-the-home-bucket)). On Home, the **Setup** page lists the same steps with their state and where to change them, and the sidebar's **Finish setup · N left** counts only the required steps.

Home keeps every credential agents use: open **Settings → Credentials and tools** (on Home, or on a runner, which forwards to Home).

- **Harness logins:** a Claude Code token (`claude setup-token`) and a Codex ChatGPT login (from then on only Home refreshes it, so do not use that login elsewhere). On a Home in a Space, **Add a Claude Code login** or **Add a Codex login** runs the sign-in on Home itself, like the setup page: open the link, approve with the subscription account (for Claude Code, paste the code it shows back), and the login goes into the vault with an optional name and a new profile that uses it. Elsewhere, paste the token or `auth.json` (**Paste a login instead** in a Space). A profile can name its login; otherwise the harness's first login is used.
- **Tokens:** tokens and API keys for tools. Secrets are write-only: Launchpad never shows them again.
- **Tools:** adding a tool asks only for its API token, with a link to create one. For GitHub CLI and Hugging Face CLI, **Use Home's token** lends Home's own login instead (agents then act with Home's full access: a fine-grained token is safer). The **Advanced** section holds the rest. Each entry says how a runner installs the CLI (GitHub release, download URL, `cargo`, `uv tool` or `npm`), which variables get a placeholder (for example `GH_TOKEN`), on which hosts the proxy adds which token (Bearer, Basic, a header or a query parameter), and in which workspaces it is granted. **Settings** holds values that are not secret, one `KEY=value` per line (an endpoint, a read-only switch, an output format): runners set them only when the tool's own commands run, through a small wrapper script on the agent's PATH, never in the agent's environment, so they need an install. GitHub CLI and Hugging Face CLI are ready-made; **Copy as TOML** exports the list. Which granted tools a new session gets is set by the [default tools](#default-tools). The proxy adds one token per host: when two tools that are on send different tokens to the same host, a launch gets only the first one by name, and switching the other on in the composer's Tools switches the first one off.
- **Slack:** the **Slack** card sets up [slackline](https://github.com/co42/slackline) in two modes, each with its own Slack app and token. **Slack (read-only)** has read permissions only and the setting `SLACKLINE_READONLY=1`, which turns off every write command. **Slack (write)** can also post, reply, react, pin and upload. For each mode, **Open Slack to create the app** opens Slack's "Create app" page with the name and permissions filled in (the same page as `slackline token create`, or `--write`): pick your Slack workspace, click **Create**, open **Install App**, click **Install to Workspace**, then copy the **User OAuth Token** (`xoxp-…`) and paste it. Home tests the token after you save it. **Delete** removes the token from Home's vault (the tool and its workspaces stay; sessions lose it at their next lease renewal) and shows the steps again: to replace a token, delete it and paste the new one. Delete does not revoke the token in Slack: it stays valid there until you uninstall the app. Both modes are granted in every workspace: a launch gets read-only, and you pick **Slack (write)** in the composer's Tools when an agent must post, or turn read-only off in the [default tools](#default-tools) of a folder or bot that always posts. The setup page shows the same card as a recommended, optional step.
- **Proposed tools:** an agent that needs a CLI or API can propose a tool (`launchpad propose tool`). The card (in Today → Approve) shows what runners would install, the variables and the hosts that receive the token; **Add tool** adds it after you confirm (its token is always a new entry, never an existing one), then **Paste its token** opens its form. Tool proposals never start in auto mode.
- **Test:** a tool with a check URL (GitHub, Hugging Face and Slack have one; set it under Advanced for others) has **Test**: Home sends one request there with the token and shows whether the host accepts it (a 2xx answer, without `"ok": false` in a JSON body). The token never leaves Home.
- **Git identity:** the author of agents' commits on every runner.

When a session starts, its runner gets a **lease** from Home: the session's login, its granted tools and their tokens. The runner keeps the lease in its proxy's memory only. Agents see placeholders: when a request goes to a host of the lease, the proxy adds the real token and records host, method and path in the session's access log (Summary → Credentials), naming actions such as "opened PR #3421". Other traffic passes through untouched. What a request may do is decided by the token's scopes: use fine-grained tokens. **Revoke** (Summary → Credentials, or the list of active leases) stops adding the tokens at once while the runner is connected; Stop and Archive end the lease too, and a runner that cannot reach Home drops it after an hour.

Quota: Home reads the quota of each vault login once and sends the readings to every runner, so quota protection works the same everywhere.

## The jail

Every agent runs contained. On a Linux runner (and in the single-process mode) it runs in a bubblewrap jail (`sudo apt install bubblewrap`; Ubuntu 24.04 needs user namespaces allowed for `bwrap`). Inside, the agent sees the system and the machine's toolchains read-only, and only its own folders read-write: its worktrees and plain folders, its harness home (`~/.local/share/launchpad-runner/sessions/<id>/home`, with its own `CLAUDE_CONFIG_DIR` and `CODEX_HOME`), its own `/tmp` and the build caches of its workspace (Cargo, and `sccache` for Rust builds when it is installed on the runner). Your home folder, `~/.ssh`, `~/.config/gh`, `~/.claude`, Launchpad's data and the Docker socket do not exist inside. Title requests and **Continue in TUI** use the same jail and lease. Sessions on SSH hosts are not contained yet.

### Network

An agent reaches the same addresses with the same credentials in the jail, on a Local VM and on Burst, whatever tool it uses. Every agent process runs under Launchpad's **network capture**: a small supervisor that the agent cannot remove (a seccomp filter) hands each connection to the runner proxy, also for programs that ignore `HTTPS_PROXY`, static binaries and background processes. So a tool works on every runner or on none.

- **Where traffic leaves:** on your runners, from the runner's own network, so agents also reach its LAN and tailnet (a private GitHub server, a local database). On Burst, every connection leaves from Home, which reaches public addresses only.
- **Tokens:** on a host of the lease (HTTPS on port 443), the proxy adds the token as above, also for a client that ignores proxy settings. The runner CA is in the sandbox's system certificate store, so Go, OpenSSL and most Python and Rust clients trust it. A client with its own built-in certificate list fails with a visible TLS error on those hosts only, the same way everywhere.
- **Policy:** `localhost` is reached directly (the Launchpad CLI, harness logins, the agent's own servers). UDP reaches DNS (port 53) only: QUIC falls back to TCP, and other UDP (NTP, `ping`, WebRTC) is refused. IPv4 and IPv6 follow the same rules. io_uring is blocked.
- **Requirements:** Linux 5.9 or newer, on x86_64 or arm64. A runner where the capture cannot run starts no agent and says why.
- Processes an agent leaves running after its main process exits lose their network.

**Check network** (Settings → Home and runners, on each connected runner) runs a fixed probe in a real session sandbox on that runner: capture, DNS, refused UDP, IPv4 and IPv6, the runner CA in the system store, and a connection the proxy received. A failed required check marks the runner **network differs**; IPv6 depends on the runner's network and is only reported.

### Local VM

Docker needs root, so it never runs in the jail. A **Local VM** is a runner inside a virtual machine, where the VM is the boundary: agents run without a jail and may use Docker, with the same lease, proxy and network capture. Agents in the VM cannot use `sudo` (the capture forbids gaining privileges), and containers that Docker starts are outside the capture: their traffic leaves directly and gets no token. The script also puts the runner CA in the VM's system store; run it again on an existing VM. `packaging/local-vm.sh <Home address>` creates one with Multipass (Ubuntu, Docker, Claude Code, Codex and a runner set to `[runner] boundary = "vm"`), then pairs it. Home shows it as **Local VM · Docker**; choose it in the composer's **Runner** option for a task that needs Docker.

## Versions

Home and its runners compare their protocol and build. A runner with another protocol cannot connect. A runner with another build of the same protocol takes no new sessions; its running sessions continue. The status bar then shows **<runner> needs an update**, the composer warns before a launch (the chosen runner, or with Automatic, where the launch goes instead), the Runner option marks it **needs an update**, and Settings → Home and runners explains how to update it.

**Update** (in Settings, in the composer warning, or on the runner's own This runner page) updates it from Home: the runner downloads Home's own executable through its tunnel, checks its SHA-256, OS and architecture, waits until none of its agents is in a turn, replaces its executable and restarts itself in place (same process, so its systemd service keeps running). Its open terminals close; its sessions continue after the restart. **Cancel** stops an update that still waits for agents. Only runners built with this feature (from November 5, 2026) offer Update; update older ones by hand once. By hand, with a Home in a Space, run the install command again on that machine: it installs Home's build and restarts the runner, which stays paired. Otherwise, install Launchpad at Home's commit and restart the runner daemon. The restart interrupts the runner's running turns: update it when its agents are done.

## Runner guard

On a runner, **Settings → This runner** holds settings that never sync to Home: **Launches from other devices** (Accept or Refuse: only windows on that machine launch there) and **Pause runner** (no new session starts there, from anywhere). Home shows them next to the runner. They are conveniences, not a security boundary: runners trust Home.

Contained sessions receive a session CLI credential bound to their ID and generation, not the runner daemon token. It allows context reads and inert local preparation; host settings, terminals, file APIs and publication still require an owner window.

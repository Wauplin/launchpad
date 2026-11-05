# Home and runners

Work in progress: this implements the [Home and runners plan](https://wauplin-launchpad-bots-research.static.hf.space/index.html) (H1 to H5). The single-process mode (`launchpad daemon`, role `all`) is Home and a runner in one process: it uses Home Git too.

- **Home** (`launchpad daemon --role home`) keeps every record and runs no agents or terminals.
- **Runners** (`launchpad daemon --role runner`) run agents. A runner works in its own folders (`~/.local/share/launchpad-runner`, `~/.config/launchpad-runner`) and port (7320), and never reuses a single-process install's data.

For a Home in a private Space, see the [Home-only deployment](../deploy/hf-home/README.md).

## Pair a runner

On a Home in a Space, open **Settings → Home and runners** and copy the install command. Run it on a Linux machine. It checks the machine, installs Home's own build next to any other Launchpad, starts the runner as a user service, then asks for approval. Run it again to update the runner. See [Add a runner](../deploy/hf-home/README.md#add-a-runner).

On another Home, pair by hand:

1. Start the runner daemon on the machine: `launchpad daemon --role runner`.
2. Run `launchpad connect <Home address>` there (with `LAUNCHPAD_ROLE=runner`). It prints a code such as `34Z-K9Z` and waits. For a Home in a private Hugging Face Space, the runner sends your HF token (`HF_TOKEN` or the `hf auth login` token) to pass the Space's sign-in.
3. On Home, open **Settings → Home and runners**, check that the code matches and click **Approve**. Requests expire after ten minutes.

The runner keeps one outbound WebSocket to Home (no inbound port) and reconnects on its own. **Remove** on Home disconnects a runner; it must pair again.

## Work started from Home

A launch on Home (from the phone, another browser, a schedule) in a Git folder goes to the first runner that takes launches: it clones the folder from Home Git on first use. A folder that exists only on one runner's disk (a scratch folder, a folder without Git) goes to that runner. Follow-ups, Stop, answers and the other session actions on Home go to the session's runner; Home serves the record, the events and the trace itself. Worktree actions (Merge, Discard, Create Local PR, Restore folder, Fix with agent) go to the runner of the worktree's session too, and Review → Agent branches lists the worktrees of the connected runners: an offline runner's branches show when it reconnects, and the list says which runner is offline. When that runner is offline, paused or refuses launches from other devices, Home says so and starts nothing. Ask Launchpad threads, accepted proposals, crons, scheduled launches and bot messages started on Home go to a runner the same way, sent once and never retried.

Profiles (including vault login IDs), workspaces and Git folders belong to Home: a runner shows Home's and sends its changes to Home, so they need Home to be reachable. **Clone from GitHub** registers the repository on Home, from any window; it is the first tab of **Add folders**, and the only way to add a Git folder on Home itself. **Add folder → Browse** on a runner's window (it warns that Home backs the folder up to its cloud bucket) registers a local checkout's remote on Home (a checkout without a remote is imported into Home Git); the runner then works in its own clones, never in your checkout. Each runner keeps a local log of the launches that came from elsewhere (Settings → This runner).

## Burst

When a launch from Home finds no runner that takes it (your laptop is off or asleep), Home starts it on **Burst**: an HF Job in Home's namespace, from Home's own Space image, so it runs the same build. Choose **Burst** in the composer's **Runner** option to use it on purpose. Crons, scheduled launches, bots and Ask Launchpad use it the same way when no runner is online.

- **No token in the job.** Home dials the job's runner through the HF Jobs proxy (the job cannot reach a private Home). The job holds no credential: its proxy sends every request that needs a token through the tunnel, and Home adds the token, sends the request and records it in the session's access log. Home Git and plain folders go through the tunnel too. Agents run without a jail inside the job: the job is the boundary.
- **Stopped when done.** One job runs one session. When its turn ends, the runner pushes the branches, saves uncommitted files to a scratch branch and sends the conversation to Home. After Home's next complete backup, Home cancels the job. Your next message starts a new job, which takes over the session and resumes its conversation (about 30 seconds). A job without Home for 10 minutes stops on its own.
- **Cost.** Settings → Home and runners shows this month's Burst cost, the cap (20 USD by default) and recent runs, with **Stop**. At the cap, Home starts no new job. The hardware is `cpu-basic` by default (0.01 USD per hour).

Burst needs Home to run in a Hugging Face Space, with its Hugging Face login.

## MCP servers

Settings → Credentials and tools → **MCP servers**: add a remote MCP server (Streamable HTTP) with its address and token. Home reads its tools; switch off each tool agents must not use (the server's read only and destructive labels help, but they are hints, not guarantees). A server is granted in every workspace or in chosen ones, and the composer's **Tools** option can leave it out of one launch.

Agents on every runner, Burst included, reach a granted server at a local address on their runner proxy. Each request goes through the tunnel to Home's gateway, which checks the session's lease, hides and refuses the tools that are off, adds the token and forwards the request. The token never leaves Home. Each tool call appears in the session's access log. A tool that is on acts directly with the server's token.

## Move a session

Between turns, a session can move to another runner: in its Summary, **Runs on** → **Move to…** lists the connected runners and Burst. The old runner pushes the branches, saves uncommitted files to a scratch branch and removes its folders; the new one clones the worktrees again from Home Git on the next message, and the harness resumes the same conversation (Claude Code and Codex). The move increases the session's generation, so the old runner can no longer push its branches or get a lease.

Every runner sends the conversation of a session to Home at the end of each turn, when it changed. So a session can also move while its runner is offline (a sleeping laptop): Move to Burst continues it from the last turn's branches and conversation; files that were not committed stay on that machine.

## Sessions on Home

Every session a runner runs is saved on Home: its record and every event, in order. The runner writes them to a local outbox in the same transaction as its own copy and drops them only after Home has saved them. While Home is unreachable, sessions keep running and accept your messages on the runner; the outbox is sent when the tunnel is back. Home never marks a runner's session Interrupted when Home itself restarts.

While a runner is disconnected (a sleeping laptop), Home shows "<runner> offline" next to its sessions' status: the status is the last one Home received. This is a runner state, never a session status. Seen and unread are kept on Home, so a finished session leaves Needs you even while its runner sleeps. **Archive** works too: Home closes the session at once, and the runner archives it (and stops it, if it still runs) when it reconnects. When you **Remove** a runner, its sessions that were running show Interrupted, and Archive closes them on Home.

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

Home keeps each Git folder as a bare mirror. For **Files**, it keeps a read-only copy of the default branch next to it, which borrows the mirror's objects and lists every branch as `origin/<branch>`. The copy is refreshed when Files opens, at most every 30 seconds.

## Plain and asset folders

A **plain folder** has no Git: Home stores its files (Add folders → **New plain folder**, or add a folder without Git from Browse on a runner, which copies its files once). Runners work in a synced copy: it comes down from Home before a session starts and goes back at the end of every turn. One agent works in a plain folder at a time; another launch fails until the first agent is archived. Changes compares with the start of the session. Home keeps every overwritten version and every deleted file for 30 days: the folder page lists them under **Versions and trash**, with **Restore**.

A plain folder can become a Git folder: on its folder page, open **Settings** and click **Turn into a Git folder**. Its current files become the first commit of `main` in a new Home Git folder without a remote; agents then work in worktrees and branches.

An **asset folder** holds large or generated files of a Git folder (datasets, media, model files) outside Git. An agent runs `launchpad assets create <path> --session-id <id>` in its worktree: the path is mapped in `.launchpad/assets.toml` (committed), kept out of Git, and synced like a plain folder, so later sessions on that branch get the same files. Assets are shared, not pinned per commit.

The folder page of a Git folder lists its asset folders with their size. An asset folder that no branch, kept tip or knowledge base note names, and that nobody changed for 14 days, goes to the trash: its files are removed and **Restore** puts them back for 30 days.

Contents are stored by hash and included in Home backups.

## Backups

Home backs up its database, `config.toml` and a Git bundle of each mirror's Launchpad branches and kept tips (only what the folder's remote does not have) to `[home] backup` (`hf://buckets/<you>/launchpad` or a folder) every minute when something changed, and before a planned stop. Objects are stored by content hash under `home/blobs/sha256/` and never overwritten; each backup is a manifest under `home/backup/<time>/`, written last, so a backup is complete only when its manifest exists. Settings → Home and runners shows the last complete backup and has **Back up now**. Backups older than a day are deleted, except the newest, together with the objects no kept backup needs. After each backup, Home writes readable copies of each workspace's knowledge base and bots (`<workspace>/kb/…`, `<workspace>/bots/<bot>/…`) and removes the copies of deleted files; restores never use them.

When Home starts on an empty data folder, it restores the newest complete backup before it opens the database, and each mirror gets its bundle back when it is cloned again (on a first boot without `config.toml`, `LAUNCHPAD_HOME_BACKUP` names the destination). Bundles include snapshot refs and remain in later backups until their mirrors are opened again. Runners then reconnect with their existing credentials and resend what Home has not acknowledged. After an unplanned restart, up to a minute of events acknowledged since the backup can be missing.

## Credentials and tools

Home’s image includes Claude Code, Codex, OpenCode, `gh` and `hf` for discovery, usage and quota checks. A hosted Home opens on a setup page that leads from an empty Home to a first agent: Hugging Face sign-in, the credential key (generated or your own, saved by you, then kept as the `LP_HOME_KEY` Space secret after a restart you confirm), GitHub sign-in, an SSH key made by Home (you add its public key on GitHub), the Git identity, a first profile (Claude Code or Codex subscription sign-in, stored in the vault, with its default model and effort), a workspace with a first cloned folder, then a runner and a first agent. Home opens when the first seven steps are done. Each sign-in and restart starts only on a click. Credentials in the bucket (vault secrets in backups, Home's own logins) are encrypted; other data stays as plain files you can browse. Agents never get Home's logins. Its backup is mandatory, with status in Settings → Hugging Face instead of optional trace-sync controls. On Home, the **Setup** page lists the same steps with their state and where to change them.

Home keeps every credential agents use: open **Settings → Credentials and tools** (on Home, or on a runner, which forwards to Home).

- **Harness logins:** a Claude Code token (`claude setup-token`) and a Codex ChatGPT login (paste its `auth.json`; from then on only Home refreshes it, so do not use that login elsewhere). A profile can name its login; otherwise the harness's first login is used.
- **Tokens and the SSH key:** tokens and API keys for tools, and one private SSH key. Secrets are write-only: Launchpad never shows them again.
- **Tools:** adding a tool asks only for its API token, with a link to create one. For GitHub CLI and Hugging Face CLI, **Use Home's token** lends Home's own login instead (agents then act with Home's full access: a fine-grained token is safer). The **Advanced** section holds the rest. Each entry says how a runner installs the CLI (GitHub release, download URL, `cargo`, `uv tool` or `npm`), which variables get a placeholder (for example `GH_TOKEN`), on which hosts the proxy adds which token (Bearer, Basic, a header or a query parameter), and in which workspaces it is granted. GitHub CLI and Hugging Face CLI are ready-made; **Copy as TOML** exports the list. The composer's options can switch a granted tool off for one launch.
- **Proposed tools:** an agent that needs a CLI or API can propose a tool (`launchpad propose tool`). The card on the Ask page shows what runners would install, the variables and the hosts that receive the token; **Add tool** adds it (its token is always a new entry, never an existing one), then **Paste its token** opens its form. Tool proposals never start in auto mode.
- **Test:** a tool with a check URL (GitHub, Hugging Face and OpenRouter have one; set it under Advanced for others) has **Test**: Home sends one request there with the token and shows whether the host accepts it. The token never leaves Home.
- **Git identity:** the author of agents' commits on every runner.
- **SSH key:** the vault shows its public key, to add on GitHub.

When a session starts, its runner gets a **lease** from Home: the session's login, its granted tools and their tokens. The runner keeps the lease in its proxy's memory only. Agents see placeholders: when a request goes to a host of the lease, the proxy adds the real token and records host, method and path in the session's access log (Summary → Credentials), naming actions such as "opened PR #3421". Other traffic passes through untouched. What a request may do is decided by the token's scopes: use fine-grained tokens. **Revoke** (Summary → Credentials, or the list of active leases) stops adding the tokens at once while the runner is connected; Stop and Archive end the lease too, and a runner that cannot reach Home drops it after an hour. The SSH key never leaves Home: sessions get an ssh-agent socket whose list and sign requests go to Home.

Quota: Home reads the quota of each vault login once and sends the readings to every runner, so quota protection works the same everywhere.

## The jail

Every agent runs contained. On a Linux runner (and in the single-process mode) it runs in a bubblewrap jail (`sudo apt install bubblewrap`; Ubuntu 24.04 needs user namespaces allowed for `bwrap`). Inside, the agent sees the system and the machine's toolchains read-only, and only its own folders read-write: its worktrees and plain folders, its harness home (`~/.local/share/launchpad-runner/sessions/<id>/home`, with its own `CLAUDE_CONFIG_DIR` and `CODEX_HOME`), its own `/tmp` and the build caches of its workspace. Your home folder, `~/.ssh`, `~/.config/gh`, `~/.claude`, Launchpad's data and the Docker socket do not exist inside. The network works normally, through the runner proxy. Title requests and **Continue in TUI** use the same jail and lease. Sessions on SSH hosts are not contained yet.

### Local VM

Docker needs root, so it never runs in the jail. A **Local VM** is a runner inside a virtual machine, where the VM is the boundary: agents run without a jail and may use Docker, with the same lease and proxy. `packaging/local-vm.sh <Home address>` creates one with Multipass (Ubuntu, Docker, Claude Code, Codex and a runner set to `[runner] boundary = "vm"`), then pairs it. Home shows it as **Local VM · Docker**; choose it in the composer's **Runner** option for a task that needs Docker.

## Versions

Home and its runners compare their protocol and build. A runner with another protocol cannot connect. A runner with another build of the same protocol shows **Update this runner** and takes no new sessions; its running sessions continue.

## Runner guard

On a runner, **Settings → This runner** holds settings that never sync to Home: **Launches from other devices** (Accept or Refuse: only windows on that machine launch there) and **Pause runner** (no new session starts there, from anywhere). Home shows them next to the runner. They are conveniences, not a security boundary: runners trust Home.

Contained sessions receive a session CLI credential bound to their ID and generation, not the runner daemon token. It allows context reads and inert local preparation; host settings, terminals, file APIs and publication still require an owner window.

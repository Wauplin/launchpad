# SSH hosts

Status: built (2026-10-14), awaiting daily use. Scope from the report `Remote SSH hosts for Launchpad` (published at https://wauplin-launchpad-remote-ssh.static.hf.space), with the recommendation of each owner decision: a helper on each host, no silent harness installs, native sign-in on each host, publication and merges push from the host, Linux hosts first, and the acceptance check below.

## Model

A host is a machine reached with the system `ssh`. Projects live on a host; everything that runs in a project (agents, task worktrees, terminals, Git, Changes, the code viewer, pre-drafts) runs on that host. The local daemon stays the only brain: agent list, history, quotas, schedules, crons, GitHub cache, publication buttons, notifications and paired devices. Any harness can run on any host; the host follows from the project, so there is no separate "Run on" control.

## Locations

A folder on a host is written `<host>:<absolute path>` (scp syntax), for example `devbox:/home/u/projects/x` (`crates/core/src/hosts/loc.rs`). The string fits every existing path field (`Project::path`, `Session::cwd`, worktree roots, terminal folders, `FileRef::root`) without schema changes. Host IDs have at least two characters, so Windows drive letters are never read as hosts. A location is a relative path to the local file system, so a local file operation that was not converted fails instead of touching a local folder.

- `crates/core/src/exec.rs`: `procs::command` returns a host-aware `Cmd`. The host comes from the working folder or from `UserEnv::for_host`. Host locations in arguments and environment values become native paths. Short commands use the helper's `Exec`; long ones (`spawn`) return a `Proc` whose stdin/stdout/stderr are resumable streams.
- `crates/core/src/hostfs.rs`: file operations (metadata, canonicalize, read, write, rename, copy, read_dir, walk, disk usage) by location. Paths returned for a host are locations again.
- Git output that prints paths (`rev-parse --show-toplevel`, `worktree list`, `--git-path`) is re-hosted with `loc::rehost`.

## Helper

The helper is the daemon's own binary, uploaded once per build to `~/.local/share/launchpad-host/bin/launchpad-<build>` (the build ID is a hash of the binary, so a rebuilt development binary replaces it), with a stable `launchpad` symlink that agents' prompts use. It is the only thing Launchpad installs by itself. Automatic upload needs the same OS and CPU type as this computer; other platforms get an explanation (`NeedsSetup`).

- `launchpad host-helper serve --profile <p>`: detached (`setsid`) server in `~/.local/share/launchpad-host/<profile>/` (one per daemon profile, guarded by a lock file). It resolves the login-shell environment like the daemon (again before each preflight), owns harness processes and PTYs (own process groups), keeps up to 64 MiB of unacknowledged output per harness stream (2 MiB per PTY), and writes input exactly once. It exits after 15 minutes without a daemon and without processes, and stops processes after 30 days without a daemon. It installs the repository skill as a managed copy (`.launchpad-managed`).
- `launchpad host-helper bridge --profile <p>`: run by `ssh`; pipes stdio to the server socket, starting it when needed and replacing a server of another build.
- Protocol (`hosts/proto.rs`): JSON lines. Output carries absolute offsets; the daemon acknowledges every 256 KiB. Input carries offsets; the helper acknowledges and skips bytes it already wrote. `Hello` lists the processes the daemon follows with the offsets it has: the helper replays the rest, stops processes of another daemon boot or unknown to the daemon, and the daemon marks processes the helper no longer has as lost.

## Connection

`hosts/conn.rs` runs one `ssh -T -x -a -o ClearAllForwardings=yes -o ServerAliveInterval=15 -o ControlMaster=auto … host-helper bridge` per host, with the user's own `~/.ssh/config` (jump hosts, certificates, agent keys). Password, passphrase and host key questions use `SSH_ASKPASS=launchpad` (`LAUNCHPAD_ASKPASS=<host>`): the daemon shows them in a dialog (`HostPrompt`), answers go to ssh and are never stored. The loop retries every 15 s (2 s after a connection that lasted more than a minute); Settings → Hosts and the session band have Retry now. Requests wait up to 25 s while a connection is in progress and fail at once while the host is unreachable.

## Sessions

- Unreachable is a connection state, never a session status: the session stays in Running, its band explains that the agent keeps running on the host, and Stop is queued until the connection returns. On reconnect the trace catches up from the helper buffer and the session moves to Done/Needs you as usual.
- A process the helper no longer has (host or helper restarted) ends with `ProcessLost`: adapters report it as Interrupted with a notice, never Done or Failed; Resume uses the harness's native resume on the host.
- Restarting the daemon still interrupts active turns: the new boot stops the processes of the previous one, so no orphan holds a Codex thread lock.
- Adapters receive `StartSpec::host` and native folders; they start the same native interfaces (Claude stream-json, Codex app-server per host and variant environment, OpenCode ACP). No adapter or UI branch per harness and host.
- Agents on a host call the helper's `launchpad` CLI; its `suggest`, `pr`, `work` and `trace` calls go through the helper socket to the daemon API (`hosts/relay.rs`, an allowlist of agent endpoints; `cwd` is re-hosted). Suggestions, pre-drafts and merges stay local behind owner buttons.
- Attached non-image files go to `~/.local/share/launchpad-host/<profile>/attachments/<session>` on the host. Images use native harness input.
- Context projects must be on the same machine as the session folder (the composer and the daemon refuse others).
- Titles are generated by the local harness, as before.
- Quotas stay per account: meters read this computer's account. Preflight says when a host's harness is signed in to the same account, or possibly another.

## Worktrees, Git and publication

Task worktrees are created under the host's projects root (`.worktrees/<project-id>/`), merges, hand-offs, cleanup and restore run there with the same rules. Pre-draft Publish and merges push from the host with its Git and gh credentials; preflight shows whether gh is signed in.

## Preflight

`hosts/preflight.rs` runs after each (re)connection and on demand: SSH (destination or ProxyJump, latency), system (OS, CPU, user; root is flagged because Claude Code refuses to skip permission prompts as root unless `IS_SANDBOX=1`), helper build, Git, GitHub CLI, and each harness (found on the login-shell PATH, version, read-only sign-in status). Failed checks show the official install or sign-in command, which Run in terminal on host starts in a visible terminal in the host's home. The composer disables harnesses missing or signed out on the project's host.

## UI

Settings → Hosts (add from `~/.ssh/config` or `user@host`, projects root, preflight, Open terminal on host, Reconnect, Remove), host chips on agent rows, the session header, terminal tabs and project rows, project picker groups by machine, the composer summary adds `on <host>`, Projects has a machine selector (Scan, Clone from GitHub, Add folder with Browse), and the status bar shows host health. An `ssh` question opens a dialog on any page.

## API

`GET/POST /api/hosts`, `PATCH/DELETE /api/hosts/{id}`, `POST /api/hosts/{id}/retry|checks`, `POST /api/hosts/{id}/askpass` (from `launchpad` as `SSH_ASKPASS`), `POST /api/hosts/{id}/prompt`, `GET /api/hosts/{id}/browse?path=`, `GET /api/hosts/ssh-config`. `POST /api/projects` and `/api/projects/clone` accept `host`; `POST /api/projects/discover` accepts `{host}`. `Status.hosts` carries every host's state.

## Limits

- Linux hosts; macOS hosts work only when this computer is a Mac of the same CPU type (the helper is this binary). Windows hosts and SSH from a Windows daemon are not supported yet.
- While this computer sleeps, nothing new starts: scheduled launches, crons, quota continuation and follow-ups wait for it. Use server mode for always-on scheduling.
- Projects of an unreachable host keep their last known GitHub mapping; remote metadata refreshes every 30 s, never during list requests.
- Port forwarding, moving a task between machines and automatic harness installation are later steps.

## Acceptance check

Performed on 2026-10-14 with an HF Jobs CPU sandbox reached over SSH (`ssh <job>@ssh.hf.jobs`): add the host, clone a project onto it, launch one Claude and one Codex session in task worktrees with merge when done, open a shell on the host, then freeze the daemon for three minutes (a suspended laptop) while both agents run a 100-second command. After waking, both turns finished, the traces were complete, both sessions were in Needs you, Changes showed their diffs and the worktree merges ran on the host. No credential was copied by Launchpad (the test copied harness logins to the sandbox by hand).

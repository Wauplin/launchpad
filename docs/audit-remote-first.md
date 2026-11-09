# Audit: Home + runners only (remote-first)

Date: 2026-11-09. Scope: the whole repository (crates, web, desktop, packaging, deploy, docs). Read-only audit, nothing implemented. Target model: Home (a server, normally on an HF Space) runs no agents; agents run only on registered runners or Burst; every folder, agent and worktree is remote; everything a review, merge or resume needs lives on Home.

Legend: **[V]** verified by reading the code during the audit; **[R]** found by a reviewer pass reading the code, not re-checked line by line; **[?]** suspicion.

## 1. Root cause: `Role::All` is still the default

`Role::All` (one process holds records and runs agents on the same disk) is the `#[default]` role (`crates/core/src/role.rs:13`) and the default of `launchpad daemon --role` (`crates/cli/src/main.rs:209`). Everything that starts a daemon without `--role` installs it **[V]**:

- `packaging/launchpad.service:6` (`ExecStart=… launchpad daemon`), so `install.sh`, `install.sh --headless`, `install-macos.sh:20`, `install.ps1:18`
- `desktop/src-tauri/src/main.rs:106` (the desktop shell starts `launchpad daemon` when port 7317 does not answer)
- `deploy/hf-space/supervisor/runtime.py:31` (the older combined Space connector) **[R]**
- `Paths::for_app_env` and `Paths::from_env` (`crates/core/src/paths.rs:42,74`)

`All` shares Home's data folder and port (`~/.local/share/launchpad`, 7317), so a machine that ran `all` and later runs `--role home` keeps old records (sessions with `runner_id = None` can never continue: `move_to` refuses them and Home runs no agents) **[R]**.

Code that exists only for `All` and should go with it **[V]**:

| Where | What |
|---|---|
| `crates/core/src/home_git.rs:71-77` | A folder without a remote mirrors the local checkout `p.path`. In `all`, `p.path` can become the runner-style clone whose origin is the mirror itself: a recreated mirror would fetch with `--prune` from it and merges would push into it **[R]**. Delete. |
| `crates/core/src/home_git.rs:47`, `vault.rs:581` | `here()` = "not a runner", so `All` acts as Home. Make it `== Home`. |
| `crates/core/src/projects.rs:321` | Clones the repo cache right after registering (runners clone on first use). |
| `crates/core/src/projects.rs:289,456` | Folder path is the mirror on Home, else a runner cache path. Keep only the Home branch. |
| `crates/server/src/git_http.rs:87` | `all` serves its own mirrors instead of forwarding to Home. |
| `crates/harness/src/codex.rs:1388` | `codex update` on the host every 6 h, only in `all`, outside any jail. |
| `crates/core/src/paths.rs:116` | CLI omits `LAUNCHPAD_ROLE` for `all`. |
| `crates/core/src/runners.rs:614,650-711`, `sessions.rs:2466-2490` | `runner_id = None` branches of `launch_anywhere`, `session_action`, `send_anywhere`, `notice_anywhere` run the session in-process. |
| `contain.rs:60-65`, `proxy.rs:660-672`, `vault.rs:744`, `mcp.rs:329`, `plain.rs:567`, `kb.rs:687,694,811` | In-process "Home" calls (lease, MCP gateway, grants, plain sync, KB) for a daemon that is both. **[R]** |
| `crates/core/src/projects.rs:561` | `adopt_git_folders`: migration of single-process data. **[R]** |

Effect beyond dead code: in `all`, Changes, Files and merges read worktrees straight from disk and handoffs are never written (`handoff::after_turn` is runner-only), so none of the remote paths the owner relies on are exercised in the default dev setup.

**Recommendation.** Delete `Role::All`. `runs_agents()` becomes `== Runner`. `launchpad daemon` requires `--role home|runner` (or defaults to `home`, which runs no agents). CLI commands default to Home's paths unless `LAUNCHPAD_ROLE=runner`. Dev setup (docs/development.md, CLAUDE.md) becomes a dev Home on 7318 plus a dev runner on 7321 paired once with `launchpad connect http://127.0.0.1:7318`. Running Home on a laptop is then exactly the Space setup, and agents run on that laptop only if it is also registered as a runner.

## 2. Data not available when the runner is off (the bugs the owner fears)

Ordered by severity.

1. **Done is announced before the branch reaches Home; Home can then merge an older tip** **[V]**. `Sink::set_status` (`crates/core/src/sessions.rs:2302-2321`) replicates Done, then spawns a task that pushes branches (`worktrees::after_turn`), syncs plain folders, refreshes the title (an LLM call) and only then uploads the handoff. If the laptop sleeps, the Burst job dies or the push fails in that gap, Home shows Done with the previous turn's branch. `away.rs:85` always reports `unpushed: false`, the card says "saved on Home", and `worktree.merge` takes no reviewed head (`actions.rs:157`), so Merge pushes the stale tip with no warning **[R]**. Fix: push branches and plain folders before replicating Done (or hold a "saving" state), send the local head with every turn end even if the full handoff fails, flag `unpushed` on Home when heads differ, and give `worktree.merge` a `head` input checked against Home's tip like `local-pr.merge`.
2. **Pushes and handoffs are never retried** **[R]**. A failed push, plain sync or handoff upload waits for the next turn end ("retries at the end of the next turn"); there is no retry on tunnel reconnect or timer. Fix: a small per-session "pending sync" record retried on reconnect and every few minutes.
3. **Files cannot open an agent's second worktree on Home, online or away** **[V]**. `code::sessions_at` (`crates/core/src/code.rs:178`) matches `/worktrees/<session id>/`, but worktree roots are `<data_dir>/worktrees/<project id>/<slug>` (`worktrees.rs:252`). Only the primary worktree (`s.cwd`) is found. Fix: resolve the root through the worktree records (or `away::worktree_at`).
4. **A connected runner answers worse than an away one when the folder is gone** **[R]**. Worktrees removed by retention (`keep`, default 15) or just moved (`handoff::relocate`) make `session_git::status/diff` (`session_git.rs:115,253`) fail, `request_merge` requires `Active` (`worktrees.rs:837`), and the UI hides Merge and Create Local PR (`Worktrees.tsx:112`, `TodayCards.tsx:103`). Fix: whenever the folder is not present, Home answers from Home Git (the `away.rs` code) regardless of runner state, and Merge/Local PR run on Home.
5. **Merges and Local PRs done on Home while the runner was away are forgotten on reconnect** **[R]**. `away::worktree_action` and `away::local_pr` update only Home Git and the Local PR table, not the runner's `TaskWorktree` (`merged`, `predraft_id`). After reconnect, Today offers Merge or Review changes again. Fix: Home is the source of truth for `merged` and `predraft_id` (looked up by folder + branch), not the runner record.
6. **Uncommitted files: snapshots are written but never read** **[R]**. `refs/lp-snapshots/<worktree>` is pushed every 5 min (`worktrees.rs:437`), included in backups, never deleted, and never used: away Changes say "uncommitted files are not on Home" (`away.rs:157`), and a move from an offline runner says files "stay on that machine" (`handoff.rs:418`). No snapshot at turn end either. Fix: snapshot at turn end, show it as "uncommitted (as of …)" in away Changes and Files, re-apply it (or save it as `lp-scratch/…`) on a move from an offline runner, prune it on discard and after 30 days.
7. **Away views fail when no handoff reached Home** **[R]**. `handoff::after_turn` returns early when `snapshot` fails (`handoff.rs:228`, e.g. the native trace cannot be read); before the first turn end there is no handoff at all. `away.rs:46` then says "Home has no record of this session's worktree". Fix: send worktree records to Home on their own, at plan/prepare time and at every push.
8. **Plain and temporary folders sync only at turn end** **[R]**. No periodic sync like worktree snapshots; a Burst kill or lost laptop mid-turn loses them. `.gitignore`d files, nested repositories and the folder's own `.git` are never synced (`plain.rs:661-673`): a clone made inside a temporary folder is lost on a move.
9. **Attachments are not in the handoff, uploads not in backups** **[R]**. Session attachments (`<data_dir>/attachments/<id>/`, `attachments.rs:207`) stay on the runner, so after a move the conversation points to missing paths. Home's `<data_dir>/uploads` and `runner-archives.json` are not in Home backups (`live_blobs` covers only plain and handoff blobs): a Space restored on an empty disk loses draft, schedule and routine attachments.
10. **Moved sessions show a partial trace on the new runner** **[R]**. `Sessions::adopt` (`sessions.rs:607`) keeps only the record; `home_client.rs:196` forwards `GET …/trace` to Home only when the runner lacks the session. `launchpad trace <id>` on the adopting runner (bots read their previous session this way) returns only turns since adoption. Fix: always serve traces from Home.
11. **Events of a moved session can be dropped** **[R]**. `apply_replica` drops items from an older generation (`sessions.rs:533`); a laptop that left with unacknowledged events loses them after a move, and the new runner reuses sequence numbers from `last_seq`.
12. **Review drafts typed on a runner's own UI stay there** **[R]**. `/api/review-state` is not in `home_client::home_owned` (`crates/server/src/home_client.rs:167-178`). Minor once runners have no UI (section 5).
13. **Changes compares with the wrong base online** **[R]**. `session_git::main_branch` (`session_git.rs:329`) uses `origin/HEAD` and a plain `git diff <base>`; away mode uses the merge-base with the worktree's base branch. Same session, two different diffs. Fix: merge-base with `origin/<base_branch>` in both.
14. **Publish all pushes unreviewed content** **[R]**. `predrafts::publish_set` (`predrafts.rs:571`) passes each member's current tip as the "reviewed" head, so the check compares the tip with itself. Fix: the UI sends each reviewed head.
15. **Pushes to a followed GitHub branch fail silently** **[R]**. `home_git::forward` (`home_git.rs:280-291`) is a fire-and-forget task per push: no retry, no record that the PR is behind Home, no notice for Local PRs without a session, possible out-of-order pushes. Fix: store the last forwarded head in `git_follow`, retry on refresh, show "not on GitHub yet".
16. **Files on Home lag up to 30 s after a push** **[R]** (`browse_copy` refresh interval). Refresh on the `home_git/pushed` event.

## 3. Resume and placement are not seamless

1. **Messages from the UI to a session on an offline laptop fail with "X is offline"** **[V]**. The UI's `POST /api/sessions/{id}/input` goes through `server::runners::relay` (`crates/server/src/runners.rs:500-529`), which handles away reviews, stopped Burst jobs and Archive, then returns 503 for everything else. The move-to-Burst fallback in `runners::send_anywhere` (`crates/core/src/runners.rs:690-699`) is reached only by bots, proposals and report replies. Fix: route input, resume and answer for a disconnected runner through one placement function.
2. **The fallback is always Burst, never another connected runner** **[V]**. `send_anywhere` calls `burst::continue_session` even when `runners.available()` has a runner, and fails if Burst is unavailable. `handoff::move_to` already handles an offline source (`handoff.rs:411-417`). Fix: one `place(session)` used by launch, message and resume: the session's runner if connected → another available runner (move) → Burst → error.
3. **A session Running on a runner that vanished stays Running forever** **[R]**. `Hub::disconnected` changes no session; `handoff::blocked` refuses to move a running session; Stop and input return 503. Fix: after a grace period (for example 10 min disconnected) mark it Interrupted on Home with a notice, so it can move from its last handoff.
4. **Pinning to a runner where Home already has the data** **[V]**. `launch_runner` (`runners.rs:584-598`) pins "New agent here / Run in" to the source session's runner and a temporary folder to its runner, and refuses when that runner is offline, although the branches are on Home Git and the files in Home's `tmp-<id>` store. Burst refuses temporary folders (`burst.rs:472`) **[R]**. Keep as folder and other writes on a temporary folder are relayed to its runner and fail with 503 although `keep_temp` could run on Home **[R]**. Fix: move the source session (or temporary folder) like Burst already does at `runners.rs:624-633`.
5. **Runner choice is arbitrary** **[V]**. With no runner chosen, Home takes the lowest runner ID (`runners.rs:524-528`, `available()` is sorted by ID). Fine for one laptop; with several, prefer the session's previous runner, then the least busy.
6. **A reconnecting runner is never told its sessions moved** **[R]**. No "release" message in `ToRunner`; the old laptop keeps Active worktrees with commits that never reach `lp-scratch/…`. Fix: on Welcome, send the sessions whose generation on Home is newer; the runner saves leftovers and drops them.
7. **OpenCode sessions cannot resume or move once archived** **[V]** for the missing method, **[R]** for the flow. OpenCode has `native_trace` but no `restore_native` (only Claude and Codex implement it), while `handoff::clean_archived` deletes the session folder once native files reached Home. Fix: implement restore, or keep the folder for harnesses without it.
8. **`sessions::send_anywhere`** (`sessions.rs:2487`, used for merge/rebase requests from `predrafts.rs:412`) has no offline or Burst handling **[R]**.

Burst **[R]**:
- `handed_off` timestamps live in memory (`burst.rs:601`): after a Home restart during Stopping, any older backup counts and the job stops before a backup newer than the handoff exists. Store it in `BurstRun`.
- Jobs have a 24 h timeout (`burst.rs:29`) and nothing hands off before it: a job waiting on a question is killed mid-turn. Hand off and stop about an hour before.
- An open merge failure keeps `w.merge` set, `remove_clean` refuses, the final handoff fails every tick and the job bills for up to 2 h (`STUCK_LIMIT_MS`), then stops with lost files.
- `runners::remove` does not clear `open_question` **[?]**: a question session on a killed job may stay blocked.

## 4. Home runs harness processes and reads its own disk

The hard rule says the Home machine runs no agents. It runs no *sessions*, but it does start the harness CLIs, and the Home image installs them on purpose (`deploy/hf-home/Dockerfile:38`, `docs/architecture.md:135`) **[R]**:

- Quota: Codex `app-server` with `CODEX_HOME` = the vault login folder (real `auth.json`) (`codex.rs:1261,1357`); `claude -p` with the real OAuth token, outside any jail, when the usage call fails (`claude.rs:1160,1247`).
- Models every 30 min (`harness/mod.rs:326`), versions every 30 min (`status.rs:197`), sign-in checks (`signin.rs:62`).
- **[?]** Home rewrites `auth.json` from the vault about once a minute while the Codex app-server may rotate the token in that file: the vault could end up with a used refresh token.

Recommendation: Home reads quota over HTTP only (Claude usage endpoint with the vault token; Codex rate limits with the access token Home refreshes) and, when that fails, asks a connected runner to do the `get_usage` / app-server read in a jail. Models and versions come from runners' status. Remove the harness CLIs from the Home image.

Home reads its own disk **[R]**:
- Add folder by path and folder discovery (`api.rs:138-146`, `projects.rs:339-352,521-527,626`) read and `git push` from the Space container's filesystem; `import_plain` copies a plain folder on Home. Refuse on Home; adding by path is a runner action (or drop it: clone from GitHub and new plain folder cover the Space case).
- `GET /api/kb/memories` without `?runner=` reads Home's own `~/.claude`/`~/.codex` memories (`kb_http.rs:165-178`). Refuse on Home.
- The setup checklist offers fix terminals on Home that `terminals.rs:428` then refuses.

## 5. Security and containment

- **Folder setup commands run outside the jail and netcap** **[V]**. `worktrees::run_setup` (`worktrees.rs:539-556`) runs `sh -c <worktree_setup>` with `app.env` and no `Jail`. A `pnpm install` postinstall script has the runner user's full disk (`~/.ssh`, the runner credential). Fix: run it through the session's jail (`Jail::command`).
- **Lease owner check accepts unknown sessions** **[V]**. `vault::check_owner` (`vault.rs:726`) returns Ok when Home does not know the session or the session has no `runner_id`: an approved runner gets a lease by naming any new ID. This may exist because a new session's lease can reach Home before its record; once `All` is gone, Home should create the record (it chooses the runner at launch) and require `s.runner_id == runner_id`.
- **macOS and Windows runners run agents uncontained** **[V]**. `contain::contained` is `runs_agents() && linux` (`contain.rs:33`). On those runners the Claude quota monitor also reads the laptop's own `~/.claude/.credentials.json` (`claude.rs:1147`) **[R]**. Either refuse non-Linux runners (Local VM on macOS) or state it in the runner page.

## 6. Legacy UI, settings, packaging and docs

Desktop shell and local-window features **[R]**:
- `desktop/` only talks to `127.0.0.1:7317/7318` and starts an `all` daemon. Remove it (an installed PWA covers a window on Home). With it go `/api/open` and `/api/window` (`crates/api/src/api.rs:625-642`, `web/src/lib/platform.ts:12-40`, `web/src/lib/workspace.ts:93-105`), "This desktop app" in Devices, `[commands] browser`, and the New window button.
- Desktop notifications from the daemon (`crates/core/src/notify.rs:86-110`, `[agents] notify`, the Settings card): pop up on whichever machine runs the daemon, never on Home. Remove; Telegram stays.

Settings **[R]**:
- `RunnersView.tsx:33-38`: the "runs everything in one process" page. Remove.
- `RunnersView.tsx:186-203` "This computer" card on Home: Folders root and Open links with. Remove on Home; on a runner keep only what the runner uses.
- `RunnersView.tsx:291`: runner guard "Refuse: only windows on this machine launch here". There are no local windows anymore: Refuse means "never". Merge with Pause into one "Takes new sessions" switch.
- `ProfilesView.tsx:118-131,488-500`: "This computer" sign-in row checking the local CLI login; sessions only use vault logins. Show the vault login and which runners have the CLI.
- `ProfilesView.tsx:555`, `HuggingFaceView.tsx:54`: "GitHub CLI / hf CLI of this computer" → Home's login.
- `WorktreesView.tsx:82`: says worktrees live under `${projects_root}/.worktrees` (wrong: `<data_dir>/worktrees/…`); `worktrees.keep` is applied per runner.
- Devices → Tailscale Serve and `[server] auto_tailnet`/`bind`: only for a self-hosted Home; hide on runners.
- `remove_published_worktree` (`config.rs:23`): laptop-era; fold into Disk cleanup or drop.
- `api.rs:268`: `worktree_setup` cannot be edited with remote access on, and a Space Home is always remote **[R]**: the owner cannot set folder setup commands from Home. Decide (with the jail fix above, editing it from Home becomes safe).

Setup and onboarding **[R]**:
- `setup_checklist.rs:636-648` always asks for GitHub CLI, Git SSH key and credential helper, local sign-ins, first folder, phone devices and the Chrome extension. On a runner keep: Home link, jail/bwrap, harness CLIs, network capture, vault logins, agent Git identity. On Home: workspace, GitHub, agent login, where agents run, first folder.
- "Set up this machine", "stays on this machine", "closing the window keeps them" wording in onboarding, tour, Suggestions (`Suggestions.tsx:72`), terminals chapter.
- Demo world runs as `role: "all"`, `host: "this-computer"` (`web/src/demo/world/core.ts:53-56`): make Home with one demo runner the base.

Wording in the app **[R]**:
- `Composer.tsx:830` "Runs on {host}" / "Agents run on the daemon's machine": on Home it names the Space. Show the chosen runner or Burst.
- `RunsOn.tsx:46-65` "This computer" when `runner_id` is empty: only possible in `all`.
- `ServerStatus.tsx:38`, `KnowledgeView.tsx:281` "this machine".

Packaging and docs **[R]**:
- `install.sh` / `install-macos.sh` / `install.ps1`: become runner installers (or point to Home's `install-runner.sh`, which already uses `--role runner`); `--home` for a self-hosted Home. No desktop entries.
- `deploy/hf-space/` (combined connector running `all`): decide whether to retire it in favor of `deploy/hf-home`.
- README (desktop app, "stay on your machine", Getting started with install.sh), `docs/architecture.md` (desktop shell in the diagram, `all` as default), `docs/getting-started.md`, `docs/remote-access.md` (headless server as `all`), `docs/configuration.md` (`[commands]`, `notify`), `docs/home-and-runners.md:3,129`, `docs/development.md`, `CLAUDE.md` (dev daemon roles, dev window, `/api/open` rule, "A Merge from a Today card needs … its runner present").

Conflict with a current rule: CLAUDE.md says a Merge from a Today card needs "its runner present". With Home merging from Home Git (and section 2.1 fixed), that requirement can go.

## 7. Sketches

```
Settings → Home and runners                      (on Home)
┌ Home ──────────────────────────────────────────────┐
│ wauplin/launchpad-home · build 00ba0b9  Up to date  │
└────────────────────────────────────────────────────┘
┌ Runners ─────────────────────────────── [Add runner]┐
│ ● laptop    connected · Linux · Jail · 2 running  ⋯ │
│ ○ desk-vm   offline 2 h · Local VM                ⋯ │
│ New sessions go to: connected runner, else Burst    │
└────────────────────────────────────────────────────┘
┌ Burst ─────────────────────────────────────────────┐
│ (On) Off   Monthly cap $[20]   ▓▓░░ 34 %           │
└────────────────────────────────────────────────────┘
(no "This computer" card, no "Open links with", no desktop notifications)
```

```
Runner page (local UI of a runner: status only)
┌ This runner ───────────────────────────────────────┐
│ Home   https://wauplin-launchpad-home.hf.space  ●   │
│ Takes new sessions  (Yes) Paused     [Open Home]    │
│ ✓ bubblewrap  ✓ network capture  ✓ Claude Code 2.1  │
│ ✗ Codex not installed                               │
└────────────────────────────────────────────────────┘
```

```
Session header, runner offline (after the fixes)
 Fix login flow   ● Done · laptop (offline 20 min)
 Changes: saved on Home at a1b2c3d · uncommitted files as of 14:02
 [Merge]  ⋯ Create Local PR · Continue on another runner · Archive
 Reply → "laptop is offline: this message continues on desk-vm"
```

```
Session header, last turn not saved
 Fix login flow   ● Done · laptop (offline)
 ⚠ The last turn's commits did not reach Home (laptop went offline at 14:05).
   Merge would use a1b2c3d from the previous turn.  [Merge anyway]  ⋯
```

```
Home setup (first run, browser)
 1 ✓ Workspace        Work
 2 ✓ GitHub           signed in as Wauplin (Home's login)
 3 ○ Agent login      [Sign in Claude] [Sign in ChatGPT]   → vault
 4 ○ Where agents run ( ) Burst only  ( ) Add a runner: [command] [Copy]
 5 ○ First folder     [Clone from GitHub] [New plain folder]
 (no SSH keys, no gh setup-git, no Chrome extension, no "this machine")
```

## 8. Suggested order

1. **Placement**: one `place(session)` (connected runner → other runner via `move_to` → Burst) used by `relay` for input/resume/answer, `send_anywhere`, `sessions::send_anywhere`, Run in and temporary folders; mark sessions on a vanished runner Interrupted after a grace period; release message on reconnect. (Section 3.)
2. **Saved before Done**: push and plain sync before replicating Done; head in every turn end; retry pending syncs; `head` input on `worktree.merge`; Home owns `merged`/`predraft_id`; serve Changes/Files from Home whenever the folder is missing; fix `code.rs:178`. (Section 2.1–2.7.)
3. **Jail the setup command** and tighten `check_owner`. (Section 5.)
4. **Delete `Role::All`**, the desktop shell, `/api/open`, `/api/window`, desktop notifications, `[commands]`; dev setup = Home + runner; installers install runners. (Sections 1 and 6.)
5. **Home without harness CLIs** (quota over HTTP or via a runner). (Section 4.)
6. Snapshots, attachments in handoffs, uploads in backups, traces from Home, OpenCode restore, Burst timeout and restart fixes. (Sections 2–3.)
7. UI wording, settings, setup checklist, demo and docs. (Section 6.)

## 9. Decisions for the owner

- Remove `desktop/` entirely, or keep a thin window on a configured Home URL?
- `launchpad daemon` without `--role`: an error, or Home?
- Retire `deploy/hf-space/` (combined connector)?
- Non-Linux runners: refuse them, require a Local VM, or allow uncontained with a visible warning?
- Keep "Add folder by path" at all (only meaningful on a runner), or only Clone from GitHub and New plain folder?
- Placement order when the session's runner is offline: another runner first, then Burst (as assumed here)?

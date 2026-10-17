# Managed agent worktrees

Status: built on 2026-10-11, awaiting daily use. The first proposal of the same day was reviewed and simplified before implementation; this document describes the built design.

New agents in a Git project run in a dedicated Git worktree that Launchpad creates before the harness starts. The daemon owns creation, setup, merging, hand-off, promotion, cleanup and restoration, the same way for every harness. Launchpad never uses a harness's native worktree feature (Codex app worktrees, `claude --worktree`). Agents only commit on their branch; Git work that needs waiting runs in the daemon and spends no agent turns.

## What it borrows from Codex, and what differs

| Codex app | Launchpad |
|---|---|
| Local or Worktree per chat | Run in: New worktree (default for Git projects), Project checkout, Pre-draft PR |
| Starts at the selected branch, detached HEAD, can include uncommitted changes | Starts at the checkout's current branch (or a chosen branch) on a named `lp/<slug>-<id>` branch; Copy uncommitted changes is an explicit checkbox |
| Hand off moves a chat between Local and its worktree | Check out locally / Move back to worktree, on the session |
| Create branch here, then commit/push/PR | Create pre-draft PR reuses the same folder and branch; publication stays an owner click |
| `.worktreeinclude` copies ignored files; `AGENTS.override.md` copied automatically | Same file format and rules: ignored regular files only, no symlinks, no overwrite |
| Setup scripts from a local environment | One owner-configured setup command per project (project page) |
| Keep 15 worktrees; archive deletes; snapshot before deleting | Keep 15 clean worktrees of idle agents; archive removes clean ones; the branch is the snapshot and Resume restores the folder; uncommitted work is never removed automatically |
| No merge to local main | When done: Merge into `<base>`, with conflicts sent back to the agent |

Named branches keep work visible in `git branch` and reachable after the folder is removed, which replaces Codex's snapshots without a separate storage format.

## Launch

- `LaunchSpec.worktree` (`WorktreeOptions`: `base`, `include_changes`, `merge_when_done`) is set on new drafts. Stored specs without it keep running in the checkout. Non-Git project folders run in place; the composer offers only Project folder for them.
- The worktree goes to `<projects_root>/.worktrees/<project-id>/<slug>-<session8>` on branch `lp/<slug>-<session8>`, the same layout as pre-drafts. A project registered below a repository top level maps to the same subdirectory in the worktree.
- Context projects have a read-only / editable toggle. Editable ones get their own worktree, unless the launch runs in the project checkout (then they are edited in place). Projects of the same repository share one worktree. During a task, `launchpad work add-project --session-id <id> --project <name>` creates another worktree and prints its folder; every harness already runs with full file access, so no restart is needed.
- The launch prompt lists each worktree, branch, base and commit, overrides only the checkout workflow rules of the repository (for example "work on main"), and replaces the `worktree` preset. The prompt preview shows a placeholder; folders are created only at launch, including for scheduled launches and each cron run.
- Setup commands run in the background after the session appears (Starting); Stop cancels them. A failed setup is reported to the agent, which starts anyway and can fix its environment. `$LAUNCHPAD_SOURCE_CHECKOUT` points to the original checkout.
- Resume with another agent joins the same worktree records. Reattaching resolves editable context projects through those records, including subfolders sharing a repository; a missing or discarded context worktree blocks resume instead of granting access to the project checkout. An explicitly handed-off worktree resolves to its recorded checkout.
- Launch preparation rolls back newly created task worktrees and new pre-drafts on any error before harness startup, including attachment or session-persistence failures. Shared worktrees are retained and only the failed session membership is removed; existing pre-drafts keep their previous session link. Cleanup failures are logged and retain recovery records.
- Copied untracked/included files require regular destination parent directories contained in the worktree; directory symlinks are refused before writing. Code and terminals accept a registered repository subfolder at the same relative path in a verified linked worktree; this does not authorize its parent repository or sibling folders.

## Ending a task

The session header chip (`⎇ branch · state`) and Summary → Next steps show one card per worktree, with:

- **Merge into `<base>`** (owner click), or **When done: Merge into `<base>`** at launch, which tells the agent to run `launchpad work finish --session-id <id>`. The request is durable. A background loop (every 15 s) waits until the agents of the worktree are idle, the checkout that has `<base>` checked out is clean and no other agent runs there, then rebases a linear branch onto `<base>` if needed and runs `git merge --ff-only` in that checkout. If a rebase is needed and the task contains merge commits, automatic merging stops with a visible reason; integrate the base manually while preserving merge resolutions and request the merge again. It never stashes, resets, cleans, commits other changes or forces.
- **Conflicts:** the daemon aborts the rebase and records the conflicting files. For an agent request, it sends the agent a message (under normal quota protection, up to three times) asking it to rebase, resolve, re-run checks and run `work finish` again. For an owner request, **Fix with agent** sends the same message. Both prompts ask the final message to restate the original task results and update recovery checks, limitations and the requested merge status, so the run summary remains useful after recovery. Cancel merge drops the request.
- **Check out locally** removes the clean worktree and switches the project checkout to the branch, so the owner can run it there. **Move back to worktree** restores the previous branch and the folder. Both require clean folders, idle agents and no live terminals in either folder. Check out locally also refuses removal when ignored files are present, so local secrets, databases and dependencies cannot disappear with the folder. Move back to worktree copies `.worktreeinclude` files again.
- **Create pre-draft PR** turns the worktree into a pre-draft record that owns the same folder and branch.
- **Discard** (with confirmation) deletes folder and branch. Discarding the primary worktree archives its agents.

## Cleanup and restoration

- Archive removes the clean worktrees whose sessions are all archived. Retention (Settings → Worktrees, `[worktrees] keep = 15`, 0 keeps all) runs at startup and hourly and removes clean worktrees of idle agents beyond the newest 15.
- Removal requires idle agents, no open terminal, no pending merge, no uncommitted or untracked file, no Git operation in progress and the expected branch; it uses non-forced `git worktree remove`. The branch keeps the commits; a branch already merged into its base is deleted.
- Sending a message to the session (or continuing it with another agent) restores the folder from the branch, or from the base when the merged branch was deleted, and copies `.worktreeinclude` files again. The setup command is not re-run on restore; a notice says so.
- Settings → Worktrees lists the records (virtualized) with Agent, Remove folder, Restore and Discard, and measures disk usage on request.

## Code

| Part | Location |
|---|---|
| Records, creation, merge loop, hand-off, cleanup | `crates/core/src/worktrees.rs` (`task_worktree` table, JSON records) |
| Launch, setup task, restore on resume, context directories | `crates/core/src/sessions.rs` |
| Pre-draft promotion | `crates/core/src/predrafts.rs` (`create` with a task worktree `cwd`) |
| API and CLI | `crates/server/src/worktrees.rs`, `launchpad work finish|add-project` |
| UI | `web/src/features/launch/Composer.tsx`, `web/src/features/agents/Worktrees.tsx`, `web/src/features/settings/WorktreesView.tsx`, project page Worktree setup |

Locks: take the session action lock before `App::worktree_lock`, never the reverse, and release both before sending a message to an agent. Shell terminal startup also takes the action lock, so terminal checks stay valid through task Git mutations. Discard takes the pre-draft lock before the worktree lock, reloads the record and checks state, activity and repository/branch ownership while locked, then closes terminals and archives or detaches sessions without releasing the action lock. Promotion holds the pre-draft lock, so it cannot race discard. Merge waits for live terminals in both source and target folders.

## Checks done

On 2026-10-11 with the dev daemon, throwaway repositories in `/tmp` and Claude Haiku: setup and `.worktreeinclude` copying, copied uncommitted changes, agent `work finish` waiting on a dirty main checkout, a conflicting commit on main sent back to the agent, agent rebase and automatic fast-forward, archive cleanup with merged-branch deletion, restore on the next message, hand-off and hand-back, an editable context project, `work add-project`, owner merge, discard of a context worktree, and pre-draft promotion with a local bare remote.

Not covered yet: Windows setup commands (PowerShell), crash recovery in the middle of a rebase (the daemon aborts conflicting rebases immediately, but a crash during `git rebase` leaves the worktree mid-rebase and cleanup keeps it), and a merge request whose target checkout is a linked worktree of another task.

## Group 2 safety checks (2026-10-17)

An isolated dev-profile application state with real throwaway repositories, SQLite databases and command terminals passed twelve manual smoke checks: refusal of automatic rebase on merge history with both heads and a merge-only file unchanged; source and target terminal merge guards; subfolder Code/revision/terminal access with parent and sibling revision reads refused; ignored-file hand-off refusal and included-file handback; directory-symlink copy refusal without an external write; discard reloading promoted ownership after waiting for the mutation lock; rollback after late attachment staging failure; rollback after an actual SQLite insert failure; new pre-draft rollback using a local bare origin; missing editable context blocking resume before harness startup; failed follow-up preserving its shared worktree and removing only its membership; and successful rebase/fast-forward of a linear branch. The check executable was temporary, and its terminals and fixture folders were removed.

Validation also passed `cargo test --locked` (36 core and 14 server tests), pinned-toolchain full-workspace Clippy with warnings denied, the Node 24 UI build/typecheck, three code-viewer safety tests and eleven terminal parser tests. SSH hosts, macOS and Windows were not exercised.

Rebase recovery against `main` at `612f64f` preserved its configurable permission modes in both new-session and resume startup specifications while retaining the launch rollback block. Revalidation passed 38 core and 14 server Rust tests, all twelve smoke checks above, full-workspace Clippy with warnings denied, the Node 24 UI build/typecheck, four composer tests, three code-viewer safety tests and eleven terminal parser tests. Temporary smoke fixtures were removed.

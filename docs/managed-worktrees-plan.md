# Managed agent worktrees

Status: proposal, 2026-10-11. Research and planning are requested; implementation and the choices below still need owner approval. Existing launch behavior remains unchanged.

Launchpad should create a linked Git worktree before starting a new agent in a Git project. The daemon should own creation, recorded paths, completion and cleanup so agents do not spend turns reproducing the worktree preset. Keep an explicit current-checkout option, keep existing pre-draft workflows, and offer an opt-in mode that brings verified commits back to a local target branch before declaring the work complete.

For tasks spanning repositories, distinguish projects the agent may modify from projects supplied as reference. Create worktrees for every explicitly writable repository in the task. Adding context alone should continue to mean reference access.

## What Codex does

The documented Codex behavior is:

- Chats can use Local or Worktree; worktrees are not mandatory for every chat.
- Managed worktrees use ordinary Git worktrees with shared commits and branches. They normally start at detached HEAD from a selected branch, can include its uncommitted changes, and live under `$CODEX_HOME/worktrees` by default.
- Handoff moves a chat between Local and its associated worktree. Permanent worktrees support multiple chats and are excluded from automatic deletion.
- Cleanup defaults to retaining the newest 15 managed worktrees, with a configurable limit or deletion disabled. Pinned and in-progress chats are protected. Archive and exceeding the limit trigger deletion; a snapshot is saved first, with restoration offered later.
- `.worktreeinclude` selects ignored setup files to copy. Ignored `AGENTS.override.md` is copied automatically; source symlinks are skipped and existing destination files are not overwritten. [Official worktree documentation](https://learn.chatgpt.com/docs/environments/git-worktrees).

Setup scripts run automatically when a new worktree is created and can have platform overrides. Configuration is stored under the project's `.codex` directory. [Official local environment documentation](https://learn.chatgpt.com/docs/environments/local-environment).

The documentation does not specify the cleanup scan interval, snapshot storage format or retention, or a transaction for modifying multiple repositories. Those details should be Launchpad design choices, rather than assumptions about Codex internals.

## Current Launchpad behavior

| Area | Existing behavior | Relevant code |
|---|---|---|
| Normal launch | Starts in the registered project checkout | `crates/core/src/sessions.rs`, `launch_with_id` |
| Run in | Checkout or a new/existing pre-draft, plus temporary folders | `web/src/features/launch/Composer.tsx`; `Target` in `crates/core/src/model.rs` |
| Context projects | Prompt explicitly says read without modifying; original paths become additional directories | `crates/core/src/compose.rs`; `preview` and `reattach` in `sessions.rs` |
| Pre-drafts | Owned branch/worktree, local record, idle/clean checks and explicit publication | `crates/core/src/predrafts.rs` |
| Git browsing | Discovers linked worktrees and verifies roots using Git registration | `crates/core/src/code.rs` |
| Session Changes | Reads one session cwd | `crates/core/src/session_git.rs` |
| Scheduling | Stores a launch spec and starts it later; follow-ups resume the same session | `crates/core/src/schedules.rs`, `crons.rs` |
| Continue with another agent | Starts a new conversation in the saved session folder with a fixed trace reference | `Target::SessionFolder`, `sessions.rs`; `ResumeOtherAgent.tsx` |

The owner's current worktree preset normally starts investigations from remote main. The merge preset waits for a clean main checkout, rebases, merges and removes its temporary worktree. The proposed base selection and merge mode must make these choices explicit rather than silently changing their meaning.

## Recommended defaults

| Choice | Recommendation | Reason |
|---|---|---|
| New Git task | New managed worktree | Parallel tasks get separate files and indexes before the harness starts |
| Branch ownership | Unique named local branch, `lp/task-<id>` | Commits remain reachable after removing a directory; simpler than detached-HEAD snapshots |
| Base | Local default branch tip; visible branch selector | Includes local unpublished main commits and works offline |
| Remote base | Explicit fetched remote branch option | Preserves the investigation preset's remote-main workflow without fetching every launch |
| Dirty source | Committed base only, with a visible notice | Avoids silently importing another agent's unfinished edits |
| Completion | Keep for review | Automatic merging changes a shared checkout and needs explicit selection |
| Additional projects | Reference by default; explicit Also modify option | Reading a dependency does not imply changing it |
| Cleanup | Conservative automatic removal, preserving committed work | Do not implement deletion of unfinished work before reliable recovery exists |

If no default branch can be determined, show the current branch as the suggested base and require a visible choice before launch. An explicit Worktree launch in a non-Git, bare, unborn or missing repository fails with a useful explanation. Ordinary non-Git and temporary-folder launches remain direct folder launches and show that fact. No silent fallback from a requested worktree to editing the original checkout.

Base selection resolves to a commit at actual launch, not while drafting or scheduling. Record both the requested ref and resolved commit. The composer preview remains read-only and explains this timing; it never creates a directory or fetches. Resume and scheduled follow-ups reuse the session's recorded worktrees. Each new cron run gets a new set.

New launches into an existing pre-draft reuse its owned worktree and retain its publication rules. An explicit Launch here action on another existing worktree offers reuse of that checkout rather than silently creating another one. Old sessions and saved drafts retain their explicit original checkout behavior; set the new default when creating a fresh draft, with a simple missing-field default for stored specs rather than a data migration framework.

Continue with another agent currently preserves the source folder. Keep that explicit reuse option and associate the new session with the same task set. All sessions sharing it participate in cleanup and input guards, including the original session's quota continuation. Serialize their turns so resuming the old agent cannot collide with the replacement. Offer a separate New worktree from this session choice only from a committed head; do not silently discard its uncommitted progress.

## Shared Git access and filesystem layout

Use `git worktree add -b <owned-branch> <path> <resolved-commit>`, with arguments passed directly to Git. This shares object storage, local/remote refs, tags and remote configuration with the original repository. Agents can inspect other branches with `git show`, compare commits, fetch and perform normal branch operations. There is no clone, container, restricted branch namespace or new sandbox.

Separate checkout files do not prevent an agent with full filesystem access from editing other checkouts. Prompt instructions establish the intended write locations; they are not a security boundary. Shared Git config changes, ref deletions and remote operations also affect the whole repository. Ask agents to work on their assigned branch and leave shared configuration and other checkouts alone unless the task calls for them. Git still prevents checking out the same branch in two worktrees.

Store owned task worktrees under `<projects_root>/.worktrees/tasks/<task-id>/<repository-id>/`, apart from existing pre-draft paths. Persist absolute paths so changing `projects_root` does not move existing runs. The agent starts in the primary project's corresponding subdirectory, preserving projects registered below a repository root.

Identify repositories through canonical Git common directories, not remote URLs or display names. Projects that are subdirectories or linked worktrees of the same repository share one task worktree; map their relative project paths into it. Reject contradictory base selections for the same repository. Independent clones sharing a remote remain independent repositories. Nested repositories and submodules need explicit detection: support ordinary nested independent repositories only when separately selected; block unsupported writable submodule layouts initially rather than claiming they are covered by the parent worktree.

Sibling task directories help tasks using relative dependencies, but cannot preserve every original path relationship. Show the mapping and let project setup recreate required links or build configuration. Do not globally symlink original checkouts, dependency directories or build outputs into task folders; that would allow concurrent tasks to modify the same files.

## Tasks spanning several projects

Keep the existing context chips and branch picker. Add a visible Reference / Also modify selector on project chips. The main project is writable; every additional Also modify Git repository gets a worktree before any harness starts. Offer Make all project context writable as an explicit convenience, not the default. GitHub refs, file references and agent traces do not implicitly add writable projects.

For example, a task launched in a client project can modify the server project while consulting an SDK as reference:

| Project | Role | Agent path |
|---|---|---|
| Client | Primary writable | Task's client worktree |
| Server | Also modify | Task's server worktree |
| SDK | Reference | Existing SDK checkout or explicit revision |

Render a short launch instruction block listing original project, assigned path, branch, base commit and role. Writable projects must no longer receive the current Do not modify instruction. Reference projects keep it. Resolve additional directories from this same recorded mapping at launch, resume, naming and terminal handoff; do not reconstruct writable paths from today's project registry.

Remap working-file context inside selected writable repositories to the task path and explain that it is the committed base copy. Explicit revision references keep their fixed source revision and provenance. If a selected working file contains uncommitted content, show that it will not be included and require removing that context or explicitly choosing a future snapshot feature; do not silently pretend the worktree contains the selected text.

Tasks can discover another project after launch. Add a proposed `launchpad work add-project` command, scoped to the session and a registered project, which deterministically provisions another worktree and returns its path. It must use the same validation and durable records, update session additional-directory support through the adapter, and work only at a safe turn boundary where necessary. If a harness cannot update directory access, require a supported idle reattach before editing. Adding a project never silently edits its original checkout.

Session ownership and workspace filtering continue to follow the primary project and existing launch workspace rules. The task record describes filesystem participation; it does not create another workspace ownership field. Session Changes and Summary need a repository selector so the owner can review every writable repository, including committed diffs against the recorded base. Project worktree rows identify the owning task/session and show all participating projects. Long worktree and repository lists follow the fixed-height virtualization rule.

## Worktree preparation and setup

Persist a task record before creating its directories. Provision all selected repositories, resolve paths and render the final prompt before starting the harness. If any repository fails, no agent starts. Remove only newly created, proven disposable resources; keep uncertain or modified resources with a recoverable error record. Creation is idempotent by task/session ID so a crash cannot produce duplicate worktrees on retry.

Add optional per-project setup commands, configured by the owner, with Linux/macOS/Windows variants. Run them in the mapped project folder and record output, exit code and cancellation. Setup failure prevents the first agent turn and offers Retry setup, Open terminal or Discard. Hold known quota-blocked launches before expensive setup where practical, while keeping their task identity durable. Setup commands and child processes participate in busy and cleanup checks.

Fresh worktrees omit ignored dependencies, `.env` and local configuration. Support an explicit owner-controlled include list, using `.worktreeinclude` conventions if selected, to copy required ignored regular files. Confine source and destination to their repository, reject symlinks/path traversal, do not overwrite tracked files, retain private file permissions, and keep copied secret contents out of prompts and logs. Track copied-file hashes for safe cleanup. Large dependencies should be installed with shared package caches, rather than copied or linked automatically. Do not automatically run arbitrary newly introduced setup configuration from a repository.

Repository instructions may currently mandate main-checkout work, as Launchpad's own `AGENTS.md` does. Generated launch instructions must explicitly authorize the chosen worktree and explain how that launch choice overrides conflicting checkout/branch workflow instructions. Preserve all other repository rules, including checks, documentation and publication restrictions. Existing free-text presets can remain user editable, but selecting a managed mode must suppress the built-in worktree preset and surface conflicting custom merge/cleanup presets rather than send contradictory lifecycle instructions.

## Merge to a local target before completion

Add Worktree then merge to local branch as an opt-in completion mode. Default its target to the local default branch, commonly `main`, and display the exact destination checkout and branch per repository. An existing checked-out feature branch must never be silently switched to main. Require an existing destination checkout of the target, or an explicit owner selection of an appropriate checkout; defer creating hidden target checkouts.

This mode authorizes local integration only. It does not push, open a PR, publish suggestions, restart production, or bypass verification. Select it per launch; an owner may later set a per-project default. Additional writable projects inherit Keep for review unless the owner explicitly selects their merge targets. Completion reports each repository separately; there is no atomic merge across repositories.

### Explicit completion request

The agent verifies its changes, commits them on the assigned branches, and calls a proposed `launchpad work finish --session-id <id> --head <commit>` using the exact injected CLI/profile. For several repositories, the command accepts a structured file listing each repository and expected head. The daemon validates ownership and records the request, then returns immediately with a completion operation ID. It does not wait inside an agent tool call for that same agent to become idle.

A normal turn becoming Done never requests a merge. Questions, planning turns, review feedback, interruption and quota pauses must not integrate work. Completion intent is durable and has its own Requested / Waiting for target / Preparing / Needs attention / Integrated state. Keep the existing session status predicates; show the operation in Summary and Next steps. A turn can end while integration is pending, but the UI must not label the task merged or complete until every requested repository is integrated.

The generated instructions tell the agent to report a pending request as waiting, not as a completed merge. Launchpad displays the eventual integration result without an extra model turn. If the owner requires the conversation itself to produce a final message after integration, that is a separate optional follow-up subject to quota protection.

### Integration sequence

1. Once the root and descendants are idle, freeze new input and pending follow-ups for the affected task. Require no outstanding tools, setup/verification process or terminal in source or destination. Account for writable context membership, not just `Session.cwd`. Quota-paused and In terminal sessions block integration. Re-check the expected source heads and clean indexes/worktrees, including untracked files and in-progress Git operations.
2. If the destination is dirty or busy, retain the request and show why it is waiting. Check again on relevant state changes and a modest periodic scan, without spending agent turns. Never stash, reset, clean, commit someone else's edits or stop their agent. The owner can Cancel merge or Keep for review.
3. When the destination is ready, record its current target commit. Prepare a separate temporary integration worktree and candidate branch from the requested source head, preserving the original task branch. Rebase the candidate's task commits onto that target with ordinary Git semantics. Initially reject merge-commit histories that need a more complex rebase policy. A conflict preserves the source branch and candidate worktree and changes the operation to Needs attention.
4. Verify the resulting candidate after rebase. Use owner-configured project verification commands, captured with the launch, and retain their output. If none is configured, require explicit launch-time acceptance of no automated verification; the agent's earlier checks still appear in Summary. Failed verification keeps the candidate for repair. Conflict or verification repair uses an explicit Resolve with agent action with the candidate path, normal quota/cost checks and the existing root conversation; do not silently launch a paid repair turn.
5. Serialize integration by Git common directory, with stable lock ordering for multiple repositories. Under the shared work-operation lock, re-check target checkout branch, cleanliness, source/candidate heads and idleness. If the target advanced, discard only the disposable candidate and prepare/verify again against the new tip. Apply `git merge --ff-only <candidate-head>` in the destination checkout. No force update, merge conflict resolution or ref manipulation behind a checked-out branch.
6. Verify that the target contains the recorded candidate head and the destination is clean. Persist the integrated head before cleanup. If Git hooks or external activity leave changes, show Needs attention and preserve them. Release the freeze when the operation is integrated, canceled or requires repair. Record partial success across repositories and retry only unfinished entries; never roll back a successful repository automatically.

Locks coordinate Launchpad agents and terminals, not arbitrary external editors or Git commands. Require a quiet destination and Git's own checks; re-check immediately around mutations. Do not promise fully atomic protection against unrelated programs writing files. If the target keeps moving, stop repeated candidate preparation and present the waiting reason for owner action.

After integration, stop/detach idle harness processes before removing source folders. Keep a ref to the original task head for trace/diff inspection. A later Resume or follow-up must provision a fresh continuation worktree from the current selected target and tell the existing conversation its new paths; never point a stale live process at a deleted directory. If automatic cleanup is deferred, keep the existing task worktree with a Merged notice and use the same explicit continuation flow before further edits.

Durable checkpoints distinguish Preparing, mutation attempted, Integrated and cleanup pending. After a crash around fast-forward, inspect the destination and candidate ancestry to determine whether it succeeded. Never replay an uncertain merge or cleanup blindly. Restarting the daemon still interrupts active turns; this proposal does not add session workers.

## Garbage collection and recovery

Adopt a configurable retention target of 15 inactive task sets, with an Off option, as a starting policy. Count a multi-repository task as one set and show actual checkout count and disk usage. Scan at startup, hourly, after archive and after successful completion. Oldest eligible sets go first; active or valuable work may keep usage above the target. Archive makes a set eligible for inspection, not immediate destruction.

The first version automatically removes directories only when all these conditions hold:

- Launchpad owns the recorded directory and branch, and Git registration still matches the expected repository and path. External, permanent and pre-draft worktrees are excluded.
- No participating session is busy, quota paused, in a terminal, pinned or has pending input, naming, a scheduled follow-up, an open question, publication or completion work. Stop/detach idle attached harnesses before removal. Pinned protection must cover all windows and cannot rely on one browser's local storage; persist protection in the daemon or add a separate durable Keep worktree control.
- The index and tracked files are clean, with no untracked files, unfinished Git operation or unexpected branch/head. Every commit remains reachable through a retained task ref or verified integrated target.
- Ignored contents are accounted for: unchanged setup copies may be recreated from recorded sources, and only owner-declared disposable outputs may be dropped. Modified copied files and unknown ignored files protect the folder. A normal clean `git status` alone is insufficient.

Use non-forced `git worktree remove` and verify the outcome. Never recursively erase an unexpected folder or broadly prune unrelated worktrees. Preserve a retained task ref for clean unmerged work; deleting a branch is a separate operation requiring confirmed integration or explicit discard. Do not apply an automatic expiration to retained unmerged commits.

Clean collected worktrees can be restored at their recorded paths from the retained ref, with setup reapplied and a clear notice about regenerated ignored outputs. Automatic resume/follow-up must restore first or fail visibly. If the repository/project was removed, keep the record and data; do not let GC erase orphaned work.

Dirty or unknown work remains on disk and appears as cleanup blocked. Offer Review changes, Keep and explicit Discard with a concrete preview. Do not initially copy Codex's snapshot-and-delete policy for dirty work. A later snapshot scope would need tested restoration of staged/unstaged changes, untracked and ignored files, permissions, binary files and secrets, with visible snapshot size and retention. Reducing checkout count while accumulating unbounded snapshots is not disk reclamation.

## Proposed implementation boundaries

Use a small `worktrees` domain module and a JSON task record in `state.db`, following pre-draft storage conventions. Record task/session ID, participating project mappings, repository common directory, source/target paths, owned refs, selected bases and commits, completion policy/checkpoints, setup metadata and retention protection. Do not build a generic job or migration framework.

| Component | Proposed change |
|---|---|
| `model.rs`, composer store | Execution mode, base ref, context role and per-repository completion policy; existing saved sessions retain explicit original checkout behavior |
| `sessions.rs`, `compose.rs` | Read-only preview; durable preparation before attach; prompt/path mapping; resume/continuation from task records |
| New core/server worktree modules and CLI | Create/status/add-project/finish/cancel/restore/discard, with shared validation and operation locks |
| `predrafts.rs` | Reuse path/idleness principles; prevent duplicate nesting; preserve publication workflow; later offer explicit single-repository task promotion |
| Harness adapters | Consume resolved directories via `StartSpec`; own any native additional-directory update/reattach details |
| `session_git.rs`, code viewer, terminals | Select participating repository; review committed and working diffs; retain registered-root validation and correct terminal cwd |
| Schedules/crons | Provision at actual new launch; reuse/restore for follow-ups; no folders created for a merely scheduled draft |
| Project/session UI, Settings | Worktree identity, operation notices, cleanup protection/reasons and retention controls; visible buttons and virtualized lists |

No editor integration, GitHub auto-publication, dark theme, new shortcuts, remote mode or daemon process-survival work belongs in this scope. Multi-repository pre-draft publication remains separate PRs with owner clicks, not a combined publish operation.

## Delivery order and acceptance

Approve each step with its acceptance checks before implementation. Steps can ship independently; automatic merging should follow reliable creation and recovery.

1. **Default managed worktrees.** Composer mode/base selection, named branches, durable creation, setup, resolved prompt and original-folder option. Launch two sessions in one disposable local repository and show independent files/indexes with shared branches/history. Launch offline, from a dirty source, from a project subdirectory and from an existing linked worktree; verify no source edits are imported or changed. Verify creation/setup failure and restart reconciliation without duplicate folders.
2. **Several writable projects.** Context roles, per-repository deduplication, path mapping, repository review selector and add-project command. Modify two disposable repositories while a third stays reference-only; verify every editable path on resume, terminal handoff and delayed launch. Check duplicate projects in one repo, conflicting bases, non-Git context and unsupported submodules. Confirm generated instructions override only the checkout workflow rule.
3. **Cleanup and restoration.** Retention settings, durable protection, eligibility reasons and clean-directory restore. Verify dirty, untracked, modified ignored, pinned, quota-held, follow-up, descendant and terminal cases are protected. Verify unmerged committed work survives removal/restoration, ownership mismatches never delete a folder, and a removed project preserves its data.
4. **Local merge completion.** Explicit finish request, target waiting, candidate rebase and verification, fast-forward, cancellation and continuation. Keep another agent's destination dirty, commit its changes, then show preparation against the new tip and a clean final target. Check conflicts, verification failure, changing target, interruption/restart around mutation, open terminals, merge-history rejection and partial integration across two repos. A planning Done must never request integration. No network push or GitHub mutation occurs.

For implementation checks, use throwaway local repositories in the dev profile and real harnesses only when necessary, with paid usage disabled unless explicitly requested. Close opened terminals, cancel test follow-ups and pause/delete test crons. Run pinned workspace Clippy and UI typecheck/build, regenerate shared types, and inspect narrow/wide layouts at all zoom positions. Add tests only for small tricky pure helpers; no mock/server/network test suite. Never restart production for validation.

## Choices to approve

The recommended first implementation is steps 1 through 3, with step 2 included because modifying several projects is a real requirement. Approve step 4 separately once the lifecycle is reliable. The main product choices are local-default-branch versus current-branch bases, explicit writable context versus treating all context as writable, named branches versus detached HEAD plus snapshots, and conservative cleanup versus snapshots of unfinished work. This proposal selects the first option in each pair.

For small projects, the eventual Worktree then merge mode can replace the current preset without requiring PR management. It makes waiting and Git operations deterministic, preserves source commits through failed integration, and leaves semantic conflict repair with the agent under the owner's normal run and spending controls.

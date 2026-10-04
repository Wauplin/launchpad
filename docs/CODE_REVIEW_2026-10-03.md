# Launchpad code and plan review — 2026-10-03

Phase 7 has been reverted. The remaining application implements most of Phases 0–6, but successful builds do not establish that its execution, publication, recovery, and review safeguards are correct. The highest priorities are command ownership after interruption, ordered durable session writes, incomplete pre-draft reservations, asynchronous stop/handoff, and Git publication destination and branch-creation races. Several of these problems predate Phase 7 and survive its removal.

**Reviewed state:** main after reverting commit 7b670af40c041dad5ee7039839a6681d8af945d6, “Add restart-safe sessions and terminal options”. Implementation files match its parent, 9fdcb0a, “Add local agent-written comment and review suggestions”; README.md, AGENTS.md, ARCHITECTURE.md and ROADMAP.md additionally record the scope correction. The revert/documentation commit is 847320f10fa3d977791ef34a13252f2a5e29b06c, “Revert unscoped Phase 7 extensions”. **Deliverable:** this updated report intentionally remains uncommitted. Only the requested revert and matching documentation are committed; no other fixes, repository branches, or Launchpad worktrees were created.

## Scope, evidence, and coverage

This reassessment covers the original brief (initial.md), README, AGENTS.md, roadmap acceptance criteria, architecture and decisions, spike notes, sketches, Rust core and both harness adapters, CLI, API/authentication/WebSocket, Tauri shell, React routes/components/stores, databases, generated types, dependency locks, packaging and CI. The earlier report was checked against the reverted code, rather than carrying forward all 45 findings as current defects.

“Reproduced” means a local check exercised the actual function, daemon, or real Git behavior. “Source-confirmed” means the inspected control flow establishes the defect, although its complete user scenario was not exercised. “Risk” means failure requires the stated circumstances and its occurrence or exploitability is not established. Priorities are relative to this single-user Ubuntu application, not CVSS scores: High concerns incorrect execution/publication or durable data loss; Medium concerns core correctness, recovery, availability, or a security boundary; Low concerns narrower usability, maintenance, or documentation.

No finite review can promise that every defect or vulnerability has been found. This report gives the coverage, concrete evidence, remaining acceptance work, and untested boundaries needed to assess its conclusions. It does not certify real GitHub publication, native harness recovery, or WebKitGTK performance from a Chrome screenshot.

| Area inspected | Main implementation and cross-checks |
|---|---|
| Startup, storage, configuration, process environment | core/lib.rs, db.rs, paths.rs, config.rs, shellenv.rs, procs.rs, events.rs, model.rs, util.rs, core/build.rs; profile isolation, migrations/cache rebuild, permission and process ownership paths. |
| Agent execution and recovery | sessions.rs, codex_rpc.rs, harness/claude.rs, harness/codex.rs, harness/mod.rs, schedules.rs, terminal.rs, status.rs, notify.rs, titles.rs; spike protocols, Done predicates, questions, stop/resume, quotas, dependencies, defaults. |
| Projects, context, external history, local Git | projects.rs, git.rs, compose.rs, drafts.rs, history.rs, workspace.rs, code.rs; source validation, revisions, file/path handling, temporary folders, import fidelity, promotion/removal. |
| GitHub and publication | github.rs, inbox.rs, items.rs, predrafts.rs, suggestions.rs and their server routes; cache/error semantics, image proxy, partial GraphQL data, review identity, publication checkpoints and retries. |
| API, desktop, CLI | server/api.rs, auth.rs, routes.rs, ws.rs, code.rs, items.rs, predrafts.rs, suggestions.rs, error.rs, ui.rs; cli/main.rs; desktop/src-tauri configuration, capabilities and main.rs. |
| Browser UI | app shell/router/sidebar/status bar, shared Markdown/popovers/toasts, queries/API/WebSocket/session stores, layout/composer/message stores; agents, launch, inbox, projects, code, GitHub, pre-draft, suggestions, settings and performance-page features. |
| Distribution and documentation | Cargo manifests/lock, web manifest/lock/.nvmrc/Vite config, generated types, install/update/icon scripts, service/desktop/autostart assets, CI, icon metadata, all phase progress and acceptance descriptions. |

## Phase 7 revert and corrections to the previous report

The reverted commit bundled three optional extensions: independent systemd session workers with socket/replay infrastructure, embedded PTY/xterm harness controls, and per-browser remote command-copy mode. Their modules, routes, CLI options, UI controls, dependencies, service changes, and sketch annotations have been removed together. Headless adapters are daemon-owned again; external terminal handoff and native conversation resume remain. A daemon/service restart interrupts active headless work. No production daemon or worker was restarted, stopped, archived, or updated to apply this change.

Phase 7 is now an unselected candidate list. Its absence is intentional, not missing implementation. Selecting another extension requires a concrete user need, one bounded scope, ownership/recovery rules, non-goals and acceptance evidence. Phases 1–6 still have outstanding daily-use checks.

| Previous finding | Reassessment after revert |
|---|---|
| R03: delayed worker mirror used for idle reservations | Withdrawn from current findings: the mirror/watch architecture is removed. Other reservation bypasses remain in R04, R46 and R47. |
| R05: workers opening/rebuilding the shared GitHub cache | Withdrawn: independent workers no longer open App/cache resources. Duplicate daemon startup remains a separate problem, R51. |
| R15: worker Claude quota events not reaching the daemon | Withdrawn: Claude's adapter updates the daemon StatusHandle directly again. Codex account events have a separate daemon monitor; a threadId filter in its session router alone does not establish a quota bug. |
| R16: embedded terminal misses generic WS resync | Withdrawn: the embedded terminal and its replay path were removed. Durable conversation catch-up still has R10. |
| R29: project removal breaks saved-cwd session resume | Narrowed: local PRs can be orphaned; native reattachment uses the saved cwd and does not require the removed project's launch preview. |
| Preset and notification documentation claims | Corrected: architecture 10.3 already records the deliberate local-first replacement of the draft-pr preset. Session notifications are daemon-owned again; example inbox notifications still lack an implementation. |

Finding ids are preserved, so these withdrawn ids are intentionally absent from the active index.

## Verification performed on the reverted code

| Check | Result and limits |
|---|---|
| cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings | Passed, including desktop targets. |
| cargo +1.93.1 test --locked | Passed all 11 existing core tests; test execution took less than a second. No permanent tests or mocks were added. |
| UI typecheck/build | Passed using Node 24.11.0 and Corepack pnpm 10.33.0, matching the repository's Node major and package manager. An earlier run also passed under ambient Node 22; the pinned-major run is the relevant check. |
| cargo +1.93.1 build --locked -p launchpad -p launchpad-desktop | Passed debug daemon/desktop builds. Fresh release artifact and native window runtime were not verified. |
| Shared TypeScript generation | Regenerated all 62 files into /tmp; no differences from checked-in types after ignoring whitespace. |
| Packaging shell syntax | bash -n passed for install.sh, update.sh and install-icons.sh. No installer/updater was executed. |
| Dependency audits | Rust: 592 dependencies, no vulnerability-category entries, but glib unsoundness and proc-macro-error unmaintained warnings (R21), advisory DB updated 2026-10-02. npmjs audit: zero reported advisories across 182 dependencies. Advisory coverage is not proof of safety. |
| Isolated dev daemon | 40 checks passed using a private network/PID namespace and an empty mounted home, with no account credentials, Claude/Codex binaries, user-manager sockets or external connectivity. Existing prod and dev daemons were untouched. |
| Local workflows in that daemon | Auth/foreign Host/Origin rejection; project registration; code reads/search/root and traversal rejection; local pre-draft creation; worktree code access; notes; local suggestion conversion; future scheduling/cancellation; clean discard; unregister preserving the regular folder. All remotes were throwaway filesystem repositories. |
| Chrome via DevTools protocol | Code loaded at all five zoom settings with real waits; Shiki worker colors appeared. Agents, Projects, local PR and Settings routes rendered with no runtime exceptions. Screenshots at 100% and 150% were visually inspected. This was a tiny fixture, not a large-list or WebKitGTK benchmark. |
| Additional daemon reproductions | A frozen pre-draft rejected ExistingPredraft launch but accepted Project launch into the same registered worktree (R04). A second daemon rewrote a seeded Working row to Interrupted before failing its occupied-port bind (R51). No paid harness task ran. |
| Diff and UI functions | Actual parse_diff extracted/compiled: header-like hunk content changes filename (R08). Actual buildRows imported with Node type stripping: a later different tool result has the same memo version (R09). Space-name Git headers contain an unhandled trailing tab. |
| Git and desktop predicates | Local bare-remote race demonstrated an ordinary push advancing a branch created after an absence check (R07); effective fetch/push URLs differ (R06). URL-prefix predicate accepts an example.com host disguised with localhost userinfo (R18), without opening that site. |
| Icons and repository diff | Master/export metadata checked without changing assets. After revert, implementation matches 9fdcb0a exactly; only four documentation files have additional scope edits. |

Ad hoc scripts, fixture repositories, screenshots and raw logs are in /tmp; they are not committed or shipped tests. Normal daemon startup can perform real account polling/harness updates, so runtime checks used isolation rather than the owner's running dev service. Real comments/reviews, labels, state, merge, subscriptions, publication, installation and owner acceptance workflows were not exercised. Roadmap descriptions of earlier real-agent/browser runs remain historical evidence, not newly repeated checks.

## Comparison with the plan and original brief

| Phase | Implementation found | Assessment against “Done when” |
|---|---|---|
| 0 — foundations | Workspace, config watcher, SQLite, HTTP/WS, auth, profiles, shell environment, UI/Tauri shell, systemd packaging, icons, CI. | Substantially present. Local build checks pass. Startup ownership/permissions need R17/R51; warm native startup and hosted release jobs were not repeated. |
| 1 — agents | Projects/temp folders, composer/context/presets, both adapters, logs/questions, messages/images, usage/titles, groups/layouts, delayed starts, native terminal handoff and update controls. | Built with significant execution/recovery findings R01/R02/R04/R09–R14/R46–R48. The required week of daily use is still pending. No restart survival is promised after the revert. |
| 2 — inbox | REST sync, durable membership/overrides, subscription checks, previews/enrichment, triage/replies, agent context. | Built. R27/R28/R50 affect responsiveness and ambiguous outcomes. A morning of owner triage remains pending. REST history is correctly distinguished from the web inbox. |
| 2b — code viewer | Root/worktree validation, revisions, files/tabs, text/image/binary views, Shiki worker, line marks, find/search, selection/context, editor/permalink. | Built; small local workflows/colors/zoom verified. R19/R20/R33/R38/R44 and silent caps remain. A week of real reading and native WebKitGTK checks is pending. |
| 3 — GitHub items | Cached conversations, paginated timelines/review threads, HTML sanitation/image proxy, checks/reviewers/labels/state/reviews/merge, list filters, local/API diffs, split/unified/views/drafts. | Built. R08/R22–R26/R50/R52/R55/R56 affect review completeness and safety. No real review/merge acceptance was repeated. |
| 4 — pre-draft PRs | Local branches/worktrees, CLI metadata, durable notes, review/feedback, explicit Publish, checkpoints/recovery lookup, cleanup/discard, linked skill. | Built; local create/review/note/discard exercised. R04/R06/R07/R29/R30/R46/R47/R49 affect the reservation/publication/recovery contract. Two weeks of daily use and intended publication remain pending. |
| 5 — workspace polish | Drag/drop, focus/move/layout/pins, rename/search, external history import, session Git panel, recent context. | Built. R31/R32/R38 expose fidelity/performance/context gaps. Four-agent comfort and finding old sessions need owner acceptance. |
| 6 — suggestions | CLI/instructions, durable drafts/attribution, edit/discard/post cards, reviewed-head checks, durable posting claims, atomic local notes. | Implemented with a sound local-first path; local note conversion exercised. Edited real-review posting remains pending. Full-access agents are trusted, not technically prevented from bypassing the path. |
| 7 — optional extensions | No selected implementation; workers/embedded terminal/remote mode reverted. Third harness, inline comments, cached-thread prompt expansion and editing also unselected. | Deferred deliberately. Re-scope one candidate before implementation; do not mark this phase In progress or Done on the strength of the removed code. |

The original brief's core goals—starting/contextualizing agents, a readable event workspace, fast cached GitHub triage/review, local pre-drafts and click-based publication—are represented. Several absolute promises (“everything cached”, “never loading”, “every message Copy”, “idle worktree”, “nothing without a click”) exceed the implemented cold-path, rendering or trust guarantees. The detailed gaps below need correction or a deliberate narrowing of the promise. No dark theme, shortcuts beyond the chosen exceptions, other operating systems, multi-user system, or broad test suite is required.

“In progress” remains appropriate for Phases 1–6 until owner acceptance is recorded. The review does not substitute compile success for a week or two of actual use. The roadmap now separates implementation evidence from acceptance and describes Phase 7 as candidates.

## Findings index

**53 active findings: 8 high, 37 medium, 8 low.** Four removed-implementation findings are withdrawn above. Conditional risks are identified individually.

| ID | Priority | Finding |
|---|---|---|
| [R01](#r01--high--codex-interruption-marks-a-still-running-command-done) | High | Codex interruption marks a still-running command Done |
| [R02](#r02--high--event-numbering-durable-writes-and-snapshot-publication-are-not-one-ordered-operation) | High | Event numbering, durable writes, and snapshot publication are not one ordered operation |
| [R04](#r04--high--pre-draft-launch-freeze-and-idle-checks-do-not-cover-every-entry-path) | High | Pre-draft launch freeze and idle checks do not cover every entry path |
| [R06](#r06--high--publication-can-push-to-a-different-repository-than-the-pr-api-targets) | High | Publication can push to a different repository than the PR API targets |
| [R07](#r07--high--remote-branch-absence-is-checked-separately-from-creation) | High | Remote branch absence is checked separately from creation |
| [R08](#r08--medium--diff-parsing-corrupts-file-identity) | Medium | Diff parsing corrupts file identity |
| [R09](#r09--medium--memoization-hides-late-results-and-freezes-live-output-tails) | Medium | Memoization hides late results and freezes live output tails |
| [R10](#r10--medium--gap-recovery-can-drop-the-final-event-until-another-reconnect) | Medium | Gap recovery can drop the final event until another reconnect |
| [R11](#r11--medium--failed-or-uncertain-message-delivery-leaves-misleading-state-and-can-duplicate-work) | Medium | Failed or uncertain message delivery leaves misleading state and can duplicate work |
| [R12](#r12--medium--answer-failures-consume-questions-before-successful-delivery) | Medium | Answer failures consume questions before successful delivery |
| [R13](#r13--medium--resume-can-relabel-an-active-session-as-waiting) | Medium | Resume can relabel an active session as Waiting |
| [R14](#r14--medium--codex-stream-overflow-discards-lifecycle-events-without-reconciliation) | Medium | Codex stream overflow discards lifecycle events without reconciliation |
| [R17](#r17--medium--token-creation-has-a-permission-window-and-private-state-relies-on-parent-directory-permissions) | Medium | Token creation has a permission window, and private state relies on parent-directory permissions |
| [R18](#r18--medium--desktop-navigation-trusts-a-url-string-prefix-instead-of-its-origin) | Medium | Desktop navigation trusts a URL string prefix instead of its origin |
| [R19](#r19--medium--read-only-diff-views-can-execute-configured-text-converters) | Medium | Read-only diff views can execute configured text converters |
| [R20](#r20--medium--code-reads-check-paths-and-sizes-before-opening-them) | Medium | Code reads check paths and sizes before opening them |
| [R21](#r21--medium--desktop-dependency-tree-contains-a-known-unsound-glib-release) | Medium | Desktop dependency tree contains a known unsound glib release |
| [R22](#r22--medium--partial-graphql-failures-can-be-saved-as-complete-conversations) | Medium | Partial GraphQL failures can be saved as complete conversations |
| [R23](#r23--medium--client-diff-identity-omits-the-base-commit) | Medium | Client diff identity omits the base commit |
| [R24](#r24--medium--deleted-file-navigation-can-open-a-different-base-than-the-displayed-three-dot-diff) | Medium | Deleted-file navigation can open a different base than the displayed three-dot diff |
| [R25](#r25--medium--cache-invalidation-does-not-reliably-mean-github-refresh) | Medium | Cache invalidation does not reliably mean GitHub refresh |
| [R26](#r26--medium--serial-image-prefetch-delays-the-first-usable-detail-and-has-no-cache-budget) | Medium | Serial image prefetch delays the first usable detail and has no cache budget |
| [R27](#r27--medium--inbox-sync-holds-the-action-lock-across-historical-network-work) | Medium | Inbox sync holds the action lock across historical network work |
| [R28](#r28--medium--unsubscribe-rollback-can-misrepresent-a-successful-remote-ignore) | Medium | Unsubscribe rollback can misrepresent a successful remote ignore |
| [R29](#r29--medium--removing-a-project-orphans-local-prs-and-their-worktrees) | Medium | Removing a project orphans local PRs and their worktrees |
| [R30](#r30--medium--promotion-crosses-filesystem-transcript-and-database-steps-without-recovery) | Medium | Promotion crosses filesystem, transcript, and database steps without recovery |
| [R31](#r31--medium--imported-history-is-a-lossy-snapshot-and-worktree-attribution-is-incomplete) | Medium | Imported history is a lossy snapshot and worktree attribution is incomplete |
| [R32](#r32--medium--several-potentially-long-lists-violate-the-webkitgtk-rendering-rule) | Medium | Several potentially long lists violate the WebKitGTK rendering rule |
| [R33](#r33--medium--regular-expression-find-can-block-the-ui-thread) | Medium | Regular-expression find can block the UI thread |
| [R34](#r34--medium--the-manual-installer-does-not-reserve-production-and-can-inspect-the-wrong-profile) | Medium | The manual installer does not reserve production and can inspect the wrong profile |
| [R35](#r35--medium--a-terminated-updater-can-leave-the-ui-permanently-updating) | Medium | A terminated updater can leave the UI permanently “Updating” |
| [R36](#r36--medium--launch-defaults-represent-current-session-status-not-last-successful-launch) | Medium | Launch defaults represent current session status, not last successful launch |
| [R37](#r37--medium--composer-draft-persistence-silently-loses-recent-or-failed-saves) | Medium | Composer draft persistence silently loses recent or failed saves |
| [R38](#r38--medium--adding-a-revision-selection-to-an-existing-agent-can-identify-the-wrong-source) | Medium | Adding a revision selection to an existing agent can identify the wrong source |
| [R39](#r39--low--hand-edited-config-bypasses-validation-applied-by-settings) | Low | Hand-edited config bypasses validation applied by Settings |
| [R40](#r40--low--settings-holds-an-old-draft-after-external-configuration-changes) | Low | Settings holds an old draft after external configuration changes |
| [R41](#r41--low--project-fields-violate-the-click-only-interaction-rule) | Low | Project fields violate the click-only interaction rule |
| [R42](#r42--low--model-refresh-does-not-invalidate-the-composers-model-query) | Low | Model refresh does not invalidate the composer's model query |
| [R43](#r43--low--a-healthy-daemon-with-an-unreadable-token-leaves-the-desktop-on-its-startup-page) | Low | A healthy daemon with an unreadable token leaves the desktop on its startup page |
| [R44](#r44--low--highlight-worker-failure-leaves-future-requests-unresolved) | Low | Highlight worker failure leaves future requests unresolved |
| [R45](#r45--medium--unbounded-reads-and-gitnetwork-waits-undermine-the-fast-always-usable-interface) | Medium | Unbounded reads and Git/network waits undermine the fast, always-usable interface |
| [R46](#r46--high--archive-and-terminal-handoff-return-before-execution-is-quiescent) | High | Archive and terminal handoff return before execution is quiescent |
| [R47](#r47--high--temporary-deletion-and-promotion-do-not-reserve-all-execution-owners) | High | Temporary deletion and promotion do not reserve all execution owners |
| [R48](#r48--medium--persisted-questions-have-no-reconciliation-when-their-owner-is-replaced) | Medium | Persisted questions have no reconciliation when their owner is replaced |
| [R49](#r49--medium--a-definitively-failed-first-push-has-no-local-abortrecovery-action) | Medium | A definitively failed first push has no local abort/recovery action |
| [R50](#r50--medium--ordinary-github-comment-and-review-failures-do-not-preserve-uncertain-outcomes) | Medium | Ordinary GitHub comment and review failures do not preserve uncertain outcomes |
| [R51](#r51--high--a-duplicate-daemon-mutates-state-before-discovering-its-port-is-occupied) | High | A duplicate daemon mutates state before discovering its port is occupied |
| [R52](#r52--medium--the-displayed-item-head-and-the-returned-diff-are-not-bound-together) | Medium | The displayed item head and the returned diff are not bound together |
| [R53](#r53--medium--a-failed-initial-conversation-fetch-leaves-permanent-loading-without-retry) | Medium | A failed initial conversation fetch leaves permanent Loading without retry |
| [R54](#r54--low--conversation-rendering-falls-short-of-the-documented-copy-and-markdown-behavior) | Low | Conversation rendering falls short of the documented Copy and Markdown behavior |
| [R55](#r55--medium--saving-untouched-labels-can-overwrite-newer-github-labels) | Medium | Saving untouched labels can overwrite newer GitHub labels |
| [R56](#r56--medium--nullable-github-authors-and-reviewer-data-can-crash-item-views) | Medium | Nullable GitHub authors and reviewer data can crash item views |
| [R57](#r57--low--notification-suppression-is-global-rather-than-per-connected-window) | Low | Notification suppression is global rather than per connected window |

## Execution, persistence, and publication

### R01 — High — Codex interruption marks a still-running command Done

**Evidence:** Source-confirmed, backed by the repository's own observed protocol behavior. crates/core/src/harness/codex.rs drains open tool items, writes synthetic Interrupted results, clears turn_id, and sets Waiting at turn/completed. docs/spikes/3-codex-app-server.md records that an interrupted sleep 40 completed successfully after the interrupt; the adapter comment also explicitly acknowledges that running commands continue.

**Failure:** Stop can make a session satisfy the Done predicate while its command is still running. Dependent launches can start immediately. Publication and cleanup can consider the worktree idle while it is still being changed. This contradicts the product rule that Running includes background commands, even when silent.

**Recommendation:** Retain outstanding command ownership after turn completion. Keep the session Background until the actual completion or confirmed process termination; handle a new foreground turn and old outstanding commands together. Validate interrupt during a long command, a chained scheduled launch, and a local pre-draft review.

### R02 — High — Event numbering, durable writes, and snapshot publication are not one ordered operation

**Evidence:** Source-confirmed concurrency defect; not stress-reproduced. In crates/core/src/sessions.rs, update (line 277) releases the record mutex before awaiting persistence, while Sink::emit (line 738) allocates a sequence under the mutex and submits its database job afterward. Multiple Tokio tasks can submit jobs and publish snapshots in a different order from their in-memory mutations. Db serializes submitted jobs, but submission order is not tied to mutation order. persist rewrites last_seq from a captured snapshot. emit inserts the event and updates last_seq in separate SQL statements without a transaction.

**Failure:** An older snapshot can overwrite newer status, usage, or last_seq. Events can reach the WebSocket out of order and trigger incomplete UI catch-up (R10). A crash between event insertion and cursor update leaves a stored event above the restart counter, so the next allocation can hit an existing primary key. emit logs storage errors and still publishes, allowing the UI to display an event that was never durable. Removing workers does not serialize the remaining tasks inside the daemon.

**Recommendation:** Serialize each session's mutations, sequence allocation, persistence, and publication through one ordered writer. Insert the event and update its cursor atomically; validate the cursor against stored events on recovery. Surface failed durable writes and avoid publishing them as successful persistence.

### R04 — High — Pre-draft launch freeze and idle checks do not cover every entry path

**Evidence:** Reproduced for the normal-project launch path; source-confirmed for subdirectory identity and answers. sessions.rs::launch_with_id checks pushed_head for Target::ExistingPredraft, but Target::Project takes the registered path without check_session_work. predrafts.rs::check_session_work and idle compare exact root/cwd strings. sessions.rs::answer omits the pre-draft freeze check. Registration accepts a worktree or any of its subdirectories as another project.

**Reproduction:** In an isolated dev daemon, a local pre-draft was set to the persisted publication checkpoint and its worktree was registered as a normal project. ExistingPredraft launch returned 400, while Project launch into the same root returned 200 and created a session. That session then failed only because Claude was deliberately absent from the isolated PATH; no agent executed and no remote push occurred.

**Failure:** Fresh launches can enter a frozen worktree even at its exact root. Registered/imported subdirectory sessions evade the idle and freeze checks. Answering an already-open question can restart work without checking the freeze. Publication or cleanup can therefore run concurrently with work in the tree it promises to reserve. Regular publication holds allow_work while it runs, but bypasses matter after an attempted publication releases that guard with its checkpoint still set.

**Recommendation:** Resolve every execution cwd to canonical Git worktree identity and apply one freeze/idle policy to project launches, existing-pre-draft launches, send, answer, resume, terminal handoff, and cleanup. Include descendant cwds and external owners. Registration must not provide a second route around the policy.

### R06 — High — Publication can push to a different repository than the PR API targets

**Evidence:** Reproduced Git behavior, source-confirmed integration bug. crates/core/src/git.rs parses only remote url from the common config. predrafts.rs derives the GitHub repository from that value, but git push origin honors pushurl and Git URL rewriting. Includes and several valid Git configurations are also not resolved by the handwritten parser.

**Failure:** With origin.url pointing to repository A and origin.pushurl to repository B, Launchpad pushes the reviewed branch to B and tries to create a PR in A. This can publish code to an unintended repository, then leave a frozen partial publication.

**Recommendation:** Ask Git for effective fetch and push URLs, verify the intended GitHub destination before publication, and either support separate destinations deliberately or reject ambiguous configurations. Display the resolved destination in the concrete publication confirmation. The current local reproduction used two different filesystem remote URLs; no external remote was contacted.

### R07 — High — Remote branch absence is checked separately from creation

**Evidence:** Reproduced using a throwaway local bare remote, and source-confirmed in publication. predrafts.rs checks ls-remote, saves the chosen branch, then performs a normal push.

**Failure:** Another process can create that branch after the absence check. If its tip is an ancestor of the reviewed head, the normal push can fast-forward the other process's branch. “Without force” prevents non-fast-forward replacement, but does not enforce “create only if still absent.” A retry has the same problem if the first attempt's outcome was uncertain.

**Recommendation:** Use an atomic expected-ref condition: initially require absence, and on recovery verify that the existing branch is exactly the recorded reviewed head. Fail on an unrelated remote head rather than advancing it. A local reproduction checked absence, created the branch at an ancestor, then showed that an ordinary push advanced it successfully. No external remote was contacted.

### R08 — Medium — Diff parsing corrupts file identity

**Evidence:** Reproduced using the actual parser from items.rs. Header handling remains active after the hunk starts. An added source line whose contents are “++ b/unrelated.txt” produces the patch line “+++ b/unrelated.txt” and changes the file's filename to unrelated.txt. The patch contents remain present; the problem is metadata corruption.

A second reproduction used real git diff with core.quotepath=false on space name.txt. Git adds a tab separator to its ---/+++ headers, and the parser returns a filename ending in a literal tab. Git's C-style quoted paths are decoded as JSON, which also does not cover all Git octal/control-character escapes.

**Failure:** File selection, viewed markers, file notes, syntax detection, and View file can target an incorrect or nonexistent path. A normal filename with a space already triggers this, without hostile input.

**Recommendation:** Parse metadata only before hunks, decode Git's actual path syntax, and avoid deriving identity from ambiguous display headers when NUL-delimited Git metadata is available. Add small pure regression cases for content resembling headers, spaces, quoted controls, renames, deletions, and binary-only patches.

### R09 — Medium — Memoization hides late results and freezes live output tails

**Evidence:** Reproduced against buildRows in web/src/features/agents/rows.ts. Tool row versions include only item id, result presence, and live-output length. EventRow's comparator in EventRow.tsx checks that version instead of the actual result/output. sessionStore.ts caps output at 8,000 characters.

**Failure:** An Interrupted result and its later successful completion have the same version, so the rendered row retains the interrupted/error output. Once live output reaches 8,000 characters, replacing the tail does not change its length, and visible streaming output stops updating. This directly defeats the documented late-result replacement.

**Recommendation:** Include a monotonic result/output revision in the row version, or compare immutable data identities that change with content. The reproductions found unchanged versions for interrupted versus successful output and for an 8,000-character all-a tail versus an all-b tail.

### R10 — Medium — Gap recovery can drop the final event until another reconnect

**Evidence:** Source-confirmed race. web/src/lib/sessionStore.ts drops a gap event and calls load. load returns immediately when that session already has a fetch in flight; the dropped event is not queued and does not request a second fetch.

**Failure:** A gap arriving after the in-flight fetch's database snapshot can be omitted from that response and discarded locally. If it is the final event, there may be no later event to trigger recovery. The UI can remain incomplete even though the durable log contains the missing message/result.

**Recommendation:** Buffer out-of-order events or mark a pending catch-up and fetch again after the current request. Reconcile by sequence and preserve both HTTP and WS arrivals. Validate the boundary where the final event arrives just after the first catch-up snapshot.

### R11 — Medium — Failed or uncertain message delivery leaves misleading state and can duplicate work

**Evidence:** Source-confirmed. sessions.rs::send stores a UserMessage and sets Working before live.send, then propagates failure without recording a delivery failure. codex.rs falls back from turn/steer to turn/start on every error, rather than only a known ended-turn response. codex_rpc.rs::request has a 60-second timeout without operation identity/reconciliation.

**Failure:** A rejected write/RPC can leave a session Working and a message appearing sent. A timeout can mean the harness accepted the operation but the caller never received acknowledgement; retrying may submit another message. Treating any steer failure as “the turn ended” also attempts a new turn after transport or unrelated protocol errors.

**Recommendation:** Distinguish queued, delivered, failed, and uncertain states. Fall back only on the specific ended-turn condition. Use the persisted client message identity across transport boundaries and reconcile uncertain outcomes before retrying. Do not infer successful delivery solely from a stored user-message event.

### R12 — Medium — Answer failures consume questions before successful delivery

**Evidence:** Source-confirmed. claude.rs removes the question from its map before writing the answer; codex.rs removes the request before responding. Both zip questions with supplied answers without enforcing the count.

**Failure:** An I/O failure can leave the session showing an unanswered question that can no longer be answered: a retry says it was already answered. Incomplete answer arrays can silently omit questions. In EventRow.tsx's AskCard, choosing the last single-select answer can auto-submit a mixed single-/multi-select question set before the owner clicks Send answer; that multi-select button also lacks the live/completeness checks used by the option buttons.

**Recommendation:** Validate answer count and required selections, retain a request until delivery is acknowledged or explicitly uncertain, and require the explicit send action whenever the request contains multi-select questions. Keep stopped-session controls disabled consistently.

### R13 — Medium — Resume can relabel an active session as Waiting

**Evidence:** Source-confirmed. sessions.rs does not require an archived/interrupted/error state. If a live adapter already exists, reattach returns it; resume then emits Resumed and sets Waiting.

**Failure:** A repeated request from a stale client, or a direct authenticated API call, can mark an actively executing session Done. The same session is then eligible for dependencies or idle checks. The normal UI reduces exposure by usually showing Resume only in resumable states, but the backend invariant is absent.

**Recommendation:** Make Resume an idempotent state transition with an authoritative owner check. Reject it while a turn or background command is active instead of unconditionally changing the status.

### R14 — Medium — Codex stream overflow discards lifecycle events without reconciliation

**Evidence:** Source-confirmed conditional failure. codex_rpc.rs uses an 8,192-message broadcast channel. harness/codex.rs handles Lagged only by logging a warning and continues. The router awaits raw/event persistence for each incoming notification.

**Failure:** Sufficient output or slow persistence can drop a user-input request, item completion, or turn completion. The adapter has no state resync after loss, so it may remain Working forever or fail to expose a question the harness is awaiting. This has not been stress-reproduced.

**Recommendation:** Use backpressure or a durable ordered ingress where loss is unacceptable. On overflow, reconcile thread/turn state and expose an error if it cannot be reconstructed; a log warning alone does not restore correctness.


## Security and trust boundaries

### R17 — Medium — Token creation has a permission window, and private state relies on parent-directory permissions

**Evidence:** Source-confirmed conditional security exposure. paths.rs creates token.tmp with std::fs::write, then chmods it to 0600. ensure_dirs uses ordinary create_dir_all. Existing 64-character tokens are accepted without correcting their mode. Databases and cached images use normal creation permissions.

**Failure:** With a permissive home/parent directory and the usual 022 umask, another local user can read the newly written temporary token before chmod. Persistent sessions, prompts, image attachments, private GitHub content, and drafts can also be readable through 0644 files in 0755 directories. A 0700 home directory blocks that access, so this is not a demonstrated exposure on the owner's current machine.

**Recommendation:** Create tokens privately from the first open using an explicit mode and unique no-follow temporary file. Make the application's private data/config directories 0700, and repair existing token permissions. Handle concurrent first-use token creation without two callers receiving different credentials.

### R18 — Medium — Desktop navigation trusts a URL string prefix instead of its origin

**Evidence:** Reproduced predicate; desktop/src-tauri/src/main.rs. The URL http://127.0.0.1:7317@example.com/ starts with the configured origin string but has example.com as its actual host. The same pattern applies to the dev origin.

**Failure:** External content can be classified as internal and loaded inside the app window, contradicting the rule that external links leave the webview. No cookie theft, Tauri capability bypass, or API execution has been demonstrated; this is a confirmed navigation trust-boundary bug, not a proven RCE exploit.

**Recommendation:** Compare parsed scheme, host, and effective port exactly, with a separate narrow rule for the actual packaged Tauri origin. Reject credentials in internal URLs. Inspect remote-origin capabilities while fixing the predicate.

### R19 — Medium — Read-only diff views can execute configured text converters

**Evidence:** Source-confirmed. GitHub local diffs at items.rs and code changed_lines at code.rs pass --no-ext-diff but omit --no-textconv. The local pre-draft and workspace diff paths already pass --no-textconv, showing the inconsistency.

**Failure:** Git's configured textconv driver can run a command and display transformed contents when the owner only opens a review/code view. It can also make line numbers and displayed text disagree with the raw file. A repository's attributes alone do not define an arbitrary executable converter; the driver must be configured in Git. This is a conditional execution/accuracy risk, not evidence that any downloaded repository can immediately execute code.

**Recommendation:** Disable text conversion consistently in all raw review and line-mark paths, or deliberately show converted output with a clear separate contract.

### R20 — Medium — Code reads check paths and sizes before opening them

**Evidence:** Source-confirmed race and availability risk, not a filesystem exploit reproduction. code.rs canonicalizes a pathname, then performs metadata and a later tokio::fs::read. It rejects directories but not FIFOs/devices. The size check is metadata-based; the subsequent read is not bounded.

**Failure:** A concurrently changed path can be replaced with a symlink after validation and before open, weakening the “never read outside root” claim. A FIFO can block a reader, and a growing file can exceed the intended load cap after the check. Registered projects are trusted local workspaces and agents already have full access, which limits the privilege-boundary significance, but the endpoint's stated boundary is still stronger than its implementation.

**Recommendation:** Open the file under the validated root with a beneath/no-follow policy, validate the opened descriptor as a regular file, and perform a bounded read. Retain explicit behavior for legitimate internal symlinks.

### R21 — Medium — Desktop dependency tree contains a known unsound glib release

**Evidence:** Dependency audit and cargo tree confirmed glib 0.18.5 through GTK/WebKitGTK/Tauri. [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) describes unsound VariantStrIter methods for affected releases and a fix in glib 0.20.0 and later. proc-macro-error 1.0.4 also has an [unmaintained advisory](https://rustsec.org/advisories/RUSTSEC-2024-0370.html).

**Failure:** The native desktop stack retains a documented unsafe implementation. This review has not established that Launchpad or a reachable dependency calls the affected methods, so exploitability is unknown. The unmaintained macro dependency is a build-time maintenance issue, not the same severity as a reachable runtime flaw.

**Recommendation:** Track the Tauri/GTK dependency path and an upstream compatible fix, investigate reachability, and record any temporary audit exception explicitly. Do not infer safety from the audit's empty “vulnerabilities” list while ignoring its unsoundness warnings.

**Trust-model observation:** Agent instructions and explicit UI buttons implement the intended publication workflow, but they are not an authorization boundary against an untrusted agent. Harnesses deliberately run with full user access and can read the API token, invoke mutation APIs, or run git/gh directly. That is consistent with the selected single-user full-access model. If “nothing an agent writes reaches GitHub without a click” is meant as a technical guarantee even against prompt injection or a disobedient agent, a distinct capability boundary would be required; the current design only enforces it along the intended CLI/UI path. This is a design limitation, not a request to add multi-user infrastructure.

## GitHub data and review fidelity

### R22 — Medium — Partial GraphQL failures can be saved as complete conversations

**Evidence:** Source-confirmed. github.rs accepts any non-null data and discards errors. items.rs detail pagination treats absent timeline/review-thread connections as empty and stores the result.

**Failure:** A field-level permission failure or transient GraphQL error can silently remove comments/reviews from the rendered conversation and refresh its fetched_at timestamp as if loading succeeded. Preserving partial inaccessible data is useful, but losing the error/completeness signal makes omissions indistinguishable from an empty thread.

**Recommendation:** Return both data and field errors, preserve known cached sections when their refresh fails, and show incomplete/stale section states. Verify field-level partial responses without introducing a permanent HTTP mock test suite.

### R23 — Medium — Client diff identity omits the base commit

**Evidence:** Source-confirmed. FilesChanged.tsx uses head-only query keys and staleTime Infinity for both PR and local diffs. The backend GitHub key includes base and head; pre-draft diffs depend on the refreshed merge base.

**Failure:** Advancing or retargeting a PR's base with an unchanged head can keep displaying a cached old diff. Item Refresh does not explicitly invalidate pr-diff. For pre-drafts, broader predrafts invalidation helps some refresh paths, but the query identity still fails to represent the reviewed commit pair.

**Recommendation:** Include the actual base/merge-base and head in the UI query identity, and invalidate diff queries after explicit item/base refresh. Scope viewed markers to the review definition if the product expects base changes to require rereading.

### R24 — Medium — Deleted-file navigation can open a different base than the displayed three-dot diff

**Evidence:** Source-confirmed. items.rs calculates a three-dot diff of baseRefOid...headRefOid, whose old side is the merge base, then returns baseRefOid as base. FilesChanged's removed-file navigation uses that returned base.

**Failure:** If the upstream base advanced and changed or deleted the same file, View file can show different old content or fail even though the diff has the deleted file's content. The pre-draft implementation correctly returns the merge base; the GitHub path does not.

**Recommendation:** Return and use the actual merge-base commit for the old side of local diffs. Keep the PR's current base separate for metadata. Validate deletion on a diverged branch after an upstream edit.

### R25 — Medium — Cache invalidation does not reliably mean GitHub refresh

**Evidence:** Source-confirmed. ItemView.tsx and RepoItems.tsx decide whether to POST refresh in effects keyed only by fetched_at. Their GET endpoints return cached objects. There is no periodic refresh while a fresh item remains mounted, and global queries disable window-focus revalidation. After a mutation, repo-items is invalidated but the subsequent GET can return the same old fetched_at.

**Failure:** A project list can continue showing an item as open after the owner closes/merges it. A fresh PR detail opened before CI finishes can retain old checks until manual Refresh or navigation that actually triggers the age effect. “Invalidated” is not equivalent to a network refresh in this two-level cache design.

**Recommendation:** Explicitly refresh or patch affected repository lists after mutations. Define when mounted detail/header/check data refreshes, rather than relying on a fetched_at value changing itself. Do not necessarily fetch every complete timeline on a timer; header freshness can remain cheap.

### R26 — Medium — Serial image prefetch delays the first usable detail and has no cache budget

**Evidence:** Source-confirmed scalability risk. items.rs gathers all HTML images and avatars, downloads each sequentially, and detail waits for proxy_images before saving the new item. Each image request has a 15-second timeout, with allowed redirects. The disk cache has no eviction/size budget or per-URL in-flight deduplication.

**Failure:** A long conversation with many inaccessible or slow images can delay usable text for minutes. Concurrent detail loads repeat failed downloads or write the same cache entries. Over time, images and disposable cached payloads accumulate without bounds. The individual image byte cap does not bound total cache size or total refresh duration.

**Recommendation:** Persist/render text first, load images with bounded concurrency and a total request budget, deduplicate in-flight downloads, and add a simple cache cleanup policy. Show image failures independently of conversation availability.

### R27 — Medium — Inbox sync holds the action lock across historical network work

**Evidence:** Source-confirmed. inbox.rs holds its mutex through sync_inner: paginated all=true notifications, serial subscription lookups with a 30-minute TTL, persistence, and summary enrichment. action uses the same lock.

**Failure:** An account with a large historical notification set, slow subscription checks, or an unavailable API can block Done/read/local unread/unsubscribe actions for the whole sync. Even purely local unread has to wait for network work. One subscription request per historical thread every full refresh can also consume a significant REST quota, although the cache reduces ordinary polls.

**Recommendation:** Fetch outside the mutation lock, then reconcile a snapshot with current local overrides in a short critical section. Bound and prioritize subscription work; an unavailable historical thread should not block all triage. Validate on a large cached history and a slow network.

### R28 — Medium — Unsubscribe rollback can misrepresent a successful remote ignore

**Evidence:** Source-confirmed. inbox.rs performs PUT ignored=true followed by DELETE Done. Any error rolls back the local override and membership without preserving the successful first step.

**Failure:** If PUT succeeds but DELETE fails or its response is lost, GitHub is already ignoring the thread while Launchpad restores it locally and reports failure. Retrying blindly obscures the actual state. Suggestions already implement a stronger uncertain-outcome model that this path lacks.

**Recommendation:** Track the individual remote outcome, retain the known ignore, and reconcile an uncertain delete separately. Keep a visible retry/recheck state instead of restoring the original subscription assumption.

## Projects, workspace, and UI

### R29 — Medium — Removing a project orphans local PRs and their worktrees

**Evidence:** Source-confirmed. server/api.rs::delete_project unregisters regular projects without checking dependent pre-drafts or sessions. Pre-draft detail/publication/cleanup need the project row. Temporary deletion closes sessions under the main folder and removes it, without resolving dedicated worktrees stored elsewhere.

**Failure:** A retained local PR can become impossible to open, publish, or discard through the app. Re-registering the same path produces a new id and does not repair the old references. Deleting a temporary Git project can remove the common Git metadata needed by dedicated worktrees outside that folder. Project links/context also become stale.

**Correction to the earlier review:** Ordinary archived session reattachment uses saved cwd in the reverted implementation. Removing a project alone does not establish a session-resume failure if that cwd and native transcript still exist. That worker-specific claim is withdrawn.

**Recommendation:** Refuse removal while dependent local PRs require the project, or retain the minimal durable identity they need. Preflight linked worktrees before temporary deletion. Make the removal action explain the actual dependent records and folders.

### R30 — Medium — Promotion crosses filesystem, transcript, and database steps without recovery

**Evidence:** Source-confirmed conditional failure. projects.rs renames the folder, optionally moves Claude transcripts, updates the project row, then updates session cwds. There is no rollback/checkpoint if a later step fails; transcript move failure is only logged. Git linked-worktree administrative paths and native transcript cwd metadata are not explicitly repaired.

**Failure:** A successful filesystem rename followed by a failed database update leaves stale records pointing at a missing folder. Claude transcript relocation can be incomplete, and sessions imported later may still report the old embedded cwd. A temporary Git repository with linked worktrees needs additional repair before renaming its common directory. The ordinary no-worktree happy path was historically checked; these failure paths were not.

**Recommendation:** Preflight all dependencies and introduce a small recoverable promotion checkpoint, or roll back safe earlier steps. Explicitly reject or handle linked worktrees. Document and surface transcript relocation failures instead of reporting fully successful promotion.

### R31 — Medium — Imported history is a lossy snapshot and worktree attribution is incomplete

**Evidence:** Source-confirmed. history.rs imports image user messages with images=[], stores Usage::default(), and handles only a subset of native item/event types. Project association uses projects::for_path, which recognizes path containment but not linked worktrees outside the main project folder. Claude candidate metadata is sampled from the beginning of the transcript.

**Failure:** Imported image-only instructions can become empty messages, usage/cost and some plans/question/turn details disappear, and imported linked-worktree sessions can lose project association and convenient Files/context navigation. The original native conversation id/cwd are retained and can support resume; this is not a claim that all imported sessions cannot resume.

**Recommendation:** Label imports as partial snapshots, preserve supported attachments/usage/turn boundaries, and resolve linked-worktree attribution through Git identity. Keep unknown native content available for diagnosis rather than silently presenting a complete conversation.

### R32 — Medium — Several potentially long lists violate the WebKitGTK rendering rule

**Evidence:** Source-confirmed. ProjectPage.tsx maps every session and worktree directly. ProjectsView.tsx maps discovered projects, regular projects, and temporary cards without a virtualizer. PredraftView.tsx maps every note. SourcePicker also renders all worktrees; branch results are capped rather than virtualized. Core inbox/session/diff/code lists are virtualized correctly.

**Failure:** A project with years of archived sessions or a large discovery result mounts every row and can regress the very WebKitGTK performance measured in spike 4. Row memoization is also absent in several of these lists. This is a product-rule violation even though small current datasets may feel fine.

**Recommendation:** Virtualize the unbounded lists, memoize their row components, and preserve stable row measurements at every zoom level. Do not add virtualizers to tiny bounded controls without a reason.

### R33 — Medium — Regular-expression find can block the UI thread

**Evidence:** Source-confirmed availability risk. code/lib.ts matcher creates a JavaScript RegExp and findAll executes it synchronously over the loaded file. TextView and search-result highlighting run this during render/memo work. The match-count cap does not cap the time needed for one regex evaluation.

**Failure:** An expensive pattern such as nested repetitions on a long nonmatching line can freeze the browser/webview. The owner has to enter or paste the pattern and enable regex mode; no drive-by exploit was established. A 5 MB file-load cap still permits a very expensive single line.

**Recommendation:** Run regex search in a cancellable worker with a time budget or choose an engine with bounded behavior. Keep invalid/expensive search errors local to the find panel.

### R34 — Medium — The manual installer does not reserve production and can inspect the wrong profile

**Evidence:** Source-confirmed. packaging/install.sh:38 probes production health, then runs the installed launchpad status without forcing LAUNCHPAD_PROFILE=prod. It checks again before copying/restarting but does not acquire the update-install-lock reservation. The UI updater does explicitly reserve production.

**Failure:** Running the installer from a shell with LAUNCHPAD_PROFILE=dev can query dev sessions while deciding whether restarting prod is safe. Even in prod, a new session can start between the final status check and replacement. This violates the script's stated idle installation policy; after this revert, restarting production also stops daemon-owned agents, making the missed reservation especially consequential.

**Recommendation:** Force the production profile for all production inspection and use the same reservation mechanism in both installation paths. Preserve the explicit --force escape hatch and release a reservation on failure.

### R35 — Medium — A terminated updater can leave the UI permanently “Updating”

**Evidence:** Source-confirmed. packaging/update.sh:24–27 uses EXIT only for reservation cleanup; TERM/INT exit without writing failed/cancelled status. API/UI update state trusts the persisted update-status file, and useAppUpdate treats state=running as busy.

**Failure:** Stopping the updater service, a reboot, or an abrupt failure after status=running can leave a stale running file. The Update button remains disabled even though no update is running. SIGKILL cannot be handled by a trap, so status reconciliation must not depend on traps alone.

**Recommendation:** Reconcile stored progress with the transient service/process state at status read or daemon startup. Write a terminal state on handled cancellation and expose retry after interrupted work.

### R36 — Medium — Launch defaults represent current session status, not last successful launch

**Evidence:** Source-confirmed. stores/composer.ts infer success from last_seq>0 and status!='error'. The launch path emits user/status events before harness message acceptance.

**Failure:** A successful launch that later reaches Error is excluded on reload, so defaults revert to an older session. Conversely, a launch can produce an event and appear eligible before delivery/launch success is established. The behavior can differ before and after restarting the browser. This conflicts with the explicit “latest successfully launched spec” rule.

**Recommendation:** Persist a small last-successful-launch record or an explicit launch-success fact at the actual acceptance boundary. Do not reconstruct it from mutable current status. Imported history should continue to leave defaults unchanged.

### R37 — Medium — Composer draft persistence silently loses recent or failed saves

**Evidence:** Source-confirmed. stores/composer.ts debounces server saves for 400 ms and catches errors without surfacing them. There is no durable browser fallback or flush-on-close acknowledgement. restore sets loaded in finally even when a read fails.

**Failure:** Closing/reloading soon after editing, or typing while the daemon is disconnected, can lose the unsent prompt or delay configuration. The UI gives no indication that the supposedly preserved draft was not saved. Session-message and quick-reply drafts are memory-only, which preserves navigation but not reload; that narrower promise should be explicit.

**Recommendation:** Save a local fallback immediately, reconcile it with the daemon draft, and show save/disconnected status when durability is uncertain. Avoid relying on a shutdown network request alone. Keep the current minimal persistence model rather than adding a general migration framework.

### R38 — Medium — Adding a revision selection to an existing agent can identify the wrong source

**Evidence:** Source-confirmed. CodeView.tsx constructs existing-session messages as path:lines (at rev) for revision views, dropping root. For new agents it uses FileRef, which retains root/rev. Existing sessions of the same project can be in different worktrees.

**Failure:** Selecting a file from a different worktree or a worktree-specific revision can send an ambiguous or wrong reference to a session. Symbolic revisions can also move after selection. The agent can read the wrong checkout or assume the selected lines exist in its current working tree.

**Recommendation:** Include the source root and resolved commit for revision references to existing sessions, or use the same structured FileRef rendering path as the composer. Preserve absolute working-tree references when cwds differ.

### R39 — Low — Hand-edited config bypasses validation applied by Settings

**Evidence:** Source-confirmed. ConfigHandle::save rejects zoom>4, but config.rs read only deserializes. Terminal ids, custom argv, preset ids and polling values also lack a common validation pass. zoomScale clamps display scaling, but Settings directly indexes the zoom list.

**Failure:** A syntactically valid manual config can be applied even when equivalent UI input would be rejected. For example zoom=255 yields an undefined displayed percentage while the shell clamps to 150%. Unknown/empty terminal settings fail only when handoff is attempted.

**Recommendation:** Apply one lightweight validation function on both file reload and API save, retaining the previous valid configuration on error.

### R40 — Low — Settings holds an old draft after external configuration changes

**Evidence:** Source-confirmed. SettingsView.tsx initializes draft only when draft is null. A config-file watch event updates q.data but does not reset an untouched existing draft.

**Failure:** The screen can show outdated settings as if they were edits, and Save can overwrite a newer manual configuration with an old copy. Multiple browsers have the same lost-update possibility.

**Recommendation:** Refresh an untouched draft when the saved config changes; if edits exist, show the change and offer Revert/reload instead of silently treating the stale draft as authoritative.

### R41 — Low — Project fields violate the click-only interaction rule

**Evidence:** Source-confirmed. ProjectsView.tsx Description handles Enter/Escape and saves on blur without a visible Save/Cancel action. Add folder also submits on Enter. These are outside the allowed composer-mention/session-message/code-find/terminal exceptions.

**Failure:** The interaction model diverges from AGENTS.md, and description editing has no explicit visible commit/cancel control. Keyboard behavior was not added to the central exceptions.

**Recommendation:** Provide visible Save/Cancel for descriptions and use the existing Add button for registration. Change the documented exceptions only if the owner deliberately chooses that product direction.

### R42 — Low — Model refresh does not invalidate the composer's model query

**Evidence:** Source-confirmed. harness/mod.rs updates ModelCache at startup and every 30 minutes without publishing a model/defaults event. useHarnessDefaults has a stale time but no timer; the composer is mounted even when closed and global focus refresh is disabled.

**Failure:** If the defaults query runs before the two-second model refresh completes, or models change while the app stays mounted, the picker can retain an empty or old list until a reconnect or explicit query-triggering change.

**Recommendation:** Broadcast model/default changes and invalidate their query, or refresh when the composer opens. Keep launch itself independent of model discovery.

### R43 — Low — A healthy daemon with an unreadable token leaves the desktop on its startup page

**Evidence:** Source-confirmed. desktop/src-tauri/src/main.rs falls back to packaged index.html when app_url fails, but the recovery/error thread runs only when warm is false.

**Failure:** When health succeeds but the token file is missing/unreadable, the shell can display “starting” indefinitely without a retry or useful error. The same issue applies to a token path/profile mismatch.

**Recommendation:** Handle URL/token construction failure explicitly regardless of the health result, and show the actionable error or retry after reading the correct token.

### R44 — Low — Highlight worker failure leaves future requests unresolved

**Evidence:** Source-confirmed. code/highlight.ts onerror resolves current requests and clears pending but keeps the failed worker reference. Subsequent highlight calls post to the same failed worker and add new pending promises.

**Failure:** New files remain plain text and unresolved promises accumulate. The comment promises “plain text from now on,” but no disabled state actually implements that promise.

**Recommendation:** Set an explicit failed/disabled state that immediately returns null for subsequent requests, or safely recreate the worker once with a controlled retry. File text should remain usable throughout.

### R45 — Medium — Unbounded reads and Git/network waits undermine the fast, always-usable interface

**Evidence:** Source-confirmed scalability/availability risk. sessions::events returns all remaining events in one array; sessionStore retains every opened log indefinitely and rebuilds/clones growing event structures. procs::output awaits commands without a shared timeout or output cap. Git fetch/push and several pre-draft operations hold the global pre-draft lock while waiting. server/api.rs task_output loads the complete event list each poll to locate an output filename.

**Failure:** Long histories increase browser memory, database/JSON work, and per-event rendering preparation despite DOM virtualization. A stuck SSH credential/network operation can block unrelated local PR edits and suggestion publication. Repeated large background-output polling re-reads historical events. These are growth/slow-path risks rather than demonstrated current-machine performance failures.

**Recommendation:** Page durable events or lazily retain bounded UI windows, release unopened logs, store task output references directly, and set explicit command deadlines/output budgets. Keep long external work outside broad locks where correctness permits. Recheck performance in WebKitGTK under CPU load, not only Chrome.

## Additional findings from the reassessment

### R46 — High — Archive and terminal handoff return before execution is quiescent

**Evidence:** Source-confirmed. sessions.rs::detach removes the live adapter first, calls stop and only logs its error; close then records Closed. ClaudeSession::stop closes stdin, schedules a SIGTERM five seconds later and returns immediately. The stdout loop continues dispatching status/events. Sink instance identity guards process_exited, but not normal emissions or status updates. CodexSession::stop ignores interrupt/unsubscribe failures and removes the routing entry while the shared app-server remains alive.

**Failure:** “Archived/stopped” does not prove that commands have ended. A late Claude result/system event can mutate the archived session back to Waiting/Working, or reach a newly attached instance's record. Codex work that survives interruption can lose its completion route entirely. Immediate Resume or Open in terminal can overlap the previous owner. Cleanup can remove a directory while its old command is still executing. This is independent of R01's incorrect turn-completion status.

**Recommendation:** Define stop as acknowledged quiescence with a bounded termination path; retain ownership until it is reached and propagate failures. Guard every adapter-originated event/mutation with instance identity. Handoff must wait for release before starting a native client, and repeated terminal-open requests must reject an existing owner. Avoid delayed signals against an unverified reused numeric PID.

### R47 — High — Temporary deletion and promotion do not reserve all execution owners

**Evidence:** Source-confirmed race. server/api.rs::delete_project and promote do not hold sessions::allow_work across checking, closing, rename/removal and record updates. close_in snapshots a list and skips InTerminal sessions; running_in checks live/is_running but omits InTerminal. Native clients are not part of these reservations.

**Failure:** A new launch/send can enter a temporary folder after the cleanup snapshot or the promotion idle check. Deletion then removes its files, or promotion changes the cwd while it is executing. An external TUI can already be running despite the “idle” check and is neither stopped nor blocked. R46 also means closing the snapshot's sessions does not establish they have exited.

**Recommendation:** Reserve the canonical project/worktree and its descendants across destructive transitions, block new work, resolve native owners, and wait for verified quiescence. Refuse the operation with a concrete dependency/owner explanation if it cannot safely complete. Keep filesystem/database recovery separate from this concurrency fix (R30).

### R48 — Medium — Persisted questions have no reconciliation when their owner is replaced

**Evidence:** Source-confirmed recovery gap. sessions.rs::load preserves open_question even though adapter question maps exist only in memory. reattach creates a new adapter without clearing/recovering the old request identity. close clears the session flag but emits no normalized cancellation of its AskUser event. AskCard enables historical unanswered options based on session live, not whether that card is the current open request; the message box selects the last unanswered AskUser when open_question is true.

**Failure:** After restart/reattach, a saved question can become actionable before any valid request is reissued by the harness, while the new map has no matching id. Answers are rejected as already answered/unknown. Previously abandoned question cards become clickable again after Resume. A newly reissued question may repair the current prompt, but there is no explicit recovery/cancelled state for the old card.

**Recommendation:** Reconcile pending requests during owner recovery. Either recover a valid native request identity or explicitly invalidate the old request and render it non-actionable. Gate answers on the current request and owner instance, rather than session live alone. Check archive/resume, daemon restart and terminal reattachment while a question is open.

### R49 — Medium — A definitively failed first push has no local abort/recovery action

**Evidence:** Source-confirmed design limitation, consistent with architecture decision 40. predrafts.rs::publish persists remote_branch and pushed_head before its first push. Any push error leaves that checkpoint. Subsequent turns and discard are blocked; retry must keep the same remote name and reviewed head. The UI/API offer no verified abort/unfreeze operation.

**Failure:** A pre-draft can remain frozen after authentication rejection, a reserved/invalid-for-policy branch, or another definite failure before a remote ref is created. The owner cannot amend work, select a different name, or discard locally through the normal flow unless the exact publication is eventually made to succeed. The UI message says the branch was pushed even though the checkpoint represents an attempted push.

**Recommendation:** Preserve conservative handling of uncertain outcomes, but provide an explicit recovery action that inspects remote refs/PRs first and can safely abort when absence is established. Distinguish attempted from confirmed push in text. Do not automatically clear a checkpoint after a timeout or delete a potentially published branch.

### R50 — Medium — Ordinary GitHub comment and review failures do not preserve uncertain outcomes

**Evidence:** Source-confirmed conditional risk. items.rs::mutate and inbox.rs::reply make direct remote mutations without the durable posting claim/reconciliation used by suggestions.rs. Their UI retains input and reports failure when a response is lost; there is no dedicated uncertain state. These requests are not automatically retried by the fetch wrapper.

**Failure:** A successful GitHub comment/review followed by a lost acknowledgement can look like a failed submission. An owner retry can publish duplicate text or another review. This requires an ambiguous transport failure and explicit retry; no duplicate was posted in this review.

**Recommendation:** Extend the lightweight posting/uncertain outcome pattern to these normal publication paths, or show a clear check-GitHub-before-retrying action with reconciliation. Keep drafts recoverable without equating request failure to remote non-publication.

### R51 — High — A duplicate daemon mutates state before discovering its port is occupied

**Evidence:** Reproduced in the isolated namespace. cli/main.rs calls App::start before server::serve binds the listening socket. App::start loads/interruption-writes sessions, recovers schedules and spawns monitors/scheduling before bind. A second process returned exit 1 with the occupied-port error, but had already changed a seeded Working session row to Interrupted.

**Failure:** Accidentally launching another daemon for the same profile can overwrite live durable session state and interfere with claimed launches before it fails. A due schedule can also race the late bind failure. The original daemon has independent in-memory records and may continue executing, so persisted state and ownership diverge. This is not the removed multi-worker cache problem.

**Recommendation:** Acquire single-instance ownership before opening mutable state or starting recovery/background services. Bind the profile's listener or take a profile lock first and retain it for the daemon lifetime. A losing startup must exit without state writes, recovery, updates, polls or launches.

### R52 — Medium — The displayed item head and the returned diff are not bound together

**Evidence:** Source-confirmed race. FilesChanged.tsx keys a query with its displayed head but sends no expected head to the GitHub /diff endpoint. items.rs::diff independently obtains detail (which can refresh stale data) and returns its own base/head. The component renders that response without comparing it with the head under which it was requested.

**Failure:** If the PR changes between header display and diff loading, the owner can read a new head's patch under an old header/review commit. Storing the new patch under an old head key compounds the inconsistency. Merge's SHA check and suggestion publication's head check are useful protections, but they do not make the viewed patch match its claimed identity. R23 separately concerns an unchanged head with a changing base.

**Recommendation:** Require/return the expected commit pair, verify it in the UI and refresh the header or refuse the stale request on mismatch. Couple reviewed/viewed markers and subsequent actions to the actual returned diff identity. Reset out-of-range file selection when a refresh shrinks the file list.

### R53 — Medium — A failed initial conversation fetch leaves permanent Loading without retry

**Evidence:** Source-confirmed. SessionPane.tsx initializes a log entry and calls load only when no log exists. sessionStore.ts::load clears its in-flight flag in finally, but records no error or retry state when the API fails. The existing log remains loaded=false, and the component effect no longer initiates another fetch.

**Failure:** A transient error opening a session can leave its conversation on Loading even after the daemon recovers, until another reconnect or reload triggers catch-up. An unhandled promise rejection is logged, but the owner sees no actionable error/retry button. Closing/reopening the pane alone retains the same failed log entry.

**Recommendation:** Store load errors and render a visible Retry action; allow another fetch for failed/unloaded logs and preserve existing content during catch-up. Keep this recovery distinct from sequence buffering (R10).

### R54 — Low — Conversation rendering falls short of the documented Copy and Markdown behavior

**Evidence:** Source-confirmed. EventRow.tsx provides whole-message Copy for assistant messages, but user and reasoning rows do not have the promised per-message Copy action. components/Markdown.tsx uses markdown-it's default fenced-code renderer with Copy buttons, without Shiki tokenization, and reparses the entire text when it changes. Architecture 9.6 promises Shiki code blocks and memoized finished streaming blocks.

**Failure:** The owner cannot use the same visible Copy action across message types; conversation code blocks lack the promised syntax colors, and long streaming Markdown rebuilds finished blocks. Code/diff highlighting does exist in separate components, so this is a conversation-renderer gap, not absence of Shiki throughout the application.

**Recommendation:** Implement the specific missing controls/rendering or narrow the documented promise. Preserve exact raw Markdown copying and separate plain-text copying. Do not introduce a new UI direction merely to satisfy an obsolete performance sketch.

### R55 — Medium — Saving untouched labels can overwrite newer GitHub labels

**Evidence:** Source-confirmed. ItemView.tsx::Controls initializes labels from item.labels once with useState. A detail refresh changes the displayed label chips but does not update that field. items.rs::mutate uses PUT labels, replacing the entire set.

**Failure:** Labels changed externally or by another window can appear correctly in the refreshed chips while the input still contains the older set. Clicking Save labels then removes the newer labels without the owner having edited them. A retained control instance across item navigation can also carry old field state.

**Recommendation:** Refresh an untouched input when item identity/labels change; preserve edits with an explicit conflict/reload choice. Disable a no-op save and reconcile the current set before replacing it. Test freshness with read-only cached fixtures rather than changing real labels during automated checks.

### R56 — Medium — Nullable GitHub authors and reviewer data can crash item views

**Evidence:** Source-confirmed schema mismatch. GitHub's official [PullRequestReview schema](https://docs.github.com/en/graphql/reference/pulls#pullrequestreview) permits a nullable author; [ReviewRequest](https://docs.github.com/en/graphql/reference/pulls#reviewrequest) permits nullable reviewer data. The handwritten web types model review authors/requestedReviewer as required objects, and Controls reads r.author.login / r.requestedReviewer.login directly. RepoItems also dereferences i.author.login while [issue](https://docs.github.com/en/graphql/reference/issues#issue) and PR authors can be absent. The normal comment renderer already uses optional author handling.

**Failure:** A valid null author/reviewer in an otherwise successful response throws during render, potentially hiding the whole PR/list behind the route error state. Incomplete union fragments for reviewer kinds beyond User/Team can also render blank names. No real GitHub data was modified or deleted to manufacture this case.

**Recommendation:** Represent GraphQL nullability and selected unions accurately, handle missing nodes/actors with a stable fallback, and keep one unavailable actor from hiding the item. Type checking handwritten shapes is not runtime/schema validation.

### R57 — Low — Notification suppression is global rather than per connected window

**Evidence:** Source-confirmed. sessions.rs stores one watched tuple for all WebSocket clients; each watch message overwrites it. server/ws.rs clears it on any client's disconnect. The suppression record expires after ten minutes; the UI sends on focus/visible-session changes, not a periodic presence heartbeat.

**Failure:** A second browser can replace a focused window's watched set, or disconnect and clear it, causing notifications while the owner is already watching. A focused unchanged view can also age out. Conversely, the last focused report can suppress a notification even when another window is currently relevant. This concerns notification accuracy, not agent execution state.

**Recommendation:** Keep a small per-connection presence map, aggregate focused watched sessions, remove only the disconnecting client and refresh presence periodically or use connection liveness.

## Documentation discrepancies and missing promised pieces

| Document or promise | Current assessment | Needed decision or implementation |
|---|---|---|
| Phase 7 status/commands/ownership | Corrected in this revert: candidate list, no workers/PTY/remote CLI, daemon-owned harnesses and restart interruption. README, AGENTS, roadmap, architecture and sketches agree again. | Scope one candidate before adding its commands and guarantees back. The first three ideas are not an approved batch. |
| Opening roadmap/architecture status | Corrected here: implementation/acceptance tracker, Phases 0–6 represented and Phase 7 deferred. | Continue recording owner acceptance separately from “built”. |
| Original three presets | Default worktree/no-unit-test are present; draft-pr intentionally replaced by the local pre-draft target and linked skill in architecture 10.3. Earlier Phase 1 text still says three presets. | Update the old enumerations to the established decision; do not restore instructions for an agent to publish to GitHub. |
| Temporary-project optional git init | Architecture describes an optional config setting, but Config/create_temp do not implement one. | Mark it deferred or scope the small setting if still wanted. Plain temporary folders are otherwise deliberate. |
| Codex schema-diff tool | Architecture 9.4/risk mitigation promise a small manual schema regeneration/diff script; none was found. Automatic harness updating exists. | Add the promised manual tool or stop presenting it as existing drift protection; record compatible protocol versions/inspection steps. |
| Conversation rendering | Per-message Copy, Shiki fences and incremental finished-block rendering are incomplete (R54). Some expanded tool/background output uses explicit dark colors in an otherwise light UI. | Implement or narrow the guarantees; decide whether those output surfaces comply with the “no dark-mode styles” rule. They do not constitute a complete dark theme. |
| Exact launch preview | preview renders user/context text, but launch adds local PR and suggestion instructions and resolves a new worktree/temp cwd afterward. | Explain which generated instructions/context are previewed, or preview the full final prompt after its source is known. Do not describe partial preview as exact sent bytes. |
| Frontend “keep list views mounted” | Router unmounts views; stores preserve some state, but project tabs/filter/selection and several local controls reset on navigation. | Describe the actual store approach or preserve the states the product expects. |
| Reusable code-line renderer | CodeLines and FilesChanged have separate line/token renderers and language mapping, although highlighting infrastructure is shared. | Correct the one-renderer claim or consolidate only if useful to avoid drift. |
| All /api/code requests through open | sources validates a registered project and discovers worktrees without code::open; content endpoints do call open. | Clarify this metadata exception. No arbitrary-content bypass was found through sources. |
| Cached images / never loading | Uncached detail/diff routes load, and serial prefetch can delay text; failed proxy URLs can fall back to the original remote image. | Treat “never loading” as a cached-path target and define a usable cold/error path (R26). Remote image fallbacks are not an offline cache guarantee. |
| Find button in the bar | Toolbar Find plus previous/next exist; the bar has no button named Find as AGENTS describes. | Clarify the rule or provide the stated visible action; keep the chosen keyboard exceptions narrow. |
| Desktop notification examples | Session notifications are implemented in the daemon. Architecture's GitHub notification examples do not correspond to an inbox desktop-notification implementation. | Label examples as future ideas or select the actual triggers deliberately. |
| Raw log replay mitigation | Raw lines are capped at 20,000 per session; no user-facing replay/renormalization tool exists. | State diagnostic limits and actual manual recovery. Raw retention is not complete reconstructibility. |
| Publication/idle/security guarantees | Full-access agents can bypass intended publication paths; reservations and pathname validation have R04/R20/R46/R47. | Qualify guarantees to the trust model and implement the invariants the app itself relies on. |
| Packaging versus CI | CI uses pinned Rust 1.93.1, --locked, Node 24/Corepack; installer/updater use ambient cargo without --locked or a repository toolchain file. | Align repeatability guarantees and force/reserve the production profile (R34). No current compile failure was observed. |
| Sketches | S1–S7 remain conceptual. Phase 7 annotations were reverted with the implementation; error/recovery states are not comprehensively specified. | Change/republish the sketch only when a selected UI correction changes direction and the Artifact tool is available. |

## Intentional limits, unresolved decisions, and untested boundaries

- **Inbox scope:** Durable local membership is intentional; REST all=true includes historical Done threads and cannot reconstruct exact web-inbox membership. Subscription 404 is correctly not treated as unsubscribe evidence. No exact parity requirement should be added by inference.
- **List and metadata completeness:** Project label filters cover the cached 30 most recently updated open items, visibly scoped. Timeline/review-thread pagination exists; nested checks, labels, participants/reviewers and selected timeline types remain bounded. Preserve and show completeness markers where those limits matter, including GitHub's patch/file limits.
- **Filesystem/source caps:** Non-Git walking caps at 200,000 files without an explicit truncation flag; ref menus also cap entries. File colors stop above 1 MB/20,000 lines, plain viewing above 5 MB, with a separate image cap. Read-only editing scope is intentional. Add completeness flags where hidden files/refs could mislead the owner.
- **Git configuration:** Common github.com origin/upstream layouts are supported. Enterprise hosts, include/URL alias complexity, multiple identities and effective push destinations are not generally covered. Publication must still fail safely on ambiguous destinations (R06); absence of broad Git configuration support is not permission to push to the wrong repository.
- **Dependency semantics:** Scheduler readiness checks all dependencies at one observation, as the current architecture says. Claim/run are later asynchronous steps, so a predecessor can be restarted while a claimed launch waits. Decide whether “after Done” means a completed observation or continued idleness at actual start; if the latter, recheck under the start reservation. This is not asserted as a violation of the current literal “same check” wording.
- **Native ownership:** /proc cmdline matching detects common local resume invocations, not every native/editor/remote client. It does not validate executable identity. Native conversation locks provide additional protection; these checks are not a universal ownership protocol. Historical native messages are intentionally not merged into the structured app log.
- **History:** Imported sessions are explicit archived snapshots, not live attachments. They retain native ids/cwd but omit some attachment/usage/event detail (R31). A daemon restart interrupts headless work after this revert; claimed launches are not automatically retried, correctly avoiding duplicate execution.
- **Process recovery:** The child-process helper sets a parent-death signal but ignores prctl's return and does not recheck parent identity after installing it. A parent-death race is a conditional hardening follow-up; no escaped child was reproduced. Delayed numeric PID signaling, ownership replacement and real descendant cleanup need the R46 acceptance check.
- **Security defenses:** Agent Markdown disables raw HTML; GitHub HTML uses DOMPurify; remote image redirects/hosts/MIME/bytes are validated without GitHub credentials; local raw images have sandbox/nosniff. No concrete sanitizer bypass was found. Main UI/Tauri have no restrictive CSP, so a compatible policy remains defense in depth, not proof of an existing XSS.
- **Agent trust:** Harnesses deliberately run with full user access. They can read tokens and run API/git/gh mutations directly. “Owner publishes with a click” holds for intended Launchpad preparation paths, not against an untrusted or prompt-injected agent. A stricter guarantee would need an explicitly chosen capability boundary, not wording alone.
- **Durable state:** state.db holds valuable history/drafts/notes. Cache deletion/rebuild is intentional; user-state backup/export/restore has no selected UI workflow. Decide a simple recovery practice before accumulating irreplaceable data. WAL synchronous=NORMAL is reasonable for speed but does not promise preservation of every recent acknowledged transaction after power loss.
- **Distribution:** Hosted Actions runs, release embedding/archive permissions, systemd install/update effects, native Tauri launch, notification clicks and external terminals were not exercised here. No production update was triggered. The intentionally untracked review means the checkout remains dirty; the updater's clean-main check will appropriately wait until that artifact is moved or otherwise handled by the owner.
- **Scope exclusions:** Other operating systems, multi-user ACLs, dark theme, generic migration/versioning framework, GitHub search/Actions logs and a large mock test suite are not required. Phase 7's unselected features are not missing selected work.

## What is working well

Durable user state and disposable GitHub cache are separated appropriately. Inbox membership survives cache rebuilds and preserves owner overrides. Localhost binding, Host/Origin checks, random token authentication, HttpOnly/SameSite cookies and per-profile names form a useful API baseline. Code revisions are resolved to commit ids before Git operations, and literal pathspec/pattern handling avoids ordinary option injection. The isolated checks confirmed basic root and traversal rejection.

The intended publication paths require visible actions. Suggested reviews carry an inspected head; merge sends the reviewed SHA; suggestions persist posting claims before remote mutation and refuse automatic retries after uncertainty. Local suggestion-to-note conversion is atomic. Pre-drafts record their chosen branch/head and persist the published URL before optional cleanup. Retain those protections while repairing the destination, reservation and recovery gaps.

Core session/inbox/conversation/code/diff lists use virtualizers with useFlushSync=false. Shiki runs in an ES-module worker and produced real colors at every zoom checked. Closing a panel is separate from Archive; the main list/count paths share Done/Running predicates. Schedules have stable ids, durable claims, cancellation and conservative crash recovery. The generated types match Rust and all existing small tests/strict lint checks pass. Local pre-draft notes/discard and regular-folder preservation also passed.

## What went wrong

**Scope:** Phase 7 was a menu of possible later work, yet one commit implemented three extensions together before their individual need, ownership/failure contracts and acceptance were settled. Worker survival, full harness TUI and remote browser control each create different boundaries. Combining them made it harder to establish what “done” meant and increased the amount that had to be reverted. The owner explicitly identified the phase as poorly scoped; the code/docs confirm the breadth, but do not establish anyone's intent or that every happy-path check was absent.

**Acceptance sequencing:** Many progress sections say “built” while their week/morning/two-week routine checks remain pending. More features were added before those acceptance loops exposed lifecycle and usability weaknesses. A build validates integration and types; it does not prove ownership, stopped commands, remote outcome, complete review data, or daily-use speed.

**Cross-feature invariants:** Several older decisions depend on status and cwd strings instead of the facts those strings represent. Waiting is treated as stopped execution; an exact cwd as worktree identity; a head-only cache as a complete diff identity. Those approximations are useful for display and insufficient for deletion, dependencies and publication. Removing Phase 7 removes new worker failure paths but does not repair these original invariants.

**Failure recovery:** Message/question state changes precede acknowledgement; stop precedes exit; rename spans filesystem/transcript/database steps; GraphQL errors disappear; direct publication lacks an uncertain outcome; the updater trusts stale progress. These are incomplete state transitions, not a need for a generalized distributed system or extensive mocks. Small, explicit checkpoints and a few local reproductions fit the project better.

**Documentation:** Detailed progress was added while some earlier guarantees and enumerations stayed stale. The previous review also generalized worker-specific defects into unaffected paths and missed real launch/startup bypasses. This revision withdraws those claims, identifies deliberate scope limits, and ties remaining findings to the current implementation and evidence.

## Recommended repair order

1. Establish real execution ownership and ordered durability: R01, R02, R46 and R51. Include old-instance callbacks and descendant/background work. A correct Done predicate depends on those facts.
2. Apply canonical worktree reservations across every work entry, cleanup and promotion path: R04 and R47, then dependency-aware removal/recoverable promotion R29/R30. Keep local publication frozen until outcomes are resolved.
3. Correct effective publication destination and atomic ref creation/recovery: R06/R07/R49. Preserve the reviewed head and explicit owner action; never blindly retry uncertainty.
4. Make displayed review identity and rendering reliable: R08/R09/R10/R23/R24/R52, nullable API data R22/R56, and stale label input R55.
5. Repair delivery/question/loading/retry behavior: R11–R14/R48/R50/R53; then draft/default persistence R36/R37 and context fidelity R31/R38.
6. Fix concrete security/availability boundaries R17–R21/R33, followed by image/inbox/growth R25–R28/R32/R45, installer/updater R34/R35 and small UI/documentation gaps. Perform owner acceptance before selecting a new Phase 7 item.

This is a repair recommendation, not authorization to publish real GitHub content, restart production, or implement every optional feature. No fixes beyond the requested revert/documentation were mixed into this review.

## Focused validation still required

| Scenario after fixes | Evidence needed |
|---|---|
| Codex interruption during a silent long command | Running/Background persists until real completion; dependency remains queued; final output/result is visible. |
| Concurrent event/status/usage plus persistence failure/crash | Ordered durable sequence and snapshots; atomic cursor recovery; failed writes are visible and never presented as durable. |
| Archive, immediate Resume, native terminal handoff, late old callback | Old execution ends before replacement/cleanup; no second owner or resurrected Closed session; instance filtering covers every callback. |
| Duplicate daemon with active/due work | Losing startup performs no state/recovery/account/harness work before rejecting its instance conflict. |
| Frozen root registered normally, subdirectory/imported cwd, pending question | All work entry paths obey the same worktree freeze; no cleanup/publication while any relevant owner runs. |
| origin pushurl/rewrites; branch created after check; definite/uncertain failure | Destination is verified; create-only ref update is atomic; recovery preserves uncertainty and permits a verified local abort. |
| Diff header-like hunk, space/control paths, rename, binary, deletion | Exact identity, status, patch and full-file navigation; no content interpreted as metadata. |
| PR head changes mid-request; base advances with same head | Header/diff/action use the same committed pair; viewed state and old-side navigation match it. |
| Final event during catch-up; initial fetch fails | No dropped final event; visible Retry recovers without an unrelated reconnect. |
| Answer/write timeout, owner replacement, mixed question choices | Recoverable requests; no premature submission or false acknowledgement; old cards are disabled; no blind duplicate work. |
| Partial GraphQL, null authors, current labels after refresh | Known data preserved, incompleteness shown, safe actor fallbacks and no stale set replacement. |
| Slow images/large inbox/history under CPU load | Text and local triage remain usable; bounded retention/work; actual WebKitGTK lists/zoom stay smooth. |
| Remove/promote temp project with linked worktrees and external TUI | Complete dependency-aware reservation plus recoverable filesystem/transcript/record changes, or clear refusal. |
| Fast draft edit/close, disconnected save, external config edit | Saved fallback or visible unsaved state; common config validation; conflicts do not silently overwrite new data. |
| Ordinary comment/review loses acknowledgement | Durable uncertain outcome/recheck before owner retry; no duplicate publication. Use an intended owner action for real acceptance. |
| Updater terminated/rebooted, manual installer from dev shell | Retryable terminal status, actual prod reservation, no active production work interrupted accidentally. |
| Owner acceptance runs | Week of agents; morning of inbox triage; week of code reading; normal PR review/merge; two weeks of local PRs; four-agent workspace; edited suggestion posting. No Phase 7 acceptance is due while unselected. |
| Hosted CI/release/native smoke check | Ubuntu jobs complete, embedded UI loads, executable permissions survive packaging, native startup/notifications/terminal behave as documented. |

**Disposition:** retain Phases 1–6 as built but awaiting acceptance; Phase 7 stays deferred. Address execution, durable-state and publication safeguards first. This report remains uncommitted.

## Addendum — independent second review (2026-10-03)

A second review was made without reading this report, then compared with it. Its full results are in `docs/CODE_REVIEW_2026-10-03_independent.md` (also uncommitted), which merges the findings of both reviews by area and maps every R finding above to its own IDs (section 14 of that file). This addendum only lists what that review found and this report does not contain. IDs refer to the other document, which gives locations, scenarios and fixes.

Note: this file and the independent review are untracked, and `packaging/update.sh` treats any untracked file as uncommitted work. The production updater (`launchpad-update.service`) has been waiting on "Waiting for committed main" since 06:22 because of this file (OPS-1).

### High

| ID | Finding |
|---|---|
| OPS-1 | `update.sh:32` waits forever while `git status --porcelain` lists untracked files, with no visible reason; combined with R35 the update can stay "running" indefinitely. Use `--untracked-files=no`, write a final status in traps, add Cancel and a timeout. |
| OPS-2 | Terminals, browsers, editors and `launchpad-desktop` windows started by the daemon (`terminal.rs`, `/api/open`, `notify.rs`) stay in `launchpad.service`'s cgroup (`KillMode=control-group`; `setsid` does not leave it) and are killed by every restart or update. `prepare_install` ignores `in_terminal` sessions, so an update kills a running `claude --resume` TUI. |
| GH-1 | `inbox.rs:235`: `subscription(&t.id).await?` aborts the whole sync before anything is saved; one thread that always fails (403 for a SAML org, lost access) stops the inbox from ever updating. |
| PD-1 | `predrafts.rs:561-570`: after a successful push, `git branch --set-upstream-to=origin/<name>` fails in single-branch or shallow clones (no remote-tracking ref is created). `pushed_head` is already saved, so retry fails at the same step, Discard is refused and turns stay frozen. Reproduced with git 2.34.1. |

### Medium

| ID | Finding |
|---|---|
| SEC-1 | No CSP or `frame-ancestors` on the UI document; `localhost:*` pages are same-site and receive the cookie, so they can frame the signed-in UI. A strict policy is easy because `index.html` has no inline script. |
| SEC-2 | The dev daemon accepts `Origin: http://localhost:5173` and `127.0.0.1:5173` (`auth.rs:29-33`): any other project's Vite dev server can read `/ws` events and send body-less POSTs to the dev daemon, which uses the real GitHub account. |
| SES-4 | `claude.rs:406-414`: an empty `background_tasks_changed` sets Waiting about 90 ms before Claude's self-started turn: a spurious notification per task, and a dependent launch can start early. |
| SES-7 | `codex.rs:191-203`: invalid replies to Codex server requests (`decline` instead of `denied`/`abort` for legacy approvals; `{}` for permissions, MCP elicitation and dynamic tool calls). |
| SES-8 | The global install guard is held across `git fetch`, `worktree add`, `/user`, push and 60 s Codex RPCs; a hung app-server or SSH connection freezes input to every session and the scheduler. |
| SCH-1 | The schedule claim is a SELECT then an UPDATE without a transaction (`schedules.rs:163`); with R51, two processes can start the same launch. |
| PRJ-1 | The dev profile shares `~/projects` with prod: dev checks left `tmp-1002-reply-ready-only-do`, `-2`, `-3` and `tmp-1002-sleep-30minutes-before-starting` in the owner's projects root. |
| GH-2 | A sync failure after page 1 changed stores the new `Last-Modified` anyway; the next poll gets 304 and new notifications wait up to 30 minutes. |
| GH-3 | Detail prefetch for the top 15 unread threads runs after every poll, not "when the inbox loads": about 18% of the GraphQL budget with an idle inbox, plus image downloads. |
| GH-4 | The inbox Refresh button obeys the 60 s poll window that every poll resets, so it almost never fetches. |
| GH-7 | `github.rs:184-187` never reads error bodies: the UI shows "422 Unprocessable Entity" instead of GitHub's reason; draft/ready drops GraphQL errors. |
| ITM-1 | `diff()` calls `detail()` with a network check before reading the diff cache: Files changed fails offline and costs a request per open. |
| ITM-3 | `items.rs:434-440`: "missing mapped remote" is an error outside the fallback block, and the remote match is case-sensitive while the project match is not, so no API fallback happens. |
| ITM-4 | The image cache is keyed by the full signed URL; `private-user-images` JWTs rotate, so the same images are downloaded again on each check; no eviction. |
| SUG-1 | Every posting error marks a suggestion `uncertain`, including errors before sending (rate-limit gate, `gh auth token`) and explicit 4xx rejections; the text can then only be discarded. |
| PD-5 | Discard's clean check ignores ignored files, and `git worktree remove` deletes them (`.env`, `target/`, agent outputs). Reproduced. |
| CODE-2 | Working-tree `git diff` rewrites the index even with `GIT_OPTIONAL_LOCKS=0` (git 2.34.1); the git panel polls it every 5 s without the variable, and an agent's `git add` fails on `index.lock`. Reproduced. |
| SRV-2 | Session search runs `json_extract` over every event on the single state.db thread (in addition to `task_output`, R45), stalling event writes of all agents. |
| SRV-3 | A panic inside a DB job ends the actor thread; the daemon stays up with a dead database and systemd never restarts it. |
| OPS-3 | The unit has `After=network-online.target` (no effect in the user manager) and no ordering after `graphical-session.target`; the environment is captured once, so GUI launches can fail all day if the daemon starts before the display variables exist (plausible). |
| OPS-4 | The install reservation never expires: a SIGKILLed updater blocks all new work until the daemon restarts. |
| UI-3 | The conversation virtualizer has no `getItemKey`, while rows move (only the latest plan is a row): size caches by index can overlap rows (plausible). |
| UI-4 | Moving the caret with the mention list open, then Enter, corrupts the prompt (`fix @moon-landing x @moo`). |
| UI-5 | The build-id auto-reload after an update discards message box text and images, quick replies and recent composer edits. |
| UI-6 | The custom terminal command field removes spaces as they are typed (`split(" ").filter(Boolean)` on each keystroke). |
| UI-8 | Besides R41: the composer GitHub link field (Enter adds, Escape closes) and a global Escape handler in `Popover.tsx`. |
| PERF-1 | Measured: production `state.db` is 68 MB after 25 sessions (raw lines 46 MB, events 12.8 MB), with no retention. |
| Docs | ARCHITECTURE 5.1, 6.2, 6.3, 6.4, 9.2, 12 and 13 still describe the first draft: wrong API routes (`/api/notifications`, `/api/gh`, `/img?u=`, `/stop`, `/api/refresh`), six created-but-unused cache tables, modules `sync`/`quota`, a `Harness` trait, Radix/shadcn, a collapsing sidebar, review-request notifications. The ROADMAP overview still says "In progress"/"Next". Details: DOC-1 to DOC-36. |

### Low and info

- Security: `/auth` open redirect `next=/\evil.com` (verified) and CR/LF panic (SEC-5); token in browser history (SEC-7); untrusted GitHub titles and unquoted branch names in prompts (SEC-8); remote Markdown images bypass camo (SEC-9); `/api/open-editor` accepts option-like paths (SEC-10); CLI honors `HTTP_PROXY` for 127.0.0.1 (SEC-11); signed image URLs logged (SEC-12); `.git/config` readable in the viewer (SEC-13).
- Sessions: invalid UTF-8 ends a reader (SES-12); `answer` sets Working after writing (SES-16); Archive during launch undone (SES-17); launch errors after the row exists (SES-18); wedged app-server never restarted (SES-19); resume may change the model (SES-20); environment stripping removes the owner's own `CLAUDE_CODE_*` settings and leaks systemd variables (SES-22); login-shell probe without `setsid` (SES-23).
- Scheduler and projects: duplicate row and React key while a scheduled launch starts (SCH-2); terminal template quoting (PRJ-4); `github_repo` misses `user@` URLs (PRJ-5).
- GitHub: 30-minute pass skippable (GH-8); local overrides written into the cache (GH-9); `unwrap_err()` panic on unfollowed 3xx (GH-11); rate-limit gaps (GH-12); unvalidated repo paths in suggest/resolve (GH-13); preview Markdown shows HTML comments (GH-14); proxied images and in-page anchors open the daemon URL in the browser (ITM-8); draft/ready full refetch (ITM-9); timeline event types and 100-check cap (ITM-10); case-sensitive cache keys (ITM-11); fork PR refs shared and never pruned (ITM-12); non-atomic image writes (ITM-13); suggestion posting holds `predraft_lock` over network calls (SUG-2); `synchronous=NORMAL` and posting claims (SUG-3).
- Pre-drafts and code: mutating git killed on request drop (PD-6); Discard fails after the folder was deleted by hand (PD-7); broken records have no Discard (PD-8); two agents in one worktree (PD-9); lost update window (PD-10); git panel wrong in a subfolder cwd (CODE-1); `search()` stderr stall (CODE-4); remote names with `/`, ref caps, relative `gitdir` (CODE-5); README reopens after closing all tabs (CODE-6).
- Daemon and packaging: symlinked config replaced, shared tmp file (SRV-4); skill link failure aborts startup (SRV-5); CLI and framework error bodies lost (SRV-6, SRV-7); zombies (SRV-8); 21 MB vs 20 MB (SRV-9); lock fd inherited, no `--locked` (OPS-5); `install.sh` without `curl`, 10 s health wait (OPS-6); build script reruns on `git add` (OPS-7); Update hidden for non-ancestor builds (OPS-8); `--quit` opens a window (OPS-9); CI actions not SHA-pinned, `LAUNCHPAD_SOURCE` from the runner, no `rust-toolchain` (OPS-10); 1.4 MB SVG wrapping a PNG, unused WebP (OPS-11).
- UI: memo defeated by inline callbacks (UI-10); inbox re-renders on scroll (UI-11); session logs never evicted (UI-12); unbounded `localStorage` and unhandled quota errors (UI-13); `start()` overwrites an open draft's harness/model (UI-14); Review with agent replaces unsent text (UI-15); fuzzy linkify turns `setup.py` into a link (UI-16); focus lost after send (UI-17); answer with images sends text twice (UI-18); `path:null` chips (UI-19); tool-output copy button without title (UI-20); the composer hides any preset with id `draft-pr`, including the owner's existing one (UI-21); Add to agent offers interrupted sessions (UI-22); one inbox action disables all rows (UI-23); silent button failures (UI-24); double local PR diff refetch (UI-25); one-click Close/Reopen/Draft/Unsubscribe/Archive (UI-27); dead code and hand-written types, no ESLint (UI-28).
- Hygiene: no git remote (CI never ran, no off-machine copy), no `state.db` backup, leftover `code-viewer` branch and worktree, `@xterm` left in `node_modules`, `.gitignore` only covers worktrees, loose objects never packed.

### Different assessments

- Severity: R48 (open questions) is rated High there, because a routine restart triggers it and every later message then fails; R49 is High with concrete triggers (`@{-1}`, `submodule.recurse=true`) plus PD-1; R35 is High together with OPS-1. R07, R17, R18, R19, R20, R21, R33, R50 and R52 are rated lower there, because their triggers are unlikely for one user on one machine (for R17: `~/.local` and `~/.config` are 0700 here).
- Documentation: the statement above that README, AGENTS, roadmap, architecture and sketches "agree again" holds for the Phase 7 wording only; ARCHITECTURE has large drift elsewhere.
- Updater: the wait on untracked files is treated there as a defect, not as appropriate behavior.

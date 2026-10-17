# Launchpad bug and vulnerability analysis (2026-10-17)

Scope: the full codebase at `48a147f` (Rust daemon, React UI, Tauri shell). Two independent analyses were merged: this one (Claude) and an earlier Codex analysis of `30cb2bc`. Every Codex claim was re-checked against `48a147f` before it was merged; some severities were adjusted. Source tags: **[C]** this analysis, **[X]** Codex (verified), **[C+X]** found by both.

Severity levels: **Critical**, **High**, **Medium**, **Low**. No critical issue was confirmed. Bugs are grouped so that each group can be fixed in one change.

Totals: 56 bugs in 12 groups: 0 critical, 6 high, 24 medium, 26 low.

## 1. SSH host relay boundary

Fixed on 2026-10-17: the relay rejects ambiguous/escaped route paths, authorizes the exact outgoing URL and disables redirects. Queued-result notices require an existing session on the sending host. Regression tests cover traversal variants, allowed CLI routes and session host isolation. Findings below describe the analyzed revision.

- **[High] Relay allowlist can be bypassed with backslashes and `..`** [C] — `crates/core/src/hosts/relay.rs:10-23`, `relay.rs:26`
  - Bug: `allowed()` checks the raw path split on `/`, but `Url::parse` then turns `\` into `/` and resolves `..`. A process on the host can send `PATCH /api/predrafts/..\projects\<id>` and the daemon calls `PATCH /api/projects/<id>` with the full bearer token (verified with the `url` crate). Setting `worktree_setup` this way runs a command on the daemon machine at the next launch.
  - Recommendation: Normalize the URL first, reject paths that contain `\`, dot segments or encoded dots, and run `allowed()` on the exact path that is sent.
  - Alternative: Replace the generic path relay with typed relay messages, one per allowed CLI call.
- **[Low] Queued relay results write notices into any session** [C] — `crates/core/src/hosts/conn.rs:488-503`
  - Bug: For queued CLI calls the daemon reads `session_id` from the request body and emits a notice into that session without checking that the session runs on this host.
  - Recommendation: Emit the notice only when the session exists and its `cwd` is on the same host.
  - Alternative: Record queued results in the host status instead of a session notice.

## 2. Worktree merge, discard and isolation

- **[High] Automatic rebase drops changes made in merge commits** [X] — `crates/core/src/worktrees.rs:874-876`
  - Bug: `try_merge` runs a plain `git rebase <base>`, which linearizes history and drops merge commits. Changes that exist only in a merge commit (for example conflict resolutions) disappear, and Launchpad still reports a successful merge (reproduced by Codex).
  - Recommendation: Refuse automatic rebase when the branch contains merge commits, or use `--rebase-merges` and compare the final tree before the fast-forward.
  - Alternative: Merge the base into the branch instead of rebasing, then fast-forward.
- **[Medium] Merges ignore running terminals** [X] — `crates/core/src/worktrees.rs:411-428`, `worktrees.rs:845`, `worktrees.rs:866`
  - Bug: `try_merge` calls `idle_reason(…, false)`, which skips the terminal check, and the target checkout check only looks at agents. Rebase and fast-forward can change files while a terminal command (build, dev server) uses them.
  - Recommendation: Wait while a command terminal is alive in the worktree, and show which terminal blocks the merge.
  - Alternative: Warn in the merge notice instead of waiting, for shells that are idle at a prompt.
- **[Medium] Worktree discard uses stale state and activity checks** [C+X] — `crates/core/src/worktrees.rs:587-618`
  - Bug: `discard` reads the record and checks busy agents before it waits for terminals and takes `action_lock`/`worktree_lock`. A turn started meanwhile is killed and its folder is force-removed; a worktree promoted to a pre-draft meanwhile loses its folder and branch.
  - Recommendation: Take `action_lock` then `worktree_lock` first, reload the record, and run state and busy checks under the locks.
  - Alternative: Re-check state and activity after taking the locks and abort if anything changed.
- **[Medium] A discarded editable context project falls back to the main checkout** [X] — `crates/core/src/sessions.rs:808-813`
  - Bug: On reattach, an editable context project without an active worktree is added as `p.path`, so the resumed agent gets write access to the checkout it was told to avoid.
  - Recommendation: Skip (or restore) editable context projects whose worktree is not active, and tell the agent.
  - Alternative: Fail the resume with a visible error that names the discarded context worktree.
- **[Medium] Projects in a repository subfolder cannot open their task worktrees in Code** [X] — `crates/core/src/code.rs:139-148`, `crates/core/src/terminals.rs:590-605`
  - Bug: `code::open` and `check_cwd` accept only an exact project path or a worktree root whose main checkout equals the project path. For a project `repo/sub`, neither `<worktree>` nor `<worktree>/sub` matches.
  - Recommendation: Accept `<worktree>/<subdir>` when the worktree's main checkout plus the same subdir equals a registered project.
  - Alternative: Check against the `task_worktree` records instead of deriving the project from paths.
- **[Low] Copied untracked files can be written through a directory symlink** [X] — `crates/core/src/worktrees.rs:301-318`
  - Bug: `copy_files` checks that the source is a regular file but does not check that the destination parent stays inside the new worktree, so a symlinked parent directory redirects the copy (reproduced by Codex with a constructed repository).
  - Recommendation: Canonicalize the destination parent and require it to be inside `dest` before copying.
  - Alternative: Refuse to copy when any parent component of the destination is a symlink.
- **[Low] Launch failures after worktree creation leave orphan worktrees and pre-drafts** [C] — `crates/core/src/sessions.rs:631-636`, `sessions.rs:648-684`
  - Bug: `abandon()` runs only when worktree preparation fails. Later errors (missing upload, persist error) keep the worktree, its branch and an empty pre-draft.
  - Recommendation: Wrap every step after worktree/pre-draft creation in one fallible block that calls `abandon` and archives the pre-draft.
  - Alternative: Resolve attachments and other fallible inputs before creating worktrees.
- **[Low] Hand-off silently deletes ignored files** [C] — `crates/core/src/worktrees.rs:636-650`, `worktrees.rs:686`
  - Bug: The dirty check does not see ignored files, but `git worktree remove` deletes them (`.env`, local databases). Move back does not copy `.worktreeinclude` files again.
  - Recommendation: List ignored files before hand-off and warn or refuse, and run `copy_included` on handback.
  - Alternative: Say clearly in the hand-off confirmation that ignored files are lost.

## 3. Pre-draft locking and guards

- **[High] Deadlock between pre-draft Discard/Publish and every session action** [C+X] — `crates/core/src/predrafts.rs:475`, `predrafts.rs:489`, `predrafts.rs:506-521`, `crates/core/src/sessions.rs:1067-1068`, `sessions.rs:1187`
  - Bug: `discard`/`publish` take `work` and then call `close_in` → `close`, which takes `actions`. Send, launch, resume and answer take `actions` then `work`. One concurrent send or cron launch blocks both mutexes forever, so every later action, including Stop, hangs until restart (reproduced by Codex).
  - Recommendation: Take `actions` before `work` in discard/publish and use a close variant that assumes the caller holds `actions`.
  - Alternative: Release `work` and `predraft_lock` before closing sessions, then re-check the pre-draft state.
- **[High] Pre-draft guards miss sessions in a project subfolder** [C+X] — `crates/core/src/predrafts.rs:144-151`, `predrafts.rs:328-333`, `predrafts.rs:446`
  - Bug: The guards compare `session.cwd == p.root` exactly. A session in `<worktree>/<subdir>` passes the idle check, so Publish can push while it commits; it can also send turns after publication started, and "Send notes to agent" always fails.
  - Recommendation: Match by folder containment (`cwd == root` or `cwd` inside `root`) in all pre-draft checks.
  - Alternative: Store the exact session folder in the pre-draft record and compare against it.

## 4. Agent process shutdown and harness protocol

- **[Medium] Archive treats the agent as stopped before it exits** [X] — `crates/core/src/sessions.rs:1177-1183`, `crates/core/src/harness/claude.rs:205-215`
  - Bug: `detach` removes the live handle and Claude `stop` only closes stdin and kills after 5 s. Archive cleanup can remove a clean worktree while the old process still writes into it.
  - Recommendation: Make `stop` wait for the process exit (with the 5 s kill as a bound) before `close` continues.
  - Alternative: Delay worktree cleanup until the harness reports its exit.
- **[Medium] Stopping a worktree setup leaves its child processes running** [X] — `crates/core/src/worktrees.rs:383-400`, `crates/core/src/procs.rs:18-20`
  - Bug: Abort or timeout kills only the `sh` process; children such as `npm install` keep writing after the session is Stopped and cleanup becomes possible (reproduced by Codex).
  - Recommendation: Start the setup in its own process group and kill the group on abort or timeout.
  - Alternative: Run setup through the helper or a PTY that already tracks the whole process tree.
- **[Medium] Codex steering falls back to a new turn on every error** [X] — `crates/core/src/harness/codex.rs:166-180`
  - Bug: Any `turn/steer` error, including a timeout whose delivery is unknown, starts a new turn with the same text. The message can be delivered twice.
  - Recommendation: Start a new turn only when the error says the expected turn is no longer active.
  - Alternative: Return an uncertain-delivery error and let the owner decide.
- **[Medium] Interrupted questions stay pending in the adapters** [X] — `crates/core/src/harness/claude.rs:174-186`, `claude.rs:363`, `crates/core/src/harness/codex.rs:206-208`
  - Bug: Interrupt clears the session's `open_question` flag, but the adapters keep the unanswered request, so `ready_for_turn()` stays false until the process is detached.
  - Recommendation: Clear pending questions/requests in the adapter when the turn is interrupted or ends.
  - Alternative: Detach and restart the process after interrupting a turn that had an open question.

## 5. Quota protection and session lifecycle

- **[High] Quota guard pauses sessions during worktree setup; the task prompt is lost and Stop is ignored** [C] — `crates/core/src/sessions.rs:729`, `sessions.rs:874`, `sessions.rs:997`, `sessions.rs:1129-1138`
  - Bug: A Starting session that runs its setup counts as running, so the guard sets a quota pause with the generic "Continue the interrupted task" text. After quota returns, the agent starts without its real task. Stop during setup leaves the session QuotaPaused, and it continues automatically later.
  - Recommendation: Skip sessions with `setup_running(id)` (or no harness conversation) in the guard, and clear `quota_pause` in the setup branch of `interrupt`.
  - Alternative: Store the real initial prompt in the `QuotaPause` when the session never received it.
- **[Medium] Resume of a never-started session starts the agent without its prompt** [C] — `crates/core/src/sessions.rs:199`, `sessions.rs:826`, `sessions.rs:1138`
  - Bug: A session interrupted before its first send (restart or Stop during setup) has no `harness_session_id`. Resume starts a new conversation and sends only "Continue the unfinished turn.".
  - Recommendation: When no prompt was delivered yet, send the stored `session.prompt` and launch images as the first turn.
  - Alternative: Disable Resume for never-started sessions and offer "Launch again".
- **[Medium] `resume()` marks a running session as Done** [C] — `crates/core/src/sessions.rs:1208-1235`
  - Bug: `resume` has no running guard: it reuses the live process and calls `set_status(Waiting)`. A stale device or a double click sends a false Done notification and satisfies dependencies of scheduled launches.
  - Recommendation: Refuse or no-op when the session is running, has an open question or its process is not ready for a turn.
  - Alternative: Only set Waiting when the previous status was Closed, Stopped, Interrupted, Error or InTerminal.
- **[Low] Scheduled and cron launches start without a known quota reading** [C] — `crates/core/src/lib.rs`, `crates/core/src/sessions.rs:750`
  - Bug: At daemon startup an overdue scheduled launch runs before the first quota reading, using the "unknown reading" path that is only allowed for manual work.
  - Recommendation: Keep automatic launches queued until a fresh reading exists for protected harnesses.
  - Alternative: Start the scheduler only after the first quota reading or a timeout.
- **[Low] `answer()` ignores the manual subscription-only retry baseline** [C] — `crates/core/src/sessions.rs:1166`
  - Bug: `answer` uses the raw quota snapshot, so after "Retry with subscription only" a question cannot be answered until a new reading arrives.
  - Recommendation: Use `self.quota_blockers(app, &s)` like `send`.
  - Alternative: Clear the baseline when a question opens.
- **[Low] A successful usage reading without windows clears exhaustion** [C] — `crates/core/src/harness/claude.rs:965-975`, `crates/core/src/quota.rs:18-31`
  - Bug: If the usage response lacks `five_hour`/`seven_day`, both windows become `None`, `exhausted()` returns nothing, and held input can be sent although no window confirmed availability.
  - Recommendation: Treat a response without both windows as an error and keep the previous windows.
  - Alternative: Keep blockers until `available()` is true, not only until the windows disappear.
- **[Low] `Session::light()` still broadcasts held image bytes** [C] — `crates/core/src/sessions.rs` (`light()`, `hold_input`)
  - Bug: `quota_pause.images` keeps resolved data URLs (up to 40 MB). They go to every browser on each session bus update and in `GET /api/sessions`.
  - Recommendation: Strip `data_url` from `quota_pause.images` in `light()`.
  - Alternative: Store held images as `upload:` references.
- **[Low] Presets with ID `worktree` or `draft-pr` are silently dropped** [C] — `crates/core/src/sessions.rs:486`
  - Bug: `resolve` removes these preset IDs for worktree and pre-draft launches, which is the reserved-ID filtering that the project rules forbid.
  - Recommendation: Remove the `retain` filter.
  - Alternative: Show a warning in the prompt preview when a preset is skipped.

## 6. Session event log ordering and trace loading

- **[Medium] Event numbers and session rows can be written out of order** [C] — `crates/core/src/sessions.rs:296`, `sessions.rs:1396-1407`
  - Bug: `emit` takes `seq` under a lock but queues the DB write later, and `persist` writes a cloned snapshot. Writes can arrive reversed, so `last_seq` goes back; after a restart a seq is reused and the INSERT fails, and an old status can overwrite a newer one.
  - Recommendation: Use `last_seq = max(last_seq, ?)`, remove `last_seq`/`last_message` from the `persist` upsert, and keep per-session write order.
  - Alternative: Assign `seq` inside the DB actor and publish from the stored result.
- **[Medium] Live events are dropped while a trace load is in flight** [C+X] — `web/src/lib/sessionStore.ts:43-52`, `sessionStore.ts:102-106`
  - Bug: An out-of-order event calls `load(id)`, which returns at once when a load is running, so the event is discarded. The final message of a turn can be missing until reload (reproduced by Codex).
  - Recommendation: Buffer events (or set a dirty flag) during a load and merge or reload after it finishes.
  - Alternative: After each load, run one more `?after=` catch-up if events were dropped.
- **[Medium] A failed first log fetch leaves the trace empty forever** [C] — `web/src/features/agents/SessionPane.tsx:202-207`, `web/src/lib/sessionStore.ts:42-69`
  - Bug: The view creates an empty log and calls `load` once. On a network error nothing retries, and a `replayed` reconnect does not reload it.
  - Recommendation: Mark the log as failed on error so the view retries, and show a Retry button.
  - Alternative: Reload logs with `loaded === false` after `replayed`.
- **[Medium] Offline cache restore can overwrite fresh data after resync** [C] — `web/src/main.tsx:31`, `main.tsx:44`, `web/src/lib/offlineCache.ts:36`
  - Bug: The socket connects before IndexedDB is restored. If `resync` comes first, the restore writes the old snapshot, and `staleTime: Infinity` queries never refetch.
  - Recommendation: Await `restoreCache` before `connectEvents`, or run `reloadAll` again after the restore when a resync already ran.
  - Alternative: Invalidate restored keys right after the restore.

## 7. Retry-safe Launch and Send

- **[High] Launch and Send fail on plain-HTTP remote origins** [C+X] — `web/src/lib/operations.ts:5`, `operations.ts:9`
  - Bug: `crypto.subtle` and `crypto.randomUUID` exist only in secure contexts. The default tailnet fallback (`http://100.x.y.z:7317`) is not secure, so every launch, reply and Resume from a phone throws before `fetch` (reproduced by Codex in Chrome).
  - Recommendation: Use a small JavaScript hash and a `getRandomValues` UUID when `crypto.subtle`/`randomUUID` are missing.
  - Alternative: Send without an `Idempotency-Key` on insecure origins and show that retries are not protected.
- **[Medium] The retry ledger never expires** [C] — `web/src/lib/operations.ts:3-13`, `web/src/features/agents/SessionSummary.tsx:170`
  - Bug: The key is a hash of path and body, kept until a "completed" response. After one lost response, a later identical message (for example "yes", or Resume's fixed text) reuses the old ID and is replayed or blocked by a 409, so nothing reaches the agent.
  - Recommendation: Store a creation time and treat old entries as new operations; add an abandon path to Resume buttons.
  - Alternative: Create a per-attempt nonce when the text is composed and keep it with the draft.

## 8. Integrated terminals

- **[Medium] Close can succeed while a foreground job survives** [X] — `crates/core/src/terminals.rs:237`, `terminals.rs:343`
  - Bug: Close signals the shell's process group, but job control puts foreground commands in another group. If the shell exits first and the job ignores SIGHUP, the terminal is marked dead and the job keeps running (reproduced by Codex).
  - Recommendation: Signal the PTY's foreground process group (`tcgetpgrp`) as well as the shell group, and escalate to SIGKILL for both.
  - Alternative: Track and kill every process whose session ID equals the shell's.
- **[Low] Blocked PTY input stalls the whole viewer connection** [C+X] — `crates/server/src/terminals.rs:211-215`
  - Bug: The handler awaits each blocking PTY write in its select loop. A process that stops reading input stops output, ACKs, pings and revocation checks for that viewer.
  - Recommendation: Send input to a per-terminal writer task through a bounded channel and keep the loop non-blocking.
  - Alternative: Drop input with a notice when the PTY write does not complete within a short timeout.

## 9. Front door, remote access and desktop shell

- **[Medium] Apps that share the Tailscale Serve origin get full Launchpad access** [X] — `crates/server/src/tailnet.rs:99-126`
  - Bug: Discovery accepts an HTTPS Serve origin where other applications are mounted on other paths. JavaScript served by such an app is same-origin, so it can use the paired cookie and drive Launchpad, including terminals.
  - Recommendation: Accept only a Serve host whose handlers contain the Launchpad root route and nothing else.
  - Alternative: Warn in Settings → Devices and require an explicit owner confirmation for shared origins.
- **[Low] Desktop navigation guard uses a string prefix** [C+X] — `desktop/src-tauri/src/main.rs:247-248`
  - Bug: `url.starts_with("http://127.0.0.1:7317")` also accepts `http://127.0.0.1:73170/` and `http://127.0.0.1:7317@evil.example/`, so such a link loads a foreign page inside the app window.
  - Recommendation: Compare `url.origin()` with the daemon origin.
  - Alternative: Parse host and port and compare them exactly.
- **[Low] Desktop stays on its start screen when the daemon is healthy but pairing fails** [X] — `desktop/src-tauri/src/main.rs:328-342`
  - Bug: With `warm == true` and `app_url` failing, the window loads the fallback page and no recovery thread runs, so no error is shown and nothing retries.
  - Recommendation: Start the recovery thread (or show the error) whenever `app_url` fails, not only when the daemon was down.
  - Alternative: Retry `app_url` a few times before loading the fallback page.
- **[Low] Project worktree setup command is editable from remote browsers** [C] — `crates/core/src/projects.rs:223-225`, `crates/core/src/worktrees.rs:383-400`
  - Bug: `PATCH /api/projects/{id}` stores `worktree_setup` with no remote check, although commands must be file-only in remote mode. It is also the target of the relay bypass in group 1.
  - Recommendation: Reject `worktree_setup` changes when `auth.remote()` is true and show it read-only.
  - Alternative: Move setup commands into `config.toml`.
- **[Low] TOML error messages expose variant credentials** [X] — `crates/core/src/config.rs:246`, `config.rs:355`, `crates/server/src/routes.rs:40`
  - Bug: TOML parse errors include the offending source line, and `GET /api/config` returns the error even in remote mode, where variant env values must be hidden (reproduced by Codex). Paired browsers can still read the file through a terminal, which limits the impact.
  - Recommendation: Return only the line and column of config errors to browsers and log the full message on the host.
  - Alternative: Redact values of `env` tables from the error text.
- **[Low] Unauthenticated reverse-pairing start has one global rate limit** [C] — `crates/server/src/auth.rs:637-650`
  - Bug: Any client that reaches the front door can use the 20/minute and 100 pending slots, so the owner cannot create a reverse-pairing code meanwhile.
  - Recommendation: Rate-limit per source address and keep a smaller global limit.
  - Alternative: Keep QR/link pairing as the documented fallback when reverse pairing is saturated.

## 10. GitHub suggestions, inbox and diffs

- **[Medium] Suggestions become "uncertain" after failures that sent nothing** [C] — `crates/core/src/suggestions.rs:241-262`
  - Bug: Every error from `items::mutate` becomes `uncertain`, including the local rate-limit gate, a missing `gh` token, refused connections and 4xx answers. Edit/Post disappear and only Discard is left.
  - Recommendation: Return to `draft` for errors before sending and for 4xx; keep `uncertain` only for timeouts, lost connections and 5xx.
  - Alternative: Add a "Return to draft (I checked GitHub)" button.
- **[Medium] Added source lines can change the parsed diff file name** [X] — `crates/core/src/items.rs:655-695`
  - Bug: `parse_diff` reads `+++ `/`--- ` headers inside hunks. An added line `++ b/fake.txt` becomes `+++ b/fake.txt` and renames the file in the review list; `++ /dev/null` marks it removed (reproduced by Codex). A PR author can disguise which file changed.
  - Recommendation: Track whether the parser is inside a hunk and only read headers between `diff --git` and the first `@@`.
  - Alternative: Take file names only from the `diff --git` line and rename metadata.
- **[Medium] Inbox poll refetches up to 15 item details over GraphQL every minute** [C] — `crates/core/src/inbox.rs:287-313`, `crates/core/src/items.rs:119-132`
  - Bug: The prefetch runs on every poll, even after a 304, and every cache entry is older than its 60 s freshness. This uses about 18% of the hourly GraphQL budget while idle.
  - Recommendation: Prefetch only threads whose `updated_at` changed.
  - Alternative: Skip the prefetch when the notifications page returned 304.
- **[Medium] Full inbox sync checks every subscription while holding the inbox lock** [C] — `crates/core/src/inbox.rs:176`, `inbox.rs:197`, `inbox.rs:236-253`
  - Bug: Every 30 minutes all history threads get one sequential request under `self.lock`, so Done/Read clicks hang. One non-404 error (for example SAML 403) aborts the whole sync.
  - Recommendation: Expire subscription cache entries only when `updated_at` changes, and skip per-thread errors.
  - Alternative: Run the checks outside the lock and apply the results in one short locked step.
- **[Medium] PR diff fails instead of using the API fallback** [C] — `crates/core/src/items.rs:512-519`
  - Bug: The remote lookup is case-sensitive and its `?` is outside the fallback block. A case difference or an unreachable host makes Files changed fail with "missing mapped remote".
  - Recommendation: Compare repository names case-insensitively and move the lookup inside the fallible block.
  - Alternative: Pick the remote from `p.gh_repo`/`p.gh_upstream` directly.
- **[Low] Manual GitHub posts can be duplicated after a lost reply** [X] — `crates/server/src/items.rs:58`, `crates/core/src/items.rs:247`, `crates/core/src/inbox.rs:514`
  - Bug: Owner comments, reviews and inbox replies have no durable posting claim (unlike agent suggestions). If the response is lost, the draft stays and a retry posts the same text again.
  - Recommendation: Reuse the suggestion posting claim (posting/uncertain states) for manual posts.
  - Alternative: Before retrying, check the latest timeline for an identical comment by the owner.
- **[Low] API-fallback diffs are not cached** [C] — `crates/core/src/items.rs:594-597`, `items.rs:625-635`
  - Bug: When the local diff failed, the fallback result is not stored, so each file click repeats a failed fetch and all REST pages.
  - Recommendation: Cache successful fallback results.
  - Alternative: Return single patches from the in-memory summary result.
- **[Low] PR diff cache keys ignore the base** [X] — `web/src/features/github/FilesChanged.tsx:284-303`
  - Bug: The diff and patch queries use `staleTime: Infinity` with keys that contain the head but not the base, so a retargeted PR keeps showing the old file list.
  - Recommendation: Add the base SHA (or base branch) to both query keys.
  - Alternative: Invalidate `pr-diff` queries when the item detail reports a new base.
- **[Low] Clicking an image in GitHub HTML opens the daemon `/img` URL** [C] — `crates/core/src/items.rs:466-477`, `web/src/features/github/lib.tsx:51-56`
  - Bug: `proxy_images` also rewrites the `<a href>` around images, so the external browser opens `http://127.0.0.1:7317/img?...` without a cookie.
  - Recommendation: Rewrite only `src` attributes.
  - Alternative: Map `/img?url=` links back to the original URL in the click handler.
- **[Low] Posting a suggestion holds the global pre-draft lock during network calls** [C] — `crates/core/src/suggestions.rs:179`, `suggestions.rs:230-255`
  - Bug: GitHub posts run a full item refresh and the POST under `predraft_lock`, which blocks pre-draft launches and publication for tens of seconds.
  - Recommendation: Use a per-suggestion lock for GitHub targets.
  - Alternative: Claim `posting` under the lock, then release it before the network calls.
- **[Low] Review suggestions with an uppercase `--head` are always stale** [C] — `crates/core/src/suggestions.rs:120-125`
  - Bug: Uppercase hex is accepted but compared exactly with GitHub's lowercase head, so Post stays disabled.
  - Recommendation: Lowercase `head` in `create`.
  - Alternative: Reject uppercase input with a clear error.
- **[Low] The proxied image cache grows without limit** [C] — `crates/core/src/items.rs:359-418`
  - Bug: Every proxied GitHub image is stored in `<data_dir>/images` forever.
  - Recommendation: Delete entries older than a few weeks at startup, like uploads.
  - Alternative: Cap the folder size and remove the oldest files.

## 11. Browser-side drafts, storage and settings

- **[Medium] Failed draft saves silently drop the last edit** [C+X] — `web/src/lib/reviewState.ts:26-35`, `web/src/stores/composer.ts:92-108`
  - Bug: A failed review-state PUT is only logged and its pending value is dropped; composer saves ignore every error except 409. Unsent replies and the composer draft can be lost on reload or on other devices.
  - Recommendation: Keep failed values pending, retry on reconnect, and show a "not saved" state.
  - Alternative: Mirror pending values in localStorage until the daemon confirms them.
- **[Medium] Concurrent review-state saves can overwrite newer edits** [X] — `web/src/lib/reviewState.ts:26-35`, `crates/core/src/review_state.rs:50-70`
  - Bug: Saves for one key are not serialized and have no revision check, and each response writes its own value into the query cache. A delayed older request can replace a newer reply or viewed-marker set.
  - Recommendation: Serialize saves per key in the browser and add a revision (compare-and-set) on the daemon.
  - Alternative: Ignore responses whose version is older than the latest pending version for that key.
- **[Low] Message drafts and pins overwrite each other across windows** [C] — `web/src/stores/messages.ts:23-35`, `web/src/stores/layout.ts:35-53`
  - Bug: Each window writes its whole persisted state, so an older window removes pins or unsent replies made in another one.
  - Recommendation: Rehydrate on `storage` events or merge per key on write.
  - Alternative: Store each draft and pin under its own key.
- **[Low] Settings → General saves a stale full config** [C] — `web/src/features/settings/SettingsView.tsx:79-96`
  - Bug: The page keeps its first config snapshot and saves all sections, reverting changes made elsewhere (for example `worktrees.keep`) or failing after the HTTPS origin was adopted.
  - Recommendation: Send only the edited fields merged on top of the latest config.
  - Alternative: Add a config revision and reject stale saves with 409.
- **[Low] Browser data stays after Sign out or revocation** [C] — `web/src/features/settings/DevicesView.tsx:35`, `web/src/lib/api.ts:30`, `web/src/lib/offlineCache.ts:7-8`
  - Bug: IndexedDB (prompts, messages, paths, inbox) and localStorage drafts stay readable after Sign out or a 401.
  - Recommendation: Delete `launchpad-cache` and `launchpad-*` keys on Sign out and on 401.
  - Alternative: Clear client storage when `/auth` loads without a valid credential.

## 12. Dependencies

- **[Low] Lockfile pins `source-map-js` 1.2.1** [X, advisory not verified] — `web/pnpm-lock.yaml:880`
  - Bug: Codex reported an advisory for this build-time dependency (CPU/memory exhaustion on crafted source maps). The version is confirmed in the lockfile; the advisory could not be checked because the advisory database was not reachable. No runtime path in Launchpad is known.
  - Recommendation: Run `pnpm audit` with network access and update the package if the advisory is confirmed.
  - Alternative: Add a scheduled dependency audit to CI.

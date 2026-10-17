# Launchpad bug and vulnerability analysis (2026-10-17)

Scope: the full codebase at `48a147f` (Rust daemon, React UI, Tauri shell). Severity levels: **Critical**, **High**, **Medium**, **Low**. Bugs are grouped so that each group can be fixed in one change. Each bug has a short description, a recommendation and one alternative.

## 1. SSH host relay boundary

- **[High] Relay allowlist can be bypassed with backslashes and `..`** — `crates/core/src/hosts/relay.rs:10-23`, `relay.rs:26`
  - Bug: `allowed()` checks the raw path split on `/`, but `Url::parse` then turns `\` into `/` and resolves `..`. A process on the host can send `PATCH /api/predrafts/..\projects\<id>` or `GET /api/predrafts/..\config`, and the daemon calls `PATCH /api/projects/<id>` or `GET /api/config` with the full bearer token. Setting `worktree_setup` this way gives code execution on the daemon machine at the next launch.
  - Recommendation: Parse and normalize the URL first, reject any path that contains `\`, `.`/`..` segments or percent-encoded dots, and run `allowed()` on the normalized path that is actually sent.
  - Alternative: Replace the generic path relay with typed relay messages (one enum variant per allowed CLI call).
- **[Low] Queued relay results write notices into any session** — `crates/core/src/hosts/conn.rs:488-503`
  - Bug: For queued CLI calls, the daemon takes `session_id` from the request body and emits a notice into that session, without checking that the session runs on this host.
  - Recommendation: Only emit the notice when the session exists and its `cwd` is on the same host.
  - Alternative: Log the queued result in the host status instead of a session notice.

## 2. Global lock order (deadlocks) and pre-draft/worktree guards

- **[High] ABBA deadlock between pre-draft Discard/Publish and every session action** — `crates/core/src/predrafts.rs:506-514`, `predrafts.rs:520-521`, `predrafts.rs:475`, `predrafts.rs:489`, `crates/core/src/sessions.rs:1067-1068`, `sessions.rs:1187`
  - Bug: `discard`/`publish` take `work` and then call `close_in` → `close`, which takes `actions`. Send, launch, resume and answer take `actions` first and then `work`. A send or cron launch during a Discard blocks both mutexes forever, and every later action (including Stop) hangs until the daemon restarts.
  - Recommendation: Take `actions` before `work` in discard/publish (the order used everywhere else), and use a close variant that assumes the caller holds `actions`.
  - Alternative: Release `work` and `predraft_lock` before closing sessions, then re-check the pre-draft state.
- **[Medium] Pre-draft guards compare `session.cwd == p.root` exactly** — `crates/core/src/predrafts.rs:144-151`, `predrafts.rs:328-333`, `predrafts.rs:446`
  - Bug: For a project in a repository subfolder, the session runs in `<worktree>/<subdir>`, so idle checks, the publication freeze, "Send notes to agent" and follow-up conversion never match its sessions. Publish can push while the agent is still committing.
  - Recommendation: Match by folder containment (`cwd == root || cwd` inside `root`) in all pre-draft checks.
  - Alternative: Store the exact session folder in the pre-draft record and compare against it.
- **[Low] Worktree discard checks for busy agents before taking the action lock** — `crates/core/src/worktrees.rs:587-618`
  - Bug: A turn that starts between the busy check and `action_lock()` is killed, and its folder is force-removed while the agent edits it.
  - Recommendation: Take `action_lock` first and run the busy checks under it, as `remove`/`handoff` do.
  - Alternative: Re-run the idle check after taking the lock and abort if anything became busy.
- **[Low] Launch failures after worktree creation leave orphan worktrees and pre-drafts** — `crates/core/src/sessions.rs:631-636`, `sessions.rs:648-684`
  - Bug: `abandon()` only runs when worktree preparation fails. Later errors (missing upload, persist error) keep the worktree, its branch and an empty pre-draft.
  - Recommendation: Wrap every step after worktree/pre-draft creation in one fallible block that calls `abandon` and archives the pre-draft on error.
  - Alternative: Resolve attachments and other fallible inputs before creating worktrees.
- **[Low] Hand-off silently deletes ignored files** — `crates/core/src/worktrees.rs:636-650`, `worktrees.rs:686`
  - Bug: The dirty check ignores ignored files, but `git worktree remove` deletes them (`.env`, local databases). Move back does not copy `.worktreeinclude` files again.
  - Recommendation: List ignored files before hand-off and warn or refuse, and run `copy_included` on handback.
  - Alternative: Say clearly in the hand-off confirmation that ignored files are lost.

## 3. Quota protection and session lifecycle

- **[High] Quota guard pauses sessions during worktree setup; the task prompt is lost and Stop is ignored** — `crates/core/src/sessions.rs:874`, `sessions.rs:729`, `sessions.rs:997`, `sessions.rs:1129-1138`
  - Bug: A Starting session that runs its setup command counts as running, so the guard sets a quota pause with the generic "Continue the interrupted task" text. After quota returns, the agent starts without its real task. Stop during setup leaves the session QuotaPaused, and it continues automatically later.
  - Recommendation: Skip sessions with `setup_running(id)` (or no harness conversation) in the guard, and clear `quota_pause` in the setup branch of `interrupt`.
  - Alternative: Store the real initial prompt in the `QuotaPause` when the session never received it.
- **[Medium] Resume of a never-started session starts the agent without its prompt** — `crates/core/src/sessions.rs:199`, `sessions.rs:826`, `sessions.rs:1138`
  - Bug: A session interrupted before its first send (restart or Stop during setup) has no `harness_session_id`. Resume starts a new conversation and sends only "Continue the unfinished turn.".
  - Recommendation: When no prompt was delivered yet, send the stored `session.prompt` and launch images as the first turn.
  - Alternative: Disable Resume for never-started sessions and offer "Launch again".
- **[Medium] `resume()` marks a running session as Done** — `crates/core/src/sessions.rs:1208-1235`
  - Bug: `resume` has no running guard: it reuses the live process and calls `set_status(Waiting)`. A stale device or a double click sends a false Done notification and satisfies dependencies of scheduled launches.
  - Recommendation: Refuse or no-op when the session is running, has an open question or the process is not ready for a turn.
  - Alternative: Only set Waiting when the previous status was Closed, Stopped, Interrupted, Error or InTerminal.
- **[Low] Scheduled and cron launches start without a known quota reading** — `crates/core/src/lib.rs`, `crates/core/src/sessions.rs:750`
  - Bug: At daemon startup an overdue scheduled launch runs before the first quota reading, using the "unknown reading" path that is only allowed for manual work.
  - Recommendation: Keep automatic launches queued until a fresh reading exists for protected harnesses.
  - Alternative: Start the scheduler only after the first quota reading or a timeout.
- **[Low] `answer()` ignores the manual subscription-only retry baseline** — `crates/core/src/sessions.rs:1166`
  - Bug: `answer` uses the raw quota snapshot, so after "Retry with subscription only" a question cannot be answered until a new reading arrives.
  - Recommendation: Use `self.quota_blockers(app, &s)` like `send`.
  - Alternative: Clear the baseline when a question opens.
- **[Low] A successful usage reading without windows clears exhaustion** — `crates/core/src/harness/claude.rs:965-975`, `crates/core/src/quota.rs:18-31`
  - Bug: If the usage response lacks `five_hour`/`seven_day`, both windows become `None`, `exhausted()` returns nothing, and held input can be sent although no window confirmed availability.
  - Recommendation: Treat a response without both windows as an error and keep the previous windows.
  - Alternative: Keep blockers until `available()` is true, not only until the windows disappear.
- **[Low] `Session::light()` still broadcasts held image bytes** — `crates/core/src/sessions.rs` (`light()`, `hold_input`)
  - Bug: `quota_pause.images` keeps resolved data URLs (up to 40 MB). They go to every browser on each session bus update and in `GET /api/sessions`.
  - Recommendation: Strip `data_url` from `quota_pause.images` in `light()`.
  - Alternative: Store held images as `upload:` references.
- **[Low] Presets with ID `worktree` or `draft-pr` are silently dropped** — `crates/core/src/sessions.rs:486`
  - Bug: `resolve` removes these preset IDs for worktree and pre-draft launches. This is the reserved-ID filtering that the project rules forbid.
  - Recommendation: Remove the `retain` filter.
  - Alternative: Show a warning in the prompt preview when a preset is skipped.

## 4. Session event log ordering and trace loading

- **[Medium] Event numbers and session rows can be written out of order** — `crates/core/src/sessions.rs:296`, `sessions.rs:1396-1407`
  - Bug: `emit` takes `seq` under a lock but queues the DB write later, and `persist` writes a cloned snapshot. Writes can arrive reversed, so `last_seq` goes back. After a restart a seq is reused and the INSERT fails, and an old status can overwrite a newer one.
  - Recommendation: Use `last_seq = max(last_seq, ?)`, remove `last_seq`/`last_message` from the `persist` upsert, and keep per-session ordering of the DB writes.
  - Alternative: Assign `seq` inside the DB actor and publish from the stored result.
- **[Medium] Live events are dropped while a trace load is in flight** — `web/src/lib/sessionStore.ts:43`, `sessionStore.ts:102-106`
  - Bug: An out-of-order event calls `load(id)`, which returns at once when a load is already running, so the event is discarded. The final message of a turn can be missing until the page reloads.
  - Recommendation: Buffer events (or set a dirty flag) during a load and merge or reload after it finishes.
  - Alternative: After each load, run one more `?after=` catch-up if events were dropped.
- **[Medium] A failed first log fetch leaves the trace empty forever** — `web/src/features/agents/SessionPane.tsx:202-207`, `web/src/lib/sessionStore.ts:42-69`
  - Bug: The view creates an empty log and calls `load` once. On a network error nothing retries, and a `replayed` reconnect does not reload it.
  - Recommendation: Remove or mark the log as failed on error so the view retries, and show a Retry button.
  - Alternative: Reload logs with `loaded === false` after `replayed`.
- **[Medium] Offline cache restore can overwrite fresh data after resync** — `web/src/main.tsx:31`, `main.tsx:44`, `web/src/lib/offlineCache.ts:36`
  - Bug: The socket connects before IndexedDB is restored. If `resync` comes first, the restore then writes the old snapshot, and `staleTime: Infinity` queries never refetch.
  - Recommendation: Await `restoreCache` before `connectEvents`, or run `reloadAll` again after the restore when a resync already ran.
  - Alternative: Invalidate restored keys right after the restore.

## 5. Retry-safe Launch and Send

- **[High] Launch and Send fail on plain-HTTP remote origins** — `web/src/lib/operations.ts:5`, `operations.ts:9`
  - Bug: `crypto.subtle` and `crypto.randomUUID` exist only in secure contexts. The default tailnet fallback (`http://100.x.y.z:7317`) is not secure, so every launch, reply and Resume on a phone throws before `fetch`.
  - Recommendation: Use a small JavaScript hash and a `getRandomValues` UUID when `crypto.subtle`/`randomUUID` are missing.
  - Alternative: Send without an `Idempotency-Key` on insecure origins and show that retries are not protected.
- **[Medium] The retry ledger never expires** — `web/src/lib/operations.ts:3-13`, `web/src/features/agents/SessionSummary.tsx:170`
  - Bug: The key is a hash of path and body, kept until a "completed" response. After one lost response, a later identical message (for example "yes", or Resume's fixed text) reuses the old ID and is replayed or blocked by a 409, so nothing reaches the agent.
  - Recommendation: Store a creation time and treat old entries as new operations; add an abandon path to Resume buttons.
  - Alternative: Create a per-attempt nonce when the text is composed and keep it with the draft.

## 6. GitHub suggestions, inbox and diffs

- **[Medium] Suggestions become "uncertain" after failures that sent nothing** — `crates/core/src/suggestions.rs:241-262`
  - Bug: Every error from `items::mutate` becomes `uncertain`, including the local rate-limit gate, a missing `gh` token, refused connections and 4xx answers. Edit/Post disappear and only Discard is left.
  - Recommendation: Return to `draft` for errors before sending and for 4xx; keep `uncertain` only for timeouts, lost connections and 5xx.
  - Alternative: Add a "Return to draft (I checked GitHub)" button.
- **[Medium] Inbox poll refetches up to 15 item details over GraphQL every minute** — `crates/core/src/inbox.rs:287-313`, `crates/core/src/items.rs:119-132`
  - Bug: The prefetch runs on every poll, even after a 304, and every cache entry is older than its 60 s freshness. This uses about 18% of the hourly GraphQL budget while idle.
  - Recommendation: Prefetch only threads whose `updated_at` changed.
  - Alternative: Skip the prefetch when the notifications page returned 304.
- **[Medium] Full inbox sync checks every subscription while holding the inbox lock** — `crates/core/src/inbox.rs:176`, `inbox.rs:197`, `inbox.rs:236-253`
  - Bug: Every 30 minutes all history threads get one sequential request under `self.lock`, so Done/Read clicks hang. One non-404 error (for example SAML 403) aborts the whole sync.
  - Recommendation: Expire subscription cache entries only when `updated_at` changes, and skip per-thread errors.
  - Alternative: Run the checks outside the lock and apply the results in one short locked step.
- **[Medium] PR diff fails instead of using the API fallback** — `crates/core/src/items.rs:512-519`
  - Bug: The remote lookup is case-sensitive, and its `?` is outside the fallback block. A case difference or an unreachable host makes the Files changed tab fail with "missing mapped remote".
  - Recommendation: Compare repository names case-insensitively and move the lookup inside the fallible block.
  - Alternative: Pick the remote from `p.gh_repo`/`p.gh_upstream` directly.
- **[Low] API-fallback diffs are not cached** — `crates/core/src/items.rs:594-597`, `items.rs:625-635`
  - Bug: When the local diff failed, the fallback result is not stored, so each file click repeats a failed fetch and all REST pages.
  - Recommendation: Cache successful fallback results.
  - Alternative: Return single patches from the summary result in memory.
- **[Low] Clicking an image in GitHub HTML opens the daemon `/img` URL** — `crates/core/src/items.rs:466-477`, `web/src/features/github/lib.tsx:51-56`
  - Bug: `proxy_images` also rewrites the `<a href>` around images, so the external browser opens `http://127.0.0.1:7317/img?...` without a cookie.
  - Recommendation: Rewrite only `src` attributes.
  - Alternative: Map `/img?url=` links back to the original URL in the click handler.
- **[Low] Posting a suggestion holds the global pre-draft lock during network calls** — `crates/core/src/suggestions.rs:179`, `suggestions.rs:230-255`
  - Bug: GitHub posts run a full item refresh and the POST under `predraft_lock`, which blocks pre-draft launches and publication for tens of seconds.
  - Recommendation: Use a per-suggestion lock for GitHub targets.
  - Alternative: Claim `posting` under the lock, then release it before the network calls.
- **[Low] Review suggestions with an uppercase `--head` are always stale** — `crates/core/src/suggestions.rs:120-125`
  - Bug: Uppercase hex is accepted but compared exactly with GitHub's lowercase head, so Post stays disabled.
  - Recommendation: Lowercase `head` in `create`.
  - Alternative: Reject uppercase input with a clear error.
- **[Low] The proxied image cache grows without limit** — `crates/core/src/items.rs:359-418`
  - Bug: Every proxied GitHub image is stored in `<data_dir>/images` forever.
  - Recommendation: Delete entries older than a few weeks at startup, like uploads.
  - Alternative: Cap the folder size and remove the oldest files.

## 7. Browser-side state and settings

- **[Medium] Unsent GitHub reply drafts are lost when a save fails** — `web/src/lib/reviewState.ts:26-35`
  - Bug: A failed PUT is only logged and the pending value is dropped, so the latest reply text is lost on reload or on other devices.
  - Recommendation: Keep failed values pending, retry on reconnect, and show a "not saved" state.
  - Alternative: Mirror pending values in localStorage until the daemon confirms them.
- **[Low] Message drafts and pins overwrite each other across windows** — `web/src/stores/messages.ts:23-35`, `web/src/stores/layout.ts:35-53`
  - Bug: Each window writes its whole persisted state, so an older window removes pins or unsent replies made in another one.
  - Recommendation: Rehydrate on `storage` events or merge per key on write.
  - Alternative: Store each draft and pin under its own key.
- **[Low] Settings → General saves a stale full config** — `web/src/features/settings/SettingsView.tsx:79-96`
  - Bug: The page keeps its first config snapshot and saves all sections, reverting changes made elsewhere (for example `worktrees.keep` or the adopted HTTPS origin).
  - Recommendation: Send only the edited fields merged on top of the latest config.
  - Alternative: Add a config revision and reject stale saves with 409.
- **[Low] Browser data stays after Sign out or revocation** — `web/src/features/settings/DevicesView.tsx:35`, `web/src/lib/api.ts:30`, `web/src/lib/offlineCache.ts:7-8`
  - Bug: IndexedDB (prompts, messages, paths, inbox) and localStorage drafts stay readable after Sign out or a 401.
  - Recommendation: Delete `launchpad-cache` and `launchpad-*` keys on Sign out and on 401.
  - Alternative: Clear client storage when `/auth` loads without a valid credential.

## 8. Remote-mode policy and desktop shell

- **[Low] Project worktree setup command is editable from remote browsers** — `crates/core/src/projects.rs:223-225`, `crates/core/src/worktrees.rs:383-400`
  - Bug: `PATCH /api/projects/{id}` stores `worktree_setup` with no remote check, although commands must be file-only in remote mode. It is also the target used by the relay bypass in group 1.
  - Recommendation: Reject `worktree_setup` changes when `auth.remote()` is true and show it read-only.
  - Alternative: Move setup commands into `config.toml`.
- **[Low] Desktop navigation guard uses a string prefix** — `desktop/src-tauri/src/main.rs:247-248`
  - Bug: `url.starts_with("http://127.0.0.1:7317")` also accepts `http://127.0.0.1:7317@evil.example/` and `http://127.0.0.1:73170/`, so such a link loads a foreign page inside the app window.
  - Recommendation: Compare `url.origin()` with the daemon origin.
  - Alternative: Parse the host and port and compare them exactly.
- **[Low] Unauthenticated reverse-pairing start has one global rate limit** — `crates/server/src/auth.rs:637-650`
  - Bug: Any client that can reach the front door can use the 20/minute and 100 pending slots, so the owner cannot create a reverse-pairing code.
  - Recommendation: Rate-limit per source address and keep a smaller global limit.
  - Alternative: Let QR/link pairing stay the documented fallback when reverse pairing is saturated.

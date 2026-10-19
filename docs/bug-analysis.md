# Launchpad bug and vulnerability analysis (updated 2026-10-19)

Scope: the full codebase at `8101446` (Rust daemon, React UI, Tauri shell). This update replaces the first analysis of 2026-10-17 (commit `48a147f`). It does two things: it checks each of the 56 earlier findings against the 18 commits made since then, and it reviews those commits for new bugs. Source tags: **[New · commit]** a bug introduced by that commit, **[Remaining]** the part of an earlier finding that is still open, **[Older]** an older bug that was not in the first report.

Severity levels: **Critical**, **High**, **Medium**, **Low**. No critical issue was confirmed. Bugs are grouped so that each group can be fixed in one change.

Totals: 27 bugs in 7 groups: 0 critical, 1 high, 8 medium, 18 low.

## Status of the 2026-10-17 findings

50 of the 56 earlier findings are fixed and 6 are partially fixed; none is left untouched. The open parts of the 6 partial fixes are listed below with the **[Remaining]** tag: Codex archive cleanup, launch-time pre-draft subfolder checks, the symlink at the destination file name, apps on other ports of the Tailscale Serve host, the retry ledger within 24 hours, and per-tab draft backups. All six High findings of the first report are fixed: the relay allowlist bypass, the pre-draft deadlock, the pre-draft subfolder guards, merges that dropped merge commits, the quota pause during worktree setup, and Launch/Send on plain HTTP. The fallback SHA-256 was compared with Node `crypto` from 0 bytes to 3 MB, including non-ASCII input, and no new relay path bypass was found (percent-encoding, `;`, query strings, double slashes, case, `//host`). One small gap remains in the Sign out cleanup: `web/src/lib/attachments.ts:74` redirects on a 401 without clearing storage, but the socket probe normally detects the revocation first.

## 1. Global locks held during slow or unbounded waits

Several fixes now hold the global session action lock (or the global operation lock) while they wait for a process, a network call or a turn. Stop, Send, Launch, Answer and the quota guard all need that lock, so one slow wait blocks every session and can let protected sessions keep using paid credits.

- **[High] Stop or Archive of an agent on an unreachable SSH host blocks every session action** [New · e575aee] — `crates/core/src/harness/claude.rs:249-251`, `crates/core/src/worktrees.rs:452-464`, `crates/core/src/hosts/conn.rs:216-251`, `crates/core/src/sessions.rs:1302-1314`
  - Bug: Claude `stop` and worktree setup cancellation now wait with no limit for the process to exit after the kill. For a process on an SSH host, the signal is only queued while the host is unreachable, and the exit wait ends only when the connection returns or the host is removed. Archive, Stop during setup, discard, model restart and Open in terminal run this wait while they hold the global action lock. If the host stays offline (laptop closed, cancelled sandbox), every send, launch, stop and answer waits forever, and the quota guard cannot pause other protected sessions when included quota runs out.
  - Recommendation: Bound the wait after the kill (for example 5 s), finish the wait in a background task, and keep the worktree out of cleanup until the exit arrives.
  - Alternative: For processes on an Unreachable host, skip the wait and mark the worktree busy until the helper reports the exit or the process is lost.
- **[Medium] Pre-draft Publish holds the action lock during push and GitHub calls** [New · 951999c] — `crates/core/src/predrafts.rs:550-552`, `predrafts.rs:615`, `predrafts.rs:642`
  - Bug: To fix the deadlock, `publish` now takes the action lock first and keeps it through `git ls-remote`, `git push` and the GitHub requests, which have no timeout. A slow or stalled push blocks Stop for every session and blocks the quota guard. Before this commit, Publish did not take that lock.
  - Recommendation: Hold the action lock only for the idle checks and the `pushed_head` claim, release it before the network calls (the stored `pushed_head` already freezes new turns), and take it again for the final cleanup.
  - Alternative: Keep the order but add timeouts to `ls-remote`, `push` and the GitHub requests, and fail with a retryable error.
- **[Medium] Cancelling worktree setup can hang forever when a child keeps the output pipe open** [New · e575aee] — `crates/core/src/worktrees.rs:448-464`, `crates/core/src/exec.rs`
  - Bug: After killing the setup process group, `setup_output` waits for the output pipes to close, with no limit. A child started with `setsid` (or a daemonizing tool) escapes the group kill and keeps stdout open (confirmed with a small POSIX demo). Stop and Archive then wait forever under the action lock, and the 30-minute setup timeout hangs the same way, so the session stays in Starting. Before this commit, the timeout always ended the wait.
  - Recommendation: Bound the wait after the kill (for example 5 s), then drop the pipes and return the error.
  - Alternative: Read the pipes in separate tasks and wait for the process exit instead of pipe EOF.
- **[Medium] Changing permissions during a turn stops the task, then fails after 20 s** [New · 4105fdd] — `crates/core/src/sessions.rs:1035-1050`
  - Bug: When a turn is running, `set_permissions` skips the background-command check, interrupts the turn and waits up to 20 s for the process to be ready. Codex commands and Claude background tasks survive the interrupt, so the wait times out: the session ends Stopped, the mode is unchanged and no "Continue" message is sent. During the 20 s, the global action lock is held, so Stop, answers, launches and quota pausing are blocked for every session.
  - Recommendation: Check for background work outside the current turn before interrupting, and release the action lock during the wait, then take it again and re-check the state.
  - Alternative: When the wait times out, still save the new mode and apply it at the next restart, instead of reporting the change as failed after the turn was stopped.
- **[Low] Manual GitHub posts block Launch and Send** [New · de26c31] — `crates/server/src/operations.rs:71`, `operations.rs:89-98`
  - Bug: Comments, reviews and inbox replies now run under the single global operation lock that also protects Launch and session messages. A slow GitHub POST (up to the 30 s timeout) makes every Launch and every reply to an agent, from any device, wait behind it.
  - Recommendation: Hold the global lock only around the claim and the completion records; retries with the same ID already see the stored claim.
  - Alternative: Use one lock per operation ID instead of one global mutex.

## 2. Agent process shutdown and harness protocol

- **[Medium] On macOS, agents keep running after a daemon restart** [New · e575aee] — `crates/core/src/exec.rs:221-222`, `crates/core/src/harness/claude.rs:105`, `packaging/install-macos.sh:31`
  - Bug: Claude and worktree setup now start in their own process group on every Unix system. Linux still stops Claude through the parent-death signal, but macOS has none; before this change, launchd killed the daemon's whole process group when it stopped. Now a running Claude turn keeps working in the worktree while the session shows Interrupted, and Resume starts a second process on the same conversation and folder.
  - Recommendation: On macOS, record the process groups of harness and setup processes, and kill them at daemon shutdown and at startup (stale groups from the previous run).
  - Alternative: On macOS, use a separate process group only for worktree setup and keep harnesses in the daemon's group.
- **[Medium] A failed Codex steer marks the session as Failed** [New · e575aee] — `crates/core/src/harness/codex.rs:216-224`, `crates/core/src/sessions.rs:1229-1232`
  - Bug: Every `turn/steer` error now goes to `sink.fail`, which sets status Failed and sends a failure notification. If the owner types just as the turn ends, Codex rejects the steer, the message shows as sent but was never delivered, and the session becomes Failed. If the steer request times out, the session shows Failed while Codex is still working.
  - Recommendation: When Codex reports that no turn is active, start a new turn (nothing was delivered). For other steer errors, return an error without `sink.fail` and show a notice that delivery is uncertain.
  - Alternative: Return a dedicated "uncertain delivery" error that `send_locked` reports as a notice without changing the status.
- **[Medium] Archive of a Codex session does not wait for its turn or commands** [Remaining] — `crates/core/src/harness/codex.rs:311-330`, `crates/core/src/sessions.rs:1293-1299`
  - Bug: The Claude part of the earlier finding is fixed, but Codex `stop` still sends `turn/interrupt` and `thread/unsubscribe` and returns at once. Archive cleanup can then remove a clean worktree while an interrupted or background Codex command still writes into it.
  - Recommendation: In Codex `stop`, wait (with a bound) for `turn/completed` and for running commands to finish before unsubscribing.
  - Alternative: Keep the worktree out of cleanup until the thread reports no active turn and no running commands.
- **[Low] A request cancelled by the harness leaves a running session shown as Done** [New · 612f64f] — `crates/core/src/harness/claude.rs:406`, `crates/core/src/harness/codex.rs:598`
  - Bug: Showing an approval card sets the status to Waiting. When Claude cancels the request (`control_cancel_request`) or Codex resolves it (`serverRequest/resolved`) during the turn, the open question is cleared but the status is not set back to Working. The session shows Done while the agent works, the quota guard skips it, and the completed-turn notification is lost. It is not confirmed when Claude cancels a request without also ending the turn.
  - Recommendation: After removing a cancelled request, set Working again when no question remains and the turn is still active, like `Sessions::answer`.
  - Alternative: Keep Working for permission cards and let `open_question` drive Needs you.
- **[Low] Summary shows requests from before a daemon restart as active cards** [New · 612f64f] — `web/src/features/agents/SessionSummary.tsx:121`, `SessionSummary.tsx:154`
  - Bug: A daemon restart does not mark pending requests as ended. After Resume and a new request, "Needs your decision" shows the old card and the new one, both with live Allow/Deny buttons. Clicking the old card fails with "already answered", and the owner can click the wrong one.
  - Recommendation: At startup, emit `AskAnswered` ("Request ended") for unanswered requests of sessions that were live.
  - Alternative: Show only requests made after the last process start as actionable.

## 3. Worktrees, pre-drafts and merges

- **[Low] Launch still matches pre-draft sessions by exact folder** [Remaining] — `crates/core/src/sessions.rs:450`, `sessions.rs:554`, `sessions.rs:613`
  - Bug: The pre-draft guards now accept subfolders, but launch still compares `cwd == root`. For a project in a repository subfolder, a follow-up from the pre-draft session does not resolve to ExistingPredraft (no pre-draft instruction, and "Send notes to agent" still targets the old session), and the "already has an active agent" check misses a running session, so a second agent can start in the same worktree.
  - Recommendation: Use `loc::within` for the pre-draft lookup in preview and launch, and for the active-agent check.
  - Alternative: Reuse `predrafts::idle()` for the ExistingPredraft guard.
- **[Low] Copied include files are still written through a symlink at the file name** [Remaining] — `crates/core/src/worktrees.rs:324`, `worktrees.rs:340`, `worktrees.rs:607`
  - Bug: Parent folders are now checked, but `copy_files` skips a destination only when `exists` is true, and `exists` follows symlinks. A dangling symlink at the file name passes, and the copy writes through it. If an agent commits `.env` as a symlink to another path, Resume after cleanup copies the owner's ignored `.env` there.
  - Recommendation: Use `symlink_metadata` and skip or refuse when anything exists at the destination, including a symlink.
  - Alternative: Open the destination with `O_NOFOLLOW | O_EXCL`, also in the helper's copy operation.
- **[Low] Automatic merge-conflict messages use the manual quota path** [New · 4894ec6] — `crates/core/src/worktrees.rs:913-917`, `crates/core/src/sessions.rs:1196-1212`
  - Bug: The background merge retry sends conflict messages as manual follow-ups. With an unknown or stale quota reading they are sent anyway, although the project rules allow that only for manual work. A conflicting `launchpad work finish` can start a new turn while included quota may be exhausted.
  - Recommendation: Send these messages with `automatic = true` and keep the conflict pending when the send is held.
  - Alternative: Hold the conflict message with `hold_input` when the reading is not fresh.

## 4. SSH hosts and remote access

- **[Low] Relayed CLI calls are not limited to the sending host** [Older] — `crates/core/src/hosts/relay.rs:19-20`, `crates/server/src/api.rs:344-345`, `crates/server/src/predrafts.rs:33-39`
  - Bug: The relay path check is now strict, but IDs in paths and bodies are not tied to the host. A process on an SSH host can list local pre-drafts to get their session IDs, read those sessions' traces, and rewrite the title, body or notes of a local pre-draft. The notes are later sent to a local agent, and the body is published by the owner.
  - Recommendation: In `relay::call`, require that the session or pre-draft of each relayed request has its folder on the sending host.
  - Alternative: Filter relayed pre-draft lists to the host's own records and reject trace and PATCH requests for records on another machine.
- **[Low] Apps on other ports of the Serve host still receive the device cookie** [Remaining] — `crates/server/src/tailnet.rs:99-115`, `crates/server/src/auth.rs:603`
  - Bug: Apps on the same Serve origin are now rejected, but cookies have no port scope. If the owner serves another app with `tailscale serve --https=8443`, the browser sends the Launchpad device cookie to it, and that app's backend can reuse it for full Launchpad access.
  - Recommendation: During discovery and Update HTTPS URL, reject the host when another HTTPS port on the same DNS name proxies to something other than this daemon.
  - Alternative: Warn in Settings → Devices and require explicit owner confirmation when other ports are served on the same host.

## 5. Integrated terminals

- **[Low] Every terminal Close waits 5 s, and folder cleanups can fail** [New · 2582a97] — `crates/core/src/terminals.rs:59-62`, `terminals.rs:403-407`, `terminals.rs:313-324`, `crates/core/src/hosts/helper.rs:449-452`
  - Bug: The new escalation always sleeps 5 s before SIGKILL, and the exit is reported only after it, so every closed tab stays open for 5 s. With a disowned background job holding the PTY, the exit lands at about 6 s, which is the `close_and_wait` deadline, so Close all and the project, promotion and pre-draft cleanups can fail with "a terminal did not exit".
  - Recommendation: Poll the captured process groups every 100 ms, return as soon as they are gone, and send SIGKILL only to groups that still exist after 5 s.
  - Alternative: Give `close_and_wait` a deadline longer than the kill delay plus the drain time (for example 8 s).
- **[Low] Typing into an exited terminal shows an error and reconnects on every key** [New · 2582a97] — `crates/server/src/terminals.rs:218-223`, `crates/core/src/terminals.rs:271-273`, `web/src/features/terminals/TerminalView.tsx:222-233`
  - Bug: Failed shells and harness TUIs keep their tab after exit. Each key press there now fails the write, shows an error toast, closes the socket and replays the whole screen. Before, the input was ignored.
  - Recommendation: Close the viewer only when the input queue is full; ignore input for an exited terminal.
  - Alternative: Disable xterm input after the `exit` control message.
- **[Low] Large pastes are cut and leave bracketed paste open** [New · 2582a97] — `crates/core/src/terminals.rs:26-35`, `crates/server/src/terminals.rs:218-223`, `web/src/features/terminals/TerminalView.tsx:129-133`
  - Bug: The browser sends a paste as 64 KiB frames, and the new input queue holds 4 frames. When the process reads slowly, a paste over about 320 KiB overflows the queue, the viewer is closed, the rest of the paste is lost and the end marker is never sent, so the shell or TUI stays in paste mode.
  - Recommendation: Apply backpressure per viewer (stop reading that viewer's socket while its frame waits) instead of failing, without blocking pings and ACKs.
  - Alternative: Refuse pastes above a limit (for example 256 KiB) in the browser with a visible message.

## 6. Browser drafts, retries and GitHub diffs

- **[Medium] "Load newer" on a conflicted review draft keeps the old text, and the next edit overwrites the newer draft** [New · 12c31da] — `web/src/lib/reviewState.ts:64-68`, `reviewState.ts:173-183`
  - Bug: Load newer clears the pending value and writes the server value into the query cache, but the cache already holds that same value, so the reference does not change and the text box keeps the local text. The banner disappears, and the next keystroke saves on top of the newer revision, which silently overwrites the other device's draft.
  - Recommendation: In Load newer, set the shown value directly (for example with a per-key reload counter), instead of relying on a changed query reference.
  - Alternative: Track the last applied revision separately and apply the cached value whenever pending is cleared and the revisions differ.
- **[Low] Unsaved draft backups are lost when a tab closes** [Remaining] — `web/src/lib/reviewState.ts:20-23`, `web/src/stores/composer.ts:98-101`
  - Bug: Failed saves are now kept and mirrored in localStorage, but under a per-tab key. If the owner closes the tab while offline, no other tab reads that backup, so the edit is lost. A duplicated tab copies the same tab ID, and one tab can erase the other's backup.
  - Recommendation: Store one backup entry per draft key and let any tab retry it; create a new tab ID when sessionStorage was copied.
  - Alternative: Keep one shared backup map in localStorage, merged per key with a revision.
- **[Low] A save whose response was lost shows a false conflict** [New · 12c31da] — `web/src/lib/reviewState.ts:99-104`, `web/src/stores/composer.ts:110-142`
  - Bug: After a reload or a network drop, the backup is replayed with its old base revision. If the first save had reached the daemon, the replay gets 409, and the owner sees "changed elsewhere" for their own edit; the composer pauses saves until they choose.
  - Recommendation: On 409, fetch the stored value and accept it silently when it equals the pending value.
  - Alternative: Send a write ID with each save and let the daemon answer success for a repeated ID.
- **[Low] Two windows on the same workspace now mirror each other's selection and dock** [New · 12c31da] — `web/src/stores/layout.ts:39-43`, `layout.ts:64-65`
  - Bug: The pin fix reloads the whole stored layout on every storage event, including the selected session and the terminal dock. Selecting a session or opening the dock in one window changes the other window too.
  - Recommendation: Merge only pins and collapsed groups from storage; keep selection and dock per window.
  - Alternative: Store shared and per-window layout fields under separate keys.
- **[Low] Identical messages within 24 hours still replay an old uncertain send** [Remaining] — `web/src/lib/operations.ts:32-40`, `web/src/features/agents/GitPanel.tsx:106-113`
  - Bug: The ledger key is still a hash of path and body. After a lost response, sending the same text again within 24 hours (for example "yes") reuses the operation ID, and the UI shows "Delivered" although nothing reached the agent. The Changes notes sender has no "start a new attempt" button, so after 24 hours the same notes cannot be sent until the text changes.
  - Recommendation: Create the operation ID when the message is composed and keep it with the draft; add the abandon button to the Changes notes sender.
  - Alternative: Clear a session's ledger entries when its reply box is emptied or a later message is delivered.
- **[Low] One short local diff failure makes the API fallback permanent for that PR head** [New · de26c31] — `crates/core/src/items.rs:523-531`, `items.rs:617-622`
  - Bug: The fallback diff is now cached under the project key with no expiry. One transient `git fetch` failure makes every later Files changed view of that head use the REST result, which misses patches for large files or PRs with many files.
  - Recommendation: Cache fallback results only under the `api` key and keep retrying the local diff.
  - Alternative: Store the fallback with a timestamp and retry the local diff after a few minutes.

## 7. Harness variants

- **[Medium] The session model switch breaks provider-variant sessions** [New · 8101446] — `web/src/features/agents/SessionSummary.tsx:253-281`, `crates/core/src/sessions.rs:1066-1093`
  - Bug: Summary → Run always lists the base Claude models and efforts; only the composer and the variant editor use `models_url`. On a Hugging Face variant session, picking `opus` restarts Claude with `--model opus`, `${LAUNCHPAD_MODEL}` expands to `opus`, the provider rejects it and every following turn fails. The owner also cannot switch to another provider model.
  - Recommendation: In the model switch, use the session's variant: show the `models_url` list and hide efforts, like the composer.
  - Alternative: Reject model changes on the server when the model is not in the variant's list, or hide the switch for these sessions.
- **[Low] A literal `${` in a variant value invalidates the whole config** [New · 8101446] — `crates/core/src/config.rs:219`, `crates/core/src/variants.rs:47-62`, `config.rs:258`
  - Bug: Validation now rejects any `${…}` that is not a plain name, for example `${FOO:-bar}` or a password that contains `${`, which were valid before. At startup the daemon then falls back to the default config, so all variants and presets disappear until the file is fixed. There is no escape syntax for a literal `${`.
  - Recommendation: Add an escape (for example `$${`) and document it next to the `${NAME}` help text.
  - Alternative: Expand only the exact `${NAME}` form and leave other `${…}` text unchanged.
- **[Low] `~/` in variant values uses the local home folder on SSH hosts** [Older] — `crates/core/src/variants.rs:38`, `crates/core/src/harness/claude.rs:808`, `crates/core/src/harness/codex.rs:895`
  - Bug: `~/` is expanded with the daemon machine's home before the host check. The new account templates (`CLAUDE_CONFIG_DIR=~/.claude-work`, `CODEX_HOME=~/.codex-work`) depend on it, so on a host with a different home the harness gets a folder that does not exist there and starts without the expected sign-in.
  - Recommendation: For host sessions, expand `~/` with the host's home folder, which the host connection already resolves.
  - Alternative: Refuse `~/` values for host sessions with a clear message, like `${NAME}`.

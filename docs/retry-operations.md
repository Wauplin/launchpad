# Retry-safe Launch and Send

Owner-approved scope (2026-10-13): fix the blocker in section D, “Unreliable networks,” of the previous server-hosting review. An immediate launch or session message must not start another agent, create another worktree, or deliver another input when the browser retries after losing the reply. Show Sending and Delivered in the UI.

## Contract

`POST /api/sessions` and `POST /api/sessions/{id}/input` accept an optional UUID `Idempotency-Key` header. The UI generates it before sending and saves a request fingerprint, ID and creation time in browser local storage. On plain HTTP origins without SubtleCrypto or randomUUID, the browser uses an equivalent SHA-256 fingerprint and a getRandomValues-based UUID v4; retry protection remains enabled. Retrying an unchanged request reuses the ID, including after reload. A changed payload is a new operation. Pending IDs may be retried for 24 hours. Older entries, entries dated in the future, malformed entries and legacy IDs without a creation time block submission until the owner inspects Agents or the trace and explicitly selects “start a new attempt.” Expiration never silently generates a new ID or deletes an uncertain claim. This prevents an old identical message from silently replaying a previous result while preserving at-most-once dispatch. No prompt or attachment bytes are stored in this browser ledger. Storage failures prevent sending.

The authenticated API middleware claims the ID in `state.db` before entering the handler. Concurrent requests with the same ID wait for the first handler and reuse its stored HTTP status and response body. Reusing an ID with a different path or body returns 409. Launch uses the ID as its session ID; input uses it as the user-message `client_id`. Clients without a key retain the existing API behavior.

After durably storing the response, the daemon returns `Idempotency-Status: completed`, including for recorded failures. The browser clears its pending ID only after receiving that marker. A subsequent click is then a new attempt. Recorded failures remain replayable using the old ID, and the message box tells the owner to inspect the trace before a new attempt. A lost response or an unconfirmed failure retains the ID and the draft.

A claim without a stored result survives daemon restart. Its retry returns 409 explaining that delivery is uncertain and requiring inspection of the session list or trace. Never automatically re-execute it: the harness may already have received the message. After inspecting Agents or the trace, the owner can explicitly select “I checked … — start a new attempt” to replace the browser ID. This provides at-most-once dispatch, not transactional exactly-once harness execution or automatic crash recovery. Claims and results are retained rather than expired into duplicate execution.

## Acceptance and limits

Automated checks use an isolated HTTP server and throwaway databases, without real harnesses or GitHub mutations:

- Discard a launch response, race retries, and reopen the database/server: the handler runs once and replies contain the original result.
- Replay a session input and a recorded error: the handler runs once and the original status/body return.
- Reject changed payloads and invalid IDs; keep a crash-interrupted claim blocked.
- Simulate a lost browser reply: the retry carries the same ID; a completed response allows a later intentional identical message; unconfirmed errors keep their ID.
- Exercise plain-HTTP crypto fallbacks, compare fallback hashes with native SHA-256 across block boundaries and large payloads, and verify expiration blocks fetch until explicit abandonment (including legacy entries and late responses).
- Verify that storage failure prevents submission, and run UI typecheck/build plus strict workspace Clippy and Rust tests.

Checked on 2026-10-13: all 21 Rust tests, the six browser retry tests on Node 24.14.1, UI typecheck/build, strict pinned-toolchain workspace Clippy (including desktop), and `git diff --check` passed. No live harness or phone deployment was exercised.

Checked for group 7 on 2026-10-19: all 10 browser retry tests on Node 24.14.1, UI typecheck/build, strict pinned-toolchain workspace Clippy and `git diff --check` passed. HTTP behavior was simulated with secure-context crypto methods disabled; no live phone or harness was exercised.

Commands: `cargo test --locked`, `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings`, and `(cd web && pnpm test:operations && pnpm typecheck && pnpm build)`.

Resume now uses the dedicated `/resume` endpoint rather than a fixed-text input, so it does not use this ledger. Its previous stale-input retry problem no longer applies.

Scope excludes scheduled/cron creation, answers, Stop, WebSocket replay, terminal reconnection, general reply-draft persistence, mobile layout, authentication and Web Push. The browser retry ledger is per origin; changing origin or clearing browser storage loses pending IDs. Real phone/network and harness acceptance remain daily-use checks. The desktop daemon and notification behavior are unchanged.

## GitHub publication protection (report group 10, 2026-10-19)

Manual `POST /api/items/{owner}/{repo}/{number}/comment`, `/review` and `/api/inbox/{id}/reply` require a UUID `Idempotency-Key` and use the same durable claims and browser fingerprint ledger. Missing keys are rejected before publication. Completed posts replay the stored response after a lost browser reply or restart. Definite pre-send errors and 4xx rejections are recorded failures, allowing a subsequent deliberate click to retry. An ambiguous transport failure, 5xx, timeout or unreadable success body leaves the claim unresolved and the browser ID intact. Subsequent requests with that ID return 409 without contacting GitHub. API errors carry `GitHub-Delivery: uncertain` to preserve this distinction through the middleware. Authentication still runs first.

The comment/review and inbox reply controls offer “I checked GitHub — start a new attempt” after a failed submission, with a confirmation explaining the duplicate risk. This removes only the browser ID; the daemon never expires unresolved claims into new posts. Changed payloads are separate operations. Protection remains per browser origin and fingerprint; clearing storage, changing origin or replacing a pending attempt after checking GitHub requires owner judgment. Agent suggestions retain their own durable posting state and per-suggestion locks.

Loopback-only regression checks cover all three publication routes, missing keys, replayed successes, uncertain failures, and reopening the ledger. Browser checks cover retention of publication IDs. No real GitHub posts are made during these tests.

Validation for group 10: all 82 Rust tests, all 11 browser operation tests on Node 24.14.1, UI typecheck/build, strict pinned-toolchain workspace Clippy (all targets, including desktop), and `git diff --check` passed. Live GitHub publication and device acceptance remain daily-use checks.

## Independent operations (report group 1, 2026-10-20)

The middleware holds a per-operation-ID lock through dispatch and durable completion. Concurrent retries of one ID still replay its completed response, while unrelated Launch, Send and GitHub publication requests proceed independently. Weak lock entries are reclaimed after their last user leaves. Claims left unresolved after a crash or uncertain delivery remain blocked.

Validation for group 1: all 89 Rust tests, all 11 browser operation tests on Node 24.14.1, UI typecheck/build and `git diff --check` passed. Regression coverage includes independent operation IDs, background shutdown ownership, commands retained across turns and an escaped setup descendant retaining an output pipe. Live SSH reconnection and GitHub publication were not exercised.

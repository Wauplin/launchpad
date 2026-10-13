# Retry-safe Launch and Send

Owner-approved scope (2026-10-13): fix the blocker in section D, “Unreliable networks,” of the previous server-hosting review. An immediate launch or session message must not start another agent, create another worktree, or deliver another input when the browser retries after losing the reply. Show Sending and Delivered in the UI.

## Contract

`POST /api/sessions` and `POST /api/sessions/{id}/input` accept an optional UUID `Idempotency-Key` header. The UI generates it before sending and saves a request fingerprint and ID in browser local storage. Retrying an unchanged request reuses the ID, including after reload. A changed payload is a new operation. No prompt or attachment bytes are stored in this browser ledger. Storage failures prevent sending.

The authenticated API middleware claims the ID in `state.db` before entering the handler. Concurrent requests wait for the first handler and reuse its stored HTTP status and response body. Reusing an ID with a different path or body returns 409. Launch uses the ID as its session ID; input uses it as the user-message `client_id`. Clients without a key retain the existing API behavior.

After durably storing the response, the daemon returns `Idempotency-Status: completed`, including for recorded failures. The browser clears its pending ID only after receiving that marker. A subsequent click is then a new attempt. Recorded failures remain replayable using the old ID, and the message box tells the owner to inspect the trace before a new attempt. A lost response or an unconfirmed failure retains the ID and the draft.

A claim without a stored result survives daemon restart. Its retry returns 409 explaining that delivery is uncertain and requiring inspection of the session list or trace. Never automatically re-execute it: the harness may already have received the message. After inspecting Agents or the trace, the owner can explicitly select “I checked … — start a new attempt” to replace the browser ID. This provides at-most-once dispatch, not transactional exactly-once harness execution or automatic crash recovery. Claims and results are retained rather than expired into duplicate execution.

## Acceptance and limits

Automated checks use an isolated HTTP server and throwaway databases, without real harnesses or GitHub mutations:

- Discard a launch response, race retries, and reopen the database/server: the handler runs once and replies contain the original result.
- Replay a session input and a recorded error: the handler runs once and the original status/body return.
- Reject changed payloads and invalid IDs; keep a crash-interrupted claim blocked.
- Simulate a lost browser reply: the retry carries the same ID; a completed response allows a later intentional identical message; unconfirmed errors keep their ID.
- Verify that storage failure prevents submission, and run UI typecheck/build plus strict workspace Clippy and Rust tests.

Checked on 2026-10-13: all 21 Rust tests, the six browser retry tests on Node 24.14.1, UI typecheck/build, strict pinned-toolchain workspace Clippy (including desktop), and `git diff --check` passed. No live harness or phone deployment was exercised.

Commands: `cargo test --locked`, `cargo +1.93.1 clippy --workspace --all-targets --locked -- -D warnings`, and `(cd web && pnpm test:operations && pnpm typecheck && pnpm build)`.

Scope excludes scheduled/cron creation, answers, Stop, WebSocket replay, terminal reconnection, general reply-draft persistence, mobile layout, authentication and Web Push. The browser retry ledger is per origin; changing origin or clearing browser storage loses pending IDs. Real phone/network and harness acceptance remain daily-use checks. The desktop daemon and notification behavior are unchanged.

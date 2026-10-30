# Acceptance and remaining work

Source: [technical report revision 3](https://wauplin-launchpad-spaces-technical-report.static.hf.space), previous Launchpad trace `24f6f408-46f8-4d67-9bc9-195f7c810a0f`, through event 128. The implementation is partial. A library helper or authored UI does not satisfy a live platform acceptance gate.

## P0 — platform contract (blocked)

Read-only inspection on 2026-10-31 confirmed the protected static report's normal SpaceInfo response is ambiguous. The API rejects explicit visibility/protected expansions. No private Docker Space, paid hardware, bucket, secret or credential was created or modified. Need an authoritative three-state visibility source, or approval of a revised startup acceptance mechanism, followed by anonymous/collaborator checks on Hub and direct app URLs for HTTP, bus and terminal sockets. Verify browser OAuth callbacks, outbound ports and secret-update restarts using synthetic credentials. Keep one machine; distributed leases and visibility polling remain outside scope.

## P1 — gateway (library implemented, integration pending)

Before wiring the proxy into the supervisor, prove separate control-plane and agent UIDs plus procfs/environment isolation in HF's container runtime. Give agents their own stable home and a strictly allowlisted environment. The management token, vault root and platform secrets must never enter the daemon environment. The daemon bearer is a same-user trust limitation in Launchpad; keep supervisor secrets inaccessible regardless. PID 1 must reap and forward signals. Bind the daemon to loopback, configure the exact external public origin and disable auto-tailnet. Do not add an inference-based pairing bypass.

Add generic deployment capabilities to hide Devices/push/local desktop actions while preserving the restricted Ask CLI. Validate deep links, back/forward, uploads/downloads and keyboard focus before enabling the shell iframe. The current shell does not mark any runtime ready or offer fake setup actions. Adapter mutations need shell-issued CSRF tokens in addition to exact Origin checking, bounded bodies and fixed operations.

## P2 — recovery (primitives implemented, orchestration pending)

The existing exporter and prefix are ready for integration. Validate private Hub viewer rendering for native Claude/Codex JSONL; do not promise OpenCode viewer support.

Build schema-specific projectors for state/config/native resume data. Extract profile environments, known credential fields and registered secret workspace files into the vault and resolve versioned references on restore. Classify credential-helper configuration and Git history deliberately. Reject unknown secret-bearing schemas. Never upload live/raw databases, arbitrary home contents or unclassified archives. Preserve native non-secret resume state and stable Git/worktree paths.

Wire file-event credential tracking plus periodic reconciliation, stable-write debounce, deletion tombstones and coherent per-provider generations. Target stable credential writes within 5 seconds, state every minute and workspaces every 5 minutes. Ordinary snapshots are crash-consistent; clean checkpoints must quiesce writers. The bucket commit helper verifies bytes but does not implement this coordination.

Add registered-root atomic restore with traversal/symlink checks, ownership and 0700/0600 modes, recovery for corrupt or missing objects, lost-key reconnect without losing ordinary data, and a restore drill. Never restore process IDs, internal tokens, device registrations or push subscriptions. Keep separate trace, ordinary recovery and credential statuses. Do not claim the latest trace upload is a recovery point. No pruning or application storage budget cap.

## P3 — browser setup (pending)

Implement restart-safe backup-key creation through HF Secrets and verify injection after restart before accepting credentials. Provide masked token forms and fixed, ephemeral setup processes for Codex device login, Claude remote-browser login, pinned OpenCode provider auth, gh device login, HF agent token and optional SSH generation/import. Reuse Launchpad's detect-first setup/profile endpoints; no duplicate Git identity wizard. Never put setup output in sessions or persistent terminal buffers. Handle expiry, cancellation, reload and reconnect; show signed-in-but-backup-pending until vault acknowledgment. SSH port 22 and the local Chrome extension must not be promised to work remotely.

## P4 — releases and updates (pending)

Produce a reproducible headless image (UI before daemon embedding) and public duplicate template with an immutable image digest and pinned tool versions/SBOM. Keep installation bootstrap separate from adapter and Launchpad release versions. No production template/image exists in this change.

Implement conditional read-only stable-manifest checks at startup and approximately every six hours. Present immutable candidate, tool changes, release notes, compatibility and interruption details. Serialize owner-clicked preparation, require idle agents/terminals and clean Git operations, verify a clean pre-update checkpoint, then patch only the managed reference using expected-parent revision. Preserve user changes. Health-check restart before marking the candidate healthy. Roll back both image and pre-migration data when needed; retain Hub-level instructions if the UI cannot start. Keep restart, rebuild, update, restore and key rotation distinct.

## P5 — deployment and daily use (pending)

Exercise paid CPU with sleep disabled and a 24–72 hour soak, scheduled work with browser closed, quota/cost stops, bucket outages, forced restarts, phones and measured recovery loss/time. Hardware/sleep diagnostics should show actual platform settings; prices and eligibility must be fetched from current HF information. No soak, live login, live bucket restore or real update rollback has been performed.

## Checks performed locally

Pinned Rust 1.93.1 workspace/all-target Clippy passed with warnings denied. Shared TypeScript types regenerated; UI typecheck and production UI build passed. Existing trace regressions passed (9 checks) and tour/demo guardrails passed (4 checks). Python compilation and shell JavaScript syntax passed. The locked shell was exercised over HTTP: status, Host/Origin rejection and CSP checks passed; its server exited and the port was confirmed closed. Manual synthetic checks passed for AEAD round trip/context/corruption rejection, unsafe backup/proxy paths, blocked route families, SQLite online snapshot and VACUUM secret-page removal with the live database preserved. No new unit tests. Browser visual QA was unavailable: computer-use reported no connected apps or browsers. These checks do not prove a private HF deployment or end-to-end restore.

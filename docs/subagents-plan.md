# Subagent monitoring

Owner-selected on 2026-10-11: display running subagents, inspect their traces and nested children, while keeping the session UI minimal.

The owner works through the parent conversation. A Subagents control appears only when the harness reports delegation. It shows a running count or a total, opens a collapsible tree and a read-only trace, and offers parent navigation. Tree rows and traces are virtualized; narrow panels stack them. Child messages stay out of the parent's Summary, Trace and last-message preview. Stored child activity remains readable after reload, archive and daemon restart.

The daemon owns observation and persistence. Child metadata and normalized event envelopes share the root session's existing durable event log and live transport. Native adapters own IDs, ancestry and lifecycle. Claude uses launching tool IDs and streamed child messages; Codex observes native descendant threads and collaboration/activity records, including nested delegation inside code mode; OpenCode negotiates its child-session ACP extension. Older OpenCode versions show the delegation prompt/result and an explicit inner-trace limitation. Missing model/state/trace data is shown as unavailable, without inventing details. Subagents do not become independently counted Launchpad sessions.

Root controls continue to own input, resume, quotas and publication. Native child questions use parent answer controls. Child turns/tools keep readiness and publication/cleanup checks busy; Codex interruption includes observed descendant turns. A native interruption is best effort and must wait for outstanding work to settle. Child usage never overwrites or adds to the root's reported usage. Restart-safe workers, independent child launch/input/billing controls, native history import and GitHub publication are outside this scope.

Acceptance checks:

- Launch native delegation, including a grandchild where the harness allows it. Observe the running state, select each trace, inspect tools/results and navigate to its parent. Completion must come from native lifecycle events, not silence.
- End a parent turn while a child still runs: the root remains busy and publication/cleanup cannot proceed. Stop should interrupt observed descendant turns; readiness remains false while their tools finish.
- Reload the UI and restart the isolated dev daemon: stored child identities, ancestry and traces remain readable. Resume/follow-up uses the same root conversation without duplicate child output or global child sessions.
- Verify parent question routing, streamed text isolation, completed/failed/interrupted states and collapsed descendants. Review narrow/wide panels and all five zoom levels.
- Build/type-check the UI and run strict workspace Clippy. Add no unit tests, post nothing to GitHub, and never restart production during checks.

Validation completed in the isolated dev instance: Codex parent/child/grandchild traces; a parent ending while a child continues (Background until child completion); parent Stop reaching an active child; archive/restart/resume preserving completed state and exactly one stored child message; installed OpenCode delegation prompt/result fallback and root follow-up; Chrome child/parent navigation at 1440 px across all five zoom levels and at 800 px with the default 120% zoom. UI typecheck/build and strict workspace Clippy pass. No new unit tests were added. All 16 existing core tests pass (0.00 s execution). Temporary native sessions and the isolated daemon/browser are cleaned up after inspection.

Initial validation uses isolated dev data and installed native harnesses. Codex's arithmetic parent/child/grandchild smoke check completed with separate child traces. OpenCode 1.18.35 supplies only delegation tool metadata/output and does not allow that child to delegate further; the fallback is therefore necessary. Claude 2.1.296 reports exhausted included quota, so its paid usage remains disabled and live validation is pending. These checks do not replace daily use or native macOS/Windows acceptance.

Protocol evidence: installed Codex app-server TypeScript schema (`codex app-server generate-ts`), [Claude SDK subagent attribution](https://code.claude.com/docs/en/agent-sdk/subagents), and OpenCode’s native [child-session extension](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/acp/child.ts) and [capability negotiation](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/acp/capabilities.ts). Live availability depends on the installed harness version.

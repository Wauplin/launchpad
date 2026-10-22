# Chrome extension E2 — page actions

2026-10-22. Owner-approved [E2](https://wauplin-launchpad-chrome-extension.static.hf.space/index.html#plan), extending [E0/E1](chrome-extension-e0.md). This implements the Work-in-tabs milestone in browser-only sessions; E3 remains deferred.

## Implementation

Read stays the default. The owner upgrades a page chip to Work; ordinary CLI bearer requests cannot approve a review or upgrade a grant. Work follows documents within the current origin. Navigation/open-tab to a different origin has a separate site review and Chrome host-permission request. Unexpected cross-site redirects require an explicit new page grant before reading the destination.

The daemon serializes calls by device/tab. One working grant controls a tab; an explicit owner transfer revokes the previous one. Outline refs include a snapshot nonce and resolve to actual Elements in the isolated world, never page-provided selectors or scripts. Visible/enabled targets are checked, highlighted and fingerprinted again after review. A fixed CDP input command set supplies trusted input; it does not accept arbitrary debugger commands or evaluate page code. Working tabs join a cyan session group. Input brings the granted tab forward so compositor-driven events, including scrolling, complete in Chrome. The agent never follows whichever tab the owner happens to select.

Consequential targets (submission, send/pay/buy/delete/publish/invite and related labels, Enter, payment fields) require independent owner review. Detection is explicitly a heuristic. Proposed forms always require Apply, showing exact proposed values before input. Typing does not press Enter, but may trigger autosave or blur handlers. Password/OTP targets and file selection are refused; handover detaches control for owner sign-in, CAPTCHA, 2FA, uploads or payment, then returns a fresh outline on Continue. Applying several fields is not atomic: partial failure is uncertain and is never rolled back or retried automatically.

Pause, Take over, handover, Stop, turn end, quota pause and disconnection detach the debugger and remove the control banner. The worker checks session state from bus-triggered reads and a five-second fallback poll. Each CDP command is bounded to ten seconds, each broker exchange to 45 seconds, and reviews to ten minutes. Claude/Codex MCP timeouts allow twelve minutes. OpenCode uses its documented-schema `experimental.mcp_timeout` override for the same budget ([schema](https://opencode.ai/config.json)); native ACP/model acceptance remains a daily-use check.

Before dispatch the daemon writes a SQLite `browser_action` claim. The worker requires a live device-bound dispatch authorization and consumes the event ID before input. Successful replies remove the pending broker request before the worker may clear its consumed ID. Socket replay cannot execute an old request. Success updates the document and clears the durable claim in one transaction. Lost/error responses, cancellation and crashes keep the claim; subsequent actions are refused while a fresh outline remains available. The panel shows the uncertain outcome. Owner Continue after inspection acknowledges it without retrying the previous action. Daemon restart retains this block. Already dispatched input may reach the site before Pause/Stop completes.

## Verification

Checks use an isolated daemon on port 7353, a separate Chrome 154 profile, and throwaway local pages only. Seeded browser-session records avoid model calls; the worker's session-status fetch is intercepted to represent an active turn during the input checks. This exercises the real daemon broker, socket, extension, isolated DOM scripts and CDP input, without claiming an end-to-end answer from any model.

- Read remains default; ordinary input is rejected, password/OTP values are omitted, and a CLI bearer cannot enable Work.
- Form proposal displays exact values and the autosave warning. Fields stay empty before Apply, then receive trusted input without submission.
- Single-select, keypress, scroll and fresh action outlines work. Submit requires a review and reaches the page once after approval.
- A second session gets a lock error; explicit tab transfer removes the first session's access.
- Handover detaches the debugger and returns an outline on Continue. Pause blocks input; Continue permits future actions.
- Same-origin navigation and new tabs work; newly opened tabs have their own session handles and cyan group. Declining a new-site review prevents navigation.
- Intentionally dropping the execute reply after Submit leaves exactly one submission, permits a fresh outline, blocks further actions and rejects expired dispatch IDs. The durable uncertainty flag survives daemon restart. Owner Continue clears it without repeating Submit. Stop removes grants and detaches control.
- Review and uncertain-outcome controls were visually checked at 420 CSS px without horizontal overflow. The page-access/review area scrolls within a bounded height so the conversation stays accessible.
- Extension and web production builds/type checks, pinned strict workspace Clippy and the 99 existing Rust tests pass. No unit tests were added.

## Remaining acceptance and limits

Daily use on signed-in sites and native OS/browser variants remains necessary. This pass does not spend model usage or claim all-three-harness completion of E2; E0's Claude quota and OpenCode model restrictions remain relevant. Native approval modes never approve browser reviews automatically.

Screenshots and model image-capability discovery, iframe/shadow-root interaction, developer tools (console/network/evaluation), and E3 access in coding sessions remain deferred. Reads and actions stay top-frame only. A new origin reached unexpectedly requires adding the page; this conservative flow does not silently inspect a redirect. “Fill but do not submit” is not a site-level guarantee. Browser-only harness isolation remains partial as described in the extension README.

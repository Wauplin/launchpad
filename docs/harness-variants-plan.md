# Named local harness variants and reported-cost limits

Owner-selected follow-up on 2026-10-10: implement variants/settings and PAYG budgets after the harness registry work. Pi and SSH/remote agents are excluded.

## Scope

Settings → Harnesses configures named variants of registered local harnesses: a stable ID, display name, environment overrides, default model/effort, subscription or PAYG billing, and an optional default USD cost limit. Variants inherit the base icon. Environment values stay in config.toml and are applied to native processes and terminal handoffs; sessions, schedules and crons persist only the variant ID. Missing variants fail visibly on startup. Existing processes keep their startup environment until reattached. Model discovery and subscription quota monitoring still use the base account; alternate accounts should use PAYG mode because their quota is not monitored.

The composer prefills the variant cost limit and allows no limit. Claude/OpenCode sessions expose cost-limit editing in Summary and progress in the header. Cost limits persist with the session and gate messages, question answers and quota continuation at cumulative reported USD cost. The daemon asks active turns to interrupt best effort. Raise or remove a limit to permit manual continuation; changing the limit sends no input. Codex cannot enforce USD limits because its protocol does not report dollar cost. Claude reports at turn end, and resumed process totals are added to earlier reported cost. OpenCode reports native cumulative session usage through ACP. Missing or delayed costs cannot enforce an exact provider spending cap, and commands may finish after interruption. Terminal TUIs and external activity are outside the guard. PAYG and limited sessions keep prompt-based titles to avoid extra unaccounted naming requests.

No billing ledger, automatic pricing estimates, remote transport, Pi integration or custom icon upload is included. Config saves are validated and use mode 0600 on Unix; Windows relies on the user's profile permissions.

## Acceptance

- Save, edit, delete and reload variants in the dev app; verify IDs stay stable when names change, environment values are not copied into session launch specs, and malformed IDs, variables and unsupported budgets fail clearly.
- Check composer selection, model/effort defaults, positive/empty/invalid limits, inherited icons and explicit billing notices. Check session Summary/header controls and reload persistence.
- Use native variants to verify environment startup and continuity; missing variants must fail instead of using another account. PAYG must bypass base subscription exhaustion, while base sessions retain the existing guard.
- Confirm reported cost at or above a limit blocks messages and answers before sending; raising/removing a limit permits manual work. Check best-effort interruption during daily use and Claude cumulative cost after process resume. Do not deliberately exhaust subscriptions or launch unbounded paid checks.
- Run strict workspace Clippy, existing fast Rust tests, UI typecheck/build and the dev app. Keep all GitHub changes local.

Checked the dev daemon and Chrome: variant create/save/reload and stable IDs, composer selection and positive/empty/invalid limits, visible session cost progress, reached-limit rejection of messages and answers, raise/remove/restore without sending, unsupported Codex limits and malformed configuration, Unix config mode 0600, and restart persistence with input still blocked. Settings forms and save controls fit 1440 px and 800 px at all five zoom positions. Recovered prior native smoke-check traces confirm Claude/Codex/OpenCode environment overrides, including Claude home expansion; those conversations were inspected without starting new paid turns. Strict workspace Clippy, all 16 existing fast Rust tests, UI typecheck/build and daemon/desktop builds pass. Temporary check variants were removed and the dev configuration and composer draft were restored. Daily-use acceptance remains pending for provider cost-report timing, live interruption, native process resume and macOS/Windows behavior.

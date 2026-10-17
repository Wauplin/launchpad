# Adding a harness

The registration list in `crates/core/src/harness/mod.rs` is the only host list of harness identities. Claude Code, Codex and OpenCode are registered integrations. Pi is intentionally left for a follow-up that checks this boundary in practice.

## Files to change

1. Add `crates/core/src/harness/<name>.rs`, with private protocol helpers and an SVG asset if needed.
2. Declare the module and add its `REGISTRATION` to `REGISTRATIONS` in `harness/mod.rs`.
3. Run `cargo run -p launchpad -- gen-types` when changing shared metadata types. Registering an adapter alone does not change the shared types.

A registration contains an ID (`HarnessKind::new("stable-id")`), display metadata, integration capabilities, an adapter factory and an inherited-environment-marker predicate. The ID is stored in sessions, launch specs and schedules. IDs must be unique; unknown IDs fail explicitly. Native conversation IDs remain opaque strings. Assets live beside the module, with their source/license recorded in `harness/icons/README.md`.

`Harness` is the adapter contract. Only startup and version discovery are required; defaults for optional hooks return empty discovery, no services/title request, or a clear unsupported error. Set capabilities only for hooks actually implemented. The factory can own a shared native process and routing state, as Codex does; it can also be stateless and create one process per session.

## Native behavior belongs in the adapter

- `start`: create or resume the native conversation from `StartSpec`, set its ID/model through `Sink`, and return a `LiveSession`.
- `LiveSession`: implement input and delivery acknowledgments, interruption, structured answers and final process stop. `turn_active` covers a turn waiting for input too. `ready_for_turn` must remain false until outstanding commands and questions settle. A silent command is still running. Expose `has_open_questions` if multiple requests can be pending. Optional `set_model` applies model/effort overrides from the next turn; return false to have the host replace the idle process with a native resume on the next input. Tool-call events upsert metadata by item ID when native inputs/titles arrive later. Emit final messages and `TurnEnd` before the final status; preserve Stopped/Interrupted on cancellation.
- `models`, `defaults`, `efforts`, `version`: read native discovery/configuration. The host polls model lists independently so one missing executable does not block other integrations. Version errors appear beside the harness in the status bar.
- `spawn_services`: start native quota readers, notification routing or updates. Publish quota data through `StatusHandle::update_harness(ID, ...)`; do not put provider parsing or daemon-owned native process fields in `App`.
- `terminal_command`/`terminal_env`/`terminal_resume_markers`: return an argument vector, native environment overrides and markers identifying the native resume command. The host resolves the executable and opens the integrated terminal on Linux/macOS/Windows.
- `project_moved`: relocate native path-dependent history when a temporary project is promoted, if necessary.
- `title_command`/`title_response`: optional auxiliary title request. The host handles timeout and subscription protection. With no hook, the local prompt-based title stays.
- `environment_marker`: remove only inherited session markers that should not reach another process. Credentials and normal native configuration stay in the user's environment.

The host owns persistence, prompts, project/worktree creation, scheduling, quota holds and publication guards. Normalize protocol output into the existing event model rather than exposing protocol-specific fields to React. A genuinely new interaction may require extending the shared contract; adding a harness with existing interactions should not.

## Reusing ACP

OpenCode is the first consumer of `harness::acp::start(command, spec, sink, extensions)`. Its module supplies native child-update capabilities/translation and delegation-tool identification through `Extensions`, as well as `opencode acp` with the correct working directory/environment, CLI model/version discovery, metadata and terminal resume arguments. The reusable client handles stdio JSON-RPC v1, negotiated new/load/resume, model config options, text/images, ordered updates, tool completion, permission cards, cancellation and transport failure. It advertises no client filesystem or terminal services: the harness runs its own tools.

ACP permission responses select an advertised option; arbitrary typed answers are rejected. Image support is checked against the negotiated agent capability as well as the static integration descriptor; a selected model may still reject an unsupported image. ACP tool output snapshots are converted to incremental output, and reported before/after texts become unified diff previews. Native agents may report only the replaced fragment; the Git panel remains the source for full-file diffs. Reported USD usage is cumulative, without claiming that the provider billed that amount.

Current OpenCode limits are explicit: no messages while a turn is running, effort override, auxiliary AI title, subscription quota monitor or hard spending cap. Native configuration, credentials, model defaults and permissions remain OpenCode's responsibility. Named local variants pass environment overrides through `StartSpec.env`; adapters must apply them to every native session process. The host resolves variant defaults and billing, and `capabilities.cost` enables limits based on cumulative reported USD usage. Claude and OpenCode support this; Codex does not report dollars. SSH and billing ledgers remain deferred.

## Checks

Run strict workspace Clippy, UI typecheck/build and the existing short pure-function tests. Exercise the installed native process through an isolated dev database: launch, stream, follow-up, permission answer, stop, archive and resume. Inspect the composer and session controls for capability handling. Do not mock harnesses, publish GitHub items, restart production or start uncancelled test schedules/crons. Keep native behavior gated appropriately for supported operating systems.


## Initial manual acceptance (2026-10-10)

Against installed OpenCode 1.18.23, a temporary dev `App` and HTTP server used a separate database/configuration on port 17318, without starting GitHub synchronization, native updates or touching either existing daemon. The native free `opencode/big-pickle` model completed a prompt and follow-up; permission cards allowed a shell command and a file edit; trace output, updated tool inputs, file-change preview, context usage and reported cost persisted. Input during a running turn was rejected. Stop while a permission request was pending settled the card and ended as Stopped; archive/resume preserved the native ID without replaying the trace. A restarted test daemon resumed that same conversation and completed another edit. Browser checks verified all three choices/marks and OpenCode's native billing notice. Unsupported subscription policy changes returned a clear error. These checks do not replace daily-use acceptance or native macOS/Windows checks.

After integrating concurrent main changes, a second isolated native check verified the new terminal dock backend: an ACP-launched OpenCode conversation opened its TUI in the PTY, produced output, was detected as running, required explicit close before re-attach, and resumed with the same native ID. Unsupported effort overrides were rejected. Both temporary native conversations were deleted afterward.

## Native subagents

Emit `Subagent` metadata with the reported native ID and direct parent, then normalize each child's events through `sink.child(id)`. Child sinks preserve the root session ID and sequence while wrapping events; they must not update root usage, model, identity or preview text. Root-facing native questions still use the root answer routing. Include observed child turns/tools in `ready_for_turn` and interruption; never infer completion from silence. Persist only reported states and show unavailable traces honestly. The UI consumes the shared envelopes without harness identity branches. OpenCode's optional ACP child extension stays in its adapter; older versions can expose delegation-only records, which native records supersede.

## Permission modes

Claude and Codex advertise `capabilities.permission_modes`. The composer offers **YOLO · Full access** (initial default) and **Ask for approval**. The launch spec stores `permission_mode`, including for scheduled launches; old specs deserialize to YOLO. Successful launches restore defaults independently for each harness/variant, like model and effort. A deliberate mode selection is also remembered immediately on that browser, before launch. Restored unsent drafts retain their own mode.

Claude uses `--permission-mode default` for approvals, with `can_use_tool` requests sent to explicit approval cards. Codex uses `approvalPolicy: untrusted` and `sandbox: workspace-write`; safe reads and operations allowed by native rules may proceed without prompting. YOLO keeps the previous bypass flags/policies. Resume and terminal handoff carry the saved mode. OpenCode continues to use its native permissions.

Approval cards show the requested tool input, command, working folder, reason or proposed file changes where supplied. They appear in Trace, Summary and phone Now, with explicit choices and touch-sized buttons. Codex offers session approval when the native protocol permits it. Typed replies cannot grant permission; invalid or stale choices are rejected. Cancellation, archive and process exits settle requests, and child requests route through their parent session. Explicit Cancel turn decisions use `LiveSession::answer_stops_turn` so the host treats them as silent owner stops. Approval waits are silent under the existing notification policy.

Permission acceptance on 2026-10-17 used Claude Code 2.1.302 and Codex CLI 0.163.0 against a temporary App with isolated databases and folders, no monitors, and notifications disabled. Both native integrations verified default YOLO execution without prompts, blocking before approval, rejection of typed consent, denial/cancellation without execution, Allow once, Stop invalidating a pending request, and archive/resume preserving approval mode. Codex advertised Allow once/Cancel turn for the sampled untrusted command; Cancel turn settled as Stopped. All temporary sessions were closed. The UI build, typecheck and regression suites passed; interactive visual QA remains for daily use because no browser was connected to this agent session.

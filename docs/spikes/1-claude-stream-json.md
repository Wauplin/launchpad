# Spike 1: Claude stream-json

Date: 2026-10-02. Claude Code 2.1.287, model `haiku` (claude-haiku-4-5), `--effort low`. Cost of the whole spike: about $0.15.

**Result: success.** One `claude -p` process per session works for many turns, with interrupt, resume, rate limits and questions. The fallback (TypeScript Agent SDK sidecar) is not needed. Three details differ from the first draft of the architecture; they are now fixed in [ARCHITECTURE.md, section 9.3](../ARCHITECTURE.md#93-claude-code-adapter).

## Command

A small Python script started the process, wrote JSON lines on stdin, and logged every stdout line with a timestamp.

```
claude -p --input-format stream-json --output-format stream-json --verbose \
       --include-partial-messages --dangerously-skip-permissions \
       --replay-user-messages --permission-prompt-tool stdio \
       --session-id <uuid> --name spike1 --model haiku --effort low
```

Input lines:

```json
{"type":"user","uuid":"65c49206-…","message":{"role":"user","content":[{"type":"text","text":"Reply with exactly the word: one"}]}}
{"type":"control_request","request_id":"int-1","request":{"subtype":"interrupt"}}
```

## Findings

| Question | Result |
|---|---|
| Several turns on one process | Yes. Each turn starts with `system/init` (sent again on every turn) and ends with one `result`. The process exits with code 0 when stdin is closed. |
| Message sent while a turn runs | It does **not** start a new turn. Claude adds it to the running turn at the next model request (after the current tool result), and one `result` covers both. If the input line has a `uuid`, Claude emits `command_lifecycle` events (`queued`, `started`, `completed`) with `command_uuid` set to that uuid. With `--replay-user-messages`, the message is echoed back (`isReplay: true`, same `uuid`) at the moment Claude reads it. |
| `interrupt` control request | Works. Reply: `control_response` with `{"still_queued": []}`. The running tool gets a rejected `tool_result`, then a user text `[Request interrupted by user for tool use]`, then `result` with `subtype: "error_during_execution"` and `terminal_reason: "aborted_tools"`. The process stays alive and the next message works. |
| `--resume` | Works: same session id, context kept (the model listed its earlier replies), title kept. **`--session-id` cannot be combined with `--resume`** (`Error: --session-id can only be used with --continue or --resume if --fork-session is also specified.`), so a resume uses `--resume <id>` instead of `--session-id <id>`. |
| `rate_limit_event` | Sent once per process, after the first model response, not on every turn. Content below. `utilization` is a fraction (0.84 = 84 %), `resetsAt` is in Unix seconds, and `unifiedWindows` has both windows. |
| AskUserQuestion with `--dangerously-skip-permissions` | Without `--permission-prompt-tool stdio`, the tool is **not offered** to the model at all (it is missing from the `init` tool list, and `--tools default,AskUserQuestion` does not add it). With `--permission-prompt-tool stdio`, it is offered and arrives as a `can_use_tool` control request with `requires_user_interaction: true`. The reply sets `updatedInput.answers` (question text → chosen label). Other tools (Bash) still run without any request. No `initialize` handshake is needed. |
| Background commands | `system/background_tasks_changed` lists the running background tasks. `task_started` has `is_backgrounded: true`, then `task_updated` and `task_notification`. When a background task ends after the turn ended, **Claude starts a new turn by itself** (`system/init` … `result`), without any input. |
| Cost and usage | `total_cost_usd` in `result` is cumulative for the session and continues across `--resume` (0.0177 → 0.0336 → 0.0390 → 0.0430, then 0.0475 after resume). `usage` is per turn. Launchpad stores the latest `total_cost_usd`; it does not add them up. |
| Other events | `system/status` (`requesting`), `system/thinking_tokens` (estimated count), `system/session_title_changed`, `system/commands_changed`. `thinking` blocks from Haiku have an empty text (signature only). |

## Sample output

Interrupt:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"int-1","response":{"still_queued":[]}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected …","is_error":true,"tool_use_id":"toolu_…"}]}}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}
{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_tools","total_cost_usd":0.0389713,…}
```

Message lifecycle (input line sent with `"uuid":"65c49206-…"`):

```json
{"type":"command_lifecycle","command_uuid":"65c49206-…","state":"queued"}
{"type":"command_lifecycle","command_uuid":"65c49206-…","state":"started"}
{"type":"user","message":{…},"uuid":"65c49206-…","isReplay":true}
{"type":"command_lifecycle","command_uuid":"65c49206-…","state":"completed"}
```

Rate limits:

```json
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1791007200,"rateLimitType":"seven_day","utilization":0.83,"isUsingOverage":false,"surpassedThreshold":0.75,"unifiedWindows":{"five_hour":{"utilization":0.3,"resetsAt":1790947800},"seven_day":{"utilization":0.83,"resetsAt":1791007200}}}}
```

Question from the agent, and the reply:

```json
{"type":"control_request","request_id":"9fd8d4e2-…","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion","input":{"questions":[{"question":"Which color do you prefer?","header":"Color Choice","options":[{"label":"Red","description":"The color red"},{"label":"Blue","description":"The color blue"}],"multiSelect":false}]},"tool_use_id":"toolu_…","requires_user_interaction":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"9fd8d4e2-…","response":{"behavior":"allow","updatedInput":{"questions":[…],"answers":{"Which color do you prefer?":"Blue"}}}}}
```

The tool result that follows: `Your questions have been answered: "Which color do you prefer?"="Blue". You can now continue with these answers in mind.`

Background task that ends after the turn:

```
[ 5.85] system/task_started {"task_id":"bs4a8ry5q","is_backgrounded":true,"description":"sleep 12 && echo bgdone",…}
[ 6.80] result success result='started'
[17.86] system/background_tasks_changed {"tasks":[]}
[17.86] system/task_notification {"task_id":"bs4a8ry5q","status":"completed","output_file":"/tmp/…/tasks/bs4a8ry5q.output",…}
[17.95] system/init …                      ← new turn, started by Claude
[19.96] result success result='Background task completed (exit code 0).'
```

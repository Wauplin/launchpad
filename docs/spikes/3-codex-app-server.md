# Spike 3: Codex app-server

Date: 2026-10-02. codex-cli 0.159.3, model `gpt-6-luna` ("Fast and affordable model for easier tasks"), effort `low`.

**Result: success.** Everything planned in [ARCHITECTURE.md, section 9.4](../ARCHITECTURE.md#94-codex-adapter) works. The fallback (`codex exec --json`) is not needed.

## Command

```
codex app-server -c check_for_update_on_startup=false
```

JSON-RPC over stdio, one JSON object per line, without a `"jsonrpc"` field. Requests sent:

```json
{"method":"initialize","id":1,"params":{"clientInfo":{"name":"launchpad-spike","version":"0.0.0"}}}
{"method":"initialized"}
{"method":"thread/start","id":2,"params":{"cwd":"/…/s3-cwd","model":"gpt-6-luna","approvalPolicy":"never","sandbox":"danger-full-access"}}
{"method":"turn/start","id":3,"params":{"threadId":"01a0fc23-…","input":[{"type":"text","text":"Reply with exactly the word: one"}],"effort":"low"}}
{"method":"turn/steer","id":5,"params":{"threadId":"01a0fc23-…","expectedTurnId":"01a0fc23-4217-…","input":[{"type":"text","text":"Also, after that, reply with exactly the word: three"}]}}
{"method":"turn/interrupt","id":7,"params":{"threadId":"01a0fc23-…","turnId":"01a0fc23-7997-…"}}
{"method":"thread/resume","id":2,"params":{"threadId":"01a0fc23-…","excludeTurns":true}}
```

## Findings

| Question | Result |
|---|---|
| `thread/start` with `approvalPolicy: "never"` and full access | Works. No approval request arrived during the spike (shell commands and file edits). |
| `turn/steer` | Works while a command runs. Reply: `{"turnId": "…"}`. The steered text appears as a `userMessage` item when Codex reads it, after the running command completes, inside the same turn. |
| `turn/interrupt` | Works. Reply: `{}`. Then `turn/completed` with `turn.status: "interrupted"`. **The running `commandExecution` item never gets an `item/completed`**, so the normalizer must close open items when a turn ends. Codex logs `exec_command failed: UnknownProcessId` on stderr (harmless). The next `turn/start` works. |
| `thread/resume` after the app-server restarts | Works. A new `codex app-server` process resumes the thread and the model remembers the earlier turns. |
| Rate limits | `account/rateLimits/read` returns `primary` (300-minute window) and `secondary` (10,080 minutes) with `usedPercent` and `resetsAt` (Unix seconds). `account/rateLimits/updated` arrives after every model response, so polling only matters when no session runs. |
| Models | `model/list` returns 8 models with `displayName`, `description`, `supportedReasoningEfforts`, `defaultReasoningEffort` and `isDefault` (`gpt-6.1-sol`). |

### Event mapping

Notifications seen, and the normalized event each one becomes (see [ARCHITECTURE.md, section 9.1](../ARCHITECTURE.md#91-session-model)):

| Codex notification | Normalized event |
|---|---|
| `turn/started` | `status` → working |
| `item/started` / `item/completed` with `type: "userMessage"` | `user_message` (sent and steered input) |
| `item/agentMessage/delta`, then `item/completed` `agentMessage` | `assistant_delta`, then `assistant_message`. `phase` is `commentary` (text between tool calls) or `final_answer`. |
| `item/started` / `item/completed` with `type: "commandExecution"`, and `item/commandExecution/outputDelta` | `tool_call` / `tool_result` (`command`, `cwd`, `aggregatedOutput`, `exitCode`, `durationMs`) |
| `item/completed` with `type: "fileChange"` | `file_change` (`changes[].path`, `kind`, `diff`) |
| `turn/diff/updated` | full unified diff of the turn, kept for the session's diff view |
| `thread/tokenUsage/updated` | `usage` (`total` and `last` token counts, `modelContextWindow` 258,400) |
| `account/rateLimits/updated` | `rate_limit` |
| `turn/completed` | `status` → waiting (or the turn ended interrupted) |
| `thread/status/changed` (`active`, `idle`), `thread/started`, `account/updated`, `mcpServer/startupStatus/updated`, `remoteControl/status/changed`, `thread/goal/cleared` | not shown; stored as raw lines only |

`turn/plan/updated` and reasoning summaries did not appear with this small model and low effort. They are mapped as planned (`plan`, `reasoning`) and will be checked with real sessions.

## Sample output

```
[ 7.94] response id=4 {"turn":{"id":"01a0fc23-4217-…","status":"inProgress",…}}
[12.16] item/started type=commandExecution {"command":"/usr/bin/zsh -lc 'sleep 8 && echo slept'","cwd":"/…/s3-cwd","processId":"67086",…}
[12.16] > turn/steer …
[12.16] response id=5 {"turnId":"01a0fc23-4217-…"}
[20.01] item/completed type=commandExecution {…,"status":"completed","aggregatedOutput":"slept\n","exitCode":0,"durationMs":7862}
[20.02] item/started type=userMessage {"content":[{"type":"text","text":"Also, after that, reply with exactly the word: three"}]}
[22.06] item/agentMessage/delta 'three'
[22.14] turn/completed {"turn":{"id":"01a0fc23-4217-…","status":"completed",…}}
…
[26.61] item/started type=commandExecution {"command":"/usr/bin/zsh -lc 'sleep 40 && echo late'",…}
[26.61] > turn/interrupt …
[26.62] response id=7 {}
[26.62] turn/completed {"turn":{"id":"01a0fc23-7997-…","items":[],"status":"interrupted","durationMs":4480}}
```

Token usage:

```json
{"threadId":"01a0fc23-…","turnId":"01a0fc23-8b1c-…","tokenUsage":{"total":{"totalTokens":87554,"inputTokens":87450,"cachedInputTokens":74496,"cacheWriteInputTokens":0,"outputTokens":104,"reasoningOutputTokens":0},"last":{"totalTokens":18132,"inputTokens":18127,"cachedInputTokens":17152,"cacheWriteInputTokens":0,"outputTokens":5,"reasoningOutputTokens":0},"modelContextWindow":258400}}
```

Rate limits:

```json
{"rateLimits":{"limitId":"codex","primary":{"usedPercent":1,"windowDurationMins":300,"resetsAt":1790943278},"secondary":{"usedPercent":22,"windowDurationMins":10080,"resetsAt":1791123677},"credits":{"hasCredits":true,"unlimited":false,"balance":null},"planType":"team","rateLimitReachedType":null,…}}
```

# Foundations spikes (H0)

Checks for the [Home and runners plan](https://wauplin-launchpad-bots-research.static.hf.space/index.html) before building on it. Run on November 4, 2026 with Claude Code 2.1.311 and Codex 0.166.0, each in an empty, isolated home (`HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME` in a scratch folder), as a runner would.

The test proxy listened on `127.0.0.1`, forwarded every request over HTTPS to one upstream host, and replaced `Authorization: Bearer lp-placeholder` with the real token kept in its own memory. It did not read or judge requests.

## 1. Harness logins through a token-adding proxy

| Harness | Setup on the runner | Result |
| --- | --- | --- |
| Claude Code (subscription OAuth) | `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>`, `CLAUDE_CODE_OAUTH_TOKEN=lp-placeholder`; proxy to `api.anthropic.com` | Works. `claude -p` answered; `POST /v1/messages?beta=true` received the token (with the `oauth-2025-04-20` beta header Claude Code sends itself). The only other call was an unauthenticated `HEAD /api/hello`. Nothing in the runner's config folder contains the real token. |
| Codex (ChatGPT login) | `auth.json` with placeholder `access_token`/`refresh_token`, an unsigned `id_token` carrying the same claims (plan, account), `last_refresh` = now; `config.toml` with `chatgpt_base_url = "http://127.0.0.1:<port>/backend-api/"` and a model provider `base_url = "http://127.0.0.1:<port>/backend-api/codex"`, `wire_api = "responses"`, `requires_openai_auth = true`; proxy to `chatgpt.com` | Works. `codex exec` answered; `POST /backend-api/codex/responses`, `GET /backend-api/codex/models`, plugin and account checks received the token. `POST /backend-api/ps/mcp` (ChatGPT connectors) sends no bearer and got HTTP 451; Codex logs it and continues. |

Notes for H3:

- Claude Code: a long-lived `claude setup-token` on Home is the intended source; the spike used the current OAuth access token, read only.
- Codex refreshes when `last_refresh` is old. The runner must write a fresh `last_refresh` at each lease so Codex never tries to refresh with the placeholder; Home stays the only place that refreshes.
- Codex needs `stdin` closed (`< /dev/null`) in `exec` mode, or it waits for piped input.
- Codex warns that `bubblewrap` is not on `PATH` and uses its bundled copy.

## 2. Resuming one conversation on another runner

| Harness | What moved | Result |
| --- | --- | --- |
| Claude Code | `<config>/projects/<cwd-slug>/<session-id>.jsonl` copied into the second runner's config folder under the slug of its own (different) working folder | Works. `claude -p --resume <id>` in the new folder quoted the earlier answer and kept the same session ID. |
| Codex | `<codex-home>/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl` copied to the same relative path in the second `CODEX_HOME`; the SQLite state files were not copied | Works. `codex exec resume <id>` in another folder quoted the earlier answer. |

So a move needs only the native transcript file, placed where the harness looks for it on the new runner (Claude: by working-folder slug). Each adapter owns that mapping (`Harness::native_trace` already exports these files).

## 3. HF token that cannot reach Home

Dropped by the owner: the owner provides the HF token and Launchpad cannot restrict it. Protecting Home's Space and bucket from a steered agent is out of scope for now.

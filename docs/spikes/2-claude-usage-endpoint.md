# Spike 2: Claude usage endpoint

Date: 2026-10-02. Claude Code 2.1.287, Team plan.

**Result: success.** `GET https://api.anthropic.com/api/oauth/usage` with the OAuth access token from `~/.claude/.credentials.json` returns both quota windows. No beta header is needed. The fallback (quotas only from `rate_limit_event`) stays as a second source.

## Command

The token is read by the script and never printed.

```python
tok = json.load(open(os.path.expanduser("~/.claude/.credentials.json")))["claudeAiOauth"]["accessToken"]
urllib.request.Request("https://api.anthropic.com/api/oauth/usage",
                       headers={"Authorization": f"Bearer {tok}", "Content-Type": "application/json"})
```

The credentials file also has `expiresAt` (milliseconds since the epoch), `scopes`, `subscriptionType` (`team`) and `rateLimitTier`. On the day of the spike, the access token was valid for about 6 more hours.

## Sample output (`200`, shortened)

```json
{
  "five_hour": {"utilization": 33.0, "resets_at": "2026-10-02T13:30:00.227664+00:00", "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
  "seven_day": {"utilization": 84.0, "resets_at": "2026-10-03T06:00:00.227681+00:00", "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
  "seven_day_opus": null,
  "seven_day_sonnet": null,
  "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 0.0, "currency": "USD", …},
  "limits": [
    {"kind": "session", "group": "session", "percent": 33, "severity": "normal", "resets_at": "2026-10-02T13:30:00.227664+00:00", "is_active": false},
    {"kind": "weekly_all", "group": "weekly", "percent": 84, "severity": "warning", "resets_at": "2026-10-03T06:00:00.227681+00:00", "is_active": true}
  ],
  "spend": {…},
  …about 20 other keys with code names, all null
}
```

## Findings

- `five_hour` and `seven_day` give `utilization` in percent (0 to 100) and `resets_at` as an ISO 8601 date. These two are what the status bar shows. `limits[].severity` (`normal`, `warning`) can color the bar.
- `rate_limit_event` in a session gives the same numbers, as a fraction (0.84) with Unix-second reset times (see [spike 1](1-claude-stream-json.md)). Both sources agree.
- A wrong or expired token gives `401`: `{"type":"error","error":{"type":"authentication_error","message":"OAuth access token is invalid."},"request_id":null}`.
- When the token expires, Launchpad checks `expiresAt` before each call and skips the call if the token has expired. It keeps the last known values with their age and waits for Claude Code to refresh the token (any `claude` run does). It never refreshes the token itself.
- The file is read again before every poll, so a token refreshed by Claude Code is picked up at the next poll.

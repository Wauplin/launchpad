//! Claude quota readings of Home's vault logins.
//!
//! A `claude setup-token` token only allows model requests: Anthropic's usage endpoint refuses it (403, scope
//! `user:profile`), so Launchpad never asks it. Readings come from two places:
//! - running sessions: Claude Code reports both windows in a `rate_limit_event` after its model requests
//!   (`rate_limit_event`, sent from runners to Home by `quota::session_reading`);
//! - a probe: a one-word `claude -p` request with Haiku at low effort (`run_probe`), read from that same event.
//!
//! A probe runs only when the reading is needed (it is unknown, automatic work waits for it,
//! `quota::want_reading`, or a paused session's reset passed, `quota::paused_wanting`) and the known reading does not confirm capacity
//! (`quota::available`); never while a used-up window has not reset (the request could then be paid extra
//! usage); at most one at a time per login (`Gate`), with a back-off after failures. The owner's Refresh probes
//! at once unless a valid reading exists, with the same guard and deduplication.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use launchpad_core::App;
use launchpad_core::quota::{self, QuotaAccount, QuotaPair};
use launchpad_core::status::{QuotaStatus, QuotaWindow};
use launchpad_core::util::now_ms;
use serde_json::Value;

/// After a successful probe, the next one waits at least this long, whatever is asked.
const MIN_GAP_MS: i64 = 2 * 60_000;
/// The first retry after a failed probe; each further failure doubles it, up to `MAX_BACKOFF_MS`.
const FIRST_BACKOFF_MS: i64 = 60_000;
const MAX_BACKOFF_MS: i64 = 60 * 60_000;
/// A rate-limited probe (429) waits at least this long.
const RATE_LIMITED_MS: i64 = 5 * 60_000;
/// Automatic work asked for a reading in this time: it still waits.
const WANTED_MS: i64 = 2 * 60_000;
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// The 5-hour and weekly windows of a Claude `rate_limit_event`'s `rate_limit_info` (`unifiedWindows`:
/// utilization as a fraction, `resetsAt` in Unix seconds). A window without a valid utilization is None; a window
/// without a reset has not started.
pub fn windows_of(info: &Value) -> QuotaPair {
    let win = |k: &str| -> Option<QuotaWindow> {
        let w = &info["unifiedWindows"][k];
        let used = w["utilization"].as_f64().filter(|u| u.is_finite() && *u >= 0.0)?;
        Some(QuotaWindow { used_percent: used * 100.0, resets_at: w["resetsAt"].as_i64().filter(|r| *r > 0) })
    };
    (win("five_hour"), win("seven_day"))
}

/// The quota a probe's stream-json output reports: the last `rate_limit_event` with both windows. Without one,
/// the error says why (the result's error text and HTTP status when the request failed).
pub fn probe_output(stdout: &str) -> Result<QuotaPair> {
    let mut found = None;
    let mut result: Option<Value> = None;
    for line in stdout.lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
        match msg["type"].as_str() {
            Some("rate_limit_event") => {
                let (five, week) = windows_of(&msg["rate_limit_info"]);
                if five.is_some() && week.is_some() { found = Some((five, week)); }
            }
            Some("result") => result = Some(msg),
            _ => {}
        }
    }
    if let Some(pair) = found { return Ok(pair); }
    let Some(result) = result else { bail!("claude ended without a result") };
    let status = result["api_error_status"].as_i64().map(|s| format!(" (HTTP {s})")).unwrap_or_default();
    let text: String = result["result"].as_str().unwrap_or_default().chars().take(200).collect();
    if result["is_error"].as_bool() == Some(true) {
        bail!("the request failed{status}: {}", if text.is_empty() { "unknown error" } else { &text });
    }
    bail!("Claude Code reported no quota windows for this login (no rate_limit_event with both windows)")
}

/// Runs the probe: a one-word `claude -p` request with Haiku at low effort, no tools, settings, MCP servers or
/// saved session, in Home's copy of the login (`vault::login_home`: its `CLAUDE_CONFIG_DIR` and token).
async fn run_probe(app: &App, env: &[(String, String)]) -> Result<QuotaPair> {
    // Only the login's token authenticates: an API key or another endpoint of Home's environment would make
    // this a paid API request without subscription windows.
    let mut vars = app.env.vars().clone();
    for k in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX"] {
        vars.remove(k);
    }
    let env_vars = launchpad_core::shellenv::UserEnv::from_vars(vars).with(env);
    let cwd = env.iter().find(|(k, _)| k == "CLAUDE_CONFIG_DIR").map(|(_, v)| std::path::PathBuf::from(v));
    let mut cmd = launchpad_core::procs::child_command(&env_vars, "claude", cwd.as_deref());
    cmd.args([
            "-p", "ok", "--output-format", "stream-json", "--verbose", "--no-session-persistence", "--strict-mcp-config",
            "--setting-sources", "", "--disable-slash-commands", "--tools", "", "--model", "haiku", "--effort", "low",
            "--max-turns", "1", "--system-prompt", "Reply with one word.",
        ])
        .stdin(launchpad_core::exec::Io::Null);
    let run = cmd.output();
    let out = tokio::time::timeout(PROBE_TIMEOUT, run).await.map_err(|_| anyhow::anyhow!("claude timed out after {} s", PROBE_TIMEOUT.as_secs()))?.context("start claude")?;
    probe_output(&String::from_utf8_lossy(&out.stdout)).map_err(|e| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() { e } else { e.context(format!("stderr: {}", stderr.chars().take(200).collect::<String>())) }
    })
}

/// The probe history of one login: shared by its monitor and the owner's Refresh, so at most one probe runs at
/// a time and failures back off.
#[derive(Debug, Default)]
pub struct Gate {
    /// When the last probe ended (Unix ms), with its result.
    last: Option<(i64, Result<QuotaPair, String>)>,
    failures: u32,
    /// No automatic probe before this (Unix ms).
    not_before: i64,
}

impl Gate {
    fn record(&mut self, result: &Result<QuotaPair>, now: i64) {
        match result {
            Ok(pair) => {
                self.failures = 0;
                self.not_before = now + MIN_GAP_MS;
                self.last = Some((now, Ok(pair.clone())));
            }
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                let text = format!("{e:#}");
                let mut wait = (FIRST_BACKOFF_MS << (self.failures - 1).min(10)).min(MAX_BACKOFF_MS);
                if text.contains("HTTP 429") { wait = wait.max(RATE_LIMITED_MS); }
                self.not_before = now + wait;
                self.last = Some((now, Err(text)));
            }
        }
    }
}

/// Why the monitor does not probe now, or None when it should.
pub fn skip_reason(q: &QuotaStatus, gate: &Gate, now: i64) -> Option<String> {
    if quota::available(q, now) { return Some("the known reading is still valid".into()); }
    if let Some(b) = quota::exhausted(q).into_iter().find(|b| b.resets_at.is_none_or(|r| r > now / 1000)) {
        return Some(format!("the {} window is used up until its reset", b.window.to_lowercase()));
    }
    (now < gate.not_before).then(|| format!("next probe in {} s", (gate.not_before - now + 999) / 1000))
}

/// Runs `probe` under the gate. A caller that waited while another probe ran (it ended after `asked`) takes
/// that probe's result instead of starting its own.
pub async fn gated<F, Fut>(gate: &tokio::sync::Mutex<Gate>, asked: i64, probe: F) -> Result<QuotaPair>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<QuotaPair>>,
{
    let mut g = gate.lock().await;
    if let Some((at, last)) = &g.last && *at >= asked {
        return last.clone().map_err(|e| anyhow::anyhow!("{e}"));
    }
    let result = probe().await;
    g.record(&result, now_ms());
    result
}

static GATES: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<Gate>>>>> = LazyLock::new(Default::default);

fn gate(key: &str) -> Arc<tokio::sync::Mutex<Gate>> {
    GATES.lock().unwrap().entry(key.to_string()).or_default().clone()
}

/// The monitor of one vault login on Home. It never polls: it probes only when a reading is wanted and the known
/// one does not confirm capacity (see the module's documentation).
pub fn spawn_quota(app: &App, account: QuotaAccount) -> tokio::task::JoinHandle<()> {
    let app = app.clone();
    tokio::spawn(async move {
        let gate = gate(&account.key);
        if !account.env.iter().any(|(k, v)| k == "CLAUDE_CODE_OAUTH_TOKEN" && !v.is_empty()) {
            app.status.quota_error(&account.key, "this login has no token in Home's vault".into());
            return;
        }
        let mut wanted_at: Option<i64> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let now = now_ms();
            if quota::take_demand(&account.key) { wanted_at = Some(now); }
            let q = quota::reading(&app, &account.key);
            // Unknown (Home just started, or every probe failed so far): read it once, so the owner sees it.
            let wanted = q.quota_at.is_none() || wanted_at.is_some_and(|at| now - at < WANTED_MS) || quota::paused_wanting(&app, &account.key, now);
            if !wanted { continue; }
            // The gate is free unless the owner's Refresh probes now: then wait for its result next time.
            let Ok(g) = gate.try_lock() else { continue };
            if skip_reason(&q, &g, now).is_some() { continue; }
            drop(g);
            match gated(&gate, now, || run_probe(&app, &account.env)).await {
                Ok((five, week)) => {
                    tracing::info!("claude quota ({}) read with a Haiku probe", account.key);
                    app.status.update_quota(&account.key, five, week);
                }
                Err(e) => {
                    let wait = (gate.lock().await.not_before - now_ms()).max(0) / 1000;
                    let text = format!("quota probe (claude -p with Haiku) failed: {e:#}; next try in {} min", (wait + 59) / 60);
                    tracing::info!("claude quota ({}): {text}", account.key);
                    // A reading that still confirms capacity stays (a session may have sent one meanwhile).
                    if !quota::available(&quota::reading(&app, &account.key), now_ms()) {
                        app.status.quota_error(&account.key, text);
                    }
                }
            }
        }
    })
}

/// The owner's Refresh of a vault login: the known reading while it confirms capacity, else a probe now, which
/// also checks the token. Never while a window is used
/// up and has not reset (the request could be paid extra usage): the token then stays unchecked.
pub async fn read_quota_now(app: &App, account: &QuotaAccount) -> Result<QuotaPair> {
    anyhow::ensure!(account.env.iter().any(|(k, v)| k == "CLAUDE_CODE_OAUTH_TOKEN" && !v.is_empty()), "this login has no token in Home's vault");
    let now = now_ms();
    let q = quota::reading(app, &account.key);
    // A reading that still confirms capacity (a session's or a probe's) shows the login works: no request.
    if quota::available(&q, now) { return Ok((q.five_hour, q.weekly)); }
    if let Some(b) = quota::exhausted(&q).into_iter().find(|b| b.resets_at.is_none_or(|r| r > now / 1000)) {
        let reset = b.resets_at.map(|r| format!(" (resets in {} min)", (r - now / 1000).max(60) / 60)).unwrap_or_default();
        bail!("the {} quota is used up{reset}: Home sent no model request, which could use extra usage, so the token was not checked", b.window.to_lowercase())
    }
    gated(&gate(&account.key), now, || run_probe(app, &account.env)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A line of `claude -p --output-format stream-json` captured from a real probe (Claude Code 2.1.325).
    const EVENT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794837600,"rateLimitType":"five_hour","overageStatus":"allowed","overageResetsAt":1796083200,"isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.11,"resetsAt":1794837600,"status":"allowed"},"seven_day":{"utilization":0.13,"resetsAt":1795240800,"status":"allowed"}}},"uuid":"86b2ec54","session_id":"81656d4f"}"#;

    #[test]
    fn probe_output_reads_both_windows_of_the_rate_limit_event() {
        let out = format!("{{\"type\":\"system\",\"subtype\":\"init\"}}\nnot json\n{EVENT}\n{{\"type\":\"result\",\"is_error\":false,\"result\":\"Acknowledged.\"}}\n");
        let (five, week) = probe_output(&out).unwrap();
        let five = five.unwrap();
        assert!((five.used_percent - 11.0).abs() < 1e-9);
        assert_eq!(five.resets_at, Some(1794837600));
        assert!((week.unwrap().used_percent - 13.0).abs() < 1e-9);
    }

    #[test]
    fn probe_output_never_invents_a_reading() {
        // A failed login: the result's text and status, no percentages.
        let failed = r#"{"type":"result","is_error":true,"api_error_status":401,"result":"Invalid bearer token"}"#;
        let e = format!("{:#}", probe_output(failed).unwrap_err());
        assert!(e.contains("HTTP 401") && e.contains("Invalid bearer token"), "{e}");
        assert!(launchpad_core::vault::login_refused(&e));
        // One window only: not a reading.
        let partial = json!({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {"five_hour": {"utilization": 0.5, "resetsAt": 10}}}}).to_string();
        assert!(probe_output(&format!("{partial}\n{{\"type\":\"result\",\"is_error\":false}}")).is_err());
        assert!(probe_output("").is_err());
        // Invalid utilization: None, never zero.
        let (five, week) = windows_of(&json!({"unifiedWindows": {"five_hour": {"utilization": "0.1"}, "seven_day": {"utilization": -1}}}));
        assert!(five.is_none() && week.is_none());
        // A window that has not started has no reset.
        let (five, _) = windows_of(&json!({"unifiedWindows": {"five_hour": {"utilization": 0}}}));
        assert_eq!(five.unwrap().resets_at, None);
    }

    #[test]
    fn probes_skip_valid_readings_used_up_windows_and_back_off() {
        let now = 100_000_000;
        let w = |used: f64, resets_at: Option<i64>| Some(QuotaWindow { used_percent: used, resets_at });
        let mut q = QuotaStatus { quota_at: Some(now - 60_000), five_hour: w(20.0, Some(now / 1000 + 3600)), weekly: w(30.0, None), ..Default::default() };
        let mut g = Gate::default();
        assert!(skip_reason(&q, &g, now).unwrap().contains("still valid"));
        // Unknown and never probed: probe.
        assert_eq!(skip_reason(&QuotaStatus::default(), &g, now), None);
        // The window's reset passed: the reading no longer counts.
        q.five_hour = w(20.0, Some(now / 1000 - 1));
        assert_eq!(skip_reason(&q, &g, now), None);
        // A used-up window that has not reset: never a model request.
        q.five_hour = w(100.0, Some(now / 1000 + 60));
        assert!(skip_reason(&q, &g, now).unwrap().contains("used up"));
        q.five_hour = w(100.0, None);
        assert!(skip_reason(&q, &g, now).unwrap().contains("used up"));
        q.five_hour = w(100.0, Some(now / 1000 - 60));
        assert_eq!(skip_reason(&q, &g, now), None);
        // Failures back off 1, 2, 4… minutes up to an hour; 429 waits at least 5 minutes; a success resets it.
        let fail = |g: &mut Gate, text: &str| g.record(&Err(anyhow::anyhow!("{text}")), now);
        fail(&mut g, "timed out");
        assert_eq!(g.not_before, now + 60_000);
        assert!(skip_reason(&QuotaStatus::default(), &g, now + 59_000).unwrap().contains("next probe"));
        assert_eq!(skip_reason(&QuotaStatus::default(), &g, now + 60_000), None);
        fail(&mut g, "timed out");
        assert_eq!(g.not_before, now + 120_000);
        fail(&mut g, "the request failed (HTTP 429): rate limited");
        assert_eq!(g.not_before, now + 5 * 60_000);
        for _ in 0..20 { fail(&mut g, "timed out"); }
        assert_eq!(g.not_before, now + 60 * 60_000);
        g.record(&Ok((w(1.0, None), w(1.0, None))), now);
        assert_eq!((g.failures, g.not_before), (0, now + MIN_GAP_MS));
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_probe() {
        let gate = Arc::new(tokio::sync::Mutex::new(Gate::default()));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let asked = now_ms();
        let probe = |calls: Arc<std::sync::atomic::AtomicUsize>| async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok((Some(QuotaWindow { used_percent: 5.0, resets_at: None }), Some(QuotaWindow { used_percent: 6.0, resets_at: None })))
        };
        let (a, b) = tokio::join!(gated(&gate, asked, || probe(calls.clone())), gated(&gate, asked, || probe(calls.clone())));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(a.unwrap(), b.unwrap());
        // A later request probes again; a failure is shared the same way.
        tokio::time::sleep(Duration::from_millis(2)).await;
        let later = now_ms();
        let failing = |calls: Arc<std::sync::atomic::AtomicUsize>| async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err::<QuotaPair, _>(anyhow::anyhow!("the request failed (HTTP 500)"))
        };
        let (a, b) = tokio::join!(gated(&gate, later, || failing(calls.clone())), gated(&gate, later, || failing(calls.clone())));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(a.is_err() && b.unwrap_err().to_string().contains("HTTP 500"));
    }
}

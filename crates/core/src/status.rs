//! Status bar data: harness versions and quotas.
//!
//! - Versions: `claude --version` and `codex --version` at startup and every 30 minutes.
//! - Claude quota: `/api/oauth/usage` every 5 minutes (60 seconds while protecting sessions), plus
//!   `rate_limit_event` from running sessions. The token is never refreshed here (docs/spikes/2-…).
//! - Codex quota: `account/rateLimits/read` every 2 minutes (30 seconds while protecting sessions),
//!   plus `account/rateLimits/updated`.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::App;
use crate::codex_rpc::Incoming;
use crate::events::Bus;
use crate::util::now_ms;

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct Status {
    pub profile: String,
    pub claude: HarnessStatus,
    pub codex: HarnessStatus,
    pub sessions: SessionCounts,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct HarnessStatus {
    pub version: Option<String>,
    pub version_error: Option<String>,
    /// Version being installed by the background update (Codex only).
    pub updating_to: Option<String>,
    pub five_hour: Option<QuotaWindow>,
    pub weekly: Option<QuotaWindow>,
    /// When the quota was last read (Unix milliseconds).
    #[ts(type = "number | null")]
    pub quota_at: Option<i64>,
    pub quota_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct QuotaWindow {
    /// 0 to 100.
    pub used_percent: f64,
    /// Unix seconds.
    #[ts(type = "number")]
    pub resets_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct SessionCounts {
    pub running: u32,
    pub done: u32,
    pub quota_paused: u32,
}

#[derive(Clone)]
pub struct StatusHandle {
    inner: Arc<RwLock<Status>>,
    bus: Bus,
}

impl StatusHandle {
    pub fn new(bus: Bus) -> StatusHandle {
        StatusHandle { inner: Arc::new(RwLock::new(Status::default())), bus }
    }

    pub fn get(&self) -> Status {
        self.inner.read().unwrap().clone()
    }

    /// Changes the status and pushes it to the UI.
    pub fn update(&self, f: impl FnOnce(&mut Status)) {
        let payload = {
            let mut s = self.inner.write().unwrap();
            f(&mut s);
            serde_json::to_value(&*s).unwrap_or(Value::Null)
        };
        self.bus.publish("status", "changed", payload);
    }

    /// Quota from a Claude `rate_limit_event` (fractions and Unix seconds).
    pub fn claude_rate_limit_event(&self, info: &Value) {
        let windows = &info["unifiedWindows"];
        let win = |k: &str| -> Option<QuotaWindow> {
            let w = &windows[k];
            Some(QuotaWindow {
                used_percent: w["utilization"].as_f64()? * 100.0,
                resets_at: w["resetsAt"].as_i64()?,
            })
        };
        let (five, week) = (win("five_hour"), win("seven_day"));
        if five.is_none() && week.is_none() {
            return;
        }
        self.update(|s| {
            s.claude.five_hour = five;
            s.claude.weekly = week;
            s.claude.quota_at = Some(now_ms());
            s.claude.quota_error = None;
        });
    }

    /// Quota from a Codex `rateLimits` object.
    pub fn codex_rate_limits(&self, rl: &Value) {
        let mut five = None;
        let mut week = None;
        for key in ["primary", "secondary"] {
            let w = &rl[key];
            let (Some(used), Some(resets), mins) =
                (w["usedPercent"].as_f64(), w["resetsAt"].as_i64(), w["windowDurationMins"].as_i64())
            else {
                continue;
            };
            let q = QuotaWindow { used_percent: used, resets_at: resets };
            match (key, mins) {
                (_, Some(300)) => five = Some(q),
                (_, Some(10080)) => week = Some(q),
                ("primary", _) => five = Some(q),
                _ => week = Some(q),
            }
        }
        self.update(|s| {
            s.codex.five_hour = five;
            s.codex.weekly = week;
            s.codex.quota_at = Some(now_ms());
            s.codex.quota_error = None;
        });
    }
}

pub fn spawn_monitors(app: &App) {
    app.status.update(|s| s.profile = app.paths.profile.name().to_string());

    let a = app.clone();
    tokio::spawn(async move {
        loop {
            refresh_versions(&a).await;
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });

    let a = app.clone();
    tokio::spawn(async move {
        let client = http_client();
        loop {
            match claude_usage(&client).await {
                Ok((five, week)) => a.status.update(|s| {
                    s.claude.five_hour = five;
                    s.claude.weekly = week;
                    s.claude.quota_at = Some(now_ms());
                    s.claude.quota_error = None;
                }),
                Err(e) => {
                    tracing::info!("claude usage: {e:#}");
                    a.status.update(|s| s.claude.quota_error = Some(format!("{e:#}")));
                }
            }
            wait_poll(&a, crate::model::HarnessKind::Claude, 300, 60).await;
        }
    });

    let a = app.clone();
    tokio::spawn(async move {
        let mut rx = a.codex.subscribe();
        loop {
            match rx.recv().await {
                Ok(Incoming::Notification { method, params }) if method == "account/rateLimits/updated" => {
                    a.status.codex_rate_limits(&params["rateLimits"]);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });

    let a = app.clone();
    tokio::spawn(async move {
        loop {
            match a.codex.request("account/rateLimits/read", Value::Null).await {
                Ok(res) => a.status.codex_rate_limits(&res["rateLimits"]),
                Err(e) => {
                    tracing::info!("codex rate limits: {e:#}");
                    a.status.update(|s| s.codex.quota_error = Some(format!("{e:#}")));
                }
            }
            wait_poll(&a, crate::model::HarnessKind::Codex, 120, 30).await;
        }
    });
}

pub async fn refresh_versions(app: &App) {
    let claude = crate::procs::output(&app.env, "claude", &["--version"], None).await;
    let codex = crate::procs::output(&app.env, "codex", &["--version"], None).await;
    app.status.update(|s| {
        match claude {
            // "2.1.287 (Claude Code)"
            Ok(v) => {
                s.claude.version = v.split_whitespace().next().map(str::to_string);
                s.claude.version_error = None;
            }
            Err(e) => s.claude.version_error = Some(format!("{e:#}")),
        }
        match codex {
            // "codex-cli 0.159.3"
            Ok(v) => {
                s.codex.version = v.split_whitespace().last().map(str::to_string);
                s.codex.version_error = None;
            }
            Err(e) => s.codex.version_error = Some(format!("{e:#}")),
        }
    });
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("launchpad/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

/// Reads `/api/oauth/usage`. Skips the call when the access token has expired: Claude Code refreshes it.
async fn claude_usage(client: &reqwest::Client) -> Result<(Option<QuotaWindow>, Option<QuotaWindow>)> {
    let path = dirs::home_dir().unwrap_or_default().join(".claude/.credentials.json");
    let creds: Value = serde_json::from_str(&std::fs::read_to_string(&path).context("read Claude credentials")?)?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().context("no Claude OAuth token")?;
    if let Some(exp) = oauth["expiresAt"].as_i64()
        && exp <= now_ms() {
        bail!("Claude OAuth token expired, waiting for Claude Code to refresh it");
    }
    let res = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await?;
    let code = res.status();
    if !code.is_success() {
        bail!("usage endpoint returned {code}");
    }
    let body: Value = res.json().await?;
    let win = |k: &str| -> Option<QuotaWindow> {
        let w = &body[k];
        let resets = w["resets_at"].as_str()?;
        let t = time::OffsetDateTime::parse(resets, &time::format_description::well_known::Rfc3339).ok()?;
        Some(QuotaWindow { used_percent: w["utilization"].as_f64()?, resets_at: t.unix_timestamp() })
    };
    Ok((win("five_hour"), win("seven_day")))
}

async fn wait_poll(app: &App, harness: crate::model::HarnessKind, idle: u64, active: u64) {
    let start = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let delay = if crate::quota::monitoring(app, harness) { active } else { idle };
        if start.elapsed().as_secs() >= delay { return; }
    }
}

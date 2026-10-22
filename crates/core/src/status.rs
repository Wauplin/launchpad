//! Status bar data: harness versions and quotas.
//!
//! Each adapter owns discovery and native quota polling; the host publishes status by registry ID.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::App;
use crate::events::Bus;

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct Status {
    pub env: String,
    pub commit: String,
    /// Host name of the machine the daemon runs on (shown to browsers on other devices).
    pub host: String,
    pub harnesses: std::collections::HashMap<String, HarnessStatus>,
    /// Included quota of each monitored subscription login, keyed by `profiles::account`.
    pub quotas: std::collections::HashMap<String, QuotaStatus>,
    pub sessions: SessionCounts,
    /// IANA name of the daemon host time zone (crons use it), when the host exposes one.
    pub time_zone: Option<String>,
    /// Current UTC offset of the daemon host, in minutes east of UTC.
    pub utc_offset_minutes: i32,
    /// SSH hosts and their connection state.
    pub hosts: Vec<crate::hosts::HostStatus>,
    /// Rolling spending limits of pay-as-you-go profiles.
    pub spend: Vec<crate::spend::ProfileSpend>,
    /// Profiles with unpriced historical usage; rolling cost management is unavailable.
    pub cost_unavailable_profiles: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct HarnessStatus {
    pub version: Option<String>,
    pub version_error: Option<String>,
    /// Version being installed by an adapter background update.
    pub updating_to: Option<String>,
}

/// The included quota of one subscription login (a harness, and the account variables of its profiles).
#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct QuotaStatus {
    pub harness: String,
    /// Subscription profiles using this login, in configuration order.
    pub profiles: Vec<String>,
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
    /// Unix seconds; `None` while the window has not started (no usage since the last reset).
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
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
    quota_db: Option<crate::db::Db>,
}

impl StatusHandle {
    pub fn new(bus: Bus) -> StatusHandle {
        StatusHandle { inner: Arc::new(RwLock::new(Status::default())), bus, quota_db: None }
    }

    pub fn with_quota_history(bus: Bus, db: crate::db::Db) -> Self {
        Self { quota_db: Some(db), ..Self::new(bus) }
    }

    /// Serialize accepted readings into the DB queue while holding the status lock, so native
    /// notifications and polling cannot persist snapshots out of order.
    pub fn update_quota(&self, account: &str, five: Option<QuotaWindow>, week: Option<QuotaWindow>) {
        let mut pending = None;
        let id = account.to_string();
        self.update(|status| {
            // A reading for a login that is no longer monitored is dropped.
            let Some(s) = status.quotas.get_mut(&id) else { return };
            crate::quota::update_reading(s, five, week);
            if s.quota_error.is_none()
                && let (Some(db), Some(week), Some(at)) = (&self.quota_db, &s.weekly, s.quota_at)
            {
                let week = week.clone();
                pending = Some(db.enqueue(move |conn| crate::quota_history::record(conn, &id, at, &week)));
            }
        });
        if let Some(pending) = pending {
            tokio::spawn(async move {
                if let Err(error) = pending.await {
                    tracing::warn!(%error, "persist weekly quota reading");
                }
            });
        }
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

    pub fn quota_error(&self, account: &str, error: String) {
        self.update(|s| if let Some(q) = s.quotas.get_mut(account) { q.quota_error = Some(error) });
    }

    pub fn update_harness(
        &self,
        id: crate::model::HarnessKind,
        f: impl FnOnce(&mut HarnessStatus),
    ) {
        self.update(|s| f(s.harnesses.entry(id.as_str().into()).or_default()));
    }
}

pub fn spawn_monitors(app: &App) {
    app.status.update(|s| {
        s.env = app.paths.app_env.name().to_string();
        s.commit = crate::COMMIT.to_string();
        s.time_zone = host_time_zone();
        s.host = host_name();
        s.utc_offset_minutes = utc_offset_minutes();
        for registration in crate::harness::REGISTRATIONS {
            s.harnesses
                .entry(registration.descriptor.id.as_str().into())
                .or_default();
        }
    });
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            // The offset changes with daylight saving time.
            let offset = utc_offset_minutes();
            if app.status.get().utc_offset_minutes != offset {
                app.status.update(|s| s.utc_offset_minutes = offset);
            }
            refresh_versions(&app).await;
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
}

fn host_name() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname writes a NUL-terminated name into it.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
        String::new()
    }
    #[cfg(not(unix))]
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

fn utc_offset_minutes() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// The zone chrono's `Local` (and so the cron scheduler) uses: `TZ`, else `/etc/localtime` (or `/etc/timezone`).
fn host_time_zone() -> Option<String> {
    let valid = |name: &str| {
        let name = name.trim().trim_start_matches(':');
        (!name.is_empty() && !name.starts_with('/') && name.chars().all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))).then(|| name.to_string())
    };
    if let Ok(tz) = std::env::var("TZ") {
        return valid(&tz).or_else(|| tz.rsplit_once("zoneinfo/").and_then(|(_, n)| valid(n)));
    }
    #[cfg(unix)]
    {
        if let Ok(target) = std::fs::read_link("/etc/localtime")
            && let Some((_, name)) = target.to_string_lossy().rsplit_once("zoneinfo/")
            && let Some(name) = valid(name)
        {
            return Some(name);
        }
        if let Ok(name) = std::fs::read_to_string("/etc/timezone") {
            return valid(&name);
        }
    }
    None
}

pub async fn refresh_versions(app: &App) {
    for registration in crate::harness::REGISTRATIONS {
        let id = registration.descriptor.id;
        let result =
            tokio::time::timeout(Duration::from_secs(15), app.harnesses.get(id).version(app)).await;
        app.status.update_harness(id, |s| match result {
            Ok(Ok(version)) => {
                s.version = Some(version);
                s.version_error = None;
            }
            Ok(Err(error)) => s.version_error = Some(format!("{error:#}")),
            Err(_) => s.version_error = Some("Version check timed out".into()),
        });
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("launchpad/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

pub(crate) async fn wait_poll(
    app: &App,
    account: &str,
    idle: u64,
    active: u64,
) {
    let start = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let delay = if crate::quota::monitoring(app, account) { active } else { idle };
        if start.elapsed().as_secs() >= delay { return; }
    }
}

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
    pub profile: String,
    pub harnesses: std::collections::HashMap<String, HarnessStatus>,
    pub sessions: SessionCounts,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct HarnessStatus {
    pub version: Option<String>,
    pub version_error: Option<String>,
    /// Version being installed by an adapter background update.
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
        s.profile = app.paths.profile.name().to_string();
        for registration in crate::harness::REGISTRATIONS {
            s.harnesses
                .entry(registration.descriptor.id.as_str().into())
                .or_default();
        }
    });
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            refresh_versions(&app).await;
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
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
    harness: crate::model::HarnessKind,
    idle: u64,
    active: u64,
) {
    let start = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let delay = if crate::quota::monitoring(app, harness) { active } else { idle };
        if start.elapsed().as_secs() >= delay { return; }
    }
}

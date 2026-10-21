//! Rolling spending limits of pay-as-you-go variants (`spend_limits`), from reported session costs.
//!
//! Every increase of a session's reported USD cost is recorded in `variant_cost` with its time.
//! A limit such as "$50 per 5 hours" sums the reported cost of the variant over the last 5 hours;
//! at or above it, new input is refused and running turns are interrupted best effort, like the
//! per-session cost limit. Costs reported late or never (Codex) cannot be enforced.
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use ts_rs::TS;

use crate::{App, config::SpendLimit, model::LaunchSpec, util::now_ms};

/// The longest window, and the age of the oldest entry kept.
pub const MAX_HOURS: u32 = 31 * 24;

#[derive(Clone, Debug)]
pub struct Entry {
    pub variant: String,
    pub at: i64,
    pub usd: f64,
}

/// Spending of one variant, published in the status for the status bar.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct VariantSpend {
    pub variant: String,
    pub name: String,
    pub harness: crate::model::HarnessKind,
    pub windows: Vec<SpendWindow>,
}

#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct SpendWindow {
    pub hours: u32,
    pub limit_usd: f64,
    /// Reported cost in the last `hours`, rounded to cents.
    pub used_usd: f64,
    /// When a reached limit has room again as older costs leave the window (Unix seconds).
    #[ts(type = "number | null")]
    pub available_at: Option<i64>,
}

impl SpendWindow {
    pub fn reached(&self) -> bool {
        self.used_usd >= self.limit_usd
    }
    pub fn label(&self) -> String {
        label(self.hours)
    }
}

/// "5 hours", "1 week", "3 days".
pub fn label(hours: u32) -> String {
    let (n, unit) = if hours.is_multiple_of(168) { (hours / 168, "week") } else if hours.is_multiple_of(24) { (hours / 24, "day") } else { (hours, "hour") };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

pub(crate) async fn load(app: &App) -> Result<Vec<Entry>> {
    let since = now_ms() - i64::from(MAX_HOURS) * 3_600_000;
    app.state_db.call(move |conn| {
        conn.execute("DELETE FROM variant_cost WHERE at < ?1", [since])?;
        let mut stmt = conn.prepare("SELECT variant, at, usd FROM variant_cost ORDER BY at")?;
        Ok(stmt.query_map([], |r| Ok(Entry { variant: r.get(0)?, at: r.get(1)?, usd: r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

pub(crate) async fn record(app: &App, variant: &str, usd: f64) {
    if !(usd.is_finite() && usd > 0.0) { return; }
    let entry = Entry { variant: variant.to_string(), at: now_ms(), usd };
    app.sessions.spend_entries(|entries| entries.push(entry.clone()));
    let result = app.state_db.call(move |conn| {
        conn.execute("INSERT INTO variant_cost (variant, at, usd) VALUES (?1, ?2, ?3)", params![entry.variant, entry.at, entry.usd])?;
        Ok(())
    }).await;
    if let Err(e) = result { tracing::warn!("record variant cost: {e:#}"); }
    publish(app);
}

fn window(entries: &[Entry], variant: &str, limit: &SpendLimit, now: i64) -> SpendWindow {
    let span = i64::from(limit.hours) * 3_600_000;
    let inside: Vec<&Entry> = entries.iter().filter(|e| e.variant == variant && e.at > now - span).collect();
    let used: f64 = inside.iter().map(|e| e.usd).sum();
    // Entries are in time order: room returns once enough of the oldest ones leave the window.
    let mut available_at = None;
    if used >= limit.usd {
        let mut left = used;
        for e in &inside {
            left -= e.usd;
            if left < limit.usd {
                available_at = Some((e.at + span) / 1000 + 1);
                break;
            }
        }
    }
    SpendWindow { hours: limit.hours, limit_usd: limit.usd, used_usd: (used * 100.0).round() / 100.0, available_at }
}

/// Every variant with spending limits, in configuration order.
pub fn current(app: &App) -> Vec<VariantSpend> {
    let config = app.config.get();
    let now = now_ms();
    app.sessions.spend_entries(|entries| {
        config.variants.iter().filter(|v| !v.spend_limits.is_empty()).map(|v| VariantSpend {
            variant: v.id.clone(),
            name: v.name.clone(),
            harness: v.harness,
            windows: v.spend_limits.iter().map(|l| window(entries, &v.id, l, now)).collect(),
        }).collect()
    })
}

/// Reached windows of the variant of a spec; empty when it has none or is not pay as you go.
pub fn reached(app: &App, spec: &LaunchSpec) -> Vec<SpendWindow> {
    if !crate::variants::pay_as_you_go(app, spec) { return vec![]; }
    let Some(id) = spec.variant.as_deref() else { return vec![] };
    current(app).into_iter().find(|v| v.variant == id).map(|v| v.windows.into_iter().filter(SpendWindow::reached).collect()).unwrap_or_default()
}

/// Refuses new input of a pay-as-you-go variant at one of its spending limits.
pub fn ensure(app: &App, spec: &LaunchSpec) -> Result<()> {
    let reached = reached(app, spec);
    let Some(w) = reached.first() else { return Ok(()) };
    let name = spec.variant.as_deref().and_then(|id| app.config.get().variant(id).map(|v| v.name.clone())).unwrap_or_default();
    anyhow::bail!("{}", message(&name, w))
}

pub fn message(name: &str, w: &SpendWindow) -> String {
    let again = w.available_at.map(|t| format!(" Room returns around {}.", crate::util::local_time(t))).unwrap_or_default();
    format!("{name} spending limit reached: ${:.2} reported in the last {} of ${:.2}.{again} Raise the limit in Settings → Harnesses to continue now.", w.used_usd, w.label(), w.limit_usd)
}

/// Publishes the spending windows when they changed (costs leave rolling windows over time).
pub fn publish(app: &App) {
    let spend = current(app);
    if app.status.get().spend != spend {
        app.status.update(|s| s.spend = spend);
    }
}

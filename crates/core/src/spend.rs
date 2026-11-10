//! Rolling spending limits of pay-as-you-go profiles (`spend_limits`), from reported session costs.
//!
//! Every increase of a session's reported USD cost is recorded in `profile_cost` with its time and an ID.
//! A limit such as "$50 per 5 hours" sums the reported cost of the profile over the last 5 hours;
//! at or above it, new input is refused and running turns are interrupted best effort, like the
//! per-session cost limit. Costs reported late or never (Codex) cannot be enforced.
//!
//! Home holds the ledger of every machine: a runner (Burst jobs included) queues each increase for Home
//! (`replica::Item::Spend`, durable), Home stores it once by ID and sends it to every runner
//! (`ToRunner::Spend`, the whole ledger on connect), so each runner checks the limits against the costs of
//! all machines plus its own increases Home has not stored yet, and Home's status shows the real spend.
use std::collections::HashSet;

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{App, config::SpendLimit, model::LaunchSpec, util::now_ms};

/// The longest window, and the age of the oldest entry kept.
pub const MAX_HOURS: u32 = 31 * 24;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unique across machines: the same increase reported twice counts once.
    pub id: String,
    pub profile: String,
    pub at: i64,
    pub usd: f64,
}

/// The entries of every machine this daemon knows, in time order, each ID once.
#[derive(Default)]
pub struct Ledger {
    entries: Vec<Entry>,
    ids: HashSet<String>,
}

impl Ledger {
    /// Adds an entry unless its ID is known; drops entries older than `MAX_HOURS`. Returns whether it was new.
    pub fn insert(&mut self, entry: Entry, now: i64) -> bool {
        let floor = now - i64::from(MAX_HOURS) * 3_600_000;
        if entry.at < floor || !self.ids.insert(entry.id.clone()) {
            return false;
        }
        let at = entry.at;
        self.entries.push(entry);
        if self.entries.len() > 1 && self.entries[self.entries.len() - 2].at > at {
            self.entries.sort_by_key(|e| e.at);
        }
        if self.entries.first().is_some_and(|e| e.at < floor) {
            self.entries.retain(|e| e.at >= floor);
            self.ids = self.entries.iter().map(|e| e.id.clone()).collect();
        }
        true
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// Spending of one profile, published in the status for the status bar.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct ProfileSpend {
    pub profile: String,
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
        conn.execute("DELETE FROM profile_cost WHERE at < ?1", [since])?;
        let mut stmt = conn.prepare("SELECT coalesce(id, 'row-' || rowid), profile, at, usd FROM profile_cost ORDER BY at")?;
        Ok(stmt.query_map([], |r| Ok(Entry { id: r.get(0)?, profile: r.get(1)?, at: r.get(2)?, usd: r.get(3)? }))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

/// A cost increase of a session that runs here; a runner also queues it for Home in the same transaction.
pub(crate) async fn record(app: &App, profile: &str, usd: f64) {
    if !(usd.is_finite() && usd > 0.0) { return; }
    let entry = Entry { id: uuid::Uuid::new_v4().to_string(), profile: profile.to_string(), at: now_ms(), usd };
    app.sessions.spend_entries(|l| l.insert(entry.clone(), entry.at));
    let result = app.state_db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO profile_cost (id, profile, at, usd) VALUES (?1, ?2, ?3, ?4)", params![entry.id, entry.profile, entry.at, entry.usd])?;
        crate::replica::queue(&tx, &crate::replica::Item::Spend { entry })?;
        tx.commit()?;
        Ok(())
    }).await;
    if let Err(e) = result { tracing::warn!("record profile cost: {e:#}"); }
    publish(app);
}

/// Home: a runner's cost increase, stored once by ID with the runner's acknowledgement; a new one goes to
/// every runner.
pub(crate) async fn apply_replica(app: &App, runner_id: &str, n: u64, mut entry: Entry) -> Result<()> {
    let now = now_ms();
    // A runner's clock may run ahead: an entry never counts in the future.
    entry.at = entry.at.min(now);
    let valid = entry.usd.is_finite() && entry.usd > 0.0 && !entry.id.is_empty();
    let runner = runner_id.to_string();
    let stored = entry.clone();
    let added = app.state_db.call(move |c| {
        let tx = c.transaction()?;
        let added = valid && tx.execute("INSERT OR IGNORE INTO profile_cost (id, profile, at, usd) VALUES (?1, ?2, ?3, ?4)", params![stored.id, stored.profile, stored.at, stored.usd])? > 0;
        crate::replica::set_acked(&tx, &runner, n)?;
        tx.commit()?;
        Ok(added)
    }).await?;
    if added && app.sessions.spend_entries(|l| l.insert(entry.clone(), now)) {
        publish(app);
        app.runners.broadcast(crate::runners::proto::ToRunner::Spend { entries: vec![entry] });
    }
    Ok(())
}

/// Home: the entries runners need, those of profiles with spending limits inside their longest window (sent on
/// connect and after a configuration change).
pub fn for_runners(app: &App) -> Vec<Entry> {
    let config = app.config.get();
    let now = now_ms();
    app.sessions.spend_entries(|l| {
        l.entries().iter().filter(|e| config.profile(&e.profile).and_then(|v| v.spend_limits.iter().map(|l| l.hours).max()).is_some_and(|h| e.at > now - i64::from(h) * 3_600_000)).cloned().collect()
    })
}

/// Runner: entries from Home's ledger (other machines' costs, and its own once Home stored them).
pub fn apply_home(app: &App, entries: Vec<Entry>) {
    let now = now_ms();
    let added = app.sessions.spend_entries(|l| entries.into_iter().fold(false, |added, e| l.insert(e, now) || added));
    if added {
        publish(app);
    }
}

fn window(entries: &[Entry], profile: &str, limit: &SpendLimit, now: i64) -> SpendWindow {
    let span = i64::from(limit.hours) * 3_600_000;
    let inside: Vec<&Entry> = entries.iter().filter(|e| e.profile == profile && e.at > now - span).collect();
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

/// Every profile with spending limits, in configuration order.
pub fn current(app: &App) -> Vec<ProfileSpend> {
    let config = app.config.get();
    let now = now_ms();
    let unavailable = app.sessions.profiles_without_cost();
    app.sessions.spend_entries(|ledger| {
        let entries = ledger.entries();
        config.profiles.iter().filter(|v| !v.spend_limits.is_empty() && crate::harness::registration(v.harness).descriptor.supports_cost(v.model.as_deref()) && !unavailable.contains(&v.id)).map(|v| ProfileSpend {
            profile: v.id.clone(),
            name: v.name.clone(),
            harness: v.harness,
            windows: v.spend_limits.iter().map(|l| window(entries, &v.id, l, now)).collect(),
        }).collect()
    })
}

/// Reached windows of the profile of a spec; empty when it has none or is not pay as you go.
pub fn reached(app: &App, spec: &LaunchSpec) -> Vec<SpendWindow> {
    if !crate::profiles::pay_as_you_go(app, spec) { return vec![]; }
    let Some(id) = spec.profile.as_deref() else { return vec![] };
    let harness = spec.harness.or_else(|| app.config.get().profile(id).map(|p| p.harness));
    if !harness.is_some_and(|h| crate::profiles::supports_cost(app, h, spec, spec.model.as_deref())) { return vec![]; }
    current(app).into_iter().find(|v| v.profile == id).map(|v| v.windows.into_iter().filter(SpendWindow::reached).collect()).unwrap_or_default()
}

/// Refuses new input of a pay-as-you-go profile at one of its spending limits.
pub fn ensure(app: &App, spec: &LaunchSpec) -> Result<()> {
    let reached = reached(app, spec);
    let Some(w) = reached.first() else { return Ok(()) };
    let name = spec.profile.as_deref().and_then(|id| app.config.get().profile(id).map(|v| v.name.clone())).unwrap_or_default();
    anyhow::bail!("{}", message(&name, w))
}

pub fn message(name: &str, w: &SpendWindow) -> String {
    let again = w.available_at.map(|t| format!(" Room returns around {}.", crate::util::local_time(t))).unwrap_or_default();
    format!("{name} spending limit reached: ${:.2} reported in the last {} of ${:.2}.{again} Raise the limit in Settings → Profiles to continue now.", w.used_usd, w.label(), w.limit_usd)
}

/// Publishes the spending windows when they changed (costs leave rolling windows over time).
pub fn publish(app: &App) {
    let spend = current(app);
    let mut unavailable: Vec<_> = app.sessions.profiles_without_cost().into_iter().collect();
    unavailable.sort();
    let status = app.status.get();
    if status.spend != spend || status.cost_unavailable_profiles != unavailable {
        app.status.update(|s| { s.spend = spend; s.cost_unavailable_profiles = unavailable; });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, at: i64, usd: f64) -> Entry {
        Entry { id: id.into(), profile: "p".into(), at, usd }
    }

    #[test]
    fn the_ledger_counts_each_entry_once_in_time_order_and_forgets_old_ones() {
        let hour = 3_600_000;
        let now = i64::from(MAX_HOURS) * hour * 2;
        let mut l = Ledger::default();
        assert!(l.insert(e("a", now - 2 * hour, 1.0), now));
        assert!(l.insert(e("b", now - hour, 2.0), now));
        // Home sends back the runner's own entry, and an older one of another machine.
        assert!(!l.insert(e("b", now - hour, 2.0), now));
        assert!(l.insert(e("c", now - 3 * hour, 4.0), now));
        assert_eq!(l.entries().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        let limit = SpendLimit { hours: 5, usd: 5.0 };
        let w = window(l.entries(), "p", &limit, now);
        assert_eq!(w.used_usd, 7.0);
        // Room returns when "c" leaves the 5-hour window.
        assert_eq!(w.available_at, Some((now - 3 * hour + 5 * hour) / 1000 + 1));
        // Older than the longest window: never added, and dropped once time passes.
        assert!(!l.insert(e("d", now - i64::from(MAX_HOURS) * hour - 1, 1.0), now));
        assert!(l.insert(e("e", now + i64::from(MAX_HOURS) * hour - 2 * hour, 1.0), now + i64::from(MAX_HOURS) * hour - 2 * hour + 1));
        assert_eq!(l.entries().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b", "e"]);
        assert!(l.insert(e("a", now, 1.0), now), "a forgotten ID may come again");
    }
}

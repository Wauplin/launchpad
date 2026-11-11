//! Best-effort per-session protection against additional subscription spending.
//!
//! Each subscription login (`profiles::account`: a harness and the account variables of its profiles) has
//! its own monitor and reading, so several accounts of one harness are protected independently.
use crate::{
    App,
    model::{HarnessKind, LaunchSpec, QuotaBlock, SessionStatus},
    status::QuotaStatus,
};
use std::collections::{BTreeMap, HashMap};

/// The 5-hour and weekly windows of one reading.
pub type QuotaPair = (Option<crate::status::QuotaWindow>, Option<crate::status::QuotaWindow>);

/// A monitored subscription login, given to `Harness::spawn_quota`.
#[derive(Clone, Debug)]
pub struct QuotaAccount {
    /// `profiles::account`: the key of its reading in `Status.quotas`.
    pub key: String,
    pub harness: HarnessKind,
    /// The variables of Home's copy of the vault login (`vault::login_home`: `CLAUDE_CONFIG_DIR` and its token,
    /// `CODEX_HOME`), never the machine's own login.
    pub env: Vec<(String, String)>,
}

pub fn reading(app: &App, account: &str) -> QuotaStatus {
    app.status.get().quotas.get(account).cloned().unwrap_or_default()
}

/// The reading of the login that protects this spec.
pub fn reading_for(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> QuotaStatus {
    reading(app, &crate::profiles::account(app, harness, spec))
}

/// An incomplete provider snapshot cannot erase an exhausted window or authorize automatic work.
pub fn update_reading(s: &mut QuotaStatus, five: Option<crate::status::QuotaWindow>, week: Option<crate::status::QuotaWindow>) {
    // Any reading came with the login: the provider accepts it.
    s.refused = false;
    if [&five, &week].into_iter().any(|w| w.as_ref().is_none_or(|w| !w.used_percent.is_finite() || w.used_percent < 0.0)) {
        // Partial evidence may add a blocker, but only a complete reading may remove one.
        for (stored, incoming) in [(&mut s.five_hour, five), (&mut s.weekly, week)] {
            if let Some(w) = incoming.filter(|w| w.used_percent.is_finite() && w.used_percent >= 100.0) {
                *stored = Some(w);
                s.quota_at = Some(crate::util::now_ms());
            }
        }
        s.quota_error = Some("Missing or invalid quota windows; included capacity is unconfirmed.".into());
        return;
    }
    s.five_hour = five;
    s.weekly = week;
    s.quota_at = Some(crate::util::now_ms());
    s.quota_error = None;
}

/// Keep an exhausted reading blocking until a newer reading confirms availability.
pub fn exhausted(s: &QuotaStatus) -> Vec<QuotaBlock> {
    [("5-hour", &s.five_hour), ("Weekly", &s.weekly)]
        .into_iter()
        .filter_map(|(name, w)| {
            w.as_ref()
                .filter(|w| w.used_percent >= 100.0)
                .map(|w| QuotaBlock {
                    window: name.into(),
                    resets_at: w.resets_at,
                })
        })
        .collect()
}

pub fn blockers(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> Vec<QuotaBlock> {
    exhausted(&reading_for(app, harness, spec))
}

pub fn available(s: &QuotaStatus, now: i64) -> bool {
    s.quota_error.is_none()
        && s.quota_at
            .is_some_and(|at| (0..=90_000).contains(&(now - at)))
        && [&s.five_hour, &s.weekly].into_iter().all(|w| {
            w.as_ref().is_some_and(|w| {
                w.used_percent.is_finite()
                    && (0.0..100.0).contains(&w.used_percent)
                    // A window that has not started has no reset; a passed reset means the reading is outdated.
                    && w.resets_at.is_none_or(|r| r > now / 1000)
            })
        })
}

/// Best-effort recovery for an already paused session when usage cannot confirm a reset.
/// Never override exhaustion reported after that reset or a known weekly blocker.
pub fn reset_fallback(s: &QuotaStatus, pause: &crate::model::QuotaPause, now: i64) -> bool {
    if available(s, now) { return false; }
    let Some(stored) = pause.blockers.iter().find(|b| b.window == "5-hour").and_then(|b| b.resets_at) else { return false };
    let reset = s.five_hour.as_ref().and_then(|w| w.resets_at).unwrap_or(stored);
    if reset <= 0 || now / 1000 < reset.saturating_add(60) { return false; }
    // A newer exhausted snapshot is evidence that this reset did not restore capacity.
    if s.quota_at.is_some_and(|at| at / 1000 >= reset)
        && !exhausted(s).is_empty() { return false; }
    // A complete newer reading can clear an old weekly blocker; unknown readings cannot.
    let weekly_blocked = s.weekly.as_ref().is_some_and(|w| w.used_percent >= 100.0)
        || (pause.blockers.iter().any(|b| b.window == "Weekly")
            && (s.quota_error.is_some() || s.weekly.is_none() || s.quota_at.is_none()));
    !weekly_blocked
}

/// Subscription protection applies: a monitored harness, paid usage not allowed, and not a pay-as-you-go profile.
pub fn protected(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> bool {
    crate::harness::registration(harness).descriptor.capabilities.quota
        && !spec.allow_paid_usage
        && !crate::profiles::pay_as_you_go(app, spec)
}

/// Automatic work requires confirmed capacity; manual starts may use best-effort protection.
pub fn automatic_ready(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> bool {
    hold_reason(app, harness, spec).is_none()
}

/// Why automatic work of this spec waits for its quota now, or None when it may start. A wait without a fresh
/// reading asks the login's monitor for one (`want_reading`) and counts in `holds`.
pub fn hold_reason(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> Option<String> {
    if !protected(app, harness, spec) { return None; }
    let key = crate::profiles::account(app, harness, spec);
    let key = resolve_vault_key(&key).unwrap_or(key);
    let q = reading(app, &key);
    let now = crate::util::now_ms();
    let why = hold_text(crate::harness::registration(harness).descriptor.name, &key, &q, now);
    let mut held = HOLDS.lock().unwrap();
    match &why {
        None => { held.remove(&key); }
        Some(_) => {
            let h = held.entry(key.clone()).or_insert(Hold { since: now, last: now });
            h.last = now;
            drop(held);
            want_reading(&key);
        }
    }
    why
}

/// The words of `hold_reason` for one reading: an exhausted window, else the missing or stale reading.
pub fn hold_text(harness: &str, key: &str, q: &QuotaStatus, now: i64) -> Option<String> {
    if available(q, now) { return None; }
    let login = login_label(key, q);
    if let Some(b) = exhausted(q).into_iter().find(|b| b.resets_at.is_none_or(|r| r > now / 1000)) {
        let reset = b.resets_at.map(|r| format!(", resets in {} min", (r - now / 1000).max(60) / 60)).unwrap_or_default();
        return Some(format!("the {} {harness} quota of {login} is used up{reset}", b.window.to_lowercase()));
    }
    Some(format!("no confirmed {harness} quota reading for {login}: {}", reading_detail(q, now)))
}

/// What is known of a login's reading: its age, then its last error.
pub fn reading_detail(q: &QuotaStatus, now: i64) -> String {
    let last = match q.quota_at {
        Some(at) => format!("last reading {} min ago", (now - at).max(0) / 60_000),
        None => "no reading since Home started".into(),
    };
    match &q.quota_error {
        Some(e) => format!("{last}; {e}"),
        None => last,
    }
}

/// A login's reading key with the profiles that use it, for messages.
pub fn login_label(key: &str, q: &QuotaStatus) -> String {
    if q.profiles.is_empty() { key.to_string() } else { format!("{key} (profile {})", q.profiles.join(", ")) }
}

/// Automatic work waits for this login's quota since `since` (Unix ms); `last` is the last check that waited.
#[derive(Clone, Copy, Debug)]
pub struct Hold { pub since: i64, pub last: i64 }

static HOLDS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Hold>>> = std::sync::LazyLock::new(Default::default);

/// The logins that hold automatic work now (checked in the last two minutes), with since when. In memory: a
/// restart starts the count again.
pub fn holds(now: i64) -> Vec<(String, Hold)> {
    HOLDS.lock().unwrap().iter().filter(|(_, h)| now - h.last <= 120_000).map(|(k, h)| (k.clone(), *h)).collect()
}

static DEMAND: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// Automatic work waits for a reading of this login: its monitor checks now instead of at its next poll
/// (`status::wait_poll`), within its own rate limits.
pub fn want_reading(key: &str) {
    DEMAND.lock().unwrap().insert(key.to_string());
}

/// Takes the pending request for a reading of this login, if any.
pub fn take_demand(key: &str) -> bool {
    DEMAND.lock().unwrap().remove(key)
}

/// A protected session of this login is running or paused: poll it more often.
pub fn monitoring(app: &App, account: &str) -> bool {
    app.sessions.list().iter().any(|s| {
        (s.status.is_running() || s.status == SessionStatus::QuotaPaused)
            && protected(app, s.harness, &s.launch_spec)
            && { let key = crate::profiles::account(app, s.harness, &s.launch_spec); resolve_vault_key(&key).unwrap_or(key) == account }
    })
}

/// The reading key of a vault login: `vault:<id>`, or `vault:default:<harness>` for the harness's default login (`vault::default_login`).
pub fn vault_key(harness: HarnessKind, login: Option<&str>) -> String {
    match login {
        Some(id) => format!("vault:{id}"),
        None => format!("vault:default:{}", harness.as_str()),
    }
}

/// Home: one monitor per vault login, with Home's own copy of the login.
async fn vault_wanted(app: &App) -> BTreeMap<String, (QuotaAccount, Vec<String>)> {
    let mut out = BTreeMap::new();
    if app.paths.role == crate::role::Role::Runner {
        return out;
    }
    for harness in crate::harness::registrations().iter().filter(|r| r.descriptor.vault_login.is_some()).map(|r| r.descriptor.id) {
        let Ok(logins) = crate::vault::logins(app, harness).await else { continue };
        let default = crate::harness::registration(harness).descriptor.vault_login.and_then(|l| crate::vault::default_login(&logins, l.kind)).map(|e| vault_key(harness, Some(&e.id)));
        {
            let mut defaults = DEFAULT_LOGINS.lock().unwrap();
            match default {
                Some(key) => defaults.insert(harness.as_str().to_string(), key),
                None => defaults.remove(harness.as_str()),
            };
        }
        for e in logins {
            match crate::vault::login_home(app, &e.id).await {
                Ok((_, env)) => {
                    let key = vault_key(harness, Some(&e.id));
                    out.insert(key.clone(), (QuotaAccount { key, harness, env }, vec![e.name.clone()]));
                }
                Err(err) => tracing::warn!("quota of {}: {err:#}", e.id),
            }
        }
    }
    out
}

/// Runner: the readings Home sent replace the vault readings here.
pub fn apply_home_readings(app: &App, quotas: BTreeMap<String, QuotaStatus>) {
    app.status.update(|s| {
        s.quotas.retain(|k, _| !k.starts_with("vault:"));
        s.quotas.extend(quotas);
    });
}

/// Home: the reading key of each harness's default login (`vault::default_login`, the lease's choice), by
/// harness id, refreshed with the monitors (`vault_wanted`).
static DEFAULT_LOGINS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> = std::sync::LazyLock::new(Default::default);

/// The monitored key of a reading, with `vault:default:<harness>` resolved to the harness's default login on
/// Home. A runner knows no defaults (None): it keeps `vault:default:<harness>`, Home's copy of that reading.
fn resolve_vault_key(key: &str) -> Option<String> {
    let Some(harness) = key.strip_prefix("vault:default:") else { return Some(key.to_string()) };
    DEFAULT_LOGINS.lock().unwrap().get(harness).cloned()
}

/// A reading reported by a running session (Claude's `rate_limit_event`). Vault logins are monitored on Home
/// for every runner, so a runner also sends complete readings to Home: they keep the quota known while the
/// usage endpoint refuses Home's polls (429).
pub fn session_reading(app: &App, account: &str, five: Option<crate::status::QuotaWindow>, week: Option<crate::status::QuotaWindow>) {
    if !account.starts_with("vault:") {
        app.status.update_quota(account, five, week);
        return;
    }
    if app.paths.role != crate::role::Role::Runner {
        if let Some(key) = resolve_vault_key(account) { app.status.update_quota(&key, five, week); }
        return;
    }
    app.status.update_quota(account, five.clone(), week.clone());
    if five.is_none() || week.is_none() { return; }
    // Sessions report after most model requests: send at most one reading per login every 15 seconds.
    static SENT: std::sync::LazyLock<std::sync::Mutex<HashMap<String, std::time::Instant>>> = std::sync::LazyLock::new(Default::default);
    {
        let mut sent = SENT.lock().unwrap();
        if sent.get(account).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(15)) { return; }
        sent.insert(account.to_string(), std::time::Instant::now());
    }
    let body = serde_json::json!({"key": account, "five_hour": five, "weekly": week});
    tokio::spawn(async move {
        if let Err(e) = crate::home_link::home_call("quota", body).await {
            tracing::debug!("send a session quota reading to Home: {e:#}");
        }
    });
}

/// Home: a session's complete reading sent by a runner (`session_reading`).
pub fn runner_reading(app: &App, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;
    let key = body["key"].as_str().filter(|k| k.starts_with("vault:")).context("key")?;
    let five: crate::status::QuotaWindow = serde_json::from_value(body["five_hour"].clone()).context("five_hour")?;
    let week: crate::status::QuotaWindow = serde_json::from_value(body["weekly"].clone()).context("weekly")?;
    if let Some(key) = resolve_vault_key(key) {
        app.status.update_quota(&key, Some(five), Some(week));
    }
    Ok(serde_json::Value::Null)
}

/// Home: vault readings, with the reading of each harness's default login (`resolve_vault_key`) as its
/// `vault:default:<harness>`.
fn vault_readings(app: &App) -> BTreeMap<String, QuotaStatus> {
    let all = app.status.get().quotas;
    let mut out: BTreeMap<String, QuotaStatus> = all.iter().filter(|(k, _)| k.starts_with("vault:") && !k.starts_with("vault:default:")).map(|(k, v)| (k.clone(), v.clone())).collect();
    for harness in crate::harness::registrations().iter().filter(|r| r.descriptor.vault_login.is_some()).map(|r| r.descriptor.id) {
        let key = vault_key(harness, None);
        if let Some(v) = resolve_vault_key(&key).and_then(|k| out.get(&k).cloned()) {
            out.insert(key, v);
        }
    }
    out
}

type Env = Vec<(String, String)>;

/// Starts a quota monitor for each wanted login and stops those no longer wanted.
fn spawn_monitors(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        // Each monitor with the login variables it started with: a replaced login (a new token) starts it again.
        let mut tasks: HashMap<String, (tokio::task::JoinHandle<()>, Env)> = HashMap::new();
        let mut sent = String::new();
        let mut tick = 0u32;
        loop {
            tick += 1;
            // Logins change rarely: read the vault every minute.
            if tick % 12 != 1 && app.paths.role != crate::role::Role::Runner {
                let readings = vault_readings(&app);
                let text = serde_json::to_string(&readings).unwrap_or_default();
                if text != sent || tick.is_multiple_of(12) {
                    sent = text;
                    app.status.update(|s| {
                        s.quotas.retain(|k, _| !k.starts_with("vault:default:"));
                        s.quotas.extend(readings.iter().filter(|(k, _)| k.starts_with("vault:default:")).map(|(k, v)| (k.clone(), v.clone())));
                    });
                    app.runners.broadcast(crate::runners::proto::ToRunner::Quotas { quotas: readings });
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            if app.paths.role == crate::role::Role::Runner {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            let wanted = vault_wanted(&app).await;
            tasks.retain(|key, (task, env)| {
                let keep = wanted.get(key).is_some_and(|(account, _)| &account.env == env);
                if !keep { task.abort(); }
                keep
            });
            let mut current = app.status.get().quotas;
            // The defaults repeat a monitored login (`vault_readings`): they are not monitors of their own.
            current.retain(|k, _| !k.starts_with("vault:default:"));
            let changed = current.len() != wanted.len()
                || wanted.iter().any(|(k, (_, profiles))| current.get(k).is_none_or(|q| &q.profiles != profiles));
            if changed {
                app.status.update(|s| {
                    s.quotas.retain(|k, _| wanted.contains_key(k) || k.starts_with("vault:default:"));
                    for (k, (account, profiles)) in &wanted {
                        let q = s.quotas.entry(k.clone()).or_default();
                        q.harness = account.harness.as_str().to_string();
                        q.profiles = profiles.clone();
                    }
                });
            }
            for (key, (account, _)) in wanted {
                if tasks.contains_key(&key) { continue; }
                let env = account.env.clone();
                if let Some(task) = app.harnesses.get(account.harness).spawn_quota(&app, account) {
                    tasks.insert(key, (task, env));
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

pub fn spawn(app: &App) {
    spawn_monitors(app);
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = app.sessions.check_capacity_retries(&app).await {
                tracing::warn!("capacity retry guard: {e:#}");
            }
            if let Err(e) = app.sessions.check_quotas(&app).await {
                tracing::warn!("quota guard: {e:#}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::QuotaWindow;
    #[test]
    fn unknown_usage_retries_only_after_the_known_five_hour_reset() {
        let pause = crate::model::QuotaPause {
            phase: crate::model::QuotaPausePhase::Paused,
            blockers: vec![QuotaBlock { window: "5-hour".into(), resets_at: Some(1000) }],
            message: "continue".into(), images: vec![], client_id: None, error: None, messages: vec![],
        };
        let mut s = QuotaStatus {
            quota_at: Some(900_000), quota_error: Some("unknown".into()),
            five_hour: Some(QuotaWindow { used_percent: 100.0, resets_at: Some(1000) }),
            weekly: Some(QuotaWindow { used_percent: 10.0, resets_at: Some(5000) }),
            ..Default::default()
        };
        assert!(!reset_fallback(&s, &pause, 1_059_999));
        assert!(reset_fallback(&s, &pause, 1_060_000));
        // A stale snapshot is unknown even without an explicit fetch error.
        s.quota_error = None;
        assert!(reset_fallback(&s, &pause, 1_060_000));
        // Fresh exhaustion must not cause an automatic retry loop.
        s.quota_at = Some(1_060_000);
        assert!(!reset_fallback(&s, &pause, 1_060_000));
        s.quota_error = Some("unknown again".into());
        assert!(!reset_fallback(&s, &pause, 1_160_000));
        s.quota_at = Some(900_000);
        s.weekly.as_mut().unwrap().used_percent = 100.0;
        assert!(!reset_fallback(&s, &pause, 1_060_000));
        // A newer reset replaces the old deadline.
        s.weekly.as_mut().unwrap().used_percent = 10.0;
        s.five_hour.as_mut().unwrap().resets_at = Some(2000);
        assert!(!reset_fallback(&s, &pause, 1_060_000));
        // After daemon restart, use the durable pause's reset timestamp.
        let s = QuotaStatus::default();
        assert!(reset_fallback(&s, &pause, 1_060_000));
        let mut no_reset = pause.clone();
        no_reset.blockers[0].resets_at = None;
        assert!(!reset_fallback(&s, &no_reset, 1_060_000));
        let mut weekly = pause;
        weekly.blockers.push(QuotaBlock { window: "Weekly".into(), resets_at: Some(5000) });
        assert!(!reset_fallback(&s, &weekly, 1_060_000));
    }

    #[test]
    fn incomplete_readings_cannot_clear_exhaustion() {
        let mut s = QuotaStatus::default();
        update_reading(&mut s, Some(QuotaWindow { used_percent: 100.0, resets_at: None }), Some(QuotaWindow { used_percent: 0.0, resets_at: None }));
        let at = s.quota_at;
        update_reading(&mut s, None, None);
        assert_eq!(exhausted(&s).len(), 1);
        assert_eq!(s.quota_at, at);
        assert!(s.quota_error.is_some());
        update_reading(&mut s, Some(QuotaWindow { used_percent: 0.0, resets_at: None }), None);
        assert_eq!(exhausted(&s).len(), 1);
        assert!(!available(&s, at.unwrap()));
        update_reading(&mut s, Some(QuotaWindow { used_percent: 0.0, resets_at: None }), Some(QuotaWindow { used_percent: 0.0, resets_at: None }));
        assert!(exhausted(&s).is_empty());
        assert!(available(&s, s.quota_at.unwrap()));
        update_reading(&mut s, None, Some(QuotaWindow { used_percent: 100.0, resets_at: None }));
        assert_eq!(exhausted(&s)[0].window, "Weekly");
        assert!(!available(&s, s.quota_at.unwrap()));
    }

    #[test]
    fn hold_reasons_name_the_login_and_what_is_missing() {
        let now = 10_000_000;
        let window = |used: f64| Some(QuotaWindow { used_percent: used, resets_at: None });
        let mut q = QuotaStatus { profiles: vec!["claude".into()], ..Default::default() };
        assert_eq!(hold_text("Claude", "vault:a", &q, now).unwrap(), "no confirmed Claude quota reading for vault:a (profile claude): no reading since Home started");
        q.quota_at = Some(now - 30 * 60_000);
        q.five_hour = window(10.0);
        q.weekly = window(10.0);
        q.quota_error = Some("usage endpoint returned 403 Forbidden".into());
        assert_eq!(hold_text("Claude", "vault:a", &q, now).unwrap(), "no confirmed Claude quota reading for vault:a (profile claude): last reading 30 min ago; usage endpoint returned 403 Forbidden");
        q.quota_error = None;
        q.quota_at = Some(now);
        assert_eq!(hold_text("Claude", "vault:a", &q, now), None);
        q.weekly = Some(QuotaWindow { used_percent: 100.0, resets_at: Some(now / 1000 + 7200) });
        assert_eq!(hold_text("Claude", "vault:a", &q, now).unwrap(), "the weekly Claude quota of vault:a (profile claude) is used up, resets in 120 min");
    }

    #[test]
    fn both_windows_must_be_fresh_and_available() {
        let now = 1_000_000;
        let mut s = QuotaStatus {
            quota_at: Some(now),
            five_hour: Some(QuotaWindow {
                used_percent: 99.9,
                resets_at: Some(1100),
            }),
            weekly: Some(QuotaWindow {
                used_percent: 100.0,
                resets_at: Some(1200),
            }),
            ..Default::default()
        };
        assert_eq!(exhausted(&s).len(), 1);
        assert!(!available(&s, now));
        s.weekly.as_mut().unwrap().used_percent = 0.0;
        assert!(available(&s, now));
        assert!(!available(&s, now + 90_001));
        s.quota_error = Some("unavailable".into());
        assert!(!available(&s, now));
        s.quota_error = None;
        s.weekly = None;
        assert!(!available(&s, now));
        s.weekly = s.five_hour.clone();
        s.five_hour.as_mut().unwrap().resets_at = Some(999);
        assert!(!available(&s, now));
        s.five_hour.as_mut().unwrap().resets_at = None;
        assert!(available(&s, now));
    }
}

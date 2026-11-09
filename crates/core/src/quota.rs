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

/// A monitored subscription login, given to `Harness::spawn_quota`.
#[derive(Clone, Debug)]
pub struct QuotaAccount {
    /// `profiles::account`: the key of its reading in `Status.quotas`.
    pub key: String,
    pub harness: HarnessKind,
    /// Account variables (such as `CLAUDE_CONFIG_DIR`) expanded on this computer; empty for the default login.
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
    !protected(app, harness, spec) || available(&reading_for(app, harness, spec), crate::util::now_ms())
}

/// A protected session of this login is running or paused: poll it more often.
pub fn monitoring(app: &App, account: &str) -> bool {
    app.sessions.list().iter().any(|s| {
        (s.status.is_running() || s.status == SessionStatus::QuotaPaused)
            && protected(app, s.harness, &s.launch_spec)
            && crate::profiles::account(app, s.harness, &s.launch_spec) == account
    })
}

/// The logins to monitor: every subscription profile of a harness with quotas, and the login of each
/// protected session that is running or paused (sessions launched without a profile use the default one).
fn wanted(app: &App) -> BTreeMap<String, (QuotaAccount, Vec<String>)> {
    let mut out: BTreeMap<String, (QuotaAccount, Vec<String>)> = BTreeMap::new();
    let config = app.config.get();
    let account_env = |harness: HarnessKind, env: &std::collections::BTreeMap<String, String>| -> Vec<(String, String)> {
        crate::harness::registration(harness).descriptor.account_env.iter()
            .filter_map(|name| env.get(*name).filter(|v| !v.trim().is_empty()).map(|v| (name.to_string(), crate::paths::expand_home(v).to_string_lossy().into_owned())))
            .collect()
    };
    for p in config.profiles.iter().filter(|p| p.billing == crate::config::Billing::Subscription && crate::harness::registration(p.harness).descriptor.capabilities.quota) {
        let key = crate::profiles::account_of(p);
        out.entry(key.clone())
            .or_insert_with(|| (QuotaAccount { key, harness: p.harness, env: account_env(p.harness, &p.env) }, vec![]))
            .1
            .push(p.id.clone());
    }
    for s in app.sessions.list() {
        if !(s.status.is_running() || s.status == SessionStatus::QuotaPaused) || !protected(app, s.harness, &s.launch_spec) { continue; }
        let key = crate::profiles::account(app, s.harness, &s.launch_spec);
        if out.contains_key(&key) { continue; }
        let env = s.launch_spec.profile.as_deref().and_then(|id| config.profile(id)).map(|p| account_env(p.harness, &p.env)).unwrap_or_default();
        out.insert(key.clone(), (QuotaAccount { key, harness: s.harness, env }, vec![]));
    }
    out
}

/// Logins live in Home's vault (Linux runners and Home): Home monitors each login once for every runner, and
/// runners receive its readings.
pub fn vault_quotas() -> bool {
    cfg!(target_os = "linux")
}

/// The reading key of a vault login: `vault:<id>`, or `vault:default:<harness>` for the harness's first login.
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

/// Home: the monitored key of a reading, with `vault:default:<harness>` resolved to the harness's first login
/// (the same choice as `vault_readings`).
fn resolve_vault_key(app: &App, key: &str) -> Option<String> {
    let Some(harness) = key.strip_prefix("vault:default:") else { return Some(key.to_string()) };
    app.status.get().quotas.into_iter().find(|(k, v)| k.starts_with("vault:") && !k.starts_with("vault:default:") && v.harness == harness).map(|(k, _)| k)
}

/// A reading reported by a running session (Claude's `rate_limit_event`). Vault logins are monitored on Home
/// for every runner, so a runner also sends complete readings to Home: they keep the quota known while the
/// usage endpoint refuses Home's polls (429).
pub fn session_reading(app: &App, account: &str, five: Option<crate::status::QuotaWindow>, week: Option<crate::status::QuotaWindow>) {
    if !vault_quotas() || !account.starts_with("vault:") {
        app.status.update_quota(account, five, week);
        return;
    }
    if app.paths.role != crate::role::Role::Runner {
        if let Some(key) = resolve_vault_key(app, account) { app.status.update_quota(&key, five, week); }
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
    if let Some(key) = resolve_vault_key(app, key) {
        app.status.update_quota(&key, Some(five), Some(week));
    }
    Ok(serde_json::Value::Null)
}

/// Home: vault readings, with the first login of each harness as its default.
fn vault_readings(app: &App) -> BTreeMap<String, QuotaStatus> {
    let all = app.status.get().quotas;
    let mut out: BTreeMap<String, QuotaStatus> = all.iter().filter(|(k, _)| k.starts_with("vault:")).map(|(k, v)| (k.clone(), v.clone())).collect();
    for harness in crate::harness::registrations().iter().filter(|r| r.descriptor.vault_login.is_some()).map(|r| r.descriptor.id) {
        let first = out.iter().filter(|(k, v)| !k.starts_with("vault:default:") && v.harness == harness.as_str()).map(|(k, v): (&String, &QuotaStatus)| (k.clone(), v.clone())).next();
        if let Some((_, v)) = first {
            out.insert(vault_key(harness, None), v);
        }
    }
    out
}

/// Starts a quota monitor for each wanted login and stops those no longer wanted.
fn spawn_monitors(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
        let mut sent = String::new();
        let mut tick = 0u32;
        loop {
            tick += 1;
            let wanted = if vault_quotas() {
                // Logins change rarely: read the vault every minute.
                if tick % 12 != 1 && app.paths.role != crate::role::Role::Runner {
                    let readings = vault_readings(&app);
                    let text = serde_json::to_string(&readings).unwrap_or_default();
                    if text != sent || tick.is_multiple_of(12) {
                        sent = text;
                        app.status.update(|s| s.quotas.extend(readings.iter().filter(|(k, _)| k.starts_with("vault:default:")).map(|(k, v)| (k.clone(), v.clone()))));
                        app.runners.broadcast(crate::runners::proto::ToRunner::Quotas { quotas: readings });
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                if app.paths.role == crate::role::Role::Runner {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                vault_wanted(&app).await
            } else {
                wanted(&app)
            };
            tasks.retain(|key, task| {
                let keep = wanted.contains_key(key);
                if !keep { task.abort(); }
                keep
            });
            let current = app.status.get().quotas;
            let changed = current.len() != wanted.len()
                || wanted.iter().any(|(k, (_, profiles))| current.get(k).is_none_or(|q| &q.profiles != profiles));
            if changed {
                app.status.update(|s| {
                    s.quotas.retain(|k, _| wanted.contains_key(k));
                    for (k, (account, profiles)) in &wanted {
                        let q = s.quotas.entry(k.clone()).or_default();
                        q.harness = account.harness.as_str().to_string();
                        q.profiles = profiles.clone();
                    }
                });
            }
            for (key, (account, _)) in wanted {
                if tasks.contains_key(&key) { continue; }
                if let Some(task) = app.harnesses.get(account.harness).spawn_quota(&app, account) {
                    tasks.insert(key, task);
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

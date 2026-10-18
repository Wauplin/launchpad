//! Best-effort per-session protection against additional subscription spending.
use crate::{
    App,
    model::{HarnessKind, LaunchSpec, QuotaBlock, SessionStatus},
    status::HarnessStatus,
};

pub fn reading(app: &App, harness: HarnessKind) -> HarnessStatus {
    app.status
        .get()
        .harnesses
        .get(harness.as_str())
        .cloned()
        .unwrap_or_default()
}

/// An incomplete provider snapshot cannot erase an exhausted window or authorize automatic work.
pub fn update_reading(s: &mut HarnessStatus, five: Option<crate::status::QuotaWindow>, week: Option<crate::status::QuotaWindow>) {
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
pub fn exhausted(s: &HarnessStatus) -> Vec<QuotaBlock> {
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

pub fn blockers(app: &App, harness: HarnessKind) -> Vec<QuotaBlock> {
    exhausted(&reading(app, harness))
}

pub fn available(s: &HarnessStatus, now: i64) -> bool {
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

/// Subscription protection applies: a monitored harness, paid usage not allowed, and not a pay-as-you-go variant.
pub fn protected(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> bool {
    crate::harness::registration(harness).descriptor.capabilities.quota
        && !spec.allow_paid_usage
        && !crate::variants::pay_as_you_go(app, spec)
}

/// Automatic work requires confirmed capacity; manual starts may use best-effort protection.
pub fn automatic_ready(app: &App, harness: HarnessKind, spec: &LaunchSpec) -> bool {
    !protected(app, harness, spec) || available(&reading(app, harness), crate::util::now_ms())
}

pub fn monitoring(app: &App, h: HarnessKind) -> bool {
    app.sessions.list().iter().any(|s| {
        s.harness == h
            && protected(app, s.harness, &s.launch_spec)
            && (s.status.is_running() || s.status == SessionStatus::QuotaPaused)
    })
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
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
    fn incomplete_readings_cannot_clear_exhaustion() {
        let mut s = HarnessStatus::default();
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
        let mut s = HarnessStatus {
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

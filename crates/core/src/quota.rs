//! Best-effort per-session protection against additional subscription spending.
use crate::{
    App,
    model::{HarnessKind, QuotaBlock, SessionStatus},
    status::HarnessStatus,
};

pub fn reading(app: &App, harness: HarnessKind) -> HarnessStatus {
    let s = app.status.get();
    match harness {
        HarnessKind::Claude => s.claude,
        HarnessKind::Codex => s.codex,
    }
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
                    && w.resets_at > now / 1000
            })
        })
}

pub fn monitoring(app: &App, h: HarnessKind) -> bool {
    app.sessions.list().iter().any(|s| {
        s.harness == h
            && !s.launch_spec.allow_paid_usage
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
    fn both_windows_must_be_fresh_and_available() {
        let now = 1_000_000;
        let mut s = HarnessStatus {
            quota_at: Some(now),
            five_hour: Some(QuotaWindow {
                used_percent: 99.9,
                resets_at: 1100,
            }),
            weekly: Some(QuotaWindow {
                used_percent: 100.0,
                resets_at: 1200,
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
        s.five_hour.as_mut().unwrap().resets_at = 999;
        assert!(!available(&s, now));
    }
}

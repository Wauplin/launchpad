//! Notifications, sent by the daemon so they work when every window is closed: Web Push to paired
//! browsers (`push.rs`) and a desktop notification on the host. Clicking a desktop notification
//! opens the window on the item (`launchpad-desktop --open <route>`).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::App;
use crate::model::{Session, SessionStatus};

/// Threads waiting for a click. GNOME keeps unanswered notifications in its tray, so the waits are
/// capped; beyond the cap, notifications are shown without the click action.
static WAITING: AtomicUsize = AtomicUsize::new(0);
const MAX_WAITING: usize = 32;

#[derive(Clone, Copy)]
pub(crate) enum SessionNotification {
    Done,
    Failed,
    Interrupted,
    QuotaPaused,
    CostLimit,
    QuotaResumed,
}

impl SessionNotification {
    pub(crate) fn summary(self, project: &str) -> String {
        let status = match self {
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
            Self::QuotaPaused => "Quota paused",
            Self::CostLimit => "Cost limit reached",
            Self::QuotaResumed => "Quota available; agent resumed",
        };
        format!("{status} in {project}")
    }
}

/// Only completed work and automatic stops qualify; setup, questions and owner actions are silent.
pub(crate) fn status_notification(before: SessionStatus, after: SessionStatus, question: bool, manual: bool, budget: bool) -> Option<SessionNotification> {
    if !before.is_running() || manual { return None; }
    match after {
        SessionStatus::Waiting if before != SessionStatus::Starting && !question => Some(SessionNotification::Done),
        SessionStatus::Error => Some(SessionNotification::Failed),
        SessionStatus::Interrupted => Some(SessionNotification::Interrupted),
        SessionStatus::QuotaPaused if before != SessionStatus::Starting => Some(SessionNotification::QuotaPaused),
        SessionStatus::Stopped if budget => Some(SessionNotification::CostLimit),
        _ => None,
    }
}

/// The caller selects an eligible event and checks active-window suppression.
pub(crate) fn session(app: &App, s: &Session, event: SessionNotification) {
    if !app.config.get().agents.notify {
        return;
    }
    app.push.session(app, s, event);
    let project = std::path::Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let summary = event.summary(&project);
    let body = s.title.clone();
    let route = format!("/agents?session={}", s.id);
    let desktop = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(format!("launchpad-desktop{}", std::env::consts::EXE_SUFFIX)))).filter(|p| p.exists());
    let env = app.env.clone();
    let app_env = app.paths.app_env;
    std::thread::spawn(move || {
        let handle = notify_rust::Notification::new()
            .appname("Launchpad")
            .summary(&summary)
            .body(&body)
            .icon("launchpad")
            .action("default", "Open")
            .show();
        match handle {
            Ok(_) if WAITING.load(Ordering::Relaxed) >= MAX_WAITING => {}
            Ok(h) => {
                WAITING.fetch_add(1, Ordering::Relaxed);
                h.wait_for_action(|action| {
                if action == "default" {
                    let exe = desktop.clone().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "launchpad-desktop".into());
                    let mut cmd = std::process::Command::new(exe);
                    cmd.args(["--open", &route]).env_clear().envs(env.vars().iter());
                    if app_env == crate::paths::AppEnv::Dev {
                        cmd.env("LAUNCHPAD_ENV", "dev");
                    }
                    let _ = cmd.spawn();
                }
                });
                WAITING.fetch_sub(1, Ordering::Relaxed);
            }
            // A headless host has no notification server: expected, not worth a warning.
            Err(e) if cfg!(target_os = "linux") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() => tracing::debug!("desktop notification: {e}"),
            Err(e) => tracing::warn!("desktop notification: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus::*;

    #[test]
    fn notifications_only_cover_completion_and_automatic_stops() {
        let cases = [
            (Starting, Waiting, false, false, false, false),
            (Starting, Working, false, false, false, false),
            (Starting, QuotaPaused, false, false, false, false),
            (Working, Background, false, false, false, false),
            (Working, Waiting, true, false, false, false),
            (Working, Waiting, false, false, false, true),
            (Background, Waiting, false, false, false, true),
            (Starting, Error, false, false, false, true),
            (Working, Error, false, false, false, true),
            (Working, Interrupted, false, false, false, true),
            (Working, QuotaPaused, false, false, false, true),
            (Working, Stopped, false, false, true, true),
            (Working, Stopped, false, true, false, false),
            (Working, Waiting, false, true, false, false),
            (Working, Error, false, true, false, false),
            (Working, Interrupted, false, true, false, false),
            (Working, Closed, false, false, false, false),
            (Working, InTerminal, false, false, false, false),
            (QuotaPaused, Working, false, false, false, false),
            (QuotaPaused, Waiting, false, false, false, false),
            (Waiting, Waiting, false, false, false, false),
        ];
        for (before, after, question, manual, budget, expected) in cases {
            assert_eq!(status_notification(before, after, question, manual, budget).is_some(), expected,
                "{before:?} -> {after:?}, question={question}, manual={manual}, budget={budget}");
        }
    }
}

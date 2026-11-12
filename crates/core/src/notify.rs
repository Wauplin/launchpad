//! Session notifications, sent by the daemon so they work when every window is closed: the Telegram chat of the
//! session's workspace, when it has one (`telegram.rs` in the channels crate, through
//! `ext::Extensions::notify_connections`). The daemon shows no desktop notifications: it may run anywhere, and the
//! owner's devices use Telegram.

use crate::App;
use crate::model::{Session, SessionStatus};

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
    /// The event's name for the Telegram notification (forwarded to Home by runners).
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::QuotaPaused => "quota_paused",
            Self::CostLimit => "cost_limit",
            Self::QuotaResumed => "quota_resumed",
        }
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
    // Each workspace's Telegram chat has its own switch (Settings → Workspace).
    crate::ext::extensions().notify_connections(app, s, event.key());
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

//! The connection state machine of the linked Home (folio page States, "Connection states"), its retry delays and
//! how a failed answer is read. Pure: the engine feeds it results and the clock.

use std::time::Duration;

/// The companion's link to Home.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conn {
    /// First run, or after Sign out.
    NotLinked,
    /// Opening the stream; `attempt` failures so far (Offline after 3).
    Connecting { attempt: u32 },
    /// The live stream is open.
    Live,
    /// 3 failed attempts in a row; `since` (Unix ms) when it went offline, `last_live` when it last had data.
    Offline { since: i64, last_live: Option<i64>, attempt: u32 },
    /// The Space sleeps or builds: it answers its own page or a 503 without Home's JSON.
    Starting { attempt: u32 },
    /// Home answers 503 with `retryable` (an update in progress).
    Updating { attempt: u32 },
    /// Hugging Face refused the token for the private Space (Sign in again).
    AuthExpired,
    /// Home refused this device: it was revoked in Settings → Devices.
    Revoked,
    /// Home's API is outside the companion's range: which side to update.
    Incompatible(String),
}

/// Why a request to Home failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Network,
    Revoked,
    AuthExpired,
    Starting,
    Updating,
    /// Any other answer (a bug, a 500): retried like a network failure.
    Other,
}

/// Reads a failed answer. `space`: Home sits behind a Hugging Face Space gateway. Home's own refusal of a device says
/// "pair this browser" (the daemon) or "Connect this computer first" (the gateway, no device header): only those
/// mean Revoked (which deletes the secrets), like the extension's `REVOKED`; another 401 (a proxy in front of a
/// self-hosted Home) is retried.
pub fn classify(status: u16, body: &str, space: bool) -> Failure {
    let json = body.trim_start().starts_with('{');
    let lower = body.to_ascii_lowercase();
    match status {
        401 if json && (lower.contains("pair this browser") || lower.contains("connect this computer") || lower.contains("connect this browser")) => Failure::Revoked,
        401 | 403 if space && !json => Failure::AuthExpired,
        503 if json && lower.contains("retryable") => Failure::Updating,
        502..=504 if space && !json => Failure::Starting,
        502..=504 => Failure::Network,
        _ if space && !json && status != 404 => Failure::Starting,
        _ => Failure::Other,
    }
}

/// Failures in a row before the companion says Offline.
pub const OFFLINE_AFTER: u32 = 3;

impl Conn {
    pub fn attempt(&self) -> u32 {
        match self {
            Conn::Connecting { attempt } | Conn::Offline { attempt, .. } | Conn::Starting { attempt } | Conn::Updating { attempt } => *attempt,
            _ => 0,
        }
    }

    /// The state after a failed attempt at `now` (Unix ms); `last_live` is when data last arrived.
    pub fn failed(&self, failure: &Failure, now: i64, last_live: Option<i64>) -> Conn {
        let attempt = self.attempt() + 1;
        match failure {
            Failure::Revoked => Conn::Revoked,
            Failure::AuthExpired => Conn::AuthExpired,
            Failure::Starting => Conn::Starting { attempt },
            Failure::Updating => Conn::Updating { attempt },
            Failure::Network | Failure::Other => match self {
                Conn::Offline { since, .. } => Conn::Offline { since: *since, last_live, attempt },
                _ if attempt >= OFFLINE_AFTER => Conn::Offline { since: now, last_live, attempt },
                _ => Conn::Connecting { attempt },
            },
        }
    }

    /// The engine retries in these states (the others wait for the owner).
    pub fn retries(&self) -> bool {
        matches!(self, Conn::Connecting { .. } | Conn::Offline { .. } | Conn::Starting { .. } | Conn::Updating { .. })
    }

    /// One line for the top of the menu.
    pub fn header(&self, clock: impl Fn(i64) -> String) -> String {
        match self {
            Conn::NotLinked => "Not connected to a Launchpad Home".into(),
            Conn::Connecting { .. } => "Connecting…".into(),
            Conn::Live => "Live".into(),
            Conn::Offline { last_live, .. } => match last_live {
                Some(t) => format!("Offline · last data {}", clock(*t)),
                None => "Offline".into(),
            },
            Conn::Starting { .. } => "Home is starting (Space building or waking)".into(),
            Conn::Updating { .. } => "Home is updating · writes paused".into(),
            Conn::AuthExpired => "Sign in again".into(),
            Conn::Revoked => "This device was removed from Home".into(),
            Conn::Incompatible(which) => which.clone(),
        }
    }
}

/// The wait before the next attempt: 1 s after a stream that dropped (Home restarts), then 5 s, 15 s, 30 s and every
/// 60 s, so the companion never asks more often than every 5 s while Home does not answer.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 1,
        2 => 5,
        3 => 15,
        4 => 30,
        _ => 60,
    })
}

/// The Home API levels this companion supports (`GET /api/version`): Home's `api` at least `MIN_HOME_API`, and this
/// companion's level at least Home's `min_client`.
pub const CLIENT_API: u32 = 1;
pub const MIN_HOME_API: u32 = 1;
/// Capabilities the companion needs from Home.
pub const NEEDED: &[&str] = &["desktop_device", "session_group"];

/// What to update when the levels do not match; None when they do. `None` for `api` means Home has no
/// `/api/version` (older than the companion).
pub fn compatibility(api: Option<u32>, min_client: u32, capabilities: &[String]) -> Option<String> {
    let Some(api) = api else { return Some("Update Home: it is older than this companion".into()) };
    if api < MIN_HOME_API || NEEDED.iter().any(|c| !capabilities.iter().any(|x| x == c)) {
        return Some("Update Home: it is older than this companion".into());
    }
    if CLIENT_API < min_client {
        return Some("Update the companion: Home needs a newer one".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_like_the_extension() {
        assert_eq!(classify(401, r#"{"error":"not authenticated; pair this browser again"}"#, false), Failure::Revoked);
        assert_eq!(classify(401, r#"{"error":"Connect this computer first"}"#, true), Failure::Revoked);
        assert_eq!(classify(401, "<html>Sign in</html>", true), Failure::AuthExpired);
        assert_eq!(classify(403, "Forbidden", true), Failure::AuthExpired);
        assert_eq!(classify(503, r#"{"error":"Home is updating","retryable":true}"#, true), Failure::Updating);
        assert_eq!(classify(503, "<html>Building</html>", true), Failure::Starting);
        assert_eq!(classify(503, "", false), Failure::Network);
        assert_eq!(classify(500, r#"{"error":"boom"}"#, false), Failure::Other);
        // Any other 401 never deletes the device: a proxy's page, or a page that quotes Home's words.
        assert_eq!(classify(401, "Unauthorized", false), Failure::Other);
        assert_eq!(classify(401, "<p>pair this browser</p>", false), Failure::Other);
    }

    #[test]
    fn three_failures_make_offline_and_live_resets() {
        let mut c = Conn::Connecting { attempt: 0 };
        for _ in 0..2 {
            c = c.failed(&Failure::Network, 10, Some(5));
            assert!(matches!(c, Conn::Connecting { .. }));
        }
        c = c.failed(&Failure::Network, 10, Some(5));
        assert_eq!(c, Conn::Offline { since: 10, last_live: Some(5), attempt: 3 });
        c = c.failed(&Failure::Other, 99, Some(5));
        assert_eq!(c, Conn::Offline { since: 10, last_live: Some(5), attempt: 4 });
        assert_eq!(c.failed(&Failure::Revoked, 0, None), Conn::Revoked);
        assert!(!Conn::Revoked.retries() && !Conn::AuthExpired.retries() && c.retries());
        // Never faster than every 5 s once offline, and capped at a minute.
        assert!((3..20).all(|a| backoff(a) >= Duration::from_secs(5)));
        assert_eq!(backoff(50), Duration::from_secs(60));
    }

    #[test]
    fn version_range() {
        let caps: Vec<String> = ["desktop_device", "devices_self", "session_group"].map(String::from).to_vec();
        assert_eq!(compatibility(Some(1), 1, &caps), None);
        assert_eq!(compatibility(Some(2), 1, &caps), None);
        assert!(compatibility(None, 1, &caps).unwrap().starts_with("Update Home"));
        assert!(compatibility(Some(1), 1, &caps[1..]).unwrap().starts_with("Update Home"));
        assert!(compatibility(Some(3), 2, &caps).unwrap().starts_with("Update the companion"));
    }
}

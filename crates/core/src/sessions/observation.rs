//! Window presence used by seen and notification rules.
use super::*;

impl Sessions {
    /// Each window reports which sessions it shows, whether it has the focus, and how long ago it last
    /// received user input (`idle_ms`, relative so device clocks do not matter). `None` when it closes.
    pub fn set_watched(&self, window: u64, presence: Option<(bool, Vec<String>, Option<i64>)>) {
        let mut w = self.inner.watched.lock().unwrap();
        let now = now_ms();
        match presence {
            Some((focused, sessions, idle_ms)) => w.insert(window, (focused, sessions, now, idle_ms.map(|idle| now - idle.max(0)))),
            None => w.remove(&window),
        };
    }

    /// Shown in a focused window that received user input in the last minute (the same rule as "seen" in the UI).
    pub(crate) fn is_watched(&self, id: &str) -> bool {
        let w = self.inner.watched.lock().unwrap();
        let now = now_ms();
        w.values().any(|(focused, sessions, at, input)| {
            *focused && now - at < 10 * 60_000 && input.is_some_and(|i| now - i < WATCH_INPUT_MS) && sessions.iter().any(|s| s == id)
        })
    }
}

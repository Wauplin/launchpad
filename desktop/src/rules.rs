//! When the companion notifies (folio page States, "Notification rules", needs-you mode): a session that starts
//! asking or fails, from Today. Never more than one notification per session per minute; the first load after a
//! start only primes; new items found when the stream comes back are summed up in one notification.

use std::collections::{HashMap, HashSet};

use crate::model::{TodayItem, cut};

/// One desktop notification: `key` replaces an earlier one of the same session (GNOME keeps one per id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub key: String,
    pub title: String,
    pub body: String,
    /// The Today item it opens (`/?card=<id>`); None for summaries (Today itself).
    pub item: Option<String>,
}

/// At most one notification per session per minute.
pub const PER_SESSION_MS: i64 = 60_000;
/// A question or a reason is cut at this many characters.
pub const BODY_CHARS: usize = 120;

#[derive(Default)]
pub struct Rules {
    /// Today items seen so far (by id), so only new ones notify.
    known: HashSet<String>,
    /// When each session (or item without a session) last notified.
    last: HashMap<String, i64>,
    primed: bool,
}

/// Kinds that notify in needs-you mode.
fn notifies(kind: &str) -> bool {
    matches!(kind, "question" | "failed" | "channel_ask")
}

impl Rules {
    /// Forgets what was seen (a new workspace or a new link): the next load only primes.
    pub fn reset(&mut self) {
        self.known.clear();
        self.primed = false;
    }

    /// The notifications for a fresh Today of the workspace. `catch_up`: the stream just came back after a gap
    /// (offline, suspend), so several new items make one summary instead of one each.
    pub fn on_today(&mut self, items: &[TodayItem], now: i64, catch_up: bool) -> Vec<Note> {
        let new: Vec<&TodayItem> = items.iter().filter(|i| !self.known.contains(&i.id)).collect();
        self.known = items.iter().map(|i| i.id.clone()).collect();
        if !self.primed {
            self.primed = true;
            return Vec::new();
        }
        let wanted: Vec<&TodayItem> = new.into_iter().filter(|i| notifies(&i.kind)).collect();
        if catch_up && wanted.len() > 1 {
            let asking = wanted.iter().filter(|i| i.kind != "failed").count();
            let failed = wanted.len() - asking;
            let parts: Vec<String> = [(asking, "asking"), (failed, "failed")].into_iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!("{n} {w}")).collect();
            return vec![Note { key: "summary".into(), title: "While you were away".into(), body: parts.join(", "), item: None }];
        }
        let mut notes = Vec::new();
        for item in wanted {
            let key = item.session_id.clone().unwrap_or_else(|| item.id.clone());
            if self.last.get(&key).is_some_and(|t| now - t < PER_SESSION_MS) {
                continue;
            }
            self.last.insert(key.clone(), now);
            let title = cut(&item.title, 80);
            let title = if item.kind == "failed" { format!("{title} failed") } else { format!("{title} asks") };
            notes.push(Note { key: format!("session-{key}"), title, body: cut(&item.detail, BODY_CHARS), item: Some(item.id.clone()) });
        }
        self.last.retain(|_, t| now - *t < PER_SESSION_MS);
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, session: &str) -> TodayItem {
        TodayItem { id: id.into(), kind: kind.into(), title: format!("Task {session}"), detail: "Q".repeat(300), session_id: Some(session.into()), ..Default::default() }
    }

    #[test]
    fn primes_then_notifies_new_asks_and_failures_once_a_minute() {
        let mut r = Rules::default();
        assert!(r.on_today(&[item("session:a", "question", "a")], 0, false).is_empty());
        let notes = r.on_today(&[item("session:a", "question", "a"), item("session:b", "failed", "b"), item("proposal:p", "proposal", "c")], 1_000, false);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Task b failed");
        assert_eq!(notes[0].body.chars().count(), BODY_CHARS);
        // The same session again within a minute: merged (nothing new); after a minute it may notify again.
        assert!(r.on_today(&[item("session:b2", "question", "b")], 30_000, false).is_empty());
        assert_eq!(r.on_today(&[item("session:b3", "question", "b")], 70_000, false).len(), 1);
    }

    #[test]
    fn catch_up_sums_up() {
        let mut r = Rules::default();
        r.on_today(&[], 0, false);
        let notes = r.on_today(&[item("1", "question", "a"), item("2", "question", "b"), item("3", "failed", "c")], 5, true);
        assert_eq!(notes, vec![Note { key: "summary".into(), title: "While you were away".into(), body: "2 asking, 1 failed".into(), item: None }]);
    }
}

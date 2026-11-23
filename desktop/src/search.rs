//! Go to's local index and its matching: every Launchpad object the companion knows (sessions, chats, bots,
//! channels, folios, Local PRs, folders, Today cards, fixed pages), matched on each keystroke. Ranking (folio page
//! Spotlight, "Go to"): every word a prefix of a word > every word contained > the letters in order; then what
//! needs you, then the most recently updated.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Page,
    Today,
    Session,
    Chat,
    Bot,
    Channel,
    Folio,
    LocalPr,
    Folder,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Today => "Today",
            Kind::Session => "agent",
            Kind::Chat => "chat",
            Kind::Bot => "bot",
            Kind::Channel => "channel",
            Kind::Folio => "folio",
            Kind::LocalPr => "Local PR",
            Kind::Folder => "folder",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub title: String,
    pub url: String,
    pub needs: bool,
    pub updated: i64,
    /// Lowercased title and kind word.
    key: String,
    /// Where each word of `key` starts.
    starts: Vec<usize>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.title == other.title && self.needs == other.needs && self.updated == other.updated
    }
}
impl Eq for Entry {}

impl Entry {
    pub fn new(kind: Kind, title: &str, url: String, needs: bool, updated: i64) -> Entry {
        let key = format!("{} {}", title.to_lowercase(), kind.word().to_lowercase());
        let mut starts = Vec::new();
        let mut prev_alnum = false;
        for (i, c) in key.char_indices() {
            let alnum = c.is_alphanumeric();
            if alnum && !prev_alnum {
                starts.push(i);
            }
            prev_alnum = alnum;
        }
        Entry { kind, title: title.to_string(), url, needs, updated, key, starts }
    }

    /// 3: every word starts a word of the entry; 2: every word is contained; 1: the letters appear in order (three or
    /// more); 0: no.
    fn tier(&self, words: &[String], letters: &str) -> u8 {
        if words.iter().all(|w| self.starts.iter().any(|&s| self.key[s..].starts_with(w.as_str()))) {
            return 3;
        }
        if words.iter().all(|w| self.key.contains(w.as_str())) {
            return 2;
        }
        // Letters in order only from three letters on: two letters match nearly everything.
        let mut chars = self.key.chars();
        if letters.chars().count() >= 3 && letters.chars().all(|c| chars.any(|k| k == c)) {
            return 1;
        }
        0
    }
}

/// The best `limit` entries for `query` (all of them, best first, for an empty query).
pub fn search<'a>(entries: &'a [Entry], query: &str, limit: usize) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    let words: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    let letters: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    let mut hits: Vec<(u8, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            let tier = if words.is_empty() { 1 } else { e.tier(&words, &letters) };
            (tier > 0).then_some((tier, e))
        })
        .collect();
    hits.sort_by(|(ta, a), (tb, b)| tb.cmp(ta).then(b.needs.cmp(&a.needs)).then(b.updated.cmp(&a.updated)).then(a.title.cmp(&b.title)));
    hits.into_iter().take(limit).map(|(_, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: Kind, title: &str, needs: bool, updated: i64) -> Entry {
        Entry::new(kind, title, format!("/{title}"), needs, updated)
    }

    #[test]
    fn ranking() {
        let entries = vec![
            e(Kind::Session, "Fix CI on main", false, 5),
            e(Kind::Session, "Explicit cache", false, 9),
            e(Kind::Chat, "Release notes for CI", true, 1),
            e(Kind::Folio, "Desktop companion", false, 3),
            e(Kind::Folder, "launchpad", false, 2),
        ];
        let titles = |q: &str| search(&entries, q, 10).into_iter().map(|e| e.title.as_str()).collect::<Vec<_>>();
        // Word prefixes first (needs you breaks the tie), then containment ("expliCIt"), then letters in order.
        assert_eq!(titles("ci"), ["Release notes for CI", "Fix CI on main", "Explicit cache"]);
        assert_eq!(titles("fix ci"), ["Fix CI on main"]);
        assert_eq!(titles("dskcmp"), ["Desktop companion"]);
        assert_eq!(titles("folio"), ["Desktop companion"]);
        assert!(titles("zzz").is_empty());
        assert_eq!(search(&entries, "", 2).len(), 2);
    }

    /// The spotlight searches on each keystroke: 2,000 objects must stay well under 50 ms per query, even in a
    /// debug build.
    #[test]
    fn two_thousand_objects_under_50_ms() {
        let words = ["release", "fix", "ci", "desktop", "companion", "launch", "bot", "folio", "review", "tests"];
        let entries: Vec<Entry> = (0..2000).map(|i| e(Kind::Session, &format!("{} {} task {i}", words[i % 10], words[(i * 7) % 10]), i % 13 == 0, i as i64)).collect();
        let start = std::time::Instant::now();
        let queries = ["r", "re", "rel", "rele", "relea", "releas", "release", "release t", "release ta", "release tas", "zq", "dkcmp"];
        for q in queries {
            let _ = search(&entries, q, 6);
        }
        let per_query = start.elapsed() / queries.len() as u32;
        eprintln!("Go to: {per_query:?} per query over 2,000 objects");
        assert!(per_query < std::time::Duration::from_millis(50), "{per_query:?} per query");
    }
}

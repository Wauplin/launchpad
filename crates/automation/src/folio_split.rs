//! The line-based block splitter of folio pages, shared in spirit with the UI's `web/src/lib/folioSplit.ts`: both
//! produce the same outline for the fixture pages of `web/tests/folio-split/` (each `<name>.md` with its expected
//! `<name>.json`), tested on both sides, so Home's check and the reader never disagree on where a block starts.
//!
//! Rules (kept tiny on purpose, so both languages stay equal):
//! - Front matter: line 1 is `---`, closed by the next `---` or `...` line.
//! - Fences: ``` or ~~~ (3 or more, up to 3 spaces of indent), closed by the same character at least as long. The
//!   info's first word is the language: a catalog name is a typed block, anything else is code.
//! - Containers: `:::name{attrs} title` (3 or more colons) opens; a line of only colons closes the innermost open
//!   container when it has at least as many colons. Nest with more colons on the outer one.
//! - Headings: ATX `#`…`######` outside fences, with an optional `{#id}` at the end; the id is the explicit one or
//!   a slug of the text, `-1`, `-2`… for repeats on one page.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One part of a page's outline. Lines are 1-based.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
#[ts(rename = "FolioOutlineItem")]
pub enum Item {
    Heading { line: u32, level: u8, id: String, text: String, explicit: bool },
    Container { line: u32, end: u32, name: String, attrs: String, title: String, depth: u32, closed: bool },
    Fence { line: u32, end: u32, lang: String, info: String, depth: u32, closed: bool },
}

impl Item {
    pub fn line(&self) -> u32 {
        match self {
            Item::Heading { line, .. } | Item::Container { line, .. } | Item::Fence { line, .. } => *line,
        }
    }
}

/// A structural problem the splitter finds: F101 (front matter never closed), F102 (a container or fence never
/// closed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "FolioSplitProblem")]
pub struct SplitProblem {
    pub line: u32,
    pub code: String,
}

/// The front matter's lines: `start` is the opening `---` line, `end` the closing one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "FolioFront")]
pub struct Front {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "FolioOutline")]
pub struct Outline {
    pub front: Option<Front>,
    pub items: Vec<Item>,
    pub problems: Vec<SplitProblem>,
}

/// The lines of a text, without their line endings (a final newline adds no empty line).
pub fn lines(text: &str) -> Vec<&str> {
    let mut v: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if text.ends_with('\n') {
        v.pop();
    }
    v
}

fn indent_ok(line: &str) -> Option<&str> {
    let n = line.len() - line.trim_start_matches(' ').len();
    (n <= 3).then(|| &line[n..])
}

/// A fence opener: its character, length and info.
pub fn fence_open(line: &str) -> Option<(char, usize, String)> {
    let rest = indent_ok(line)?;
    let c = rest.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = rest.len() - rest.trim_start_matches(c).len();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim();
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info.to_string()))
}

fn fence_close(line: &str, c: char, len: usize) -> bool {
    let Some(rest) = indent_ok(line) else { return false };
    let n = rest.len() - rest.trim_start_matches(c).len();
    n >= len && rest[n..].trim().is_empty()
}

/// A container opener: colons, name, `{…}` attributes (with braces) and title.
pub fn container_open(line: &str) -> Option<(usize, String, String, String)> {
    let n = line.len() - line.trim_start_matches(':').len();
    if n < 3 {
        return None;
    }
    let rest = line[n..].trim_start();
    let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(rest.len());
    let name = &rest[..name_len];
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut tail = rest[name_len..].trim_start();
    let mut attrs = String::new();
    if tail.starts_with('{')
        && let Some(end) = tail.find('}')
    {
        attrs = tail[..=end].to_string();
        tail = tail[end + 1..].trim_start();
    }
    Some((n, name.to_string(), attrs, tail.trim_end().to_string()))
}

fn container_close(line: &str) -> Option<usize> {
    let n = line.len() - line.trim_start_matches(':').len();
    (n >= 3 && line[n..].trim().is_empty()).then_some(n)
}

/// An ATX heading: level, text and explicit id.
pub fn heading(line: &str) -> Option<(u8, String, Option<String>)> {
    let rest = indent_ok(line)?;
    let n = rest.len() - rest.trim_start_matches('#').len();
    if n == 0 || n > 6 {
        return None;
    }
    let after = &rest[n..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = after.trim().to_string();
    // A closing sequence of #s (after a space) is not part of the text.
    let stripped = text.trim_end_matches('#');
    if stripped.len() < text.len() && (stripped.is_empty() || stripped.ends_with([' ', '\t'])) {
        text = stripped.trim_end().to_string();
    }
    let mut id = None;
    if text.ends_with('}')
        && let Some(open) = text.rfind("{#")
    {
        let cand = &text[open + 2..text.len() - 1];
        if !cand.is_empty() && cand.chars().all(id_char) {
            id = Some(cand.to_string());
            text = text[..open].trim_end().to_string();
        }
    }
    Some((n as u8, text, id))
}

/// Characters of an id: letters, digits, `-`, `_`, `.` and `:`.
pub fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// The anchor id of a heading's text: lowercase, links reduced to their text, letters and digits kept, spaces and
/// dashes as `-`, everything else dropped; `section` when nothing is left.
pub fn slug(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    // `[text](url)` → `text`.
    while let Some(c) = chars.next() {
        if c == ']' && chars.peek() == Some(&'(') {
            for d in chars.by_ref() {
                if d == ')' {
                    break;
                }
            }
            continue;
        }
        plain.push(c);
    }
    let mut out = String::new();
    for c in plain.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '-' || c == '\t' {
            out.push('-');
        }
    }
    if out.trim_matches('-').is_empty() { "section".into() } else { out }
}

/// Splits a page into its outline.
pub fn outline(text: &str) -> Outline {
    let ls = lines(text);
    let mut items = vec![];
    let mut problems = vec![];
    let mut front = None;
    let mut i = 0;
    if ls.first().map(|l| l.trim_end()) == Some("---") {
        match ls.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")) {
            Some(p) => {
                front = Some(Front { start: 1, end: p as u32 + 2 });
                i = p + 2;
            }
            None => problems.push(SplitProblem { line: 1, code: "F101".into() }),
        }
    }
    // Open containers: (index in items, colons).
    let mut stack: Vec<(usize, usize)> = vec![];
    let mut seen: std::collections::HashMap<String, u32> = Default::default();
    while i < ls.len() {
        let line = ls[i];
        let no = i as u32 + 1;
        if let Some((c, len, info)) = fence_open(line) {
            let lang = info.split_whitespace().next().unwrap_or("").to_string();
            let mut j = i + 1;
            while j < ls.len() && !fence_close(ls[j], c, len) {
                j += 1;
            }
            let closed = j < ls.len();
            if !closed {
                problems.push(SplitProblem { line: no, code: "F102".into() });
            }
            let end = if closed { j as u32 + 1 } else { ls.len() as u32 };
            items.push(Item::Fence { line: no, end, lang, info, depth: stack.len() as u32, closed });
            i = j + 1;
            continue;
        }
        if let Some(n) = container_close(line) {
            if let Some(&(idx, colons)) = stack.last()
                && colons <= n
            {
                stack.pop();
                if let Item::Container { end, closed, .. } = &mut items[idx] {
                    *end = no;
                    *closed = true;
                }
            }
            i += 1;
            continue;
        }
        if let Some((colons, name, attrs, title)) = container_open(line) {
            stack.push((items.len(), colons));
            items.push(Item::Container { line: no, end: ls.len() as u32, name, attrs, title, depth: stack.len() as u32 - 1, closed: false });
            i += 1;
            continue;
        }
        if let Some((level, text, explicit)) = heading(line) {
            let base = explicit.clone().unwrap_or_else(|| slug(&text));
            let n = seen.entry(base.clone()).or_insert(0);
            let id = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
            *n += 1;
            items.push(Item::Heading { line: no, level, id, text, explicit: explicit.is_some() });
        }
        i += 1;
    }
    for (idx, _) in stack {
        problems.push(SplitProblem { line: items[idx].line(), code: "F102".into() });
    }
    problems.sort_by_key(|p| p.line);
    Outline { front, items, problems }
}

/// Parsed `{#id .class key=value key="two words"}` attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs {
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub pairs: Vec<(String, String)>,
    /// Bare words (`{open}`).
    pub flags: Vec<String>,
}

impl Attrs {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.pairs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
}

pub fn attrs(raw: &str) -> Attrs {
    let inner = raw.trim().trim_start_matches('{').trim_end_matches('}');
    let mut out = Attrs::default();
    let mut rest = inner.trim();
    while !rest.is_empty() {
        let (word, tail) = if let Some(eq) = rest.find('=').filter(|e| !rest[..*e].contains(char::is_whitespace)) {
            let key = &rest[..eq];
            let after = &rest[eq + 1..];
            let (value, tail) = if let Some(q) = after.strip_prefix('"') {
                match q.find('"') {
                    Some(e) => (&q[..e], &q[e + 1..]),
                    None => (q, ""),
                }
            } else {
                let e = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..e], &after[e..])
            };
            out.pairs.push((key.to_string(), value.to_string()));
            ("", tail)
        } else {
            let e = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..e], &rest[e..])
        };
        if let Some(id) = word.strip_prefix('#') {
            out.id = Some(id.to_string());
        } else if let Some(c) = word.strip_prefix('.') {
            out.classes.push(c.to_string());
        } else if !word.is_empty() {
            out.flags.push(word.to_string());
        }
        rest = tail.trim_start();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared fixtures (`web/tests/folio-split/`): each page's outline equals its expected JSON, the same files
    /// the UI's splitter is tested on (`pnpm test:folios`).
    #[test]
    fn shared_fixtures() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/tests/folio-split");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).expect("fixture folder") {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            // `FOLIO_FIXTURES_WRITE=1 cargo test shared_fixtures` writes the expected outlines of new fixtures.
            if std::env::var("FOLIO_FIXTURES_WRITE").is_ok() && !p.with_extension("json").exists() {
                std::fs::write(p.with_extension("json"), serde_json::to_string_pretty(&outline(&text)).unwrap() + "\n").unwrap();
            }
            let want: Outline = serde_json::from_str(&std::fs::read_to_string(p.with_extension("json")).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert_eq!(outline(&text), want, "{}", p.display());
            n += 1;
        }
        assert!(n >= 3);
    }

    #[test]
    fn headings_and_slugs() {
        assert_eq!(heading("## Waves {#waves}"), Some((2, "Waves".into(), Some("waves".into()))));
        assert_eq!(heading("#hashtag"), None);
        assert_eq!(heading("### Closing ###"), Some((3, "Closing".into(), None)));
        assert_eq!(slug("What [blocks](x.md) us?"), "what-blocks-us");
        assert_eq!(slug("¿?"), "section");
        assert_eq!(slug("Déjà vu"), "déjà-vu");
    }

    #[test]
    fn attributes() {
        let a = attrs("{#d1 .wide tone=verdict title=\"Two words\" open}");
        assert_eq!(a.id.as_deref(), Some("d1"));
        assert_eq!(a.classes, vec!["wide"]);
        assert_eq!(a.get("tone"), Some("verdict"));
        assert_eq!(a.get("title"), Some("Two words"));
        assert_eq!(a.flags, vec!["open"]);
    }
}

//! The daemon's recent log, for diagnosis: every tracing line, redacted, in `<data_dir>/logs/launchpad.jsonl`
//! (two files of at most 8 MB, so it survives restarts). `GET /api/logs` reads it (`query`); sessions with admin
//! Launchpad CLI access read it through `launchpad logs`. Secrets never reach the file: `redact` runs first.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Size of one log file before it rotates to `launchpad.jsonl.1`.
const FILE_MAX: u64 = 8 << 20;
/// Lines kept in memory before `init` names the file.
const EARLY_MAX: usize = 2000;
/// Longest message kept.
const MESSAGE_MAX: usize = 4000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Line {
    /// Unix milliseconds.
    pub at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

struct Sink {
    file: Option<(PathBuf, std::fs::File, u64)>,
    early: Vec<Line>,
}

static SINK: Mutex<Sink> = Mutex::new(Sink { file: None, early: Vec::new() });

fn path_in(dir: &Path) -> PathBuf {
    dir.join("logs").join("launchpad.jsonl")
}

/// Starts writing to `<data_dir>/logs/launchpad.jsonl`, with the lines logged before.
pub fn init(data_dir: &Path) -> std::io::Result<()> {
    let path = path_in(data_dir);
    std::fs::create_dir_all(path.parent().expect("logs folder"))?;
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let size = file.metadata()?.len();
    let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    sink.file = Some((path, file, size));
    let early = std::mem::take(&mut sink.early);
    for line in early {
        write(&mut sink, &line);
    }
    Ok(())
}

fn write(sink: &mut Sink, line: &Line) {
    let Some((path, file, size)) = sink.file.as_mut() else {
        if sink.early.len() >= EARLY_MAX {
            sink.early.remove(0);
        }
        sink.early.push(line.clone());
        return;
    };
    let Ok(mut text) = serde_json::to_string(line) else { return };
    text.push('\n');
    if *size + text.len() as u64 > FILE_MAX {
        let old = path.with_extension("jsonl.1");
        let _ = std::fs::rename(&*path, &old);
        match std::fs::OpenOptions::new().create(true).append(true).open(&*path) {
            Ok(f) => {
                *file = f;
                *size = 0;
            }
            Err(_) => return,
        }
    }
    if file.write_all(text.as_bytes()).is_ok() {
        *size += text.len() as u64;
    }
}

/// Records one log line (redacted, cut to `MESSAGE_MAX`).
pub fn push(level: &str, target: &str, message: &str) {
    let mut message = redact(message);
    if message.len() > MESSAGE_MAX {
        let mut end = MESSAGE_MAX;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push('…');
    }
    let line = Line { at: launchpad_core::util::now_ms(), level: level.to_string(), target: target.to_string(), message };
    let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    write(&mut sink, &line);
}

/// What `query` returns.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Filter {
    /// Lines at or after this time (Unix ms).
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// The lowest level: error, warn, info (default) or debug.
    pub level: Option<String>,
    /// Words that must all appear (case-insensitive), for example a session or runner id.
    #[serde(default)]
    pub any_of: Vec<String>,
    /// A text that must appear (case-insensitive).
    pub grep: Option<String>,
    /// The newest lines kept (default 300, at most 5000).
    pub limit: Option<usize>,
}

fn rank(level: &str) -> u8 {
    match level.to_ascii_lowercase().as_str() {
        "error" => 4,
        "warn" | "warning" => 3,
        "info" => 2,
        "debug" => 1,
        _ => 0,
    }
}

impl Filter {
    pub fn keeps(&self, l: &Line) -> bool {
        let lower = l.message.to_lowercase();
        self.since.is_none_or(|s| l.at >= s)
            && self.until.is_none_or(|u| l.at <= u)
            && rank(&l.level) >= rank(self.level.as_deref().unwrap_or("info"))
            && (self.any_of.is_empty() || self.any_of.iter().any(|w| !w.is_empty() && lower.contains(&w.to_lowercase())))
            && self.grep.as_deref().is_none_or(|g| lower.contains(&g.to_lowercase()))
    }

    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(300).clamp(1, 5000)
    }
}

/// The newest lines of this daemon's log that pass the filter, oldest first.
pub fn query(data_dir: &Path, filter: &Filter) -> Vec<Line> {
    let path = path_in(data_dir);
    let mut out: Vec<Line> = vec![];
    for p in [path.with_extension("jsonl.1"), path] {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()).filter(|l| filter.keeps(l)));
    }
    if out.is_empty() {
        let sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
        out = sink.early.iter().filter(|l| filter.keeps(l)).cloned().collect();
    }
    let extra = out.len().saturating_sub(filter.limit());
    out.drain(..extra);
    out
}

/// A duration such as `30m`, `2h` or `1d`, or Unix milliseconds: the time it names.
pub fn parse_since(s: &str, now: i64) -> Option<i64> {
    let s = s.trim();
    if let Ok(ms) = s.parse::<i64>() {
        return Some(ms);
    }
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok()?;
    let ms = match unit {
        "s" => 1000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(now - n * ms)
}

// ------------------------------------------------------------------ redaction

/// Token prefixes of well-known services.
const PREFIXES: &[&str] = &["hf_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "sk-ant-", "sk-", "xoxb-", "xoxp-", "xoxa-", "xoxe-", "xapp-", "glpat-", "AKIA", "ya29."];
/// Words that name a secret in `name=value` or `name: value`.
const SECRET_KEYS: &[&str] = &["token", "secret", "password", "passwd", "credential", "api_key", "apikey", "api-key", "authorization", "cookie", "private_key"];
const REDACTED: &str = "[redacted]";

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '=' | '~')
}

/// A word that looks like a credential: a known prefix or a long random string
/// (mixed case and digits). Commit hashes and UUIDs stay readable.
fn secret_word(w: &str) -> bool {
    let w = w.trim_end_matches(['.', ',', ';', ')']);
    if PREFIXES.iter().any(|p| w.len() >= p.len() + 16 && w.starts_with(p)) {
        return true;
    }
    if w.len() >= 32 && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '=' | '.')) {
        let upper = w.chars().any(|c| c.is_ascii_uppercase());
        let lower = w.chars().any(|c| c.is_ascii_lowercase());
        let digit = w.chars().any(|c| c.is_ascii_digit());
        return upper && lower && digit;
    }
    false
}

/// The line without credentials: known token formats, long random strings, `Bearer …`, `name=value` for secret
/// names, and the password of URLs.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    // After `Bearer`/`Basic`, or a secret name followed by `=`, `:` or `": "`: the next word is a secret.
    let mut hide_next = false;
    while i < chars.len() {
        let c = chars[i];
        if !token_char(c) {
            out.push(c);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && token_char(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        // `user:password@host` in a URL.
        if i < chars.len() && chars[i] == ':' && out.ends_with("://") {
            let rest: String = chars[i..].iter().take_while(|c| !c.is_whitespace() && **c != '/').collect();
            if let Some(at) = rest.find('@') {
                out.push_str(REDACTED);
                i += rest[..at].chars().count();
                continue;
            }
        }
        // A Telegram bot token: `123456789:AA…`, also in a Bot API path (`/bot123456789:AA…`). Every workspace's
        // own bot has one.
        let digits = word.strip_prefix("bot").unwrap_or(&word);
        if digits.len() >= 6 && digits.chars().all(|c| c.is_ascii_digit()) && chars.get(i) == Some(&':') {
            let n = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_' || **c == '-').count();
            if n >= 30 {
                out.push_str(REDACTED);
                i += 1 + n;
                continue;
            }
        }
        let lower = word.to_ascii_lowercase();
        if lower == "bearer" || lower == "basic" {
            hide_next = true;
            out.push_str(&word);
            continue;
        }
        if hide_next {
            out.push_str(REDACTED);
            hide_next = false;
            continue;
        }
        if let Some((name, value)) = word.split_once('=')
            && !name.is_empty()
        {
            let named = SECRET_KEYS.iter().any(|k| name.to_ascii_lowercase().contains(k));
            out.push_str(name);
            out.push('=');
            if value.is_empty() {
                hide_next = named;
            } else if named || secret_word(value) {
                out.push_str(REDACTED);
            } else {
                out.push_str(value);
            }
            continue;
        }
        if secret_word(&word) {
            out.push_str(REDACTED);
            continue;
        }
        out.push_str(&word);
        if SECRET_KEYS.iter().any(|k| lower.contains(k)) {
            // `token: value`, `"token": "value"`, `token=value` with a space.
            let mut j = i;
            while j < chars.len() && matches!(chars[j], '"' | '\'') {
                j += 1;
            }
            if j < chars.len() && matches!(chars[j], ':' | '=') {
                hide_next = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_hides_credentials_and_keeps_ids() {
        let r = redact("token hf_abcdefghijklmnopqrstuvwxyz1234 for ghp_ABCDEFGHIJKLMNOPQRSTUV0123");
        assert!(!r.contains("hf_abc") && !r.contains("ghp_ABC"), "{r}");
        assert_eq!(redact("Authorization: Bearer abc.def-ghi"), "Authorization: Bearer [redacted]");
        assert_eq!(redact("GET /x?token=abc123&b=2"), "GET /x?token=[redacted]&b=2");
        assert_eq!(redact(r#"{"secret": "s3cr3t", "id": "x"}"#), r#"{"secret": "[redacted]", "id": "x"}"#);
        assert_eq!(redact("clone https://user:pa55@github.com/o/r failed"), "clone https://[redacted]@github.com/o/r failed");
        assert_eq!(redact("telegram 123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ failed"), "telegram [redacted] failed");
        assert_eq!(redact("POST https://api.telegram.org/bot123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ/getUpdates"), "POST https://api.telegram.org/[redacted]/getUpdates");
        assert_eq!(redact("LAUNCHPAD_BURST_CREDENTIAL=r1.abc"), "LAUNCHPAD_BURST_CREDENTIAL=[redacted]");
        assert!(redact("key Zk3PqLm9Xa2Bc7Dd8Ee1Ff4Gg5Hh6Ii0Jj").contains(REDACTED));
        // Ids and hashes are what diagnosis needs.
        let keep = "Burst 3f2a9c1e-77aa-4c1b-9d10-2b3c4d5e6f70: job 6727bd0f8fdd1b0e3a2c4d5e failed at e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(redact(keep), keep);
        assert_eq!(redact("listening on 127.0.0.1:7317"), "listening on 127.0.0.1:7317");
    }

    #[test]
    fn filter_matches_levels_words_and_time() {
        let l = |level: &str, m: &str, at: i64| Line { at, level: level.into(), target: "x".into(), message: m.into() };
        let f = Filter { level: Some("warn".into()), any_of: vec!["ABC".into(), "zzz".into()], since: Some(10), ..Default::default() };
        assert!(f.keeps(&l("WARN", "Burst abc: no", 10)));
        assert!(f.keeps(&l("ERROR", "zzz", 11)));
        assert!(!f.keeps(&l("INFO", "abc", 11)));
        assert!(!f.keeps(&l("WARN", "other", 11)));
        assert!(!f.keeps(&l("WARN", "abc", 9)));
        assert_eq!(parse_since("2h", 10_000_000), Some(10_000_000 - 7_200_000));
        assert_eq!(parse_since("1700000000000", 0), Some(1_700_000_000_000));
        assert_eq!(parse_since("2w", 0), None);
    }
}

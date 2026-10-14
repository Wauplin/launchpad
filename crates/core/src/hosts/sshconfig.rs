//! Concrete `Host` entries of `~/.ssh/config` (and the files it includes), for Settings → Hosts → Add host.
//! Pattern entries (`*`, `?`, `!`) are skipped. Connections always use the system `ssh`, which applies the
//! whole configuration (Match, ProxyJump, certificates); this list is only a picker.

use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub proxy_jump: Option<String>,
}

pub fn hosts() -> Vec<SshConfigHost> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    read(&home.join(".ssh/config"), &home, &mut out, &mut seen, 0);
    let mut unique = std::collections::HashSet::new();
    out.retain(|h| unique.insert(h.alias.clone()));
    out
}

fn read(path: &Path, home: &Path, out: &mut Vec<SshConfigHost>, seen: &mut std::collections::HashSet<PathBuf>, depth: u32) {
    if depth > 8 || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else { return };
    parse(&text, out, &mut |pattern| {
        let pattern = if let Some(rest) = pattern.strip_prefix("~/") { home.join(rest) } else if Path::new(pattern).is_absolute() { PathBuf::from(pattern) } else { home.join(".ssh").join(pattern) };
        expand(&pattern)
    }, &mut |p, out| read(p, home, out, seen, depth + 1));
}

/// Files matching a simple `Include` glob (a `*` in the last component only).
fn expand(pattern: &Path) -> Vec<PathBuf> {
    let name = pattern.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.contains('*') {
        return vec![pattern.to_path_buf()];
    }
    let Some(dir) = pattern.parent() else { return vec![] };
    let (prefix, suffix) = name.split_once('*').unwrap_or((&name, ""));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.starts_with(prefix) && n.ends_with(suffix))).collect())
        .unwrap_or_default();
    files.sort();
    files
}

fn parse(text: &str, out: &mut Vec<SshConfigHost>, include: &mut dyn FnMut(&str) -> Vec<PathBuf>, read: &mut dyn FnMut(&Path, &mut Vec<SshConfigHost>)) {
    let mut current: Vec<usize> = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim().trim_start_matches('=').trim().trim_matches('"').to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace() {
                    if alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    current.push(out.len());
                    out.push(SshConfigHost { alias: alias.to_string(), hostname: None, user: None, proxy_jump: None });
                }
            }
            "match" => current.clear(),
            "include" if current.is_empty() => {
                for pattern in value.split_whitespace() {
                    for file in include(pattern) {
                        read(&file, out);
                    }
                }
            }
            "hostname" | "user" | "proxyjump" => {
                for &i in &current {
                    let field = match key.as_str() {
                        "hostname" => &mut out[i].hostname,
                        "user" => &mut out[i].user,
                        _ => &mut out[i].proxy_jump,
                    };
                    field.get_or_insert_with(|| value.clone());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn concrete_hosts_only() {
        let mut out = vec![];
        super::parse("Host *\n  User x\nHost devbox gpu-*\n  HostName devbox.lan\n  User me\nHost gpu-01\n  ProxyJump bastion\nMatch host foo\n  User y\n", &mut out, &mut |_| vec![], &mut |_, _| {});
        assert_eq!(out.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["devbox", "gpu-01"]);
        assert_eq!(out[0].hostname.as_deref(), Some("devbox.lan"));
        assert_eq!(out[0].user.as_deref(), Some("me"));
        assert_eq!(out[1].proxy_jump.as_deref(), Some("bastion"));
    }
}

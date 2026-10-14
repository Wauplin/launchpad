//! Host locations. A folder on an SSH host is written `<host>:<absolute path>` (the scp syntax), so it
//! fits every existing path field (`Project::path`, `Session::cwd`, worktree roots). Local paths stay
//! absolute native paths. A host location is a relative path to the local file system, so a local
//! file operation that was not converted fails instead of touching a local folder.

use std::path::Path;

/// Host IDs: 2 to 63 lowercase letters, digits, `.`, `_` or `-`, starting with a letter or digit. Two
/// characters at least, so a Windows drive letter (`C:/`) is never read as a host.
pub fn valid_id(id: &str) -> bool {
    (2..=63).contains(&id.len())
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// `(host, native path)` of a host location.
pub fn split(path: &str) -> Option<(&str, &str)> {
    let (host, rest) = path.split_once(':')?;
    (valid_id(host) && rest.starts_with('/')).then_some((host, rest))
}

pub fn split_path(path: &Path) -> Option<(&str, &str)> {
    split(path.to_str()?)
}

pub fn host_of(path: &str) -> Option<&str> {
    split(path).map(|(h, _)| h)
}

pub fn host_of_path(path: &Path) -> Option<&str> {
    split_path(path).map(|(h, _)| h)
}

/// The path as the host's processes see it.
pub fn native(path: &str) -> &str {
    split(path).map(|(_, p)| p).unwrap_or(path)
}

pub fn join(host: Option<&str>, native: &str) -> String {
    match host {
        Some(h) => format!("{h}:{native}"),
        None => native.to_string(),
    }
}

/// A native path printed by a process on the host of `reference` (git output), as a location.
pub fn rehost(reference: &str, native: &str) -> String {
    join(host_of(reference), native)
}

/// Both locations are on the same machine.
pub fn same_host(a: &str, b: &str) -> bool {
    host_of(a) == host_of(b)
}

/// Rewrites host locations of `host` inside a command argument into native paths, so `--add-dir
/// devbox:/x` or `projects."devbox:/x".trust_level` reach the host process as `/x`.
pub fn localize_arg(host: &str, arg: &str) -> String {
    arg.replace(&format!("{host}:/"), "/")
}

/// A short label for the UI: `devbox · ~/projects/x`, with the host's home shortened when known.
pub fn display(path: &str, home: Option<&str>) -> String {
    match split(path) {
        Some((host, p)) => {
            let p = match home.and_then(|h| p.strip_prefix(h)) {
                Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
                _ => p.to_string(),
            };
            format!("{host} · {p}")
        }
        None => crate::paths::tilde(Path::new(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_locations_round_trip_and_never_match_local_paths() {
        assert_eq!(split("devbox:/home/u/x"), Some(("devbox", "/home/u/x")));
        assert_eq!(split("gpu-01.lan:/srv"), Some(("gpu-01.lan", "/srv")));
        for local in ["/home/u/x", "C:/Users/x", r"C:\Users\x", "devbox:relative", "Devbox:/x", "-x:/y", "a:/x", ""] {
            assert_eq!(split(local), None, "{local:?}");
        }
        assert_eq!(native("devbox:/a/b"), "/a/b");
        assert_eq!(native("/a/b"), "/a/b");
        assert_eq!(rehost("devbox:/a", "/b/c"), "devbox:/b/c");
        assert_eq!(rehost("/a", "/b/c"), "/b/c");
        let p = Path::new("devbox:/home/u").join("proj");
        assert_eq!(p.to_str(), Some("devbox:/home/u/proj"));
        assert!(p.starts_with("devbox:/home"));
        assert!(!Path::new("/home/u/proj").starts_with("devbox:/home"));
        assert_eq!(localize_arg("devbox", "projects.\"devbox:/x y\".trust=1"), "projects.\"/x y\".trust=1");
        assert_eq!(display("devbox:/home/u/p", Some("/home/u")), "devbox · ~/p");
    }
}

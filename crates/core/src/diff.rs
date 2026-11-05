//! Unified diff parsing into GitHub's pull request file shape (`filename`, `status`, `patch`, …), for Local PRs,
//! Files and pull request views.
use serde_json::{Value, json};

pub fn line_stats(patch: &str) -> (usize, usize) {
    let mut out = (0, 0);
    for line in patch.lines() {
        if line.starts_with('+') {
            out.0 += 1;
        } else if line.starts_with('-') {
            out.1 += 1;
        }
    }
    out
}

pub fn parse_diff(patch: &str) -> Vec<Value> {
    let mut out = vec![];
    let mut file: Option<Value> = None;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            if let Some(f) = file.take() {
                out.push(f);
            }
            let path = line
                .rsplit_once(" \"b/")
                .map(|(_, p)| unquote(&format!("\"{p}")))
                .or_else(|| line.rsplit_once(" b/").map(|(_, p)| p.to_string()))
                .unwrap_or_default();
            file = Some(json!({"filename":path,"status":"modified","patch":""}));
        } else if let Some(f) = &mut file {
            in_hunk |= line.starts_with("@@");
            if in_hunk {
                if let Value::String(patch) = &mut f["patch"] {
                    patch.push_str(line);
                    patch.push('\n');
                }
                continue;
            }
            if let Some(path) = line.strip_prefix("+++ ") {
                // Git terminates unquoted headers containing spaces with a tab separator.
                let path = unquote(path.strip_suffix('\t').unwrap_or(path));
                if let Some(path) = path.strip_prefix("b/") {
                    f["filename"] = json!(path);
                } else if path == "/dev/null" {
                    f["status"] = json!("removed");
                }
            } else if let Some(path) = line.strip_prefix("--- ") {
                let path = unquote(path.strip_suffix('\t').unwrap_or(path));
                if f["filename"] == ""
                    && let Some(path) = path.strip_prefix("a/")
                {
                    f["filename"] = json!(path);
                }
            } else if let Some(path) = line.strip_prefix("rename from ") {
                f["previous_filename"] = json!(unquote(path));
            } else if let Some(path) = line.strip_prefix("rename to ") {
                f["filename"] = json!(unquote(path));
                f["status"] = json!("renamed");
            } else if line.starts_with("new file mode") {
                f["status"] = json!("added");
            } else if line.starts_with("deleted file mode") {
                f["status"] = json!("removed");
            }
            if let Some(path) = line
                .strip_prefix("Binary files a/")
                .and_then(|s| s.split_once(" and ").map(|(a, _)| a))
                && f["filename"] == ""
            {
                f["filename"] = json!(path);
            }
        }
    }
    if let Some(f) = file {
        out.push(f);
    }
    out
}

fn unquote(s: &str) -> String {
    serde_json::from_str::<String>(s).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_lines_cannot_spoof_diff_headers() {
        let files = parse_diff("diff --git a/real b/real\n--- a/real\n+++ b/real\n@@ -1,2 +1,4 @@\n--- a/fake\n+++ b/fake\n+++ /dev/null\n+added\n@@ -9 +11 @@\n+++ b/another\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["filename"], "real");
        assert_eq!(files[0]["status"], "modified");
        assert!(files[0]["patch"].as_str().unwrap().contains("+++ /dev/null"));
        assert_eq!(line_stats(files[0]["patch"].as_str().unwrap()), (4, 1));
    }

    #[test]
    fn diff_paths_and_empty_changes() {
        let files = parse_diff(
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\ndiff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        );
        assert_eq!(files.len(), 3);
        assert_eq!(files[0]["filename"], "a");
        assert!(files[0]["patch"].as_str().unwrap().contains("+new"));
        assert_eq!(files[1]["filename"], "new");
        assert_eq!(files[1]["previous_filename"], "old");
        assert_eq!(files[1]["patch"], "");
        assert_eq!(files[2]["status"], "removed");
        let binary = parse_diff(
            "diff --git a/new image.png b/new image.png\nnew file mode 100644\nBinary files /dev/null and b/new image.png differ\n",
        );
        assert_eq!(binary[0]["filename"], "new image.png");
        assert_eq!(binary[0]["status"], "added");
    }
}

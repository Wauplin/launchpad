//! `launchpad slack --profile <handle> <slackline arguments>`: slackline's commands, run by Home with one of the
//! session's Slack profiles (`launchpad_core::slack`). This machine needs neither slackline nor a token: the
//! arguments go to Home, which answers slackline's exit code and output. The CLI reads the file of `files upload`
//! and writes the file of `files download -o`, in the agent's folder.

use anyhow::{Context, Result};
use base64::Engine;
use launchpad_core::slack::{MAX_UPLOAD, Output};
use std::io::Write;

/// Pure: `--profile <handle>` (or `--profile=<handle>`) taken out of slackline's arguments, before any `--`.
pub fn take_profile(args: &mut Vec<String>) -> Option<String> {
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let i = args[..end].iter().position(|a| a == "--profile" || a.starts_with("--profile="))?;
    if let Some(v) = args[i].strip_prefix("--profile=") {
        let v = v.to_string();
        args.remove(i);
        return Some(v);
    }
    if i + 1 >= end {
        return None;
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

/// Runs the call and returns slackline's exit code.
pub async fn run(client: &crate::Client, profile: Option<String>, mut args: Vec<String>) -> Result<i32> {
    let profile = profile.or_else(|| take_profile(&mut args));
    let plan = launchpad_core::slack::plan(&args).map_err(|e| anyhow::anyhow!(e))?;
    let mut args = plan.args.clone();
    // `files download -o <path>`: Home answers the bytes, written here.
    let output = plan.output.map(|i| {
        let path = args[i + 1].clone();
        args.drain(i..=i + 1);
        path
    });
    let file = match plan.upload {
        Some(i) => {
            let path = std::path::PathBuf::from(&args[i]);
            let meta = std::fs::metadata(&path).with_context(|| format!("read {}", path.display()))?;
            anyhow::ensure!(meta.is_file(), "{} is not a file", path.display());
            anyhow::ensure!(meta.len() as usize <= MAX_UPLOAD, "{} is larger than {} MB, the most launchpad slack uploads", path.display(), MAX_UPLOAD >> 20);
            let data = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            Some(serde_json::json!({"name": name, "data": base64::engine::general_purpose::STANDARD.encode(data)}))
        }
        None => None,
    };
    let session_id = std::env::var("LAUNCHPAD_SESSION_ID").ok();
    let body = serde_json::json!({"session_id": session_id, "profile": profile, "args": args, "file": file});
    let answer = client.json("POST", "/api/slack/run", Some(body)).await?;
    let out: Output = serde_json::from_value(answer).context("Home's answer is not a Slack result")?;
    let stdout = base64::engine::general_purpose::STANDARD.decode(out.stdout.as_bytes()).context("Home's answer is not base64")?;
    match output {
        Some(path) if out.code == 0 => {
            std::fs::write(&path, &stdout).with_context(|| format!("write {path}"))?;
            eprintln!("Saved {} bytes to {path}", stdout.len());
        }
        _ => std::io::stdout().write_all(&stdout)?,
    }
    std::io::stderr().write_all(out.stderr.as_bytes())?;
    Ok(out.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_comes_out_of_the_arguments() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut a = v(&["search", "messages", "x", "--profile", "user-read"]);
        assert_eq!(take_profile(&mut a).as_deref(), Some("user-read"));
        assert_eq!(a, v(&["search", "messages", "x"]));
        let mut a = v(&["--profile=bot", "users", "list"]);
        assert_eq!(take_profile(&mut a).as_deref(), Some("bot"));
        assert_eq!(a, v(&["users", "list"]));
        let mut a = v(&["dms", "send", "--", "U1", "--profile", "p"]);
        assert_eq!(take_profile(&mut a), None);
        assert_eq!(a.len(), 6);
    }
}

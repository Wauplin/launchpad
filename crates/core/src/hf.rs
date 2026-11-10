//! The `hf` CLI of this machine: bucket commands for Home's backups and trace copies, and the sign-in check.
use std::time::Duration;
use anyhow::{Context, Result};
use crate::{exec::Io, shellenv::UserEnv};

pub(crate) async fn hf(env: &UserEnv, args: &[&str], data: Option<Vec<u8>>) -> Result<String> {
    match run(env, args, data).await? {
        (true, out, _) => Ok(out),
        (false, _, err) => anyhow::bail!("hf command failed ({}); check CLI version, sign-in and bucket permissions", last_line(&err)),
    }
}

/// Runs `hf`: whether it succeeded, its output and its error output.
pub(crate) async fn run(env: &UserEnv, args: &[&str], data: Option<Vec<u8>>) -> Result<(bool, String, String)> {
    let mut cmd = crate::procs::command(env, "hf", None);
    cmd.args(args).stdin(Io::Null).stderr(Io::Piped);
    if let Some(data) = data { cmd.stdin_bytes(data); }
    let out = tokio::time::timeout(Duration::from_secs(if args.first() == Some(&"auth") { 20 } else { 120 }), cmd.output()).await.context("hf timed out")??;
    Ok((out.status.success(), String::from_utf8(out.stdout)?, String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// The last non-empty line of an error output, cut to 300 characters.
pub(crate) fn last_line(err: &str) -> String {
    err.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no error output").chars().take(300).collect()
}

pub async fn account(env: &UserEnv) -> Result<String> {
    let out = hf(env, &["auth", "whoami", "--format", "json"], None).await?;
    let value: serde_json::Value = serde_json::from_str(&out)?;
    let user = value["user"].as_str().context("hf did not report an account")?;
    anyhow::ensure!(!user.is_empty() && user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')), "hf reported an invalid account");
    Ok(user.into())
}

/// Read-only setup check, using the same contract as the GitHub card, plus the signed-in account.
pub async fn check(env: &UserEnv) -> (crate::signin::SignInCheck, Option<String>) {
    let cli_found = env.which("hf").is_some_and(|p| p.is_file());
    let mut c = crate::signin::SignInCheck { id: "huggingface".into(), profile: None, name: "Hugging Face CLI".into(), cli_found, signed_in: None, detail: None, command: "hf auth login".into() };
    let mut user = None;
    if cli_found {
        match account(env).await {
            Ok(u) => { c.signed_in = Some(true); c.detail = Some(format!("Account {u}")); user = Some(u); }
            Err(_) => { c.signed_in = Some(false); c.detail = Some("hf auth whoami failed. Sign in, or update hf if it does not support JSON output.".into()); }
        }
    }
    (c, user)
}

// Names are user-editable. Escape separators and percent signs so each stays one distinct segment.
pub(crate) fn name_segment(name: &str) -> String {
    if name.is_empty() { return "_".into(); }
    if matches!(name, "." | "..") { return name.replace('.', "%2E"); }
    let mut segment = String::new();
    for c in name.chars() {
        if matches!(c, '/' | '\\' | '%' | '?' | '#') || c.is_control() {
            for byte in c.to_string().bytes() {
                use std::fmt::Write;
                write!(segment, "%{byte:02X}").unwrap();
            }
        } else {
            segment.push(c);
        }
    }
    segment
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_stay_in_distinct_path_segments() {
        assert_eq!(name_segment("My workspace"), "My workspace");
        assert_eq!(name_segment("a/b\\c%?#"), "a%2Fb%5Cc%25%3F%23");
        assert_eq!(name_segment("a%2Fb"), "a%252Fb");
        assert_eq!(name_segment(".."), "%2E%2E");
        assert_eq!(name_segment(""), "_");
    }
}

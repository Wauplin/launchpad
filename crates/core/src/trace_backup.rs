//! Best-effort, single-worker trace export. Local SQLite remains the source of truth.
use std::{collections::BTreeMap, sync::{Arc, RwLock}, time::Duration};
use anyhow::{Context, Result};
use crate::{App, exec::Io, shellenv::UserEnv};

/// The outcome of the last pass, shown in Settings → Hugging Face.
#[derive(Clone, Default, Debug, serde::Serialize)]
pub struct BackupState {
    pub error: Option<String>,
    /// When the last chunk was uploaded (ms since the epoch); `None` since startup.
    pub last_upload: Option<i64>,
}

#[derive(Clone, Default)]
pub struct BackupStatus(pub Arc<RwLock<BackupState>>);

async fn hf(env: &UserEnv, args: &[&str], data: Option<Vec<u8>>) -> Result<String> {
    let mut cmd = crate::procs::command(env, "hf", None);
    cmd.args(args).stdin(Io::Null).stderr(Io::Null);
    if let Some(data) = data { cmd.stdin_bytes(data); }
    let out = tokio::time::timeout(Duration::from_secs(if args.first() == Some(&"auth") { 20 } else { 120 }), cmd.output()).await.context("hf timed out")??;
    anyhow::ensure!(out.status.success(), "hf command failed; check CLI version, sign-in and bucket permissions");
    Ok(String::from_utf8(out.stdout)?)
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

fn destination(bucket: &str, user: &str, folder: &str, harness: &str, id: &str) -> String {
    let bucket = if bucket.contains('/') { bucket.to_owned() } else { format!("{user}/{bucket}") };
    format!("hf://buckets/{bucket}/{folder}/{harness}/{id}")
}

pub(crate) async fn upload(app: &App) -> Result<()> {
    let config = app.config.get().trace_backup;
    if !config.enabled { return Ok(()); }
    config.validate()?;
    let user = account(&app.env).await?;
    let bucket = if config.bucket.contains('/') { config.bucket.clone() } else { format!("{user}/{}", config.bucket) };
    let cursor_path = app.paths.data_dir.join("trace-backup-cursors.json");
    let mut cursors: BTreeMap<String, i64> = match tokio::fs::read(&cursor_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(e.into()),
    };
    let mut created = false;
    for session in app.sessions.list() {
        // Disabling or changing the destination stops this pass before the next session.
        if app.config.get().trace_backup != config { break; }
        let dest = destination(&config.bucket, &user, &config.folder, session.harness.as_str(), &session.id);
        let after = cursors.get(&dest).copied().unwrap_or(0);
        if session.last_seq <= after { continue; }
        let id = session.id.clone();
        // Export raw stored bodies, preserving attachments, unknown event fields and sequence numbers.
        let rows = app.state_db.call(move |c| {
            let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq > ?2 ORDER BY seq LIMIT 500")?;
            let rows = st.query_map(rusqlite::params![id, after], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?;
        let Some((through, _, _)) = rows.last() else { continue };
        let through = *through;
        let mut data = Vec::new();
        for (seq, ts, json) in rows {
            let body: serde_json::Value = serde_json::from_str(&json)?;
            serde_json::to_writer(&mut data, &serde_json::json!({"session_id": session.id, "seq": seq, "ts": ts, "body": body}))?;
            data.push(b'\n');
        }
        if !created {
            hf(&app.env, &["buckets", "create", &bucket, "--private", "--exist-ok", "--quiet"], None).await?;
            created = true;
        }
        // Keep the original task and context even when startup failed before a user-message event.
        let metadata = serde_json::to_vec(&serde_json::json!({
            "id": session.id, "harness": session.harness, "title": session.title,
            "prompt": session.prompt, "model": session.model, "effort": session.effort,
            "cwd": session.cwd, "project_id": session.project_id, "started_at": session.started_at,
        }))?;
        hf(&app.env, &["buckets", "cp", "-", &format!("{dest}/session.json"), "--quiet"], Some(metadata)).await?;
        let target = format!("{dest}/events-{}.jsonl", after + 1);
        hf(&app.env, &["buckets", "cp", "-", &target, "--quiet"], Some(data)).await?;
        cursors.insert(dest, through);
        crate::paths::write_private(&cursor_path, &serde_json::to_string(&cursors)?)?;
        app.trace_backup.0.write().unwrap().last_upload = Some(crate::util::now_ms());
    }
    Ok(())
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let result = upload(&app).await;
            let error = result.err().map(|e| {
                tracing::warn!("Hugging Face trace backup failed; retrying in 60 seconds: {e:#}");
                "Trace backup failed. Check hf auth whoami, update hf and verify bucket write access. Retrying automatically; agents are unaffected.".to_string()
            });
            app.trace_backup.0.write().unwrap().error = error;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_include_account_machine_harness_and_session() {
        assert_eq!(destination("launchpad-traces", "alice", "laptop", "codex", "session"), "hf://buckets/alice/launchpad-traces/laptop/codex/session");
        assert_eq!(destination("team/traces", "alice", "desktop", "claude", "id"), "hf://buckets/team/traces/desktop/claude/id");
    }
    #[test]
    fn defaults_and_invalid_paths() {
        let mut c = crate::config::TraceBackup::default();
        assert!(!c.enabled);
        assert_eq!(c.bucket, "launchpad-traces");
        c.validate().unwrap();
        for bad in ["", "../x", "--option", "a/b/c", "a b"] { c.bucket = bad.into(); assert!(c.validate().is_err()); }
        c.bucket = "team/traces".into();
        for bad in ["..", ".", "", "a/b", "a\\b"] { c.folder = bad.into(); assert!(c.validate().is_err()); }
    }
}

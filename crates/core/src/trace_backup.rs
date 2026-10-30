//! Best-effort, single-worker trace export. Local SQLite remains the source of truth.
use std::{collections::BTreeMap, sync::{Arc, RwLock}, time::Duration};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
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

// Names are user-editable. Escape separators and percent signs so each stays one distinct segment.
fn name_segment(name: &str) -> String {
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

fn destination(bucket: &str, user: &str, folder: &str, started_at: i64, id: &str) -> Result<String> {
    let bucket = if bucket.contains('/') { bucket.to_owned() } else { format!("{user}/{bucket}") };
    let folder = name_segment(folder);
    let started_at = chrono::DateTime::from_timestamp_millis(started_at)
        .context("invalid session start time")?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(format!("hf://buckets/{bucket}/{folder}/{started_at}-{id}"))
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Cursor {
    events: i64,
    files: BTreeMap<String, String>,
}

async fn changed_file(env: &UserEnv, dest: &str, cursor: &mut Cursor, name: &str, data: Vec<u8>) -> Result<bool> {
    let hash = hex::encode(Sha256::digest(&data));
    if cursor.files.get(name) == Some(&hash) { return Ok(false); }
    hf(env, &["buckets", "cp", "-", &format!("{dest}/{name}"), "--quiet"], Some(data)).await?;
    cursor.files.insert(name.into(), hash);
    Ok(true)
}

/// Keep every serialized run field and the resolved destination context, without profile secrets.
fn metadata(session: &crate::model::Session, workspace: &str, folder: &str) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&serde_json::json!({
        "format": "launchpad", "version": 1,
        "session": session, "workspace": workspace, "folder": folder,
    }))?)
}

pub(crate) async fn upload(app: &App) -> Result<()> {
    let config = app.config.get().trace_backup;
    if !config.enabled { return Ok(()); }
    config.validate()?;
    let user = account(&app.env).await?;
    let bucket = if config.bucket.contains('/') { config.bucket.clone() } else { format!("{user}/{}", config.bucket) };
    let cursor_path = app.paths.data_dir.join("trace-backup-native-cursors.json");
    let mut cursors: BTreeMap<String, Cursor> = match tokio::fs::read(&cursor_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(e.into()),
    };
    // Resolve current names every pass; renames naturally get new destination-keyed cursors.
    let workspaces = crate::workspace::list(app).await?;
    let projects = crate::projects::list(app).await?;
    let mut created = false;
    let mut first_error = None;
    for session in app.sessions.list() {
        // Disabling or changing the destination stops this pass before the next session.
        if app.config.get().trace_backup != config { break; }
        let workspace_id = match &session.launch_spec.target {
            crate::model::Target::Assistant { workspace_id } => workspace_id.as_deref(),
            _ => session.launch_spec.workspace_id.as_deref(),
        };
        let workspace = workspace_id.and_then(|id| workspaces.iter().find(|w| w.id == id))
            .map(|w| w.name.as_str()).unwrap_or("Everything");
        let folder = session.project_id.as_deref().and_then(|id| projects.iter().find(|p| p.id == id))
            .map(|p| p.name.as_str()).unwrap_or(if session.project_id.is_some() { "Removed folder" } else { "Scratch folder" });
        let dest = destination(&config.bucket, &user, folder, session.started_at, &session.id)?;
        let cursor = cursors.entry(dest.clone()).or_default();
        let after = cursor.events;
        let id = session.id.clone();
        // Export raw stored bodies, preserving attachments, unknown event fields and sequence numbers.
        let rows = app.state_db.call(move |c| {
            let mut st = c.prepare("SELECT seq, ts, json FROM session_event WHERE session_id = ?1 AND seq > ?2 ORDER BY seq LIMIT 500")?;
            let rows = st.query_map(rusqlite::params![id, after], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?;
        let through = rows.last().map(|r| r.0).unwrap_or(after);
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
        // Native snapshots are checked independently of event cursors: a writer may flush late,
        // or the owner may continue a conversation in its native terminal UI.
        let mut uploaded = false;
        let result: Result<()> = async {
            uploaded |= changed_file(&app.env, &dest, cursor, "launchpad.json", metadata(&session, workspace, folder)?).await?;
            if !data.is_empty() {
                hf(&app.env, &["buckets", "cp", "-", &format!("{dest}/launchpad-events-{}.jsonl", after + 1), "--quiet"], Some(data)).await?;
                cursor.events = through;
                uploaded = true;
            }
            if session.harness_session_id.is_some() {
                let files = tokio::time::timeout(Duration::from_secs(60), app.harnesses.get(session.harness).native_trace(app, &session)).await
                    .context("native trace export timed out")??;
                for file in files {
                    uploaded |= changed_file(&app.env, &dest, cursor, &file.name, file.data).await?;
                }
            }
            Ok(())
        }.await;
        if uploaded { app.trace_backup.0.write().unwrap().last_upload = Some(crate::util::now_ms()); }
        // Preserve successful uploads even when a native file was removed or its host is offline.
        // One unavailable historical session must not starve the rest of the backup.
        crate::paths::write_private(&cursor_path, &serde_json::to_string(&cursors)?)?;
        if let Err(e) = result {
            tracing::warn!(session = %session.id, "native trace backup: {e:#}");
            if first_error.is_none() { first_error = Some(e); }
        }
    }
    if let Some(e) = first_error { return Err(e); }
    Ok(())
}

pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let result = upload(&app).await;
            let error = result.err().map(|e| {
                tracing::warn!("Hugging Face trace backup failed; retrying in 60 seconds: {e:#}");
                "Trace backup incomplete. Check hf auth whoami, bucket write access, native trace files and SSH hosts. Missing or cleaned-up native traces cannot be reconstructed; other sessions continue backing up. Retrying automatically; agents are unaffected.".to_string()
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
    fn trace_metadata_contains_the_full_run_even_without_native_history() {
        let session: crate::model::Session = serde_json::from_value(serde_json::json!({
            "id":"failed-start", "harness":"claude", "cwd":"/project", "title":"Run",
            "status":"error", "status_since":10, "live":false, "open_question":false,
            "prompt":"original task", "usage": {"input_tokens":3,"cached_tokens":1,"output_tokens":2},
            "started_at":1,"ended_at":10,"last_seq":0,"seen_at":0,
            "launch_spec":{"profile":"profile-id","permission_mode":"auto"}
        })).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&metadata(&session, "Work", "Project").unwrap()).unwrap();
        assert_eq!(value["session"], serde_json::to_value(&session).unwrap());
        assert_eq!(value["workspace"], "Work");
        assert_eq!(value["folder"], "Project");
        assert_eq!(value["session"]["usage"]["input_tokens"], 3);
        assert_eq!(value["session"]["launch_spec"]["profile"], "profile-id");
        assert!(value["session"]["harness_session_id"].is_null());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn trace_snapshot_uploads_only_changes_and_retries_failures() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = crate::harness::trace::tests::Temp::new();
        tmp.write("hf", b"#!/bin/sh\nif [ -f \"$TRACE_TEST_DIR/fail\" ]; then exit 1; fi\n/bin/cat > \"$TRACE_TEST_DIR/payload\"\n");
        std::fs::set_permissions(tmp.0.join("hf"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let env = UserEnv::resolve().await.with(&[("PATH".into(), tmp.0.to_string_lossy().into_owned()), ("TRACE_TEST_DIR".into(), tmp.0.to_string_lossy().into_owned())]);
        let mut cursor = Cursor::default();
        assert!(changed_file(&env, "hf://buckets/test/traces/run", &mut cursor, "native.jsonl", b"{}\n".to_vec()).await.unwrap());
        assert_eq!(std::fs::read(tmp.0.join("payload")).unwrap(), b"{}\n");
        std::fs::remove_file(tmp.0.join("payload")).unwrap();
        assert!(!changed_file(&env, "hf://buckets/test/traces/run", &mut cursor, "native.jsonl", b"{}\n".to_vec()).await.unwrap());
        assert!(!tmp.0.join("payload").exists());
        tmp.write("fail", b"");
        assert!(changed_file(&env, "hf://buckets/test/traces/run", &mut cursor, "native.jsonl", b"{\"new\":true}\n".to_vec()).await.is_err());
        std::fs::remove_file(tmp.0.join("fail")).unwrap();
        // Persisted hashes and independent event progress survive a restart.
        cursor.events = 42;
        let mut cursor: Cursor = serde_json::from_slice(&serde_json::to_vec(&cursor).unwrap()).unwrap();
        assert!(changed_file(&env, "hf://buckets/test/traces/run", &mut cursor, "native.jsonl", b"{\"new\":true}\n".to_vec()).await.unwrap());
        assert_eq!(cursor.events, 42);
    }

    #[test]
    fn destinations_group_by_folder_and_sort_by_session_start_time() {
        let start = 1_793_363_696_789; // 2026-10-30T12:34:56.789Z
        assert_eq!(destination("launchpad-traces", "alice", "Launchpad", start, "session").unwrap(), "hf://buckets/alice/launchpad-traces/Launchpad/2026-10-30T12:34:56.789Z-session");
        assert_eq!(destination("team/traces", "alice", "Notes", 0, "id").unwrap(), "hf://buckets/team/traces/Notes/1970-01-01T00:00:00.000Z-id");
        let earlier = destination("traces", "alice", "Folder/Name", start, "z").unwrap();
        let later = destination("traces", "alice", "Folder/Name", start + 1, "a").unwrap();
        assert!(earlier < later);
        assert!(earlier.contains("/Folder%2FName/"));
        assert!(destination("traces", "alice", "Notes", i64::MAX, "id").is_err());
    }
    #[test]
    fn names_stay_in_distinct_path_segments() {
        assert_eq!(name_segment("My workspace"), "My workspace");
        assert_eq!(name_segment("a/b\\c%?#"), "a%2Fb%5Cc%25%3F%23");
        assert_eq!(name_segment("a%2Fb"), "a%252Fb");
        assert_eq!(name_segment(".."), "%2E%2E");
        assert_eq!(name_segment(""), "_");
    }
    #[test]
    fn defaults_and_invalid_paths() {
        let mut c = crate::config::TraceBackup::default();
        assert!(!c.enabled);
        assert_eq!(c.bucket, "launchpad-traces");
        c.validate().unwrap();
        for bad in ["", "../x", "--option", "a/b/c", "a b"] { c.bucket = bad.into(); assert!(c.validate().is_err()); }
        c.bucket = "team/traces".into();
        c.validate().unwrap();
    }
}

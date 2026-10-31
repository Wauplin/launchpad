//! Read-only replicas of Bucket traces. Only the originating instance exports a session.
use std::collections::BTreeMap;
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use crate::{App, model::{LaunchSpec, Project, Session, SessionEvent, SessionStatus, Target}};
use crate::trace_backup::{hf, name_segment};

type Files = BTreeMap<String, String>;

#[derive(Deserialize)]
struct Entry {
    path: String,
    #[serde(default)]
    xet_hash: Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
    format: String,
    version: u32,
    folder: String,
    session: Session,
}

fn localize(mut meta: Metadata, project: &Project, source: &str) -> Result<Session> {
    anyhow::ensure!(meta.format == "launchpad" && meta.version == 1, "unsupported trace format");
    anyhow::ensure!(meta.folder == project.name, "trace folder does not match");
    let s = &mut meta.session;
    anyhow::ensure!(s.imported_from.is_none(), "cannot import a replica");
    anyhow::ensure!(uuid::Uuid::parse_str(&s.id).is_ok(), "invalid trace session ID");
    let done = s.dependency_done();
    s.imported_from = Some(source.into());
    s.project_id = Some(project.id.clone());
    s.cwd = project.path.clone();
    // Retain the historical prompt and reported usage, but no foreign execution context.
    s.launch_spec = LaunchSpec { target: Target::Project { project_id: project.id.clone() }, ..Default::default() };
    s.harness_session_id = None;
    s.title_pending = false;
    s.archived_done = done;
    s.status = SessionStatus::Waiting;
    s.live = false;
    s.open_question = false;
    s.quota_pause = None;
    s.capacity_retry_count = 0;
    s.capacity_retry_at = None;
    s.seen_at = s.status_since; // Historical imports do not create notifications or Needs you items.
    Ok(meta.session)
}

/// The cursor advances in the same transaction as the trace. Replays are harmless, gaps retry.
pub(crate) fn store(c: &mut rusqlite::Connection, mut session: Session, events: Vec<SessionEvent>, cursor: String) -> Result<i64> {
    let tx = c.transaction()?;
    for event in events {
        anyhow::ensure!(event.session_id == session.id && event.seq > 0, "invalid trace event identity");
        tx.execute("INSERT INTO session_event (session_id, seq, ts, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (session_id, seq) DO UPDATE SET ts = excluded.ts, kind = excluded.kind, json = excluded.json",
            params![session.id, event.seq, event.ts, event.body.kind(), serde_json::to_string(&event.body)?])?;
    }
    let (count, last): (i64, i64) = tx.query_row("SELECT count(*), coalesce(max(seq), 0) FROM session_event WHERE session_id = ?1", [&session.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    anyhow::ensure!(count == last, "trace event chunks are incomplete; retrying next pass");
    session.last_seq = last;
    crate::sessions::Sessions::store(&tx, &session)?;
    tx.execute("UPDATE session SET last_seq = ?2, last_message = ?3, prompt = ?4 WHERE id = ?1", params![session.id, last, session.last_message, session.prompt])?;
    tx.execute("INSERT INTO trace_import_cursor VALUES (?1, ?2) ON CONFLICT (source) DO UPDATE SET json = excluded.json", params![session.imported_from, cursor])?;
    tx.commit()?;
    Ok(last)
}

pub(crate) async fn download(app: &App) -> Result<()> {
    let config = app.config.get().trace_backup;
    if !config.enabled { return Ok(()); }
    config.validate()?;
    let user = crate::trace_backup::account(&app.env).await?;
    let bucket = if config.bucket.contains('/') { config.bucket.clone() } else { format!("{user}/{}", config.bucket) };
    let projects = crate::projects::list(app).await?;
    if projects.is_empty() { return Ok(()); }
    // One listing avoids a failing request for every local folder not yet in the bucket.
    let entries: Vec<Entry> = serde_json::from_str(&hf(&app.env, &["buckets", "list", &format!("hf://buckets/{bucket}"), "--recursive", "--format", "json"], None).await?)?;
    let mut runs: BTreeMap<String, Files> = BTreeMap::new();
    let prefix = if config.prefix.is_empty() { String::new() } else { format!("{}/", config.prefix) };
    for e in entries {
        let Some(relative) = e.path.strip_prefix(&prefix) else { continue };
        let Some((dir, file)) = relative.rsplit_once('/') else { continue };
        let Some((folder, run)) = dir.split_once('/') else { continue };
        if run.contains('/') || !projects.iter().any(|p| name_segment(&p.name) == folder) { continue; }
        if file != "launchpad.json" && !(file.starts_with("launchpad-events-") && file.ends_with(".jsonl")) { continue; }
        // Only normalized sidecars are downloaded; native snapshots stay in the bucket.
        let hash = e.xet_hash.context("bucket listing omitted file hash; update hf")?;
        runs.entry(dir.into()).or_default().insert(file.into(), hash);
    }
    let mut first_error = None;
    for (dir, files) in runs {
        if app.config.get().trace_backup != config { break; }
        if !files.contains_key("launchpad.json") { continue; }
        let (folder, run) = dir.split_once('/').unwrap();
        let Some((_, id)) = run.rsplit_once('_') else { continue };
        let Ok(id) = uuid::Uuid::parse_str(id) else { continue };
        let id = id.to_string();
        let source = format!("hf://buckets/{bucket}/{prefix}{dir}");
        let project = projects.iter().find(|p| name_segment(&p.name) == folder).unwrap();
        if app.sessions.get(&id).is_some_and(|s| s.imported_from.is_none() || s.project_id.as_deref() != Some(&project.id)) { continue; }
        let result = async {
            let key = source.clone();
            let old: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM trace_import_cursor WHERE source = ?1", [key], |r| r.get(0)).optional()?)).await?;
            let old: Files = old.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default();
            if old == files && app.sessions.get(&id).is_some() { return Ok(()); }
            let text = hf(&app.env, &["buckets", "cp", &format!("{source}/launchpad.json"), "-", "--quiet"], None).await?;
            let session = localize(serde_json::from_str(&text)?, project, &source)?;
            anyhow::ensure!(session.id == id, "trace directory and metadata IDs differ");
            let mut events = Vec::new();
            for (name, hash) in &files {
                if name == "launchpad.json" || old.get(name) == Some(hash) { continue; }
                if app.config.get().trace_backup != config { return Ok(()); }
                let text = hf(&app.env, &["buckets", "cp", &format!("{source}/{name}"), "-", "--quiet"], None).await?;
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    events.push(serde_json::from_str::<SessionEvent>(line)?);
                }
            }
            if app.config.get().trace_backup != config { return Ok(()); }
            app.sessions.import_trace(app, session, events, serde_json::to_string(&files)?).await
        }.await;
        if let Err(e) = result {
            tracing::warn!(%id, "Bucket trace import: {e:#}");
            if first_error.is_none() { first_error = Some(e); }
        }
    }
    if let Some(e) = first_error { return Err(e); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        serde_json::from_value(serde_json::json!({"id":"local-project", "name":"Project", "path":"/local/project", "description":"", "worktree_setup":"", "is_temp":false,"pinned":false,"created_at":1})).unwrap()
    }

    fn metadata(id: &str) -> Metadata {
        serde_json::from_value(serde_json::json!({
            "format":"launchpad", "version":1, "folder":"Project", "workspace":"Foreign workspace",
            "session": {
                "id":id, "harness":"claude", "harness_session_id":"native-foreign", "project_id":"foreign-project",
                "cwd":"/foreign/worktree", "title":"Remote task", "status":"working", "status_since":10,
                "live":true, "open_question":true, "prompt":"original prompt", "usage":{"input_tokens":123,"cached_tokens":0,"output_tokens":0,"cost_usd":0.5},
                "started_at":1, "last_seq":8, "seen_at":0, "capacity_retry_at":500, "capacity_retry_count":3,
                "launch_spec":{"profile":"foreign-login", "workspace_id":"foreign-workspace", "target":{"kind":"project","project_id":"foreign-project"}, "allow_paid_usage":true,"worktree":{"branch":"foreign"}}
            }
        })).unwrap()
    }

    fn event(id: &str, seq: i64) -> SessionEvent {
        serde_json::from_value(serde_json::json!({"session_id":id,"seq":seq,"ts":10,"body":{"kind":"notice","text":format!("event {seq}")}})).unwrap()
    }

    #[test]
    fn localization_keeps_history_and_clears_machine_state() {
        let id = uuid::Uuid::new_v4().to_string();
        let session = localize(metadata(&id), &project(), "hf://buckets/user/traces/Project/run").unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.prompt, "original prompt");
        assert_eq!(session.usage.input_tokens, 123);
        assert_eq!(session.usage.cost_usd, Some(0.5));
        assert_eq!(session.project_id.as_deref(), Some("local-project"));
        assert_eq!(session.cwd, "/local/project");
        assert!(matches!(session.launch_spec.target, Target::Project { .. }));
        assert!(session.launch_spec.profile.is_none() && session.launch_spec.workspace_id.is_none() && session.launch_spec.worktree.is_none());
        assert!(session.harness_session_id.is_none() && session.capacity_retry_at.is_none());
        assert!(!session.live && !session.open_question && !session.needs_you());
        let mut bad = metadata(&id);
        bad.folder = "Another folder".into();
        assert!(localize(bad, &project(), "source").is_err());
    }

    #[tokio::test]
    async fn transaction_retries_gaps_and_persists_incremental_progress() {
        let tmp = crate::harness::trace::tests::Temp::new();
        let db = crate::db::open_state(&tmp.0.join("state.db")).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let s = localize(metadata(&id), &project(), "source").unwrap();
        let copy = s.clone();
        assert!(db.call(move |c| store(c, copy.clone(), vec![event(&copy.id, 2)], "{}".into())).await.is_err());
        db.call(|c| {
            assert_eq!(c.query_row("SELECT count(*) FROM session_event", [], |r| r.get::<_, i64>(0))?, 0);
            assert_eq!(c.query_row("SELECT count(*) FROM trace_import_cursor", [], |r| r.get::<_, i64>(0))?, 0);
            Ok(())
        }).await.unwrap();
        let copy = s.clone();
        assert_eq!(db.call(move |c| store(c, copy.clone(), vec![event(&copy.id, 1), event(&copy.id, 2)], "{\"chunk1\":\"hash\"}".into())).await.unwrap(), 2);
        let copy = s.clone();
        assert_eq!(db.call(move |c| store(c, copy.clone(), vec![event(&copy.id, 2), event(&copy.id, 3)], "{\"chunk2\":\"hash2\"}".into())).await.unwrap(), 3);
        let db = crate::db::open_state(&tmp.0.join("state.db")).unwrap();
        db.call(move |c| {
            let (source, seq): (String, i64) = c.query_row("SELECT imported_from, last_seq FROM session WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!(source, "source");
            assert_eq!(seq, 3);
            assert_eq!(c.query_row("SELECT json FROM trace_import_cursor WHERE source = 'source'", [], |r| r.get::<_, String>(0))?, "{\"chunk2\":\"hash2\"}");
            Ok(())
        }).await.unwrap();
    }
}

//! Home: plain copies of every session's native conversation next to the backups, for browsing the bucket (its
//! trace viewer reads harness files). One folder per session, one file per trace:
//!
//! ```text
//! <workspace>/traces/<folder>/<YYYYMMDD_HHMM>_<session id without hyphens>/
//! ├── <native files>   # from `Harness::native_trace`, unchanged (Claude Code JSONL, Codex rollout, OpenCode export)
//! └── launchpad.json   # the session record, its workspace and folder names
//! ```
//!
//! The native files come from the session's latest handoff (`handoff::store`: runners send one at every turn end
//! when the conversation changed). Only changed files are uploaded; a renamed workspace or folder moves its
//! sessions' copies. Home never reads them back: `state.db` in the backups stays the source.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::App;
use crate::home_backup::Store;
use crate::model::Session;

/// What the bucket holds for one session.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Copied {
    dir: String,
    /// `at` of the handoff the native files came from.
    handoff_at: i64,
    /// File name → SHA-256.
    files: BTreeMap<String, String>,
}

fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// The session's folder in the destination.
fn dir(workspace: &str, folder: &str, started_at: i64, id: &str) -> Result<String> {
    let started = chrono::DateTime::from_timestamp_millis(started_at).context("invalid session start time")?.format("%Y%m%d_%H%M");
    Ok(format!("{workspace}/traces/{}/{started}_{}", crate::hf::name_segment(folder), id.replace('-', "")))
}

/// The record without image bytes or bot instructions (in the bot's own files), with the full prompt.
fn metadata(session: &Session, workspace: &str, folder: &str) -> Result<Vec<u8>> {
    let mut s = session.light();
    s.prompt = session.prompt.clone();
    Ok(serde_json::to_vec_pretty(&serde_json::json!({ "format": "launchpad", "version": 2, "workspace": workspace, "folder": folder, "session": s }))?)
}

/// One pass after a complete backup.
pub(crate) async fn run(app: &App, store: &Store) -> Result<()> {
    let state_path = app.paths.data_dir.join("trace-copies.json");
    let mut done: BTreeMap<String, Copied> = tokio::fs::read(&state_path).await.ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    let handoffs: BTreeMap<String, i64> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT session_id, at FROM session_handoff")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
        .await?;
    let workspaces = crate::workspace::list(app).await?;
    let projects = crate::projects::list(app).await?;
    let mut first_error = None;
    for session in app.sessions.list() {
        let project = session.project_id.as_deref().and_then(|id| projects.iter().find(|p| p.id == id));
        let workspace = project.and_then(|p| p.workspace_id.as_deref()).or(session.launch_spec.workspace_id.as_deref()).and_then(|id| workspaces.iter().find(|w| w.id == id));
        let workspace_name = workspace.map(|w| w.name.as_str()).unwrap_or("No workspace");
        let prefix = workspace.map(|w| crate::home_backup::workspace_prefix(&w.name, &w.id)).unwrap_or_else(|| "No workspace".into());
        let folder = match (project, &session.launch_spec.bot) {
            (Some(p), Some(bot)) if p.is_temp => format!("bot {}", bot.slug),
            (Some(p), None) if p.is_temp => "Temporary folders".into(),
            (Some(p), _) => p.name.clone(),
            (None, _) if session.project_id.is_some() => "Removed folder".into(),
            (None, _) => "No folder".into(),
        };
        let result: Result<()> = async {
            let dir = dir(&prefix, &folder, session.started_at, &session.id)?;
            let meta = metadata(&session, workspace_name, &folder)?;
            let handoff_at = handoffs.get(&session.id).copied().unwrap_or(0);
            let prev = done.get(&session.id).cloned().unwrap_or_default();
            let moved = prev.dir != dir;
            let mut want: BTreeMap<String, Vec<u8>> = BTreeMap::new();
            let native_unchanged = !moved && prev.handoff_at == handoff_at;
            if !native_unchanged && let Some(h) = crate::handoff::latest(app, &session.id).await? {
                for f in h.files {
                    anyhow::ensure!(f.name.split('/').all(|c| !c.is_empty() && c != "." && c != ".."), "bad trace file name {:?}", f.name);
                    want.insert(f.name, base64::engine::general_purpose::STANDARD.decode(f.data)?);
                }
            }
            let mut next = Copied { dir: dir.clone(), handoff_at: prev.handoff_at, files: BTreeMap::new() };
            if native_unchanged {
                next.files = prev.files.iter().filter(|(n, _)| n.as_str() != "launchpad.json").map(|(n, h)| (n.clone(), h.clone())).collect();
            }
            want.insert("launchpad.json".into(), meta);
            // Record progress as it goes, so a failed upload is retried next pass and the rest is not repeated.
            let outcome: Result<()> = async {
                for (name, data) in want {
                    let hash = sha(&data);
                    if moved || prev.files.get(&name) != Some(&hash) {
                        store.put(&app.env, &format!("{dir}/{name}"), data).await?;
                    }
                    next.files.insert(name, hash);
                }
                // Files the conversation no longer has, or the whole old folder after a rename.
                for name in prev.files.keys().filter(|n| moved || !next.files.contains_key(*n)) {
                    store.remove(&app.env, &format!("{}/{name}", prev.dir)).await?;
                }
                next.handoff_at = handoff_at;
                Ok(())
            }
            .await;
            // `files` lists what the bucket holds: on failure, also what is still there from before.
            if outcome.is_err() {
                if moved {
                    next = prev;
                } else {
                    for (n, h) in prev.files {
                        next.files.entry(n).or_insert(h);
                    }
                }
            }
            done.insert(session.id.clone(), next);
            outcome
        }
        .await;
        if let Err(e) = result {
            tracing::debug!(session = %session.id, "trace copy: {e:#}");
            first_error.get_or_insert(e);
        }
    }
    crate::paths::write_private(&state_path, &serde_json::to_string(&done)?)?;
    match first_error {
        Some(e) => Err(e.context("some sessions' traces were not copied; retrying after the next backup")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_folders_sort_by_start_time_under_their_workspace() {
        let start = 1_793_363_696_789; // 2026-10-30T12:34:56.789Z
        let uuid = "a25e3b16-6d72-478d-9e25-b281f76d8825";
        assert_eq!(dir("Work", "launchpad", start, uuid).unwrap(), "Work/traces/launchpad/20261030_1234_a25e3b166d72478d9e25b281f76d8825");
        assert!(dir("Work", "Folder/Name", start, "z").unwrap().contains("/Folder%2FName/"));
        assert!(dir("Work", "f", start, "z").unwrap() < dir("Work", "f", start + 60_000, "a").unwrap());
        assert!(dir("Work", "f", i64::MAX, "id").is_err());
    }
}

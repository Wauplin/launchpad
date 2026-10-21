//! The unsent composer content (`launch_draft` table), kept across windows and restarts. Each workspace
//! has its own draft, so two windows on different workspaces do not overwrite each other.
//!
//! Several devices can edit the same draft: each saved draft carries a `revision`, a save names the
//! revision it was based on, and a stale save is rejected (the UI then offers to load the newer draft or
//! keep its own). Every save is broadcast as `drafts/changed`, so idle windows reload the new version.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::App;

fn key(workspace: Option<&str>) -> String {
    workspace.map_or_else(|| "current".into(), |w| format!("workspace:{w}"))
}

fn revision(draft: &Value) -> u64 {
    draft["revision"].as_u64().unwrap_or(0)
}

/// Drafts saved before harness variants were renamed profiles keep a `variant` key. Sent back with the
/// `profile` the composer sets, it is a duplicate field of `LaunchSpec`, which rejects the launch.
fn upgrade(draft: &mut Value) {
    if let Some(spec) = draft.get_mut("spec").and_then(Value::as_object_mut)
        && let Some(old) = spec.remove("variant")
        && spec.get("profile").is_none_or(Value::is_null)
    {
        spec.insert("profile".into(), old);
    }
}

/// The stored draft (with its `revision`), or null.
pub async fn get(app: &App, workspace: Option<&str>) -> Result<Value> {
    let key = key(workspace);
    let spec: Option<String> = app
        .state_db
        .call(move |c| Ok(c.query_row("SELECT spec_json FROM launch_draft WHERE id = ?1", [key], |r| r.get(0)).optional()?))
        .await?;
    let mut draft = spec.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or(Value::Null);
    upgrade(&mut draft);
    Ok(draft)
}

/// The result of a save: the new revision, or the newer stored revision the save did not start from.
pub enum Put {
    Saved(u64),
    Conflict(u64),
}

/// Saves `draft` if the stored revision is still `base_revision` (0 when none was stored), or always with
/// `force`. `client` identifies the saving window, so it can ignore its own broadcast.
pub async fn put(app: &App, workspace: Option<&str>, mut draft: Value, base_revision: u64, force: bool, client: Option<String>) -> Result<Put> {
    let db_key = key(workspace);
    if let Some(o) = draft.as_object_mut() {
        o.remove("base_revision");
        o.remove("force");
        o.remove("client");
    }
    let result = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let stored: Option<String> =
                tx.query_row("SELECT spec_json FROM launch_draft WHERE id = ?1", [&db_key], |r| r.get(0)).optional()?;
            let current = stored.and_then(|j| serde_json::from_str::<Value>(&j).ok()).map_or(0, |v| revision(&v));
            if !force && current != base_revision {
                return Ok(Put::Conflict(current));
            }
            let next = current + 1;
            if let Some(o) = draft.as_object_mut() {
                o.insert("revision".into(), json!(next));
            }
            tx.execute(
                "INSERT INTO launch_draft (id, spec_json, updated_at) VALUES (?3, ?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET spec_json = ?1, updated_at = ?2",
                rusqlite::params![draft.to_string(), crate::util::now_ms(), db_key],
            )?;
            tx.commit()?;
            Ok(Put::Saved(next))
        })
        .await?;
    if let Put::Saved(revision) = result {
        app.bus.publish("drafts", "changed", json!({ "workspace": workspace, "revision": revision, "client": client }));
    }
    Ok(result)
}

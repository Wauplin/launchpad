//! Review progress shared by every browser and device: PR file viewed markers, unsent comment drafts,
//! pre-draft note drafts and suggestion edits. One generic key/value table with namespaced keys
//! (`viewed:`, `reply:`, `predraft:`, `suggestion:`); the UI owns the value shapes. Pure UI state
//! (layout, collapsed groups, selection) stays in each browser's local storage.

use std::collections::HashMap;

use anyhow::{Result, bail};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::{Value, json};

use crate::App;
use crate::util::now_ms;

const MAX_KEY: usize = 512;
const MAX_VALUE: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    pub updated_at: i64,
}

fn check(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_KEY || !key.contains(':') {
        bail!("review state keys look like <namespace>:<id>");
    }
    Ok(())
}

pub async fn get(app: &App, key: &str) -> Result<Option<Entry>> {
    check(key)?;
    let key = key.to_string();
    app.state_db
        .call(move |c| {
            let row = c
                .query_row("SELECT json, updated_at FROM review_state WHERE key = ?1", [&key], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .optional()?;
            Ok(row.map(|(json, updated_at)| Entry { value: serde_json::from_str(&json).unwrap_or(Value::Null), key, updated_at }))
        })
        .await
}

/// Stores `value` (null clears the value while retaining its revision) and tells every window. `origin` identifies the writing
/// window, so it can ignore its own echo.
pub async fn put(app: &App, key: &str, value: Value, origin: Option<String>, base_revision: i64) -> Result<Option<i64>> {
    check(key)?;
    let json = serde_json::to_string(&value)?;
    if json.len() > MAX_VALUE {
        bail!("review state value is too large");
    }
    let k = key.to_string();
    let revision = app.state_db.call(move |c| save(c, &k, &json, base_revision)).await?;
    if let Some(revision) = revision {
        app.bus.publish("review-state", "changed", json!({ "key": key, "updated_at": revision, "origin": origin }));
    }
    Ok(revision)
}

// Keep null tombstones: deleting and recreating a draft must not reuse revision zero.
fn save(c: &mut rusqlite::Connection, key: &str, json: &str, base_revision: i64) -> Result<Option<i64>> {
    let tx = c.transaction()?;
    let current: i64 = tx.query_row("SELECT updated_at FROM review_state WHERE key = ?1", [key], |r| r.get(0)).optional()?.unwrap_or(0);
    if current != base_revision { return Ok(None); }
    let next = now_ms().max(current + 1);
    tx.execute(
        "INSERT INTO review_state (key, json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
        rusqlite::params![key, json, next],
    )?;
    tx.commit()?;
    Ok(Some(next))
}

/// One-time import of values a browser kept locally. Existing daemon values win.
pub async fn import(app: &App, entries: HashMap<String, Value>) -> Result<usize> {
    for key in entries.keys() {
        check(key)?;
    }
    let now = now_ms();
    let n = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let mut n = 0;
            for (key, value) in entries {
                if value.is_null() {
                    continue;
                }
                n += tx.execute(
                    "INSERT OR IGNORE INTO review_state (key, json, updated_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![key, serde_json::to_string(&value)?, now],
                )?;
            }
            tx.commit()?;
            Ok(n)
        })
        .await?;
    if n > 0 {
        app.bus.publish("review-state", "changed", json!({ "key": null, "updated_at": now, "origin": null }));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisions_reject_stale_writes_and_survive_deletion() {
        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE review_state (key TEXT PRIMARY KEY, json TEXT NOT NULL, updated_at INTEGER NOT NULL)").unwrap();
        let first = save(&mut c, "reply:1", "\"one\"", 0).unwrap().unwrap();
        assert_eq!(save(&mut c, "reply:1", "\"stale\"", 0).unwrap(), None);
        let deleted = save(&mut c, "reply:1", "null", first).unwrap().unwrap();
        assert!(deleted > first);
        assert_eq!(save(&mut c, "reply:1", "\"stale\"", first).unwrap(), None);
        assert_eq!(save(&mut c, "reply:1", "\"new\"", 0).unwrap(), None);
        assert!(save(&mut c, "reply:1", "\"new\"", deleted).unwrap().unwrap() > deleted);
    }
}

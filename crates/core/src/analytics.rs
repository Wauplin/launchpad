//! Small, historical session metadata for the activity chart; no prompts or attachments.
use anyhow::Result;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
pub struct AgentRun {
    #[ts(type = "number")]
    pub started_at: i64,
    pub harness: String,
    pub variant: Option<String>,
}

/// Count each saved session once, including archived sessions. Group into local days in the UI.
pub async fn runs(db: &crate::db::Db, from: i64, until: i64) -> Result<Vec<AgentRun>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT started_at, harness, json_extract(launch_spec_json, '$.variant') FROM session WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at, id")?;
        Ok(stmt.query_map(rusqlite::params![from, until], |row| Ok(AgentRun {
            started_at: row.get(0)?, harness: row.get(1)?, variant: row.get(2)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn includes_archived_and_old_specs_with_exclusive_end() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("launchpad-analytics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        let db = crate::db::open_state(&dir.join("state.db"))?;
        db.call(|conn| {
            for (id, harness, spec, status, at) in [
                ("old", "codex", "{}", "done", 10),
                ("archived", "codex", r#"{"variant":"fast"}"#, "closed", 20),
                ("other", "claude", r#"{"variant":"fast"}"#, "running", 20),
                ("end", "opencode", "{}", "done", 30),
            ] {
                conn.execute("INSERT INTO session (id, harness, cwd, title, status, launch_spec_json, prompt, started_at) VALUES (?1, ?2, '.', '', ?3, ?4, '', ?5)", rusqlite::params![id, harness, status, spec, at])?;
            }
            Ok(())
        }).await?;
        let rows = runs(&db, 10, 30).await?;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].started_at, 10);
        assert!(rows[0].variant.is_none());
        assert_eq!(rows[1].variant.as_deref(), Some("fast"));
        assert_eq!(rows[1].harness, "codex");
        assert_eq!(rows[2].harness, "claude");
        assert!(runs(&db, 21, 30).await?.is_empty());
        drop(db);
        // The database actor may still be closing its file on Windows.
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}

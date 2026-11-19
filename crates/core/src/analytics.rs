//! Small, historical session metadata for the activity chart; no prompts or attachments.
use anyhow::Result;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
pub struct AgentRun {
    #[ts(type = "number")]
    pub started_at: i64,
    pub harness: String,
    pub profile: Option<String>,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    /// The session's estimated USD cost as its harness reported it (also on subscriptions); None when the
    /// harness reports none (Codex), its pricing is unknown, or the cost is zero, so charts leave it out.
    pub cost_usd: Option<f64>,
}

/// A reported cost that counts toward usage value: positive, finite and with known pricing.
fn known_cost(cost: Option<f64>, unavailable: bool) -> Option<f64> {
    cost.filter(|c| !unavailable && c.is_finite() && *c > 0.0)
}

/// Count each agent session once, including archives. Group into local days in the UI.
pub async fn runs(db: &crate::db::Db, from: i64, until: i64) -> Result<Vec<AgentRun>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT started_at, harness, json_extract(launch_spec_json, '$.profile'), usage_json, cost_usd FROM session WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at, id")?;
        Ok(stmt.query_map(rusqlite::params![from, until], |row| {
            let stored: Option<String> = row.get(3)?;
            let usage = stored.and_then(|s| serde_json::from_str::<crate::model::Usage>(&s).ok()).unwrap_or_default();
            Ok(AgentRun {
                started_at: row.get(0)?, harness: row.get(1)?, profile: row.get(2)?,
                input_tokens: usage.input_tokens, cached_tokens: usage.cached_tokens, output_tokens: usage.output_tokens,
                cost_usd: known_cost(row.get::<_, Option<f64>>(4)?.or(usage.cost_usd), usage.cost_unavailable),
            })
        })?.collect::<rusqlite::Result<Vec<_>>>()?)
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
                ("archived", "codex", r#"{"profile":"fast"}"#, "closed", 20),
                ("other", "claude", r#"{"profile":"fast"}"#, "running", 20),
                ("end", "opencode", "{}", "done", 30),
            ] {
                conn.execute("INSERT INTO session (id, harness, cwd, title, status, launch_spec_json, prompt, started_at) VALUES (?1, ?2, '.', '', ?3, ?4, '', ?5)", rusqlite::params![id, harness, status, spec, at])?;
            }
            let usage = crate::model::Usage { input_tokens: 120, cached_tokens: 80, output_tokens: 30, ..Default::default() };
            conn.execute("UPDATE session SET usage_json = ?1 WHERE id = 'archived'", [serde_json::to_string(&usage)?])?;
            conn.execute("UPDATE session SET usage_json = 'invalid' WHERE id = 'other'", [])?;
            Ok(())
        }).await?;
        let rows = runs(&db, 10, 30).await?;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].started_at, 10);
        assert!(rows[0].profile.is_none());
        assert_eq!(rows[0].input_tokens + rows[0].cached_tokens + rows[0].output_tokens, 0);
        assert_eq!((rows[1].input_tokens, rows[1].cached_tokens, rows[1].output_tokens), (120, 80, 30));
        assert_eq!(rows[2].input_tokens + rows[2].cached_tokens + rows[2].output_tokens, 0);
        assert_eq!(rows[1].profile.as_deref(), Some("fast"));
        assert_eq!(rows[1].harness, "codex");
        assert_eq!(rows[2].harness, "claude");
        assert!(runs(&db, 21, 30).await?.is_empty());
        assert!(rows.iter().all(|r| r.cost_usd.is_none()));
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn known_cost_keeps_only_positive_priced_costs() {
        assert_eq!(known_cost(Some(1.25), false), Some(1.25));
        assert_eq!(known_cost(None, false), None);
        assert_eq!(known_cost(Some(0.0), false), None);
        assert_eq!(known_cost(Some(-0.5), false), None);
        assert_eq!(known_cost(Some(f64::NAN), false), None);
        assert_eq!(known_cost(Some(2.0), true), None);
    }

    #[tokio::test]
    async fn reports_recorded_costs_and_skips_unknown_ones() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("launchpad-analytics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        let db = crate::db::open_state(&dir.join("state.db"))?;
        db.call(|conn| {
            for (id, harness, cost, unavailable) in [
                ("sub", "claude", Some(3.5), false),
                ("codex", "codex", None, false),
                ("zero", "claude", Some(0.0), false),
                ("unpriced", "claude", Some(1.0), true),
            ] {
                let usage = crate::model::Usage { input_tokens: 10, cost_usd: cost, cost_unavailable: unavailable, ..Default::default() };
                conn.execute("INSERT INTO session (id, harness, cwd, title, status, launch_spec_json, prompt, started_at, cost_usd, usage_json) VALUES (?1, ?2, '.', '', 'done', '{}', '', 5, ?3, ?4)",
                    rusqlite::params![id, harness, cost, serde_json::to_string(&usage)?])?;
            }
            Ok(())
        }).await?;
        let rows = runs(&db, 0, 10).await?;
        let costs: Vec<_> = rows.iter().map(|r| (r.harness.as_str(), r.cost_usd)).collect();
        assert!(costs.contains(&("claude", Some(3.5))));
        assert!(costs.contains(&("codex", None)));
        assert_eq!(rows.iter().filter_map(|r| r.cost_usd).sum::<f64>(), 3.5);
        drop(db);
        // The database actor may still be closing its file on Windows.
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}

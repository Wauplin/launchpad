//! Observed weekly quota resets. Predicted timestamps never create historical resets.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use ts_rs::TS;

use crate::status::QuotaWindow;

#[derive(Debug, Serialize, TS)]
pub struct WeeklyReset {
    pub harness: String,
    /// Detection time, not a provider-supplied reset timestamp (Unix milliseconds).
    #[ts(type = "number")]
    pub observed_at: i64,
    /// The reset occurred somewhere after this last reading (Unix milliseconds).
    #[ts(type = "number")]
    pub previous_at: i64,
    pub previous_percent: f64,
    pub used_percent: f64,
}

/// Provider readings move by a point or two from rounding and usage corrections. A weekly reset
/// drops usage by a wide margin, or ends a used window at (near) zero.
const RESET_MIN_DROP: f64 = 10.0;
const RESET_NEAR_ZERO: f64 = 1.0;
const RESET_MIN_PREVIOUS: f64 = 5.0;

fn is_weekly_reset(previous_percent: f64, used_percent: f64) -> bool {
    previous_percent - used_percent >= RESET_MIN_DROP
        || (used_percent <= RESET_NEAR_ZERO && previous_percent >= RESET_MIN_PREVIOUS)
}

pub(crate) fn record(conn: &mut Connection, harness: &str, at: i64, week: &QuotaWindow) -> Result<()> {
    if !week.used_percent.is_finite() || !(0.0..=100.0).contains(&week.used_percent) {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let previous: Option<(i64, f64)> = tx.query_row(
        "SELECT observed_at, used_percent FROM quota_weekly_reading WHERE harness = ?1",
        [harness], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    if let Some((previous_at, used)) = previous {
        if at <= previous_at { return Ok(()); }
        if is_weekly_reset(used, week.used_percent) {
            tx.execute("INSERT INTO quota_weekly_reset (harness, observed_at, previous_at, previous_percent, used_percent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![harness, at, previous_at, used, week.used_percent])?;
        }
    }
    tx.execute("INSERT INTO quota_weekly_reading (harness, observed_at, used_percent, resets_at) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(harness) DO UPDATE SET observed_at = excluded.observed_at, used_percent = excluded.used_percent, resets_at = excluded.resets_at",
        params![harness, at, week.used_percent, week.resets_at])?;
    tx.commit()?;
    Ok(())
}

/// One calendar month in the viewer's timezone; callers bound the interval.
pub async fn list(db: &crate::db::Db, from: i64, until: i64) -> Result<Vec<WeeklyReset>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT harness, observed_at, previous_at, previous_percent, used_percent FROM quota_weekly_reset WHERE observed_at >= ?1 AND observed_at < ?2 ORDER BY observed_at")?;
        Ok(stmt.query_map(params![from, until], |row| Ok(WeeklyReset {
            harness: row.get(0)?, observed_at: row.get(1)?, previous_at: row.get(2)?,
            previous_percent: row.get(3)?, used_percent: row.get(4)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_resets_not_noise_or_predictions_and_survives_reopen() -> Result<()> {
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let dir = Scratch(std::env::temp_dir().join(format!("launchpad-quota-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0)?;
        let path = dir.0.join("state.db");
        let db = crate::db::open_state(&path)?;
        // Predictions alone never record, and one-point decreases are reading noise.
        for (at, used, reset) in [(1, 60.0, Some(100)), (2, 65.0, Some(90)), (3, 65.0, Some(200)), (4, 70.0, Some(200)), (5, 69.0, Some(200)), (6, 68.0, Some(200))] {
            db.call(move |c| record(c, "codex", at, &QuotaWindow { used_percent: used, resets_at: reset })).await?;
        }
        assert!(list(&db, 0, 10).await?.is_empty());
        drop(db);
        let db = crate::db::open_state(&path)?;
        for (h, at, used, reset) in [("codex", 7, 2.0, Some(200)), ("codex", 8, 2.0, Some(200)), ("claude", 9, 50.0, None), ("claude", 10, 49.0, None), ("claude", 11, 0.0, None)] {
            db.call(move |c| record(c, h, at, &QuotaWindow { used_percent: used, resets_at: reset })).await?;
        }
        let events = list(&db, 0, 20).await?;
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].previous_at, events[0].observed_at), (6, 7));
        assert_eq!(events[1].harness, "claude");
        assert_eq!((events[1].previous_at, events[1].observed_at), (10, 11));
        assert_eq!(list(&db, 5, 8).await?.len(), 1);
        // Ignore out-of-order and malformed readings instead of moving the baseline back.
        db.call(|c| {
            record(c, "codex", 4, &QuotaWindow { used_percent: 80.0, resets_at: None })?;
            record(c, "codex", 12, &QuotaWindow { used_percent: f64::NAN, resets_at: None })?;
            record(c, "codex", 13, &QuotaWindow { used_percent: 3.0, resets_at: None })
        }).await?;
        assert_eq!(list(&db, 0, 20).await?.len(), 2);
        Ok(())
    }
}

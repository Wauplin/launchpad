//! Routine record reads and serialized writes.
use super::*;

pub async fn list(app: &App) -> Result<Vec<CronAgent>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT id, json FROM cron_agent ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(crate::decode_rows("routine", rows, |s| Ok(serde_json::from_str(s)?)))
        })
        .await
        .map(|mut records: Vec<CronAgent>| {
            let held = HELD.lock().unwrap();
            for r in &mut records {
                r.waiting = held.get(&r.id).filter(|(due, _)| r.next_run_at == Some(*due)).map(|(_, why)| why.clone());
            }
            records
        })
}

pub async fn get(app: &App, id: &str) -> Result<CronAgent> {
    list(app).await?.into_iter().find(|r| r.id == id).context("this routine no longer exists")
}

/// Read-modify-write in the DB actor, which serializes the scheduler and UI requests.
pub(crate) async fn modify<T: Send + 'static>(
    app: &App,
    id: &str,
    f: impl FnOnce(&mut CronAgent) -> Result<T> + Send + 'static,
) -> Result<(CronAgent, T)> {
    let id = id.to_string();
    let out = app
        .state_db
        .call(move |c| {
            let json: String = c
                .query_row("SELECT json FROM cron_agent WHERE id = ?1", [&id], |r| r.get(0))
                .context("this routine no longer exists")?;
            let mut record: CronAgent = serde_json::from_str(&json)?;
            let out = f(&mut record)?;
            c.execute(
                "UPDATE cron_agent SET json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&record)?],
            )?;
            Ok((record, out))
        })
        .await?;
    changed(app);
    Ok(out)
}

pub(super) async fn insert(app: &App, record: &CronAgent) -> Result<()> {
    let r = record.clone();
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO cron_agent (id, json) VALUES (?1, ?2)", params![r.id, serde_json::to_string(&r)?])?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}

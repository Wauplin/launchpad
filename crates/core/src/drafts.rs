//! The unsent composer content (`launch_draft` table), kept across windows and restarts.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::App;

pub async fn get(app: &App) -> Result<Value> {
    let spec: Option<String> = app
        .state_db
        .call(|c| Ok(c.query_row("SELECT spec_json FROM launch_draft WHERE id = 'current'", [], |r| r.get(0)).optional()?))
        .await?;
    Ok(spec.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or(Value::Null))
}

pub async fn put(app: &App, draft: &Value) -> Result<()> {
    let text = draft.to_string();
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO launch_draft (id, spec_json, updated_at) VALUES ('current', ?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET spec_json = ?1, updated_at = ?2",
                rusqlite::params![text, crate::util::now_ms()],
            )?;
            Ok(())
        })
        .await
}

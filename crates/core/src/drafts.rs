//! The unsent composer content (`launch_draft` table), kept across windows and restarts. Each workspace
//! has its own draft, so two windows on different workspaces do not overwrite each other.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::App;

fn key(workspace: Option<&str>) -> String {
    workspace.map_or_else(|| "current".into(), |w| format!("workspace:{w}"))
}

pub async fn get(app: &App, workspace: Option<&str>) -> Result<Value> {
    let key = key(workspace);
    let spec: Option<String> = app
        .state_db
        .call(move |c| Ok(c.query_row("SELECT spec_json FROM launch_draft WHERE id = ?1", [key], |r| r.get(0)).optional()?))
        .await?;
    Ok(spec.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or(Value::Null))
}

pub async fn put(app: &App, workspace: Option<&str>, draft: &Value) -> Result<()> {
    let (key, text) = (key(workspace), draft.to_string());
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO launch_draft (id, spec_json, updated_at) VALUES (?3, ?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET spec_json = ?1, updated_at = ?2",
                rusqlite::params![text, crate::util::now_ms(), key],
            )?;
            Ok(())
        })
        .await
}

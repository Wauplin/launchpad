//! Workspace collections of sessions. Owned by Home, never overwritten by runner replication.
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::App;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Topic {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub session_ids: Vec<String>,
}

pub async fn list(app: &App, workspace: &str) -> Result<Vec<Topic>> {
    let workspace = workspace.to_owned();
    app.state_db.call(move |c| {
        let mut q = c.prepare("SELECT id, name FROM topic WHERE workspace_id = ?1 ORDER BY name COLLATE NOCASE")?;
        let rows = q.query_map([&workspace], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(id, name)| {
            let mut q = c.prepare("SELECT session_id FROM topic_session WHERE topic_id = ?1 ORDER BY session_id")?;
            let session_ids = q.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(Topic { id, workspace_id: workspace.clone(), name, session_ids })
        }).collect()
    }).await
}
pub async fn get(app: &App, id: &str) -> Result<Topic> {
    let key = id.to_owned();
    let workspace: String = app.state_db.call(move |c| Ok(c.query_row("SELECT workspace_id FROM topic WHERE id = ?1", [key], |r| r.get(0))?)).await?;
    list(app, &workspace).await?.into_iter().find(|t| t.id == id).context("Topic no longer exists")
}
fn name(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty() && value.chars().count() <= 80 && !value.chars().any(char::is_control), "Use a topic name of 1–80 characters");
    Ok(value.to_owned())
}
fn changed(app: &App) { app.bus.publish("topics", "changed", serde_json::json!({})); }
pub async fn create(app: &App, workspace: &str, value: &str) -> Result<Topic> {
    crate::workspace::get(app, workspace).await?;
    let topic = Topic { id: uuid::Uuid::new_v4().to_string(), workspace_id: workspace.into(), name: name(value)?, session_ids: vec![] };
    let t = topic.clone();
    app.state_db.call(move |c| { c.execute("INSERT INTO topic (id, workspace_id, name) VALUES (?1, ?2, ?3)", params![t.id, t.workspace_id, t.name])?; Ok(()) }).await?;
    changed(app); Ok(topic)
}
pub async fn rename(app: &App, id: &str, value: &str) -> Result<Topic> {
    let t = get(app, id).await?; let id = t.id; let value = name(value)?; let key = id.clone();
    app.state_db.call(move |c| { c.execute("UPDATE topic SET name = ?2 WHERE id = ?1", params![key, value])?; Ok(()) }).await?;
    changed(app); get(app, &id).await
}
pub async fn delete(app: &App, id: &str) -> Result<()> {
    let id = get(app, id).await?.id;
    app.state_db.call(move |c| { let tx = c.transaction()?; tx.execute("DELETE FROM topic_session WHERE topic_id = ?1", [&id])?; tx.execute("DELETE FROM topic WHERE id = ?1", [&id])?; tx.commit()?; Ok(()) }).await?;
    changed(app); Ok(())
}
/// Add/remove only the named memberships, atomically. Other topics and sessions are untouched.
pub async fn membership(app: &App, id: &str, sessions: &[String], present: bool) -> Result<()> {
    ensure!(!sessions.is_empty() && sessions.len() <= 1000, "Choose between 1 and 1000 sessions");
    let topic = get(app, id).await?;
    for session in sessions { ensure!(crate::scope::of_session(app, session).await.as_deref() == Some(&topic.workspace_id), "Session does not belong to the topic's workspace"); }
    let sessions = sessions.to_vec();
    app.state_db.call(move |c| { let tx = c.transaction()?; for session in sessions { if present { tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) VALUES (?1, ?2)", params![topic.id, session])?; } else { tx.execute("DELETE FROM topic_session WHERE topic_id = ?1 AND session_id = ?2", params![topic.id, session])?; } } tx.commit()?; Ok(()) }).await?;
    changed(app); Ok(())
}

#[cfg(test)]
mod tests {
    #[test] fn names_are_short_and_visible() {
        assert_eq!(super::name("  roadmap  ").unwrap(), "roadmap");
        for value in ["", "  ", "a\nb", &"x".repeat(81)] { assert!(super::name(value).is_err()); }
    }
}

//! Workspace collections of sessions. Owned by Home, never overwritten by runner replication.
use crate::App;
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
    let workspace: String = app
        .state_db
        .call(move |c| {
            Ok(
                c.query_row("SELECT workspace_id FROM topic WHERE id = ?1", [key], |r| {
                    r.get(0)
                })?,
            )
        })
        .await?;
    list(app, &workspace)
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .context("Topic no longer exists")
}
fn name(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= 80 && !value.chars().any(char::is_control),
        "Use a topic name of 1–80 characters"
    );
    Ok(value.to_owned())
}
fn changed(app: &App) {
    app.bus.publish("topics", "changed", serde_json::json!({}));
}
pub async fn create(app: &App, workspace: &str, value: &str) -> Result<Topic> {
    crate::workspace::get(app, workspace).await?;
    let topic = Topic {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: workspace.into(),
        name: name(value)?,
        session_ids: vec![],
    };
    let t = topic.clone();
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO topic (id, workspace_id, name) VALUES (?1, ?2, ?3)",
                params![t.id, t.workspace_id, t.name],
            )?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(topic)
}
/// Attach an item topic within the session's workspace. The unique name makes concurrent links idempotent.
pub async fn item(app: &App, session: &str, repo: &str, number: u64) -> Result<()> {
    let workspace = crate::scope::of_session(app, session)
        .await
        .context("Session has no workspace")?;
    crate::workspace::get(app, &workspace).await?;
    let value = name(&format!(
        "{}#{number}",
        repo.rsplit('/').next().unwrap_or(repo)
    ))?;
    let session = session.to_owned();
    app.state_db
        .call(move |c| attach_item(c, &workspace, &value, &session))
        .await?;
    changed(app);
    Ok(())
}
fn attach_item(
    c: &mut rusqlite::Connection,
    workspace: &str,
    value: &str,
    session: &str,
) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO topic (id, workspace_id, name) VALUES (?1, ?2, ?3)",
        params![uuid::Uuid::new_v4().to_string(), workspace, value],
    )?;
    tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) SELECT id, ?3 FROM topic WHERE workspace_id = ?1 AND name = ?2", params![workspace, value, session])?;
    tx.commit()?;
    Ok(())
}
pub async fn rename(app: &App, id: &str, value: &str) -> Result<Topic> {
    let t = get(app, id).await?;
    let id = t.id;
    let value = name(value)?;
    let key = id.clone();
    app.state_db
        .call(move |c| {
            c.execute(
                "UPDATE topic SET name = ?2 WHERE id = ?1",
                params![key, value],
            )?;
            Ok(())
        })
        .await?;
    changed(app);
    get(app, &id).await
}
pub async fn delete(app: &App, id: &str) -> Result<()> {
    let id = get(app, id).await?.id;
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM topic_session WHERE topic_id = ?1", [&id])?;
            tx.execute("DELETE FROM topic WHERE id = ?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    changed(app);
    Ok(())
}
/// Add/remove only the named memberships, atomically. Other topics and sessions are untouched.
pub async fn membership(app: &App, id: &str, sessions: &[String], present: bool) -> Result<()> {
    ensure!(
        !sessions.is_empty() && sessions.len() <= 1000,
        "Choose between 1 and 1000 sessions"
    );
    let topic = get(app, id).await?;
    for session in sessions {
        ensure!(
            crate::scope::of_session(app, session).await.as_deref() == Some(&topic.workspace_id),
            "Session does not belong to the topic's workspace"
        );
    }
    let sessions = sessions.to_vec();
    app.state_db.call(move |c| { let tx = c.transaction()?;
        ensure!(tx.query_row("SELECT COUNT(*) FROM topic WHERE id = ?1", [&topic.id], |r| r.get::<_, i64>(0))? == 1, "Topic no longer exists");
        for session in sessions { if present { tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) VALUES (?1, ?2)", params![topic.id, session])?; } else { tx.execute("DELETE FROM topic_session WHERE topic_id = ?1 AND session_id = ?2", params![topic.id, session])?; } } tx.commit()?; Ok(()) }).await?;
    changed(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn item_topics_are_idempotent_and_workspace_scoped() {
        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE topic (id TEXT PRIMARY KEY, workspace_id TEXT, name TEXT COLLATE NOCASE, UNIQUE(workspace_id, name)); CREATE TABLE topic_session (topic_id TEXT, session_id TEXT, PRIMARY KEY(topic_id, session_id));").unwrap();
        for (w, n, s) in [
            ("w1", "repo#12", "s1"),
            ("w1", "Repo#12", "s1"),
            ("w1", "repo#12", "s2"),
            ("w2", "repo#12", "s3"),
        ] {
            super::attach_item(&mut c, w, n, s).unwrap();
        }
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM topic", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM topic_session", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
    #[test]
    fn names_are_short_and_visible() {
        assert_eq!(super::name("  roadmap  ").unwrap(), "roadmap");
        for value in ["", "  ", "a\nb", &"x".repeat(81)] {
            assert!(super::name(value).is_err());
        }
    }
}

/// A live topic selection becomes a fixed source index on Home at dispatch.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TopicContext {
    pub id: String,
    pub name: String,
    #[ts(optional)]
    pub snapshot: Option<Vec<crate::model::AgentTraceRef>>,
    #[serde(default)]
    #[ts(optional, as = "Option<Vec<String>>")]
    pub notes: Vec<String>,
}

/// Resolve once, without following any context attached to the sources themselves.
pub async fn resolve(
    app: &App,
    workspace: &str,
    topics: &mut [TopicContext],
    traces: &mut Vec<crate::model::AgentTraceRef>,
) -> Result<Vec<String>> {
    let original = traces.clone();
    let mut inherited = Vec::new();
    for selected in topics {
        if selected.snapshot.is_none() {
            let topic = get(app, &selected.id).await?;
            ensure!(
                topic.workspace_id == workspace,
                "Topic belongs to another workspace"
            );
            selected.name = topic.name;
            let mut snapshot = Vec::new();
            selected.notes.clear();
            for id in topic.session_ids {
                if crate::scope::of_session(app, &id).await.as_deref() != Some(workspace) {
                    selected
                        .notes
                        .push("Some history is unavailable in this workspace".into());
                    continue;
                }
                if let Some(session) = app.sessions.get(&id).filter(|s| s.last_seq > 0) {
                    if matches!(
                        session.status,
                        crate::model::SessionStatus::Starting
                            | crate::model::SessionStatus::Working
                            | crate::model::SessionStatus::Background
                            | crate::model::SessionStatus::Stopped
                            | crate::model::SessionStatus::Interrupted
                            | crate::model::SessionStatus::QuotaPaused
                            | crate::model::SessionStatus::InTerminal
                            | crate::model::SessionStatus::Error
                    ) {
                        selected.notes.push(format!(
                            "{} is unfinished; only stored events are included",
                            session.title
                        ));
                    }
                    snapshot.push(crate::model::AgentTraceRef {
                        session_id: id,
                        through_seq: session.last_seq,
                    });
                } else {
                    selected
                        .notes
                        .push(format!("Session {id} has no stored history available"));
                }
            }
            selected.snapshot = Some(snapshot);
        }
        // Deleted collections do not invalidate their already frozen source references.
        if get(app, &selected.id)
            .await
            .is_ok_and(|t| t.workspace_id == workspace)
        {
            inherited.push(selected.id.clone());
        }
        traces.extend(selected.snapshot.clone().unwrap_or_default());
    }
    *traces = dedup(traces);
    for trace in traces.iter() {
        ensure!(
            crate::scope::of_session(app, &trace.session_id)
                .await
                .as_deref()
                == Some(workspace),
            "Context agent belongs to another workspace or is unavailable"
        );
        let source = app
            .sessions
            .get(&trace.session_id)
            .context("Context agent unavailable")?;
        ensure!(
            trace.through_seq > 0 && trace.through_seq <= source.last_seq,
            "Context trace unavailable"
        );
    }
    for topic in list(app, workspace).await? {
        if original
            .iter()
            .any(|r| topic.session_ids.contains(&r.session_id))
        {
            inherited.push(topic.id);
        }
    }
    inherited.sort();
    inherited.dedup();
    Ok(inherited)
}
fn dedup(traces: &[crate::model::AgentTraceRef]) -> Vec<crate::model::AgentTraceRef> {
    let mut out: Vec<crate::model::AgentTraceRef> = Vec::new();
    for trace in traces {
        if let Some(old) = out.iter_mut().find(|t| t.session_id == trace.session_id) {
            old.through_seq = old.through_seq.max(trace.through_seq);
        } else {
            out.push(trace.clone());
        }
    }
    out
}
/// Home has already validated the receiving workspace, including launches not placed yet.
pub async fn inherit(app: &App, session: &str, workspace: &str, ids: Vec<String>) -> Result<()> {
    let session = session.to_owned();
    let workspace = workspace.to_owned();
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        for id in ids { tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) SELECT id, ?2 FROM topic WHERE id = ?1 AND workspace_id = ?3", params![id, session, workspace])?; }
        tx.commit()?; Ok(())
    }).await?;
    changed(app);
    Ok(())
}

#[cfg(test)]
mod context_tests {
    #[test]
    fn overlapping_sources_keep_latest_cutoff_once() {
        use crate::model::AgentTraceRef as R;
        let refs = vec![
            R {
                session_id: "a".into(),
                through_seq: 3,
            },
            R {
                session_id: "b".into(),
                through_seq: 2,
            },
            R {
                session_id: "a".into(),
                through_seq: 8,
            },
        ];
        let result = super::dedup(&refs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].through_seq, 8);
    }
}

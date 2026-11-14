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
    /// Home created it for a launch and its generated name has not arrived yet (`group`).
    #[serde(default)]
    pub naming: bool,
}

pub async fn list(app: &App, workspace: &str) -> Result<Vec<Topic>> {
    let workspace = workspace.to_owned();
    app.state_db.call(move |c| {
        let mut q = c.prepare("SELECT id, name, auto_session IS NOT NULL FROM topic WHERE workspace_id = ?1 ORDER BY name COLLATE NOCASE")?;
        let rows = q.query_map([&workspace], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(id, name, naming)| {
            let mut q = c.prepare("SELECT session_id FROM topic_session WHERE topic_id = ?1 ORDER BY session_id")?;
            let session_ids = q.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(Topic { id, workspace_id: workspace.clone(), name, session_ids, naming })
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
        naming: false,
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
                // The owner's name wins over a generated one that has not arrived yet.
                "UPDATE topic SET name = ?2, auto_session = NULL WHERE id = ?1",
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

/// What the runner needs to name a topic Home created for a launch, once. Home sets `topic_id` and `context`;
/// the runner writes only `name` (`Some("")` once claimed), which Home applies (`named`) only to the topic it
/// created for that session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TopicNaming {
    pub topic_id: String,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Home, at a launch with other agents' traces (`sources`): the new agent and its sources form one group. The new
/// agent inherits the sources' topics (through `inherited`) and sources without a topic join them; when no source
/// has one, Home creates a topic for all of them under a temporary name and returns what the runner needs to
/// name it.
pub async fn group(app: &App, workspace: &str, session: &str, sources: &[String], inherited: &mut Vec<String>) -> Result<Option<TopicNaming>> {
    let mut sources = sources.to_vec();
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        return Ok(None);
    }
    let all = list(app, workspace).await?;
    let held: Vec<&Topic> = all.iter().filter(|t| sources.iter().any(|s| t.session_ids.contains(s))).collect();
    if !held.is_empty() {
        let ids: Vec<String> = held.iter().map(|t| t.id.clone()).collect();
        let bare: Vec<String> = sources.iter().filter(|s| !held.iter().any(|t| t.session_ids.contains(s))).cloned().collect();
        inherited.extend(ids.iter().cloned());
        if !bare.is_empty() {
            app.state_db.call(move |c| {
                let tx = c.transaction()?;
                for id in &ids {
                    for s in &bare {
                        tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) VALUES (?1, ?2)", params![id, s])?;
                    }
                }
                tx.commit()?;
                Ok(())
            }).await?;
            changed(app);
        }
        return Ok(None);
    }
    let titles: Vec<(String, String)> = sources.iter().filter_map(|id| app.sessions.get(id).map(|s| (id.clone(), s.title))).collect();
    let base = titles.first().map_or("Related agents".to_string(), |(_, t)| t.chars().take(60).collect::<String>().trim().to_string());
    let base = name(&base).unwrap_or_else(|_| "Related agents".into());
    let mut context = String::new();
    for (id, title) in &titles {
        context.push_str(&format!("Earlier agent: {title}\n"));
        if let Some(last) = last_answer(app, id).await {
            context.push_str(&format!("Its final message: {}\n", last.chars().take(1500).collect::<String>()));
        }
    }
    let taken: Vec<&str> = all.iter().map(|t| t.name.as_str()).take(40).collect();
    if !taken.is_empty() {
        context.push_str(&format!("Topic names already used: {}\n", taken.join("; ")));
    }
    let topic_id = uuid::Uuid::new_v4().to_string();
    let (id, ws, owner) = (topic_id.clone(), workspace.to_owned(), session.to_owned());
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        let value = unique(&tx, &ws, &base, None)?;
        tx.execute("INSERT INTO topic (id, workspace_id, name, auto_session) VALUES (?1, ?2, ?3, ?4)", params![id, ws, value, owner])?;
        for s in &sources {
            tx.execute("INSERT OR IGNORE INTO topic_session (topic_id, session_id) VALUES (?1, ?2)", params![id, s])?;
        }
        tx.commit()?;
        Ok(())
    }).await?;
    changed(app);
    inherited.push(topic_id.clone());
    Ok(Some(TopicNaming { topic_id, context, name: None }))
}

/// The newest assistant message Home stored for a session.
async fn last_answer(app: &App, id: &str) -> Option<String> {
    let id = id.to_owned();
    let json: String = app.state_db.call(move |c| {
        Ok(c.query_row("SELECT json FROM session_event WHERE session_id = ?1 AND kind = 'assistant_message' ORDER BY seq DESC LIMIT 1", [id], |r| r.get(0))?)
    }).await.ok()?;
    match serde_json::from_str::<crate::model::EventBody>(&json).ok()? {
        crate::model::EventBody::AssistantMessage { text, .. } => Some(text),
        _ => None,
    }
}

/// `value`, or `value` and three random digits when another topic of the workspace has that name.
fn unique(c: &rusqlite::Connection, workspace: &str, value: &str, except: Option<&str>) -> Result<String> {
    let taken = |v: &str| -> rusqlite::Result<bool> {
        c.query_row("SELECT COUNT(*) FROM topic WHERE workspace_id = ?1 AND name = ?2 AND id IS NOT ?3", params![workspace, v, except], |r| r.get::<_, i64>(0)).map(|n| n > 0)
    };
    let mut candidate = value.to_owned();
    for _ in 0..20 {
        if !taken(&candidate)? {
            return Ok(candidate);
        }
        let digits = 100 + uuid::Uuid::new_v4().as_u128() % 900;
        let stem: String = value.chars().take(76).collect();
        candidate = format!("{} {digits}", stem.trim_end());
    }
    anyhow::bail!("no free topic name")
}

/// Home: the name a runner generated for the topic Home created at this session's launch. Applied once, only while
/// that topic still waits for it (an owner rename or a delete wins).
pub async fn named(app: &App, session: &str, value: &str) -> Result<()> {
    let value = name(value)?;
    let session = session.to_owned();
    let renamed = app.state_db.call(move |c| {
        let tx = c.transaction()?;
        let Some((id, ws)) = tx.query_row("SELECT id, workspace_id FROM topic WHERE auto_session = ?1", [&session], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok() else {
            return Ok(false);
        };
        let value = unique(&tx, &ws, &value, Some(&id))?;
        tx.execute("UPDATE topic SET name = ?2, auto_session = NULL WHERE id = ?1", params![id, value])?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if renamed {
        changed(app);
    }
    Ok(())
}

/// Home: a launch the runner refused takes the topic Home created for it along.
pub async fn forget_auto(app: &App, session: &str) {
    let session = session.to_owned();
    let result = app.state_db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM topic_session WHERE topic_id IN (SELECT id FROM topic WHERE auto_session = ?1)", [&session])?;
        let n = tx.execute("DELETE FROM topic WHERE auto_session = ?1", [&session])?;
        tx.commit()?;
        Ok(n)
    }).await;
    match result {
        Ok(0) => {}
        Ok(_) => changed(app),
        Err(e) => tracing::warn!("topic of a refused launch: {e:#}"),
    }
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
    #[test]
    fn taken_names_get_three_digits() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE topic (id TEXT PRIMARY KEY, workspace_id TEXT, name TEXT COLLATE NOCASE, UNIQUE(workspace_id, name)); INSERT INTO topic VALUES ('t1', 'w1', 'Login fix');").unwrap();
        assert_eq!(super::unique(&c, "w2", "login fix", None).unwrap(), "login fix");
        assert_eq!(super::unique(&c, "w1", "Login fix", Some("t1")).unwrap(), "Login fix");
        let other = super::unique(&c, "w1", "login FIX", None).unwrap();
        assert!(other.starts_with("login FIX ") && other.len() == "login FIX ".len() + 3, "{other}");
    }
}

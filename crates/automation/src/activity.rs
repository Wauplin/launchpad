//! The activity list: actions that sessions with admin Launchpad CLI access ran directly (`actions::ACTIONS`),
//! recorded per session on Home with the values before the change and the call that undoes it. The front door
//! records them (`launchpad_server` guard); the bot chat shows them as Changed lines with Undo. Undo is the owner's
//! click on the undo action itself (same API call), then `undone`.
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use launchpad_core::{App, actions, util::now_ms};

/// The call that undoes an activity: an admin action of the action list, its route parameters and its body.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct UndoCall {
    pub action: String,
    pub args: Vec<String>,
    #[ts(type = "Record<string, unknown> | null")]
    pub body: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Activity {
    pub id: String,
    /// The session that ran the action.
    pub session_id: String,
    /// The session's workspace, when it has one.
    pub workspace_id: Option<String>,
    /// The action's name (`folder.set`) and its route parameters.
    pub action: String,
    pub args: Vec<String>,
    /// What the record is called after the change (a bot, routine, session or folder name), when known.
    pub target: Option<String>,
    /// The request body: the new values.
    #[ts(type = "Record<string, unknown> | null")]
    pub body: Option<Value>,
    /// The values before the change, for the same keys.
    #[ts(type = "Record<string, unknown> | null")]
    pub before: Option<Value>,
    pub undo: Option<UndoCall>,
    #[ts(type = "number | null")]
    pub undone_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// What the front door sends after a direct admin action succeeded.
#[derive(Debug, Deserialize)]
pub struct Record {
    pub session_id: String,
    pub action: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub body: Option<Value>,
    /// The answer of the action's `before` route, taken before the action ran.
    #[serde(default)]
    pub before: Option<Value>,
    /// The action's answer (a created record's `id`, a name).
    #[serde(default)]
    pub result: Option<Value>,
}

/// The undo call of an action, from the values before it and its answer (see `actions::Action::before`).
pub fn undo_call(a: &actions::Action, args: &[String], body: Option<&Value>, before: Option<&Value>, result: Option<&Value>) -> (Option<Value>, Option<UndoCall>) {
    let Some(u) = a.undo.and_then(actions::get) else { return (None, None) };
    if u.access != actions::ActionAccess::Admin {
        return (None, None);
    }
    if a.before.is_some() {
        let (Some(body), Some(before)) = (body.and_then(Value::as_object), before) else { return (None, None) };
        let mut old = serde_json::Map::new();
        for k in body.keys() {
            // A key the record does not show cannot be put back.
            let Some(v) = before.get(k) else { return (None, None) };
            old.insert(k.clone(), v.clone());
        }
        let old = Value::Object(old);
        return (Some(old.clone()), Some(UndoCall { action: u.name.into(), args: args.to_vec(), body: Some(old) }));
    }
    match result.and_then(|r| r["id"].as_str()) {
        Some(id) => (None, Some(UndoCall { action: u.name.into(), args: vec![id.into()], body: None })),
        None => (None, None),
    }
}

fn target_of(result: Option<&Value>) -> Option<String> {
    let r = result?;
    [&r["settings"]["name"], &r["title"], &r["name"]].into_iter().find_map(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

pub async fn record(app: &App, r: Record) -> Result<Activity> {
    let a = actions::get(&r.action).context("no such action")?;
    ensure!(a.access == actions::ActionAccess::Admin, "only admin actions are recorded");
    let (before, undo) = undo_call(a, &r.args, r.body.as_ref(), r.before.as_ref(), r.result.as_ref());
    let workspace_id = launchpad_core::kb::workspace_of_session(app, &r.session_id).await;
    // The body only explains a change without values to put back: a large one (a launch spec, a long message) is
    // not kept, so the list and its bus events stay small.
    let body = r.body.filter(|b| a.before.is_some() || b.to_string().len() <= 16 << 10);
    let item = Activity {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: r.session_id,
        workspace_id,
        action: r.action,
        args: r.args,
        target: target_of(r.result.as_ref()),
        body,
        before,
        undo,
        undone_at: None,
        created_at: now_ms(),
    };
    save(app, &item).await?;
    Ok(item)
}

async fn save(app: &App, a: &Activity) -> Result<()> {
    let (id, session, at, json) = (a.id.clone(), a.session_id.clone(), a.created_at, serde_json::to_string(a)?);
    app.state_db.call(move |c| Ok(c.execute("INSERT OR REPLACE INTO activity (id, session_id, created_at, json) VALUES (?1, ?2, ?3, ?4)", params![id, session, at, json]).map(|_| ())?)).await?;
    app.bus.publish("activity", "changed", serde_json::to_value(a)?);
    Ok(())
}

/// The newest 500 activities, of one workspace when given.
pub async fn list(app: &App, workspace: Option<String>) -> Result<Vec<Activity>> {
    let all: Vec<Activity> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM activity ORDER BY created_at DESC LIMIT 2000")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await?;
    Ok(all.into_iter().filter(|a| workspace.is_none() || a.workspace_id == workspace).take(500).collect())
}

/// After the owner ran the undo call.
pub async fn undone(app: &App, id: &str) -> Result<Activity> {
    let id2 = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM activity WHERE id = ?1", [id2], |r| r.get(0)).ok())).await?;
    let mut a: Activity = serde_json::from_str(&json.context("no such activity")?)?;
    ensure!(a.undo.is_some(), "this change has no undo");
    a.undone_at.get_or_insert(now_ms());
    save(app, &a).await?;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn undo_puts_back_the_changed_keys_or_deletes_the_created_record() {
        let rename = actions::get("session.rename").unwrap();
        let (before, undo) = undo_call(rename, &["s".into()], Some(&json!({"title": "New"})), Some(&json!({"title": "Old", "id": "s"})), None);
        assert_eq!(before, Some(json!({"title": "Old"})));
        assert_eq!(undo, Some(UndoCall { action: "session.rename".into(), args: vec!["s".into()], body: Some(json!({"title": "Old"})) }));
        // A key the record does not show: no undo.
        assert_eq!(undo_call(rename, &["s".into()], Some(&json!({"other": 1})), Some(&json!({"title": "Old"})), None).1, None);
        let create = actions::get("workspace.create").unwrap();
        assert_eq!(undo_call(create, &[], Some(&json!({"name": "W"})), None, Some(&json!({"id": "w1"}))).1, Some(UndoCall { action: "workspace.delete".into(), args: vec!["w1".into()], body: None }));
        assert_eq!(undo_call(actions::get("session.archive").unwrap(), &["s".into()], None, None, None).1, None);
    }
}

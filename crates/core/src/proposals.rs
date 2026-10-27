//! Ask Launchpad: inert proposals written by the assistant session, started only by an owner click.
//! Like agent suggestions, the agent drafts and the owner publishes. Accepting claims the proposal
//! durably before acting; a claim interrupted by a crash becomes Uncertain and is never retried
//! automatically. Launches use the session ID chosen at proposal time, so a finished launch is found
//! again by that ID.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::{
    App,
    crons::CronCondition,
    model::{LaunchSpec, Target},
    schedules::LaunchDelay,
    util::now_ms,
};

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalBody {
    /// A new agent.
    Launch { spec: LaunchSpec },
    /// A message to an existing session.
    Message { session_id: String, text: String },
    /// A new agent that starts when every condition is met.
    Schedule { spec: LaunchSpec, conditions: Vec<LaunchDelay> },
    /// A repeating agent (cron, local time).
    Cron { spec: LaunchSpec, schedule: String, conditions: Vec<CronCondition> },
    /// A list of sessions to look at; nothing to start.
    Status { session_ids: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Proposed,
    /// Claimed by an owner click; the action is in progress.
    Starting,
    /// Done: `result_id` is the session, schedule or cron.
    Started,
    Dismissed,
    /// Superseded by another proposal before starting.
    Replaced,
    /// The action failed before doing anything; the owner may start it again.
    Failed,
    /// A claim whose outcome is unknown (daemon restart while starting). Never retried automatically.
    Uncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Proposal {
    pub id: String,
    /// The workspace of the assistant thread (None: Everything).
    pub workspace_id: Option<String>,
    /// The assistant session that wrote it.
    pub session_id: Option<String>,
    pub title: String,
    /// Why the assistant proposes it, one or two sentences.
    pub note: Option<String>,
    pub body: ProposalBody,
    pub state: ProposalState,
    /// For launches: the session ID reserved at proposal time. Then the started session, schedule or cron.
    pub result_id: Option<String>,
    /// Successor card when this proposal was replaced.
    #[serde(default)]
    pub replaced_by: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize)]
pub struct Create {
    #[serde(default)]
    pub replaces: Option<String>,
    pub title: String,
    #[serde(default)]
    pub note: Option<String>,
    pub body: ProposalBody,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// The folder of the assistant thread of a workspace.
pub fn assistant_dir(app: &App, workspace_id: Option<&str>) -> std::path::PathBuf {
    let name: String = workspace_id.unwrap_or("everything").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    app.paths.data_dir.join("assistant").join(if name.is_empty() { "everything".into() } else { name })
}

/// The restricted credential of the assistant's CLI: read endpoints and proposals only (server allowlist).
pub fn assistant_token(main_token: &str) -> String {
    hex::encode(Sha256::digest(format!("launchpad-assistant\n{main_token}").as_bytes()))
}

/// Requests the assistant token may make. Everything else is refused by the server.
pub fn assistant_allows(method: &str, path: &str) -> bool {
    (method == "GET" && (path.starts_with("/api/context/") || (path.starts_with("/api/sessions/") && path.ends_with("/trace")) || path == "/api/proposals" || path == "/api/status"))
        || (method == "POST" && (path == "/api/proposals" || path == "/api/launch/preview"))
}

/// Launch instructions of the assistant session.
pub async fn instruction(app: &App, workspace_id: Option<&str>, cli: &str) -> String {
    let token = app.paths.token().map(|t| assistant_token(&t)).unwrap_or_default();
    let ws = match workspace_id {
        Some(id) => crate::workspace::get(app, id).await.map(|w| format!("the workspace \"{}\" (id {id})", w.name)).unwrap_or_else(|_| format!("the workspace {id}")),
        None => "every workspace (Everything)".into(),
    };
    let ws_arg = workspace_id.map(|id| format!(" --workspace {id}")).unwrap_or_default();
    let cmd = if cfg!(windows) { format!("$env:LAUNCHPAD_TOKEN='{token}'; {cli}") } else { format!("LAUNCHPAD_TOKEN={token} {cli}") };
    format!(
        "You are Ask Launchpad, the owner's assistant inside Launchpad, scoped to {ws}. You turn requests into proposals that the owner starts with one tap. You never start agents, send messages, schedule, publish or post to GitHub yourself, and you do not edit files.\n\n\
Read Launchpad state with these read-only commands (JSON output):\n\
- `{cmd} folders list{ws_arg}`\n- `{cmd} sessions list{ws_arg}` and `{cmd} sessions show <session-id>`\n- `{cmd} profiles list` (profiles and the quota of each subscription login)\n- `{cmd} presets list`\n- `{cmd} inbox list{ws_arg}`\n- `{cmd} schedules list{ws_arg}`\n- `{cmd} proposals list{ws_arg}` (proposal ids, titles, states and contents)\n- `{cmd} trace <session-id>` for an agent's full trace\n\n\
Write proposals with `{cmd} propose <launch|message|schedule|cron|status> --title \"…\" [--note \"…\"] [--replaces <proposal-id>] --spec-file <file.json>`. The JSON file holds:\n\
- launch: {{\"spec\": <launch spec>}}\n- message: {{\"session_id\": \"…\", \"text\": \"…\"}}\n- schedule: {{\"spec\": <launch spec>, \"conditions\": [{{\"kind\": \"in\", \"minutes\": 30}} | {{\"kind\": \"at\", \"at\": <unix ms>}} | {{\"kind\": \"agents\", \"session_ids\": [\"…\"]}} | {{\"kind\": \"quota\", \"harness\": \"claude\", \"window\": \"five_hour\"}}]}}\n- cron: {{\"spec\": <launch spec>, \"schedule\": \"0 9 * * 1\", \"conditions\": []}}\n- status: {{\"session_ids\": [\"…\"]}}\n\
A launch spec is {{\"text\": \"<prompt>\", \"profile\": \"<profile id>\", \"target\": {{\"kind\": \"project\", \"project_id\": \"<folder id>\"}} or {{\"kind\": \"temp\"}}, \"worktree\": {{\"base\": null, \"include_changes\": false, \"merge_when_done\": false}} for Git folders (null for the checkout), \"github_refs\": [], \"presets\": [<preset ids>]}}; other fields take their defaults. Launchpad validates each proposal with the launch preview and refuses invalid ones; read the error and fix the file. Paid usage stays off. Prefer one proposal per independent task, a message proposal to steer an existing agent, a status proposal to answer \"what needs me\", and a schedule with a quota condition for work that should wait for quota. Crons always need the owner's tap.\n\n\
When refining an earlier waiting proposal, find it with `proposals list` and use `--replaces <proposal-id>` so only the refined card waits. Only proposed or failed cards in this workspace can be replaced; starting, started, dismissed, uncertain and already replaced cards cannot. Tell the owner which card was replaced. The note on the new card is optional.\n\nAnswer in short sentences. After proposing, tell the owner what each card does; they can accept it directly in the thread. Accepting a proposal closes this thread; the next request starts fresh, and the owner can resume this conversation."
    )
}

async fn save(app: &App, p: &Proposal) -> Result<()> {
    let (id, json) = (p.id.clone(), serde_json::to_string(p)?);
    app.state_db.call(move |c| c.execute("INSERT OR REPLACE INTO launch_proposal (id, json) VALUES (?1, ?2)", params![id, json]).map(|_| ()).map_err(Into::into)).await?;
    app.bus.publish("proposals", "changed", serde_json::to_value(p)?);
    Ok(())
}

pub async fn list(app: &App) -> Result<Vec<Proposal>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM launch_proposal ORDER BY rowid DESC LIMIT 500")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await
}

pub async fn get(app: &App, id: &str) -> Result<Proposal> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM launch_proposal WHERE id = ?1", params![id], |r| r.get(0)).ok())).await?;
    Ok(serde_json::from_str(&json.context("no such proposal")?)?)
}

/// Paid usage is always off on proposals; the owner changes it in the composer.
fn sanitize(spec: &mut LaunchSpec, workspace_id: &Option<String>) -> Result<()> {
    ensure!(!spec.target.is_assistant() && !spec.target.is_browser(), "proposals cannot start assistant or browser sessions");
    spec.allow_paid_usage = false;
    spec.workspace_id = workspace_id.clone();
    Ok(())
}

/// Stores an inert proposal after the same validation as the launch preview.
pub async fn create(app: &App, c: Create) -> Result<Proposal> {
    ensure!(!c.title.trim().is_empty(), "give the proposal a title");
    let mut body = c.body;
    let workspace_id = match (&c.workspace_id, &c.session_id) {
        (Some(w), _) => Some(w.clone()),
        (None, Some(sid)) => app.sessions.get(sid).and_then(|s| match s.launch_spec.target { Target::Assistant { workspace_id } => workspace_id, _ => None }),
        _ => None,
    };
    let mut result_id = None;
    match &mut body {
        ProposalBody::Launch { spec } | ProposalBody::Schedule { spec, .. } | ProposalBody::Cron { spec, .. } => {
            sanitize(spec, &workspace_id)?;
            if spec.profile.is_some() && spec.harness.is_none() {
                let config = app.config.get();
                spec.harness = config.profiles.iter().find(|p| Some(&p.id) == spec.profile.as_ref()).map(|p| p.harness);
            }
            ensure!(spec.harness.is_some(), "set a profile (see `profiles list`)");
            ensure!(!spec.text.trim().is_empty(), "write a prompt in spec.text");
            app.sessions.preview(app, spec).await.context("invalid launch spec")?;
            if matches!(body, ProposalBody::Launch { .. }) {
                result_id = Some(uuid::Uuid::new_v4().to_string());
            }
        }
        ProposalBody::Message { session_id, text } => {
            ensure!(app.sessions.get(session_id).is_some(), "no session {session_id}");
            ensure!(!text.trim().is_empty(), "the message is empty");
        }
        ProposalBody::Status { session_ids } => ensure!(!session_ids.is_empty(), "list at least one session"),
    }
    if let ProposalBody::Cron { schedule, .. } = &body {
        crate::crons::next_after(schedule, now_ms()).context("invalid cron schedule")?;
    }
    let now = now_ms();
    let p = Proposal { id: uuid::Uuid::new_v4().to_string(), workspace_id, session_id: c.session_id, title: c.title.trim().to_string(), note: c.note.filter(|n| !n.trim().is_empty()), body, state: ProposalState::Proposed, result_id, replaced_by: None, error: None, created_at: now, updated_at: now };
    // Share the claim lock with accept/dismiss: a replacement cannot race an owner click.
    let _lock = ACCEPT.lock().await;
    let stored = p.clone();
    app.state_db.call(move |db| store_created(db, &stored, c.replaces.as_deref())).await?;
    app.bus.publish("proposals", "changed", serde_json::to_value(&p)?);
    Ok(p)
}

/// Atomically retire the old card and insert its successor, or leave both untouched on error.
fn store_created(db: &mut rusqlite::Connection, p: &Proposal, replaces: Option<&str>) -> Result<()> {
    let tx = db.transaction()?;
    if let Some(id) = replaces {
        let json: Option<String> = tx.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let mut old: Proposal = serde_json::from_str(&json.context("no such proposal to replace")?)?;
        ensure!(matches!(old.state, ProposalState::Proposed | ProposalState::Failed), "cannot replace proposal '{}' ({:?}); only waiting, unstarted proposals can be replaced", old.title, old.state);
        ensure!(old.workspace_id == p.workspace_id, "cannot replace a proposal from another workspace");
        old.state = ProposalState::Replaced;
        old.replaced_by = Some(p.id.clone());
        old.updated_at = p.updated_at;
        tx.execute("UPDATE launch_proposal SET json = ?2 WHERE id = ?1", params![id, serde_json::to_string(&old)?])?;
    }
    tx.execute("INSERT INTO launch_proposal (id, json) VALUES (?1, ?2)", params![p.id, serde_json::to_string(p)?])?;
    tx.commit()?;
    Ok(())
}

/// Serializes claims, dismissals and replacements so only one can retire a waiting card.
static ACCEPT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Owner click: claim, act, record the result. `body` may replace the proposal's spec (edited in the card).
pub async fn accept(app: &App, id: &str) -> Result<Proposal> {
    let mut p = {
        let _lock = ACCEPT.lock().await;
        let mut p = get(app, id).await?;
        match p.state {
            ProposalState::Proposed | ProposalState::Failed => {}
            ProposalState::Started => return Ok(p),
            ProposalState::Uncertain => bail!("Delivery not confirmed. Check Agents before starting a new attempt."),
            ProposalState::Starting => bail!("This proposal is already starting"),
            ProposalState::Dismissed => bail!("This proposal was dismissed"),
            ProposalState::Replaced => bail!("This proposal was replaced; open its replacement card"),
        }
        p.state = ProposalState::Starting;
        p.error = None;
        p.updated_at = now_ms();
        save(app, &p).await?;
        p
    };
    let result: Result<String> = async {
        match p.body.clone() {
            ProposalBody::Launch { spec } => {
                let id = p.result_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if app.sessions.get(&id).is_some() {
                    return Ok(id);
                }
                Ok(app.sessions.launch_with_id(app, spec, id).await?.id)
            }
            ProposalBody::Message { session_id, text } => {
                app.sessions.send_with_id(app, &session_id, &text, vec![], format!("proposal-{}", p.id)).await?;
                Ok(session_id)
            }
            ProposalBody::Schedule { spec, conditions } => Ok(crate::schedules::create(app, spec, conditions).await?.id),
            ProposalBody::Cron { spec, schedule, conditions } => Ok(crate::crons::create(app, spec, schedule, conditions).await?.id),
            ProposalBody::Status { .. } => bail!("a status card has nothing to start"),
        }
    }
    .await;
    match result {
        Ok(id) => {
            p.state = ProposalState::Started;
            p.result_id = Some(id);
        }
        Err(e) => {
            p.state = ProposalState::Failed;
            p.error = Some(format!("{e:#}"));
        }
    }
    p.updated_at = now_ms();
    save(app, &p).await?;
    close_accepted_thread(app, &p).await?;
    Ok(p)
}

/// Close only the assistant that authored a successful proposal, never a newer workspace thread.
/// Keep the accepted result durable even if shutting down the assistant fails.
async fn close_accepted_thread(app: &App, p: &Proposal) -> Result<()> {
    if p.state == ProposalState::Started
        && let Some(s) = p.session_id.as_deref().and_then(|id| app.sessions.get(id))
        && s.launch_spec.target.is_assistant()
        && s.status != crate::model::SessionStatus::Closed
    {
        app.sessions.close(app, &s.id).await.context("Proposal accepted, but its assistant thread could not close")?;
    }
    Ok(())
}

pub async fn dismiss(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    ensure!(matches!(p.state, ProposalState::Proposed | ProposalState::Failed | ProposalState::Uncertain), "only waiting proposals can be dismissed");
    p.state = ProposalState::Dismissed;
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// After the owner checked Agents: an uncertain launch becomes Started when its reserved session exists,
/// otherwise it can be started again (with a new reserved session ID).
pub async fn recheck(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    ensure!(p.state == ProposalState::Uncertain, "only uncertain proposals need a check");
    if matches!(p.body, ProposalBody::Launch { .. }) && p.result_id.as_deref().is_some_and(|id| app.sessions.get(id).is_some()) {
        p.state = ProposalState::Started;
    } else {
        p.state = ProposalState::Failed;
        p.error = Some("Not confirmed after a restart; checked by the owner.".into());
        if matches!(p.body, ProposalBody::Launch { .. }) {
            p.result_id = Some(uuid::Uuid::new_v4().to_string());
        }
    }
    p.updated_at = now_ms();
    save(app, &p).await?;
    close_accepted_thread(app, &p).await?;
    Ok(p)
}

/// At startup: a claim still Starting was interrupted; its outcome is unknown.
pub async fn recover(app: &App) -> Result<()> {
    for mut p in list(app).await? {
        if p.state == ProposalState::Starting {
            p.state = ProposalState::Uncertain;
            p.error = Some("The daemon restarted while this was starting. Check Agents before starting a new attempt.".into());
            p.updated_at = now_ms();
            save(app, &p).await?;
        }
    }
    Ok(())
}

/// Starts (or continues) the assistant thread of a workspace with the owner's message.
/// Images go to the assistant as native image input; other files become private local copies it can read.
pub async fn ask(app: &App, workspace_id: Option<String>, text: String, attachments: Vec<crate::model::FileAttachment>) -> Result<crate::model::Session> {
    ensure!(!text.trim().is_empty() || !attachments.is_empty(), "the message is empty");
    if let Some(s) = thread(app, workspace_id.as_deref()) {
        let (text, images) = crate::attachments::for_message(app, &s.id, &text, &attachments).await?;
        app.sessions.send(app, &s.id, &text, images).await?;
        return app.sessions.get(&s.id).context("the assistant session disappeared");
    }
    let config = app.config.get();
    let profile = config.assistant.profile.as_ref().and_then(|id| config.profiles.iter().find(|p| &p.id == id)).or_else(|| config.profiles.iter().find(|p| p.billing == crate::config::Billing::Subscription)).or(config.profiles.first()).context("add a profile in Settings → Profiles first")?;
    let spec = LaunchSpec {
        text,
        attachments,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        model: config.assistant.model.clone().or_else(|| profile.model.clone()),
        effort: config.assistant.effort.clone().or_else(|| profile.effort.clone()),
        target: Target::Assistant { workspace_id: workspace_id.clone() },
        workspace_id,
        ..Default::default()
    };
    app.sessions.launch(app, spec).await
}

/// Remembers a model change of an assistant thread as the assistant's default (never the composer's).
pub fn remember_model(app: &App, session: &crate::model::Session) -> Result<()> {
    if !session.launch_spec.target.is_assistant() {
        return Ok(());
    }
    let mut config = app.config.get();
    let next = crate::config::Assistant { profile: session.launch_spec.profile.clone(), model: session.model.clone(), effort: session.effort.clone() };
    if config.assistant == next {
        return Ok(());
    }
    config.assistant = next;
    app.config.save(config)
}

/// The open assistant thread of a workspace: its newest unarchived assistant session.
pub fn thread(app: &App, workspace_id: Option<&str>) -> Option<crate::model::Session> {
    app.sessions
        .list()
        .into_iter()
        .filter(|s| s.status != crate::model::SessionStatus::Closed && matches!(&s.launch_spec.target, Target::Assistant { workspace_id: w } if w.as_deref() == workspace_id))
        .max_by_key(|s| s.started_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(id: &str, state: ProposalState) -> Proposal {
        Proposal {
            id: id.into(), workspace_id: None, session_id: None, title: format!("Card {id}"), note: None,
            body: ProposalBody::Status { session_ids: vec!["session".into()] }, state, result_id: None,
            replaced_by: None, error: None, created_at: 1, updated_at: 2,
        }
    }

    fn database() -> rusqlite::Connection {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE launch_proposal (id TEXT PRIMARY KEY, json TEXT NOT NULL)").unwrap();
        db
    }

    fn read(db: &rusqlite::Connection, id: &str) -> Proposal {
        let json: String = db.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn replaces_waiting_cards_and_keeps_successor_links() {
        for state in [ProposalState::Proposed, ProposalState::Failed] {
            let mut db = database();
            store_created(&mut db, &proposal("old", state), None).unwrap();
            store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap();
            let old = read(&db, "old");
            assert_eq!(old.state, ProposalState::Replaced);
            assert_eq!(old.replaced_by.as_deref(), Some("new"));
            assert_eq!(old.title, "Card old");
            assert_eq!(read(&db, "new").state, ProposalState::Proposed);
            assert!(read(&db, "new").note.is_none());
            store_created(&mut db, &proposal("newer", ProposalState::Proposed), Some("new")).unwrap();
            assert_eq!(read(&db, "new").replaced_by.as_deref(), Some("newer"));
        }
    }

    #[test]
    fn rejects_nonwaiting_missing_and_cross_workspace_targets_without_creating_a_card() {
        for state in [ProposalState::Starting, ProposalState::Started, ProposalState::Dismissed, ProposalState::Uncertain, ProposalState::Replaced] {
            let mut db = database();
            store_created(&mut db, &proposal("old", state.clone()), None).unwrap();
            let error = store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap_err();
            assert!(error.to_string().contains("only waiting, unstarted proposals"));
            assert_eq!(read(&db, "old").state, state);
            assert_eq!(db.query_row("SELECT count(*) FROM launch_proposal", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
        let mut db = database();
        assert!(store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("missing")).unwrap_err().to_string().contains("no such proposal"));
        let mut old = proposal("old", ProposalState::Proposed);
        old.workspace_id = Some("other".into());
        store_created(&mut db, &old, None).unwrap();
        assert!(store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap_err().to_string().contains("another workspace"));
        assert_eq!(read(&db, "old").state, ProposalState::Proposed);
    }

    #[test]
    fn insertion_failure_rolls_back_replacement_and_legacy_cards_still_load() {
        let mut db = database();
        let old = proposal("old", ProposalState::Proposed);
        store_created(&mut db, &old, None).unwrap();
        assert!(store_created(&mut db, &old, Some("old")).is_err());
        assert_eq!(read(&db, "old").state, ProposalState::Proposed);
        let mut json = serde_json::to_value(old).unwrap();
        json.as_object_mut().unwrap().remove("replaced_by");
        assert!(serde_json::from_value::<Proposal>(json).unwrap().replaced_by.is_none());
    }
}

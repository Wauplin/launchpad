//! Moving a session to another runner between turns (Move to…, Continue on Burst).
//!
//! The record and its events are on Home, the branches are on Home Git and plain folders are synced. What a
//! new runner still needs is a *handoff*: the harness's native conversation files and the session's worktree
//! records. Runners upload one at the end of every turn (when the conversation changed), so a session can
//! continue elsewhere while its machine sleeps, and a final one before a planned move (branches pushed,
//! uncommitted files saved to a scratch branch, worktrees removed). On Home, a move increases the session's
//! generation and names the new runner, then that runner adopts the session: it fetches the handoff, records
//! the worktrees (cloned again from their branches on the next message) and puts the conversation where its
//! harness resumes it. Home refuses pushes, leases and records of the older generation.

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::App;
use crate::model::Session;
use crate::util::now_ms;
use crate::worktrees::{TaskWorktree, WorktreeState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandoffFile {
    pub name: String,
    /// base64. Empty in Home's `session_handoff` row, which names the content by `sha256` instead.
    #[serde(default)]
    pub data: String,
    /// Home: the file's content in the blob store (`plain::blob_path`), never in `state.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    pub session_id: String,
    pub generation: u32,
    pub harness_session_id: Option<String>,
    pub cwd: String,
    pub worktrees: Vec<TaskWorktree>,
    pub files: Vec<HandoffFile>,
    /// Made for a planned move: branches pushed, leftovers saved, folders removed, no process left.
    #[serde(rename = "final")]
    pub is_final: bool,
    pub at: i64,
}

/// What a new runner fetches to adopt a session.
#[derive(Serialize, Deserialize)]
pub struct Adoption {
    pub session: Session,
    pub handoff: Option<Handoff>,
}

// ------------------------------------------------------------------ runner

/// The handoff of an idle session of this runner. `is_final`: prepare the move (push, save leftovers, remove
/// the folders and stop the idle process).
pub async fn export(app: &App, id: &str, is_final: bool) -> Result<Handoff> {
    if is_final {
        prepare_move(app, id).await?;
    }
    let mut h = snapshot(app, id).await?;
    h.is_final = is_final;
    Ok(h)
}

/// Pushes the session's branches, saves leftovers, removes its folders and stops its idle process.
async fn prepare_move(app: &App, id: &str) -> Result<()> {
    let s = app.sessions.get(id).context("no such session")?;
    {
        ensure!(!s.status.is_running() && !s.open_question && !app.sessions.setup_running(id), "Wait for the current turn to end before moving this session.");
        ensure!(app.terminals.tui_of(id).is_none(), "Close the session's terminal before moving it.");
        for w in crate::worktrees::for_session(app, id).await?.into_iter().filter(|w| w.state == WorktreeState::Active) {
            crate::worktrees::remove(app, &w.id).await.with_context(|| format!("{} cannot move yet", w.branch))?;
        }
        crate::plain::after_turn(app, id).await;
        let _action = app.sessions.action_lock().await;
        app.sessions.release(app, id).await?;
    }
    // Its temporary folder leaves this disk with it (Home holds the files) unless another agent here uses it.
    if let Some(pid) = &s.project_id
        && crate::projects::get(app, pid).await?.is_some_and(|p| p.is_temp)
        && !app.sessions.list().iter().any(|o| o.id != id && o.project_id.as_deref() == Some(pid.as_str()) && o.status != crate::model::SessionStatus::Closed)
    {
        crate::projects::remove_temp_copy(app, pid, false).await.context("its temporary folder cannot move yet")?;
    }
    Ok(())
}

/// The conversation and worktree records as they are now.
async fn snapshot(app: &App, id: &str) -> Result<Handoff> {
    let s = app.sessions.get(id).context("no such session")?;
    let files = match s.harness_session_id {
        Some(_) => app.harnesses.get(s.harness).native_trace(app, &s).await.context("read the agent's conversation")?,
        None => vec![],
    };
    Ok(Handoff {
        session_id: s.id.clone(),
        generation: s.generation,
        harness_session_id: s.harness_session_id.clone(),
        cwd: s.cwd.clone(),
        worktrees: crate::worktrees::for_session(app, id).await?,
        files: files.into_iter().map(|f| HandoffFile { name: f.name, data: base64::engine::general_purpose::STANDARD.encode(f.data), sha256: None }).collect(),
        is_final: false,
        at: now_ms(),
    })
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30)).timeout(std::time::Duration::from_secs(600)).build()?)
}

fn to_home(app: &App, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
    let link = crate::home_link::load(&app.paths).context("this runner is not connected to a Home")?;
    let mut r = client()?.request(method, format!("{}{path}", link.url.trim_end_matches('/'))).header("x-launchpad-runner", link.credential());
    if let Some(t) = crate::home_link::space_token(&link.url) {
        r = r.bearer_auth(t);
    }
    Ok(r)
}

async fn answer(r: reqwest::RequestBuilder) -> Result<serde_json::Value> {
    let res = r.send().await?;
    let status = res.status();
    let v: serde_json::Value = res.json().await.unwrap_or_default();
    ensure!(status.is_success(), "Home: {}", v["error"].as_str().unwrap_or(status.as_str()));
    Ok(v)
}

/// Sends a handoff to Home.
pub async fn upload(app: &App, id: &str, is_final: bool) -> Result<()> {
    let h = export(app, id, is_final).await?;
    answer(to_home(app, reqwest::Method::POST, &format!("/api/handoff/{id}"))?.json(&h)).await?;
    if is_final {
        // Everything the runner still holds for Home goes before it lets the session go.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while crate::replica::backlog(app).await? > 0 {
            ensure!(std::time::Instant::now() < deadline, "Home has not saved this runner's latest events yet");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Ok(())
}

/// Hash of a session's native conversation files.
fn files_digest(files: &[HandoffFile]) -> String {
    let mut hasher = Sha256::new();
    for f in files {
        hasher.update(f.name.as_bytes());
        hasher.update(f.data.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Runner: removes the folder of an archived session (`<data_dir>/sessions/<id>`, the harness home with its
/// native conversation) once Home confirmed the same conversation. Resume gets it back from Home (`restore`).
/// Returns whether the folder was removed.
pub async fn clean_archived(app: &App, id: &str) -> Result<bool> {
    if app.paths.role != crate::role::Role::Runner {
        return Ok(false);
    }
    let dir = crate::contain::session_dir(app, id);
    let idle = |s: &Session| s.status == crate::model::SessionStatus::Closed && !s.live && !app.sessions.setup_running(id) && app.terminals.tui_of(id).is_none();
    let Some(s) = app.sessions.get(id).filter(|s| idle(s)) else { return Ok(false) };
    if !dir.exists() {
        return Ok(false);
    }
    let h = snapshot(app, id).await?;
    ensure!(s.harness_session_id.is_none() || !h.files.is_empty(), "no native conversation found");
    answer(to_home(app, reqwest::Method::POST, &format!("/api/handoff/{id}"))?.json(&h)).await?;
    let _action = app.sessions.action_lock().await;
    // Nothing may have changed since Home confirmed its copy.
    let Some(s) = app.sessions.get(id).filter(|s| idle(s)) else { return Ok(false) };
    if s.generation != h.generation || s.harness_session_id != h.harness_session_id {
        return Ok(false);
    }
    let again = snapshot(app, id).await?;
    if files_digest(&again.files) != files_digest(&h.files) {
        return Ok(false);
    }
    tokio::fs::remove_dir_all(&dir).await.with_context(|| format!("remove {}", dir.display()))?;
    SENT.lock().unwrap().remove(id);
    Ok(true)
}

/// Runner: puts back the native conversation of a session whose folder `clean_archived` removed, from Home's
/// copy, before its harness starts again.
pub async fn restore(app: &App, s: &Session) -> Result<()> {
    let a: Adoption = serde_json::from_value(answer(to_home(app, reqwest::Method::GET, &format!("/api/handoff/{}", s.id))?).await?)?;
    let h = a.handoff.filter(|h| h.generation == s.generation && h.harness_session_id == s.harness_session_id && !h.files.is_empty());
    let h = h.context("Home has no copy of this agent's conversation")?;
    let files = h
        .files
        .into_iter()
        .map(|f| Ok(crate::harness::trace::TraceFile { name: f.name, data: base64::engine::general_purpose::STANDARD.decode(f.data)? }))
        .collect::<Result<Vec<_>>>()?;
    std::fs::create_dir_all(crate::contain::harness_home(app, &s.id))?;
    app.harnesses.get(s.harness).restore_native(app, s, files).await
}

/// Runner: every hour, removes the folders of archived sessions that Home holds (see `clean_archived`).
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Runner {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            for s in app.sessions.list().into_iter().filter(|s| s.status == crate::model::SessionStatus::Closed) {
                if let Err(e) = clean_archived(&app, &s.id).await {
                    tracing::debug!("session folder of {}: {e:#}", s.id);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600 - 120)).await;
        }
    });
}

/// Hashes of the last conversation each session uploaded, so an unchanged one is not sent again.
static SENT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::LazyLock::new(Default::default);

/// At the end of every turn on a runner: the conversation goes to Home when it changed.
pub async fn after_turn(app: &App, id: &str) {
    if app.paths.role != crate::role::Role::Runner {
        return;
    }
    let Ok(h) = snapshot(app, id).await else { return };
    let mut hasher = Sha256::new();
    for f in &h.files {
        hasher.update(f.name.as_bytes());
        hasher.update(f.data.as_bytes());
    }
    for w in &h.worktrees {
        hasher.update(format!("{}:{:?}:{:?}", w.id, w.state, w.pushed).as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    if SENT.lock().unwrap().get(id) == Some(&digest) {
        return;
    }
    let sent = async { answer(to_home(app, reqwest::Method::POST, &format!("/api/handoff/{id}"))?.json(&h)).await };
    match sent.await {
        Ok(_) => {
            SENT.lock().unwrap().insert(id.to_string(), digest);
        }
        Err(e) => tracing::debug!("handoff of {id}: {e:#}"),
    }
}

/// Where a moved session's folders are on this runner: its worktrees here, its plain folder or its temporary folder.
async fn relocate(app: &App, s: &mut Session, worktrees: &mut [TaskWorktree]) -> Result<()> {
    let old_cwd = s.cwd.clone();
    let mut placed = false;
    for w in worktrees.iter_mut() {
        let slug = std::path::Path::new(&w.root).file_name().map(|n| n.to_string_lossy().into_owned()).context("bad worktree folder")?;
        let root = app.paths.data_dir.join("worktrees").join(&w.project_id).join(&slug).to_string_lossy().into_owned();
        if let Some(rest) = old_cwd.strip_prefix(&w.root)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            s.cwd = format!("{root}{rest}");
            placed = true;
        }
        w.repo = crate::repo_cache::dir(app, &w.project_id).to_string_lossy().into_owned();
        w.root = root;
        // The folder comes back from its branch on Home when the session continues.
        if w.state == WorktreeState::Active {
            w.state = WorktreeState::Removed;
        }
    }
    if placed {
        return Ok(());
    }
    if let Some(pid) = &s.project_id
        && let Some(p) = crate::projects::get(app, pid).await?
        && p.plain
    {
        s.cwd = p.path.clone();
        return Ok(());
    }
    // A temporary folder: the same folder here, its files from Home before the next turn (`plain::prepare`).
    if let Some(p) = crate::projects::ensure_temp(app, s).await? {
        s.cwd = p.path;
        return Ok(());
    }
    bail!("this session's folder is no longer stored on Home")
}

/// A runner takes over a session Home moved here: it fetches the handoff, records the worktrees and puts the
/// conversation where its harness resumes it.
pub async fn adopt(app: &App, id: &str) -> Result<()> {
    ensure!(app.paths.role == crate::role::Role::Runner, "only runners take over sessions");
    let a: Adoption = serde_json::from_value(answer(to_home(app, reqwest::Method::GET, &format!("/api/handoff/{id}"))?).await?)?;
    let mut s = a.session;
    if let Some(local) = app.sessions.get(id) {
        ensure!(local.generation < s.generation, "this runner already has the session");
    }
    s.runner_id = None;
    s.live = false;
    s.open_question = false;
    let (mut worktrees, files) = match a.handoff {
        Some(h) => (h.worktrees, h.files),
        None => (vec![], vec![]),
    };
    relocate(app, &mut s, &mut worktrees).await?;
    for w in &mut worktrees {
        if !w.sessions.iter().any(|x| x == id) {
            w.sessions.push(id.to_string());
        }
        crate::worktrees::save(app, w).await?;
    }
    if s.harness_session_id.is_some() {
        let files = files
            .into_iter()
            .map(|f| Ok(crate::harness::trace::TraceFile { name: f.name, data: base64::engine::general_purpose::STANDARD.decode(f.data)? }))
            .collect::<Result<Vec<_>>>()?;
        ensure!(!files.is_empty(), "Home has no copy of this agent's conversation");
        std::fs::create_dir_all(crate::contain::harness_home(app, id))?;
        app.harnesses.get(s.harness).restore_native(app, &s, files).await.context("restore the agent's conversation")?;
    }
    app.sessions.adopt(app, s).await
}

// ------------------------------------------------------------------ Home

/// Home: stores a runner's handoff of a session it runs (current generation only).
/// The native files go to the blob store; the row keeps their names and hashes.
pub async fn store(app: &App, runner_id: &str, mut h: Handoff) -> Result<()> {
    let s = app.sessions.get(&h.session_id).context("no such session")?;
    ensure!(s.runner_id.as_deref() == Some(runner_id) && s.generation == h.generation, "this session runs elsewhere now");
    for f in &mut h.files {
        let data = base64::engine::general_purpose::STANDARD.decode(&f.data)?;
        let sha = hex::encode(Sha256::digest(&data));
        crate::plain::write_blob(app, &sha, &data).await?;
        f.sha256 = Some(sha);
        f.data.clear();
    }
    let (id, generation, json) = (h.session_id.clone(), h.generation, serde_json::to_string(&h)?);
    let runner = runner_id.to_string();
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO session_handoff (session_id, generation, runner_id, at, json) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (session_id) DO UPDATE SET generation = ?2, runner_id = ?3, at = ?4, json = ?5",
                rusqlite::params![id, generation, runner, now_ms(), json],
            )?;
            Ok(())
        })
        .await?;
    app.bus.publish("sessions", "handoff", serde_json::json!({"id": h.session_id}));
    Ok(())
}

/// Home: the latest handoff of a session.
pub async fn latest(app: &App, id: &str) -> Result<Option<Handoff>> {
    let key = id.to_string();
    let json: Option<String> = app
        .state_db
        .call(move |c| Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT json FROM session_handoff WHERE session_id = ?1", [key], |r| r.get(0)))?))
        .await?;
    let Some(mut h) = json.and_then(|j| serde_json::from_str::<Handoff>(&j).ok()) else { return Ok(None) };
    for f in &mut h.files {
        if let Some(sha) = &f.sha256 {
            f.data = base64::engine::general_purpose::STANDARD.encode(crate::plain::read_blob(app, sha).await?);
        }
    }
    Ok(Some(h))
}

/// Home: what the new runner of a moved session fetches.
pub async fn adoption(app: &App, runner_id: &str, id: &str) -> Result<Adoption> {
    let s = app.sessions.get(id).context("no such session")?;
    ensure!(s.runner_id.as_deref() == Some(runner_id), "this session runs elsewhere");
    let handoff = latest(app, id).await?;
    Ok(Adoption { session: s, handoff })
}

/// Why a session cannot move now, or None.
pub fn blocked(s: &Session) -> Option<&'static str> {
    if s.status.is_running() || s.open_question {
        return Some("Wait for the current turn to end before moving this session.");
    }
    if s.quota_pause.is_some() {
        return Some("This session is paused for quota: resume it first.");
    }
    if s.status == crate::model::SessionStatus::Closed {
        return Some("This session is archived.");
    }
    None
}

/// Home: the handoff a session continues from while its runner is offline. The latest one Home stored, even
/// from an older generation: a runner that adopted the session but stopped before its first turn end (a Burst
/// job that failed) uploaded none, and the previous one still holds the conversation.
pub async fn offline_copy(app: &App, s: &Session) -> Result<Handoff> {
    let h = latest(app, &s.id).await?.filter(|h| h.generation <= s.generation && (s.harness_session_id.is_none() || !h.files.is_empty()));
    h.context("its runner is offline and Home has no copy of this agent's conversation yet: wait for the runner to be back")
}

/// Home: moves an idle session to `to` (a connected runner). The old runner prepares the move when it is
/// connected; otherwise its last handoff is used (its uncommitted files stay there).
pub async fn move_to(app: &App, id: &str, to: &str) -> Result<Session> {
    ensure!(app.paths.role == crate::role::Role::Home, "only Home moves sessions");
    let s = app.sessions.get(id).context("no such session")?;
    if let Some(reason) = blocked(&s) {
        bail!("{reason}");
    }
    let from = s.runner_id.clone().context("this session has no runner")?;
    ensure!(from != to, "the session already runs there");
    ensure!(app.runners.connected(to), "the chosen runner is not connected");
    let mut lost_files = false;
    if app.runners.connected(&from) {
        let (status, body) = app.runners.request(&from, "POST", &format!("/api/sessions/{id}/handoff"), Some(serde_json::json!({"final": true}).to_string())).await?;
        ensure!((200..300).contains(&status), "{}", serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or(body));
    } else {
        let h = offline_copy(app, &s).await?;
        lost_files = !h.is_final && h.worktrees.iter().any(|w| w.state == WorktreeState::Active);
    }
    let s = app.sessions.reassign(app, id, to).await?;
    if let Some(pid) = &s.project_id {
        crate::projects::move_temp(app, pid, to).await?;
    }
    let (status, body) = app.runners.request(to, "POST", &format!("/api/sessions/{id}/adopt"), None).await?;
    ensure!((200..300).contains(&status), "{}", serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or(body));
    let name = if crate::runners::is_burst(app, to).await { "Burst".to_string() } else { crate::runners::list(app).await?.into_iter().find(|r| r.runner.id == to).map(|r| r.runner.name).unwrap_or_else(|| "another runner".into()) };
    let mut text = format!("Moved to {name}.");
    if lost_files {
        text.push_str(" Its previous runner was offline: files not committed there stay on that machine.");
    }
    crate::sessions::notice_anywhere(app, id, &text).await;
    Ok(s)
}

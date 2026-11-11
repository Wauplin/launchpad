//! Moving a session to another runner between turns (Move to…, Continue on Burst).
//!
//! The record and its events are on Home, the branches are on Home Git and plain folders are synced. What a
//! new runner still needs is a *handoff*: the harness's native conversation files, the session's attached
//! files and its worktree records. Runners upload one at the end of every turn (when the conversation changed; files Home already holds go by hash or as their appended tail), so a session can
//! continue elsewhere while its machine sleeps, and a final one before a planned move (branches pushed,
//! uncommitted files saved to a scratch branch, worktrees removed). On Home, a move increases the session's
//! generation and names the new runner, then that runner adopts the session: it fetches the handoff, records
//! the worktrees (cloned again from their branches on the next message, with the uncommitted files of their
//! last snapshot) and puts the conversation and attached files where its harness resumes them. Home refuses pushes, leases and records of the older generation.

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
    /// Runner to Home only: `data` is the new tail of an append-only file whose earlier content is the blob
    /// `base` of the session's latest handoff on Home; `sha256` names the whole file, which Home checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    pub session_id: String,
    pub generation: u32,
    pub harness_session_id: Option<String>,
    pub cwd: String,
    pub worktrees: Vec<TaskWorktree>,
    pub files: Vec<HandoffFile>,
    /// The session's attached files (`attachments::dir`), which its conversation names.
    #[serde(default)]
    pub attachments: Vec<HandoffFile>,
    /// Made for a planned move: branches pushed, leftovers saved, folders removed, no process left.
    #[serde(rename = "final")]
    pub is_final: bool,
    pub at: i64,
    /// A Git repository in the session's temporary folder (`plain::git_inside`), which Home's copy never holds:
    /// Home refuses Keep as folder while that runner is away (`projects::keep_away`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_git: Option<String>,
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
        // Refused before anything changes: Home's copy of a temporary folder never holds Git data.
        if let Some(pid) = &s.project_id
            && let Some(p) = crate::projects::get(app, pid).await?.filter(|p| p.is_temp)
        {
            let dir = std::path::PathBuf::from(&p.path);
            if let Some(git) = tokio::task::spawn_blocking(move || crate::plain::git_inside(&dir)).await? {
                bail!("Its temporary folder holds a Git repository ({git}), which Home does not store for a temporary folder, so it cannot move. Keep the folder as a folder first (a repository at its top becomes a Git folder), or push or delete the repository.");
            }
        }
        // A save of the last turn that runs now finishes first, and no retry starts during the move.
        let _saving = crate::turn_sync::hold(id).await;
        // Plain folders first: asset folders inside the worktrees go with them.
        crate::plain::after_turn(app, id).await.context("its files cannot move yet")?;
        for w in crate::worktrees::for_session(app, id).await?.into_iter().filter(|w| w.state == WorktreeState::Active) {
            crate::worktrees::remove(app, &w.id).await.with_context(|| format!("{} cannot move yet", w.branch))?;
        }
        let _action = app.sessions.action_lock(id).await;
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
    let temp_git = match &s.project_id {
        Some(pid) => match crate::projects::get(app, pid).await?.filter(|p| p.is_temp) {
            Some(p) => tokio::task::spawn_blocking(move || crate::plain::git_inside(std::path::Path::new(&p.path))).await?,
            None => None,
        },
        None => None,
    };
    Ok(Handoff {
        session_id: s.id.clone(),
        generation: s.generation,
        harness_session_id: s.harness_session_id.clone(),
        cwd: s.cwd.clone(),
        worktrees: crate::worktrees::for_session(app, id).await?,
        files: files.into_iter().map(|f| HandoffFile { name: f.name, data: base64::engine::general_purpose::STANDARD.encode(f.data), sha256: None, base: None }).collect(),
        attachments: read_attachments(app, id).await?,
        is_final: false,
        at: now_ms(),
        temp_git,
    })
}

/// The files of the session's attachment folder (flat: `attachments::prepare` writes no subfolders).
async fn read_attachments(app: &App, id: &str) -> Result<Vec<HandoffFile>> {
    let mut out = vec![];
    let Ok(mut rd) = tokio::fs::read_dir(crate::attachments::dir(app, id)).await else { return Ok(out) };
    while let Some(e) = rd.next_entry().await? {
        if !e.file_type().await?.is_file() {
            continue;
        }
        let data = tokio::fs::read(e.path()).await?;
        out.push(HandoffFile { name: e.file_name().to_string_lossy().into_owned(), data: base64::engine::general_purpose::STANDARD.encode(data), sha256: None, base: None });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Puts a handoff's attachments in the session's attachment folder on this runner, where its conversation names
/// them. Names are plain file names.
async fn write_attachments(app: &App, id: &str, files: &[HandoffFile]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let dir = crate::attachments::dir(app, id);
    crate::files::create_dir_all_mode(&dir, Some(0o700)).await?;
    for f in files {
        ensure!(!f.name.is_empty() && !f.name.contains(['/', '\\']) && f.name != "." && f.name != "..", "invalid attachment name {}", f.name);
        let data = base64::engine::general_purpose::STANDARD.decode(&f.data)?;
        crate::files::write_mode(&dir.join(&f.name), &data, Some(0o600)).await?;
    }
    Ok(())
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
    answer_or_refusal(r).await?.map_err(|e| anyhow::anyhow!("Home: {e}"))
}

/// Ok(Err(reason)): Home answered and refused.
async fn answer_or_refusal(r: reqwest::RequestBuilder) -> Result<std::result::Result<serde_json::Value, String>> {
    let res = r.send().await?;
    let status = res.status();
    let v: serde_json::Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        return Ok(Err(v["error"].as_str().unwrap_or(status.as_str()).to_string()));
    }
    Ok(Ok(v))
}

/// Attached files Home confirmed, by session: a later handoff names them by hash only (`send`).
static ATTACHED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::collections::HashSet<String>>>> = std::sync::LazyLock::new(Default::default);
/// Native files Home confirmed, by session and name (length, SHA-256): a later handoff sends only what was
/// appended since (`send`).
static CONFIRMED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Confirmed>>> = std::sync::LazyLock::new(Default::default);
type Confirmed = std::collections::HashMap<String, (usize, String)>;

/// Runner: what Home's latest handoff of a session holds (fetched to adopt or restore it), so the next handoff
/// sends only what changed since.
fn remember(id: &str, files: &[HandoffFile], attachments: &[HandoffFile]) {
    let decode = |f: &HandoffFile| base64::engine::general_purpose::STANDARD.decode(&f.data).ok();
    let files: Confirmed = files.iter().filter_map(|f| decode(f).map(|d| (f.name.clone(), (d.len(), hex::encode(Sha256::digest(&d)))))).collect();
    let attached = attachments.iter().filter_map(|f| decode(f).map(|d| hex::encode(Sha256::digest(&d)))).collect();
    CONFIRMED.lock().unwrap().insert(id.to_string(), files);
    ATTACHED.lock().unwrap().insert(id.to_string(), attached);
}

/// How a native file goes to Home, given what Home confirmed of it before.
#[derive(Debug, PartialEq)]
enum Part {
    /// Unchanged: by hash only.
    Same,
    /// Only appended to since: the bytes from this offset.
    Tail(usize),
    /// New, shorter or rewritten: the whole file.
    Full,
}

/// The part of `data` to send and its whole SHA-256 (the prefix Home holds is hashed once, on the way).
fn part(data: &[u8], known: Option<&(usize, String)>) -> (Part, String) {
    let mut hasher = Sha256::new();
    let Some((len, sha)) = known.filter(|(len, _)| *len <= data.len()) else {
        hasher.update(data);
        return (Part::Full, hex::encode(hasher.finalize()));
    };
    hasher.update(&data[..*len]);
    let prefix = hex::encode(hasher.clone().finalize());
    hasher.update(&data[*len..]);
    let whole = hex::encode(hasher.finalize());
    let part = if prefix != *sha {
        Part::Full
    } else if *len == data.len() {
        Part::Same
    } else {
        Part::Tail(*len)
    };
    (part, whole)
}

/// Sends a handoff to Home. Attached files Home already confirmed for the session go by hash only, and native
/// files it confirmed go by hash or as their appended tail. If Home lost one (or another runner's handoff
/// replaced them), it refuses, and the handoff goes again in full at once. A shortened handoff goes to
/// `/api/handoff/{id}/parts`, a path an older Home does not know: it refuses it, so it never stores a tail as a
/// whole file, and gets the full handoff.
async fn send(app: &App, id: &str, h: &Handoff) -> Result<()> {
    let known = ATTACHED.lock().unwrap().get(id).cloned().unwrap_or_default();
    let confirmed = CONFIRMED.lock().unwrap().get(id).cloned().unwrap_or_default();
    // `h` names nothing by hash (`snapshot`): it is the full handoff, sent when Home refuses the short one.
    let mut short = h.clone();
    let mut sent = std::collections::HashSet::new();
    let mut files = std::collections::HashMap::new();
    let mut shortened = false;
    for f in &mut short.attachments {
        let sha = hex::encode(Sha256::digest(base64::engine::general_purpose::STANDARD.decode(&f.data)?));
        if known.contains(&sha) {
            f.data.clear();
            f.sha256 = Some(sha.clone());
            shortened = true;
        }
        sent.insert(sha);
    }
    for f in &mut short.files {
        let data = base64::engine::general_purpose::STANDARD.decode(&f.data)?;
        let (part, sha) = part(&data, confirmed.get(&f.name));
        match part {
            Part::Same => {
                f.data.clear();
                f.sha256 = Some(sha.clone());
                shortened = true;
            }
            Part::Tail(at) => {
                f.data = base64::engine::general_purpose::STANDARD.encode(&data[at..]);
                f.base = confirmed.get(&f.name).map(|(_, sha)| sha.clone());
                f.sha256 = Some(sha.clone());
                shortened = true;
            }
            Part::Full => {}
        }
        files.insert(f.name.clone(), (data.len(), sha));
    }
    let post = |path: &str, h: &Handoff| -> Result<reqwest::RequestBuilder> { Ok(to_home(app, reqwest::Method::POST, path)?.json(h)) };
    let full = format!("/api/handoff/{id}");
    let mut result = if shortened { answer_or_refusal(post(&format!("{full}/parts"), &short)?).await } else { answer_or_refusal(post(&full, h)?).await };
    if shortened && matches!(result, Ok(Err(_))) {
        result = answer_or_refusal(post(&full, h)?).await;
    }
    let result = result.and_then(|r| r.map_err(|e| anyhow::anyhow!("Home: {e}")));
    match result {
        Ok(_) => {
            ATTACHED.lock().unwrap().insert(id.to_string(), sent);
            CONFIRMED.lock().unwrap().insert(id.to_string(), files);
        }
        Err(_) => {
            ATTACHED.lock().unwrap().remove(id);
            CONFIRMED.lock().unwrap().remove(id);
        }
    }
    result.map(|_| ())
}

/// Sends a handoff to Home.
pub async fn upload(app: &App, id: &str, is_final: bool) -> Result<()> {
    let h = export(app, id, is_final).await?;
    send(app, id, &h).await?;
    if is_final {
        // The final handoff holds everything: the turn is saved (sent to Home with the backlog below).
        if app.sessions.get(id).is_some_and(|s| s.unsaved.is_some()) {
            app.sessions.update(app, id, |s| s.unsaved = None).await?;
        }
        // Everything the runner still holds for Home goes before it lets the session go.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while crate::replica::backlog(app).await? > 0 {
            ensure!(std::time::Instant::now() < deadline, "Home has not saved this runner's latest events yet");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Ok(())
}

/// A final handoff on this runner, started by Home with a key (`final_job`).
struct FinalJob {
    key: String,
    /// None while it runs, then its result.
    done: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}

/// Final handoffs by session: they run on their own, so a move's outcome never depends on how long Home's
/// request waits (`final_on`).
static FINAL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, FinalJob>>> = std::sync::LazyLock::new(Default::default);

/// How long one request for a final handoff waits for it before answering "running".
const FINAL_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

/// What a request for a final handoff does, given the job this runner holds for the session (its key, and
/// whether it is done).
#[derive(Debug, PartialEq)]
enum FinalStep {
    /// Its own job: wait for it.
    Join,
    /// Another key's job still runs: wait for it, then answer "running" (the next request starts its own).
    WaitOther,
    /// Start a job with this key (none yet, or another key's job is done).
    Start,
}

fn final_step(job: Option<(&str, bool)>, key: &str) -> FinalStep {
    match job {
        Some((k, _)) if k == key => FinalStep::Join,
        Some((_, false)) => FinalStep::WaitOther,
        _ => FinalStep::Start,
    }
}

/// Runner: the final handoff of a session for a move, as a job Home polls with the same `key`. Ok(true): done;
/// Ok(false): still running (ask again). A result is answered once; asking again with the same key starts a new
/// final handoff, which is safe to repeat.
pub async fn final_job(app: &App, id: &str, key: &str) -> Result<bool> {
    let (mut rx, mine) = {
        let mut jobs = FINAL.lock().unwrap();
        let job = jobs.get(id).map(|j| (j.key.clone(), j.done.clone()));
        match final_step(job.as_ref().map(|(k, rx)| (k.as_str(), rx.borrow().is_some())), key) {
            FinalStep::Join => (job.map(|(_, rx)| rx).context("no job")?, true),
            FinalStep::WaitOther => (job.map(|(_, rx)| rx).context("no job")?, false),
            FinalStep::Start => {
                let (tx, rx) = tokio::sync::watch::channel(None);
                jobs.insert(id.to_string(), FinalJob { key: key.to_string(), done: rx.clone() });
                let (app, id) = (app.clone(), id.to_string());
                tokio::spawn(async move {
                    let result = upload(&app, &id, true).await.map_err(|e| format!("{e:#}"));
                    let _ = tx.send(Some(result));
                });
                (rx, true)
            }
        }
    };
    let _ = tokio::time::timeout(FINAL_WAIT, rx.wait_for(Option::is_some)).await.map(|r| r.map(|_| ()));
    let result = rx.borrow().clone();
    // A task that ended without a result (it panicked) leaves its job: the next request starts again.
    let gone = result.is_none() && rx.has_changed().is_err();
    let drop_job = || {
        let mut jobs = FINAL.lock().unwrap();
        if jobs.get(id).is_some_and(|j| j.done.same_channel(&rx)) {
            jobs.remove(id);
        }
    };
    match result {
        // Answered once: asking again with this key starts a new one.
        Some(r) if mine => {
            drop_job();
            r.map(|_| true).map_err(anyhow::Error::msg)
        }
        None if gone => {
            drop_job();
            ensure!(!mine, "the handoff stopped before it finished");
            Ok(false)
        }
        _ => Ok(false),
    }
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
    send(app, id, &h).await?;
    let _action = app.sessions.action_lock(id).await;
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
    ATTACHED.lock().unwrap().remove(id);
    CONFIRMED.lock().unwrap().remove(id);
    Ok(true)
}

/// Runner: puts back the native conversation of a session whose folder `clean_archived` removed, from Home's
/// copy, before its harness starts again.
pub async fn restore(app: &App, s: &Session) -> Result<()> {
    let a: Adoption = serde_json::from_value(answer(to_home(app, reqwest::Method::GET, &format!("/api/handoff/{}", s.id))?).await?)?;
    let h = a.handoff.filter(|h| h.generation == s.generation && h.harness_session_id == s.harness_session_id && !h.files.is_empty());
    let h = h.context("Home has no copy of this agent's conversation")?;
    remember(&s.id, &h.files, &h.attachments);
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

/// At the end of every turn on a runner (`turn_sync`): the conversation goes to Home when it changed. The
/// worktree records also reach Home on their own (`worktrees::save`), so a failure here never hides them.
pub async fn after_turn(app: &App, id: &str) -> Result<()> {
    if app.paths.role != crate::role::Role::Runner {
        return Ok(());
    }
    let h = snapshot(app, id).await.context("could not read the agent's conversation")?;
    let mut hasher = Sha256::new();
    for f in h.files.iter().chain(&h.attachments) {
        hasher.update(f.name.as_bytes());
        hasher.update(f.data.as_bytes());
    }
    for w in &h.worktrees {
        hasher.update(format!("{}:{:?}:{:?}", w.id, w.state, w.pushed).as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    if SENT.lock().unwrap().get(id) == Some(&digest) {
        return Ok(());
    }
    send(app, id, &h).await.context("could not send the conversation to Home")?;
    SENT.lock().unwrap().insert(id.to_string(), digest);
    Ok(())
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
/// conversation where its harness resumes it. Nothing happens when it already holds that generation.
pub async fn adopt(app: &App, id: &str) -> Result<()> {
    adopt_with(app, id, false).await
}

/// Runner: `adopt` for a request Home relayed about a session this runner does not hold (`home_client::local`).
/// Only a session Home moved (generation above 0): a new session's launch may still be on its way.
pub async fn adopt_moved(app: &App, id: &str) -> Result<()> {
    adopt_with(app, id, true).await
}

async fn adopt_with(app: &App, id: &str, moved_only: bool) -> Result<()> {
    ensure!(app.paths.role == crate::role::Role::Runner, "only runners take over sessions");
    // Home's adopt request, a relayed request that adopts lazily (`home_client::local`) and `release` take turns.
    let lock = app.suggestion_locks.get(&format!("adopt:{id}"));
    let _lock = lock.lock().await;
    let a: Adoption = serde_json::from_value(answer(to_home(app, reqwest::Method::GET, &format!("/api/handoff/{id}"))?).await?)?;
    let mut s = a.session;
    ensure!(!moved_only || s.generation > 0, "no such session on this runner");
    if app.sessions.get(id).is_some_and(|local| local.generation >= s.generation) {
        return Ok(());
    }
    s.runner_id = None;
    s.live = false;
    s.open_question = false;
    // It continues from what Home holds; work its previous runner did not save goes to `lp-scratch/…` (`release`).
    s.unsaved = None;
    // Its events continue after a gap, never over Home's numbers: events of the previous generation that Home
    // did not receive yet (its runner was offline) still fit below (`Sessions::apply_replica`).
    s.last_seq += crate::sessions::MOVE_SEQ_GAP;
    let (mut worktrees, files, attachments) = match a.handoff {
        Some(h) => (h.worktrees, h.files, h.attachments),
        None => (vec![], vec![], vec![]),
    };
    relocate(app, &mut s, &mut worktrees).await?;
    write_attachments(app, id, &attachments).await.context("restore the session's attached files")?;
    remember(id, &files, &attachments);
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

/// Runner: Home moved this session on while this runner was away (Home's generation is `generation`). What
/// still runs stops, each worktree's commits and uncommitted files go to `lp-scratch/…` on Home, the folders
/// and the local record go. When Home's record still names this runner (Home kept its temporary folder as a
/// folder), the session is adopted again from Home.
pub async fn release(app: &App, id: &str, generation: u32) -> Result<()> {
    ensure!(app.paths.role == crate::role::Role::Runner, "only runners release sessions");
    {
        // An adoption of the same session (Home moved it back here meanwhile) waits until the old copy is gone.
        let lock = app.suggestion_locks.get(&format!("adopt:{id}"));
        let _adopt = lock.lock().await;
        release_locked(app, id, generation).await?;
    }
    if let Err(e) = adopt(app, id).await {
        tracing::debug!("session {id} stays released: {e:#}");
    }
    Ok(())
}

async fn release_locked(app: &App, id: &str, generation: u32) -> Result<()> {
    let Some(s) = app.sessions.get(id) else { return Ok(()) };
    if s.generation >= generation {
        return Ok(());
    }
    if let Some(t) = app.terminals.tui_of(id) {
        app.terminals.close_and_wait(&[t]).await?;
    }
    {
        let _action = app.sessions.action_lock(id).await;
        app.sessions.detach(app, id).await?;
        crate::worktrees::release_moved(app, id, s.generation).await?;
    }
    // Its temporary folder leaves this disk when Home has every file, it holds no Git repository and no other
    // agent here uses it; otherwise it stays, with what Home lacks (`remove_temp_copy`).
    if let Some(pid) = &s.project_id
        && crate::projects::get(app, pid).await?.is_some_and(|p| p.is_temp)
        && !app.sessions.list().iter().any(|o| o.id != id && o.project_id.as_deref() == Some(pid.as_str()) && o.status != crate::model::SessionStatus::Closed)
        && let Err(e) = crate::projects::remove_temp_copy(app, pid, false).await
    {
        tracing::warn!("temporary folder of released session {id} stays: {e:#}");
    }
    app.sessions.forget_local(app, id).await?;
    SENT.lock().unwrap().remove(id);
    ATTACHED.lock().unwrap().remove(id);
    CONFIRMED.lock().unwrap().remove(id);
    let _ = tokio::fs::remove_dir_all(crate::contain::session_dir(app, id)).await;
    tracing::info!("released session {id}: Home moved it on (generation {generation})");
    Ok(())
}

// ------------------------------------------------------------------ Home

/// Home: stores a runner's handoff of a session it runs (current generation only).
/// The native files go to the blob store; the row keeps their names and hashes.
pub async fn store(app: &App, runner_id: &str, mut h: Handoff) -> Result<()> {
    let s = app.sessions.get(&h.session_id).context("no such session")?;
    ensure!(s.runner_id.as_deref() == Some(runner_id) && s.generation == h.generation, "this session runs elsewhere now");
    // A file named by hash only, or a tail on top of a base: the same file of this session's latest handoff on
    // Home (an attached file: one of its attached files), never another session's blob.
    let (native, attached) = latest_hashes(app, &h.session_id).await?;
    let files = h.files.iter_mut().map(|f| (f, true)).chain(h.attachments.iter_mut().map(|f| (f, false)));
    for (f, is_native) in files {
        let name = f.name.clone();
        let held = |sha: &str| if is_native { native.get(&name).is_some_and(|s| s == sha) } else { attached.contains(sha) };
        let given = base64::engine::general_purpose::STANDARD.decode(&f.data)?;
        let data = match (f.base.take(), &f.sha256) {
            (Some(base), Some(sha)) => {
                ensure!(is_native && held(&base), "Home does not have the start of {}", f.name);
                let mut data = crate::plain::read_blob(app, &base).await.with_context(|| format!("Home does not have the start of {}", f.name))?;
                data.extend_from_slice(&given);
                ensure!(hex::encode(Sha256::digest(&data)) == *sha, "{} changed differently than its new end says", f.name);
                data
            }
            (Some(_), None) => bail!("{} has a start but no hash", f.name),
            (None, Some(sha)) if given.is_empty() => {
                ensure!(held(sha) && crate::plain::have_blob(app, sha).await, "Home does not have the file {}", f.name);
                f.data.clear();
                continue;
            }
            (None, _) => given,
        };
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

/// Home: the hashes of the native files (by name) and of the attached files of a session's latest handoff.
async fn latest_hashes(app: &App, id: &str) -> Result<(std::collections::HashMap<String, String>, std::collections::HashSet<String>)> {
    let key = id.to_string();
    let json: Option<String> = app
        .state_db
        .call(move |c| Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT json FROM session_handoff WHERE session_id = ?1", [key], |r| r.get(0)))?))
        .await?;
    let Some(h) = json.and_then(|j| serde_json::from_str::<Handoff>(&j).ok()) else { return Ok(Default::default()) };
    let native = h.files.into_iter().filter_map(|f| Some((f.name, f.sha256?))).collect();
    Ok((native, h.attachments.into_iter().filter_map(|f| f.sha256).collect()))
}

/// Home: the latest handoff of a session.
pub async fn latest(app: &App, id: &str) -> Result<Option<Handoff>> {
    let key = id.to_string();
    let json: Option<String> = app
        .state_db
        .call(move |c| Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT json FROM session_handoff WHERE session_id = ?1", [key], |r| r.get(0)))?))
        .await?;
    let Some(mut h) = json.and_then(|j| serde_json::from_str::<Handoff>(&j).ok()) else { return Ok(None) };
    for f in h.files.iter_mut().chain(h.attachments.iter_mut()) {
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

/// Home: asks `runner` for the final handoff of session `id` (`final_job`, under `key`) until it is done or
/// `until` passed. Ok(false): it still runs there (ask again with the same key to learn its outcome).
pub async fn final_on(app: &App, runner: &str, id: &str, key: &str, until: std::time::Instant) -> Result<bool> {
    loop {
        let (status, body) = app.runners.request(runner, "POST", &format!("/api/sessions/{id}/handoff"), Some(serde_json::json!({"final": true, "job": key}).to_string())).await?;
        ensure!((200..300).contains(&status), "{}", error_of(&body));
        let running = serde_json::from_str::<serde_json::Value>(&body).is_ok_and(|v| v["state"] == "running");
        if !running {
            return Ok(true);
        }
        if std::time::Instant::now() >= until {
            return Ok(false);
        }
    }
}

fn error_of(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_else(|| body.to_string())
}

/// How long a move waits for the old runner's final handoff (pushes, leftovers, plain folders, conversation).
const FINAL_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Home: moves an idle session to `to` (a connected runner). The old runner prepares the move when it is
/// connected; otherwise its last handoff is used. Either way the old runner releases its copy when it is next
/// connected (`release`: unsaved work goes to `lp-scratch/…`). Moves and placements of a session take turns
/// (`runners::place_lock`), so two runners never adopt the same session.
pub async fn move_to(app: &App, id: &str, to: &str) -> Result<Session> {
    let lock = crate::runners::place_lock(app, id);
    let _lock = lock.lock().await;
    move_locked(app, id, to).await
}

/// `move_to`, for a caller that holds the session's placement lock (`runners::place`).
pub(crate) async fn move_locked(app: &App, id: &str, to: &str) -> Result<Session> {
    ensure!(app.paths.role == crate::role::Role::Home, "only Home moves sessions");
    let s = app.sessions.get(id).context("no such session")?;
    if let Some(reason) = blocked(&s) {
        bail!("{reason}");
    }
    let from = s.runner_id.clone().context("this session has no runner")?;
    ensure!(from != to, "the session already runs there");
    ensure!(app.runners.connected(to), "the chosen runner is not connected");
    let mut lost_files = false;
    let offline = !app.runners.connected(&from);
    if !offline {
        // A job on the runner: a slow handoff is waited for, never reported failed while it still runs there.
        let key = uuid::Uuid::new_v4().to_string();
        if !final_on(app, &from, id, &key, std::time::Instant::now() + FINAL_LIMIT).await? {
            bail!("{} is still saving this session for the move: try again in a few minutes.", crate::runners::runner_name(app, &from).await);
        }
    } else {
        let h = offline_copy(app, &s).await?;
        lost_files = !h.is_final && h.worktrees.iter().any(|w| w.state == WorktreeState::Active);
    }
    let s = app.sessions.reassign(app, id, to).await?;
    // The old runner still holds the older generation: it releases it when it is connected (at once if it is, or
    // came back during the move).
    crate::runners::release_later(app, &from, id, s.generation).await?;
    if app.runners.connected(&from) {
        let (app, from) = (app.clone(), from.clone());
        tokio::spawn(async move { crate::runners::deliver_pending(&app, &from).await });
    }
    if let Some(pid) = &s.project_id {
        crate::projects::move_temp(app, pid, to).await?;
    }
    // A failed adoption is not the end of the move: the new runner adopts the session when Home relays its next
    // request (`home_client::local` on the runner).
    let adopted = match app.runners.request(to, "POST", &format!("/api/sessions/{id}/adopt"), None).await {
        Ok((status, _)) if (200..300).contains(&status) => None,
        Ok((_, body)) => Some(error_of(&body)),
        Err(e) => Some(format!("{e:#}")),
    };
    let name = if crate::runners::is_burst(app, to).await { "Burst".to_string() } else { crate::runners::list(app).await?.into_iter().find(|r| r.runner.id == to).map(|r| r.runner.name).unwrap_or_else(|| "another runner".into()) };
    let mut text = format!("Moved to {name}.");
    if let Some(why) = adopted {
        tracing::warn!("session {id}: adoption on {to} failed: {why}");
        text.push_str(&format!(" {name} could not take it over yet ({why}): it tries again with the next message."));
    }
    if lost_files {
        text.push_str(" Its previous runner was offline: it continues from its last saved turn and Home's last snapshot of its uncommitted files; work that runner did not save goes to an lp-scratch branch on Home when it is back.");
    }
    crate::sessions::notice_anywhere(app, id, &text).await;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::{FinalStep, Part, final_step, part};
    use sha2::{Digest, Sha256};

    #[test]
    fn only_the_appended_tail_of_a_file_home_holds_is_sent() {
        let sha = |d: &[u8]| hex::encode(Sha256::digest(d));
        let known = (5, sha(b"hello"));
        assert_eq!(part(b"hello world", Some(&known)), (Part::Tail(5), sha(b"hello world")));
        assert_eq!(part(b"hello", Some(&known)), (Part::Same, sha(b"hello")));
        // Rewritten, cut or never sent: the whole file.
        assert_eq!(part(b"jello world", Some(&known)).0, Part::Full);
        assert_eq!(part(b"hell", Some(&known)).0, Part::Full);
        assert_eq!(part(b"hello", None), (Part::Full, sha(b"hello")));
    }

    #[test]
    fn a_final_handoff_joins_its_own_job_and_never_runs_twice_at_once() {
        assert_eq!(final_step(None, "a"), FinalStep::Start);
        assert_eq!(final_step(Some(("a", false)), "a"), FinalStep::Join);
        assert_eq!(final_step(Some(("a", true)), "a"), FinalStep::Join);
        // Another move's handoff still runs: wait for it, never start a second one beside it.
        assert_eq!(final_step(Some(("a", false)), "b"), FinalStep::WaitOther);
        assert_eq!(final_step(Some(("a", true)), "b"), FinalStep::Start);
    }
}

//! Task worktrees owned by Launchpad (docs/managed-worktrees-plan.md). The daemon creates them before an
//! agent starts, the same way for every harness, and owns merging, hand-off, promotion, cleanup and
//! restoration. Agents only commit on their branch; Git operations that need waiting run here, without
//! spending agent turns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::App;
use crate::model::{EventBody, Project, Session, SessionStatus, WorktreeOptions};
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TaskWorktree {
    pub id: String,
    /// Sessions working in it: the launching session first, then continuations with another agent.
    pub sessions: Vec<String>,
    pub project_id: String,
    /// Top level of the project's own checkout. Repository commands and hand-off run there.
    pub repo: String,
    /// Shared Git directory, which identifies the repository.
    pub common_dir: String,
    pub root: String,
    /// The project folder below the repository top level ("" at the top), with `/` separators.
    pub subdir: String,
    pub branch: String,
    /// The branch it started from, where merges go. None: started from a detached checkout.
    pub base_branch: Option<String>,
    pub base_commit: String,
    /// The session's working directory, rather than an editable context project.
    pub primary: bool,
    pub state: WorktreeState,
    pub merge: Option<MergeRequest>,
    pub merged: Option<Merged>,
    pub predraft_id: Option<String>,
    /// During a hand-off: what the project checkout had checked out before (a branch, or a commit).
    pub handoff_from: Option<String>,
    /// Why the last automatic cleanup kept the folder.
    pub cleanup_note: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// The folder exists.
    Active,
    /// The clean folder was removed; the branch keeps the commits and Resume restores the folder.
    Removed,
    /// The branch is checked out in the project checkout instead (Check out locally).
    HandedOff,
    /// Promoted to a pre-draft PR, which now owns the folder and branch.
    Predraft,
    /// Folder and branch deleted by an explicit Discard.
    Discarded,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct MergeRequest {
    pub phase: MergePhase,
    /// Why the merge waits or stopped.
    pub reason: String,
    /// Requested by the agent with `launchpad work finish`: conflicts go back to it.
    pub by_agent: bool,
    /// Conflict messages already sent to the agent for this request.
    pub agent_retries: u32,
    #[ts(type = "number")]
    pub requested_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergePhase {
    /// Retried automatically: the agent is running, or the target checkout is busy or has changes.
    Waiting,
    /// Rebasing onto the target conflicts: the agent (or the owner) must rebase.
    Conflict,
    /// Something unexpected; the owner decides.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Merged {
    pub into: String,
    pub head: String,
    #[ts(type = "number")]
    pub at: i64,
}

/// A worktree with its current Git state, for the session view and settings.
#[derive(Clone, Debug, Serialize, TS)]
pub struct WorktreeInfo {
    pub worktree: TaskWorktree,
    pub project_name: String,
    /// The folder agents work in (root plus the project subdirectory).
    pub path: String,
    pub exists: bool,
    pub dirty_files: u32,
    /// Commits of the branch missing from the base branch, and the reverse.
    pub ahead: u32,
    pub behind: u32,
    /// The checkout that has the base branch checked out: where a merge lands.
    pub target_checkout: Option<String>,
}

const MAX_AGENT_RETRIES: u32 = 3;

async fn git(app: &App, dir: &str, args: &[&str]) -> Result<String> {
    crate::procs::output(&app.env, "git", args, Some(Path::new(dir))).await
}

async fn canonical(path: &str) -> Result<String> {
    Ok(crate::hostfs::canonicalize(path).await.with_context(|| format!("{path} does not exist"))?.to_string_lossy().into_owned())
}

/// A path printed by git in `dir`, on the machine of `dir`.
fn on(dir: &str, native: &str) -> String {
    crate::hosts::loc::rehost(dir, native)
}

pub fn folder(w: &TaskWorktree) -> String {
    folder_in(&w.root, &w.subdir)
}

fn folder_in(root: &str, subdir: &str) -> String {
    if subdir.is_empty() { root.to_string() } else { Path::new(root).join(subdir).to_string_lossy().into_owned() }
}

pub async fn list(app: &App) -> Result<Vec<TaskWorktree>> {
    let mut rows: Vec<TaskWorktree> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM task_worktree")?;
            let texts = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(texts.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await?;
    rows.sort_by_key(|w| std::cmp::Reverse(w.created_at));
    Ok(rows)
}

pub async fn get(app: &App, id: &str) -> Result<TaskWorktree> {
    list(app).await?.into_iter().find(|w| w.id == id).context("worktree record not found")
}

pub async fn for_session(app: &App, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let mut rows: Vec<_> = list(app).await?.into_iter().filter(|w| w.sessions.iter().any(|s| s == session_id)).collect();
    rows.sort_by_key(|w| (!w.primary, w.created_at));
    Ok(rows)
}

/// The record whose folder contains `path` (Active or handed off).
pub async fn find_by_path(app: &App, path: &str) -> Result<Option<TaskWorktree>> {
    Ok(list(app).await?.into_iter().find(|w| w.state == WorktreeState::Active && crate::terminals::inside(path, &w.root)))
}

pub async fn save(app: &App, w: &TaskWorktree) -> Result<()> {
    let (id, text) = (w.id.clone(), serde_json::to_string(w)?);
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO task_worktree (id, json) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET json = excluded.json", rusqlite::params![id, text])?;
            Ok(())
        })
        .await?;
    app.bus.publish("worktrees", "updated", json!(w));
    Ok(())
}

async fn notice(app: &App, w: &TaskWorktree, text: &str) {
    for id in &w.sessions {
        if app.sessions.get(id).is_some() {
            app.sessions.sink(app, id).emit(EventBody::Notice { text: text.to_string() }).await;
        }
    }
}

/// Repository identity: top level and shared Git directory of a project folder.
async fn repository(app: &App, path: &str) -> Result<(String, String)> {
    let top = git(app, path, &["rev-parse", "--show-toplevel"]).await.map_err(|_| anyhow::anyhow!("{path} is not in a Git repository; run in the project checkout instead"))?;
    let common = git(app, path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
    Ok((canonical(&on(path, &top)).await?, canonical(&on(path, &common)).await?))
}

async fn subdir_of(project_path: &str, top: &str) -> String {
    let project = crate::hostfs::canonicalize(project_path).await.unwrap_or_else(|_| PathBuf::from(project_path));
    project.strip_prefix(top).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).unwrap_or_default()
}

/// Resolve editable context by repository, including projects sharing the primary worktree.
pub(crate) async fn context_folder(app: &App, records: &[TaskWorktree], project: &Project) -> Result<String> {
    let (top, common) = repository(app, &project.path).await?;
    let w = records.iter().find(|w| w.common_dir == common && matches!(w.state, WorktreeState::Active | WorktreeState::HandedOff))
        .with_context(|| format!("editable context project {} has no active worktree; restore it or remove it from the task before resuming", project.name))?;
    let root = if w.state == WorktreeState::HandedOff { &w.repo } else { &w.root };
    Ok(folder_in(root, &subdir_of(&project.path, &top).await))
}

/// Creates the worktree of `project` for a session, or returns the one it already has in the same
/// repository. Returns the record and the project folder inside it. Never edits the source checkout.
pub async fn create(app: &App, session_id: &str, project: &Project, opts: &WorktreeOptions, primary: bool, title: &str) -> Result<(TaskWorktree, String)> {
    let _lock = app.worktree_lock.lock().await;
    let (top, common_dir) = repository(app, &project.path).await?;
    let subdir = subdir_of(&project.path, &top).await;
    if let Some(w) = for_session(app, session_id).await?.into_iter().find(|w| w.common_dir == common_dir && w.state == WorktreeState::Active) {
        return Ok((w.clone(), folder_in(&w.root, &subdir)));
    }
    git(app, &top, &["rev-parse", "--verify", "HEAD^{commit}"]).await.map_err(|_| anyhow::anyhow!("{} has no commit yet; run in the project checkout instead", project.name))?;
    let current = git(app, &top, &["branch", "--show-current"]).await.unwrap_or_default();
    let base_branch = opts.base.clone().filter(|b| !b.is_empty()).or_else(|| (!current.is_empty()).then(|| current.clone()));
    if let Some(b) = &base_branch {
        ensure!(!b.starts_with('-'), "invalid base branch");
    }
    let base_ref = base_branch.as_deref().map(|b| format!("refs/heads/{b}^{{commit}}")).unwrap_or_else(|| "HEAD^{commit}".into());
    let base_commit = git(app, &top, &["rev-parse", "--verify", &base_ref]).await.with_context(|| format!("branch {} not found", base_branch.as_deref().unwrap_or("HEAD")))?;
    let id = uuid::Uuid::new_v4().to_string();
    let name = if primary { crate::compose::slug_from(title) } else { format!("{}-{}", crate::compose::slug_from(&project.name), crate::compose::slug_from(title)) };
    let name: String = name.chars().take(48).collect::<String>().trim_matches('-').to_string();
    let slug = format!("{}-{}", if name.is_empty() { "task" } else { &name }, &session_id[..8.min(session_id.len())]);
    let branch = format!("lp/{slug}");
    ensure!(git(app, &top, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_err(), "branch {branch} already exists");
    let parent = crate::projects::projects_root(app, crate::hosts::loc::host_of(&project.path))?.join(".worktrees").join(&project.id);
    crate::hostfs::create_dir_all(&parent).await?;
    let root_path = parent.join(&slug);
    ensure!(!crate::hostfs::exists(&root_path).await, "{} already exists", root_path.display());
    let root = root_path.to_string_lossy().into_owned();
    git(app, &top, &["worktree", "add", "-b", &branch, &root, &base_commit]).await?;
    let root = canonical(&root).await?;
    let w = TaskWorktree {
        id,
        sessions: vec![session_id.to_string()],
        project_id: project.id.clone(),
        repo: top.clone(),
        common_dir,
        root: root.clone(),
        subdir: subdir.clone(),
        branch: branch.clone(),
        base_branch: base_branch.clone(),
        base_commit: base_commit.clone(),
        primary,
        state: WorktreeState::Active,
        merge: None,
        merged: None,
        predraft_id: None,
        handoff_from: None,
        cleanup_note: None,
        created_at: now_ms(),
    };
    let prepared = async {
        if opts.include_changes && primary && base_branch.as_deref().is_none_or(|b| b == current) {
            copy_changes(app, &top, &root).await?;
        }
        copy_included(app, &top, &root).await?;
        save(app, &w).await
    }
    .await;
    if let Err(e) = prepared {
        let _ = git(app, &top, &["worktree", "remove", "--force", &root]).await;
        let _ = git(app, &top, &["branch", "-D", &branch]).await;
        return Err(e);
    }
    Ok((w, folder_in(&root, &subdir)))
}

/// Applies the source checkout's staged, unstaged and untracked changes to a fresh worktree.
async fn copy_changes(app: &App, source: &str, dest: &str) -> Result<()> {
    let out = crate::procs::command(&app.env, "git", Some(Path::new(source))).args(["diff", "--binary", "HEAD"]).output().await?;
    ensure!(out.status.success(), "git diff failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    if !out.stdout.is_empty() {
        let applied = crate::procs::command(&app.env, "git", Some(Path::new(dest))).args(["apply", "--binary", "--whitespace=nowarn", "-"]).stdin_bytes(out.stdout).output().await?;
        ensure!(applied.status.success(), "could not copy uncommitted changes: {}", String::from_utf8_lossy(&applied.stderr).trim());
    }
    let untracked = crate::procs::command(&app.env, "git", Some(Path::new(source))).args(["ls-files", "-z", "--others", "--exclude-standard"]).output().await?;
    copy_files(source, dest, &untracked.stdout).await?;
    Ok(())
}

/// Copies the ignored files listed by `.worktreeinclude` (gitignore patterns, as in Codex) and an ignored
/// `AGENTS.override.md`: symlinks are skipped and existing files are never overwritten.
async fn copy_included(app: &App, source: &str, dest: &str) -> Result<()> {
    copy_files(source, dest, &included_files(app, source).await?).await
}

async fn included_files(app: &App, source: &str) -> Result<Vec<u8>> {
    let ignored = crate::procs::command(&app.env, "git", Some(Path::new(source))).args(["ls-files", "-z", "--others", "--ignored", "--exclude-standard"]).output().await?;
    ensure!(ignored.status.success(), "could not list ignored worktree files");
    let ignored: std::collections::HashSet<&[u8]> = ignored.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let mut wanted: Vec<u8> = Vec::new();
    if ignored.contains(b"AGENTS.override.md".as_slice()) {
        wanted.extend_from_slice(b"AGENTS.override.md\0");
    }
    let include = Path::new(source).join(".worktreeinclude");
    if crate::hostfs::is_file(&include).await {
        let listed = crate::procs::command(&app.env, "git", Some(Path::new(source)))
            .args(["ls-files", "-z", "--others", "--ignored", "--exclude-from"])
            .arg(&include)
            .output()
            .await?;
        ensure!(listed.status.success(), "could not apply .worktreeinclude rules");
        for p in listed.stdout.split(|b| *b == 0).filter(|p| !p.is_empty() && ignored.contains(p)) {
            wanted.extend_from_slice(p);
            wanted.push(0);
        }
    }
    Ok(wanted)
}

async fn copy_files(source: &str, dest: &str, list: &[u8]) -> Result<()> {
    let dest_root = crate::hostfs::canonicalize(dest).await?;
    for rel in list.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let rel = String::from_utf8_lossy(rel);
        let rel = Path::new(rel.as_ref());
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            continue;
        }
        let (from, to) = (Path::new(source).join(rel), Path::new(dest).join(rel));
        let Ok(meta) = crate::hostfs::symlink_metadata(&from).await else { continue };
        if !meta.is_file() || crate::hostfs::exists(&to).await {
            continue;
        }
        // Refuse symlink parents before creating directories or writing files.
        let mut parent = PathBuf::from(dest);
        if let Some(parts) = rel.parent() {
            for part in parts.components() {
                parent.push(part);
                if let Ok(meta) = crate::hostfs::symlink_metadata(&parent).await {
                    ensure!(meta.is_dir(), "copy destination {} is not a regular directory", parent.display());
                } else {
                    crate::hostfs::create_dir_all(&parent).await?;
                }
                ensure!(crate::hostfs::canonicalize(&parent).await?.starts_with(&dest_root), "copy destination is outside the worktree");
            }
        }
        crate::hostfs::copy(&from, &to).await.with_context(|| format!("copy {}", rel.display()))?;
    }
    Ok(())
}

/// Removes worktrees created for a launch that failed before its agent started.
pub async fn abandon(app: &App, created: &[TaskWorktree]) {
    let _lock = app.worktree_lock.lock().await;
    for w in created {
        let result: Result<()> = async {
            if crate::hostfs::exists(&w.root).await {
                git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
            }
            if git(app, &w.repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", w.branch)]).await.is_ok() {
                git(app, &w.repo, &["branch", "-D", &w.branch]).await?;
            }
            let id = w.id.clone();
            app.state_db.call(move |c| Ok(c.execute("DELETE FROM task_worktree WHERE id = ?1", [id])?)).await?;
            app.bus.publish("worktrees", "removed", json!({"id": w.id}));
            Ok(())
        }.await;
        if let Err(e) = result { tracing::warn!("could not clean up failed launch worktree {}: {e:#}", w.root); }
    }
}

/// Remove a failed launch's membership from worktrees shared with an older session.
pub(crate) async fn leave(app: &App, session_id: &str) -> Result<()> {
    let _lock = app.worktree_lock.lock().await;
    for mut w in for_session(app, session_id).await?.into_iter().filter(|w| w.sessions.iter().any(|id| id != session_id)) {
        w.sessions.retain(|id| id != session_id);
        save(app, &w).await?;
    }
    Ok(())
}

/// A continuation with another agent works in the same worktrees.
pub async fn join(app: &App, source_session: &str, session_id: &str) -> Result<Vec<TaskWorktree>> {
    let _lock = app.worktree_lock.lock().await;
    let mut joined = vec![];
    for mut w in for_session(app, source_session).await? {
        if !w.sessions.iter().any(|s| s == session_id) {
            w.sessions.push(session_id.to_string());
            save(app, &w).await?;
        }
        joined.push(w);
    }
    Ok(joined)
}

/// Launch instructions: where to work, and how the task ends. Overrides checkout workflow rules only.
pub fn instruction(app: &App, session_id: &str, worktrees: &[(TaskWorktree, String)], projects: &[Project], merge_when_done: bool, changes_copied: bool) -> String {
    let cli = worktrees.first().map(|(w, _)| app.cli_command_in(&w.root)).unwrap_or_else(|| app.paths.cli_command());
    let name = |id: &str| projects.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| "project".into());
    let mut text = String::from(
        "Launchpad worktree: Launchpad created dedicated Git worktrees for this task and manages them. This launch setting overrides repository or preset instructions to work on the main branch, in the main checkout, or to create, merge or delete worktrees yourself; follow all other repository rules (checks, documentation, commit conventions).",
    );
    for (w, path) in worktrees {
        text.push_str(&format!(
            "\n- {}{}: work in `{path}` on branch `{}`, created from {} ({}).",
            name(&w.project_id),
            if w.primary { " (your working directory)" } else { " (editable)" },
            w.branch,
            w.base_branch.as_deref().map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached checkout".into()),
            &w.base_commit[..12.min(w.base_commit.len())]
        ));
    }
    text.push_str("\nCommit your work on these branches. Do not push, switch branches, or remove these worktrees. The other branches and the shared history stay readable, for example with `git log` or `git show`.");
    if changes_copied {
        text.push_str(" The uncommitted changes of the project checkout were copied into your worktree; they are not committed yet.");
    }
    text.push_str(&format!(
        "\nTo modify another registered project, first run `{cli} work add-project --session-id {session_id} --project <name>`. It prints a dedicated worktree folder: edit that folder, never the original checkout."
    ));
    let bases: Vec<String> = worktrees.iter().filter_map(|(w, _)| w.base_branch.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().map(|b| format!("`{b}`")).collect();
    let base = if bases.is_empty() { "its base branch".to_string() } else { bases.join(", ") };
    if merge_when_done {
        text.push_str(&format!(
            "\nWhen the task is done and checked, commit everything, then run `{cli} work finish --session-id {session_id}`. Launchpad merges your branches into {base} after your turn ends, as soon as the target checkout is clean. Do not merge yourself, and report the merge as requested, not as done. If Launchpad later reports a conflict, rebase onto the target branch in your worktree, resolve it, re-run your checks, commit, and run the finish command again."
        ));
    } else {
        text.push_str(&format!("\nDo not merge into {base} yourself: the owner merges, publishes or discards the branch from Launchpad."));
    }
    text
}

/// Runs the owner-configured setup command of a project in a new worktree folder.
pub async fn run_setup(app: &App, project: &Project, folder: &str, sink: &crate::sessions::Sink, cancelled: &mut tokio::sync::watch::Receiver<bool>) -> Result<()> {
    anyhow::ensure!(!*cancelled.borrow(), "setup cancelled");
    let command = project.worktree_setup.trim();
    if command.is_empty() {
        return Ok(());
    }
    sink.emit(EventBody::Notice { text: format!("Running the worktree setup of {}: {command}", project.name) }).await;
    let started = std::time::Instant::now();
    let mut cmd = if cfg!(windows) && crate::hosts::loc::host_of(folder).is_none() {
        let mut c = crate::procs::child_command(&app.env, "powershell", Some(Path::new(folder)));
        c.args(["-NoProfile", "-Command", command]);
        c
    } else {
        let mut c = crate::procs::child_command(&app.env, "sh", Some(Path::new(folder)));
        c.args(["-c", command]);
        c
    };
    cmd.process_tree().stdin(crate::exec::Io::Null).stdout(crate::exec::Io::Piped).stderr(crate::exec::Io::Piped).env("LAUNCHPAD_WORKTREE", folder).env("LAUNCHPAD_SOURCE_CHECKOUT", &project.path);
    let out = setup_output(&mut cmd, cancelled, Duration::from_secs(30 * 60), Some((app.sessions.clone(), sink.id.clone()))).await?;
    finish_setup(sink, command, started, out).await
}

async fn setup_output(cmd: &mut crate::exec::Cmd, cancelled: &mut tokio::sync::watch::Receiver<bool>, timeout: Duration, owner: Option<(crate::sessions::Sessions, String)>) -> Result<crate::exec::Output> {
    anyhow::ensure!(!*cancelled.borrow(), "setup cancelled");
    let child = cmd.spawn().await?;
    let killer = child.killer();
    let mut output = tokio::spawn(child.wait_with_output());
    let reason = tokio::select! {
        out = &mut output => return Ok(out??),
        _ = cancelled.wait_for(|cancel| *cancel) => "setup cancelled",
        _ = tokio::time::sleep(timeout) => "setup did not finish within 30 minutes",
    };
    // Keep ownership until exit is confirmed, including SSH helper-owned processes. Dropping a
    // remote Exec request cannot cancel it; setup deliberately uses Spawn instead.
    killer.kill().await;
    if let Some((sessions, id)) = owner {
        sessions.wait_for_shutdown(&id, async move { output.await??; Ok(()) }).await.with_context(|| reason)?;
    } else {
        let _ = tokio::time::timeout(Duration::from_secs(5), output).await;
    }
    bail!("{reason}")
}

async fn finish_setup(sink: &crate::sessions::Sink, command: &str, started: std::time::Instant, out: crate::exec::Output) -> Result<()> {
    let mut log = String::from_utf8_lossy(&out.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    let tail = crate::sessions::truncate_output(log.trim(), 4000);
    if !out.status.success() {
        bail!("{} exited with {}\n{tail}", command, out.status);
    }
    sink.emit(EventBody::Notice { text: format!("Worktree setup finished in {} s", started.elapsed().as_secs()) }).await;
    Ok(())
}

/// Sessions of a record are idle enough for Git operations on its folder.
fn idle_reason(app: &App, w: &TaskWorktree, strict: bool) -> Option<String> {
    for id in &w.sessions {
        let Some(s) = app.sessions.get(id) else { continue };
        if app.sessions.is_busy(&s) || app.sessions.setup_running(id) {
            return Some("an agent is running in this worktree".into());
        }
        if s.status == SessionStatus::InTerminal || s.quota_pause.is_some() {
            return Some("an agent of this worktree is quota paused or open in a terminal".into());
        }
        if strict && s.open_question {
            return Some("an agent of this worktree waits for an answer".into());
        }
    }
    if !app.terminals.alive_in(if w.state == WorktreeState::HandedOff { &w.repo } else { &w.root }).is_empty() {
        return Some("a terminal is open in this worktree".into());
    }
    None
}

async fn dirty_files(app: &App, dir: &str) -> Result<Vec<String>> {
    Ok(git(app, dir, &["status", "--porcelain", "--untracked-files=all"]).await?.lines().map(|l| l.get(3..).unwrap_or(l).to_string()).collect())
}

/// A rebase, merge or cherry-pick stopped in this worktree.
async fn operation_in_progress(app: &App, dir: &str) -> bool {
    for marker in ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD"] {
        if let Ok(p) = git(app, dir, &["rev-parse", "--path-format=absolute", "--git-path", marker]).await
            && crate::hostfs::exists(on(dir, &p)).await
        {
            return true;
        }
    }
    false
}

/// The checkout that has `branch` checked out, from `git worktree list`.
async fn checkout_of(app: &App, repo: &str, branch: &str) -> Result<Option<String>> {
    let list = git(app, repo, &["worktree", "list", "--porcelain"]).await?;
    let mut path = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(on(repo, p));
        } else if line.strip_prefix("branch refs/heads/") == Some(branch) {
            return Ok(path);
        }
    }
    Ok(None)
}

async fn count(app: &App, dir: &str, range: &str) -> u32 {
    git(app, dir, &["rev-list", "--count", range, "--"]).await.ok().and_then(|n| n.parse().ok()).unwrap_or(0)
}

pub async fn info(app: &App, w: TaskWorktree) -> WorktreeInfo {
    let project_name = crate::projects::get(app, &w.project_id).await.ok().flatten().map(|p| p.name).unwrap_or_else(|| "Removed project".into());
    let path = folder(&w);
    let exists = crate::hostfs::is_dir(&w.root).await;
    let dir = if exists { w.root.clone() } else { w.repo.clone() };
    let dirty_files = if exists { dirty_files(app, &w.root).await.map(|f| f.len() as u32).unwrap_or(0) } else { 0 };
    let (mut ahead, mut behind, mut target_checkout) = (0, 0, None);
    if let Some(base) = &w.base_branch
        && git(app, &dir, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", w.branch)]).await.is_ok()
    {
        ahead = count(app, &dir, &format!("refs/heads/{base}..refs/heads/{}", w.branch)).await;
        behind = count(app, &dir, &format!("refs/heads/{}..refs/heads/{base}", w.branch)).await;
        target_checkout = checkout_of(app, &dir, base).await.ok().flatten();
    }
    WorktreeInfo { worktree: w, project_name, path, exists, dirty_files, ahead, behind, target_checkout }
}

pub async fn session_info(app: &App, session_id: &str) -> Result<Vec<WorktreeInfo>> {
    let mut out = vec![];
    for w in for_session(app, session_id).await? {
        out.push(info(app, w).await);
    }
    Ok(out)
}

/// `launchpad work add-project`: a worktree for another registered project, during the task.
pub async fn add_project(app: &App, session_id: &str, project: &str) -> Result<String> {
    let s = app.sessions.get(session_id).context("no such session")?;
    let projects = crate::projects::list(app).await?;
    let wanted = project.trim();
    let matches: Vec<&Project> = projects.iter().filter(|p| p.id == wanted || p.name.eq_ignore_ascii_case(wanted)).collect();
    let p = match matches.as_slice() {
        [p] => (*p).clone(),
        [] => bail!("no registered project named {wanted:?}; registered projects: {}", projects.iter().filter(|p| !p.is_temp).map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")),
        _ => bail!("several projects are named {wanted:?}; use the project ID"),
    };
    let base = s.launch_spec.context_projects.iter().find(|c| c.project_id == p.id).and_then(|c| c.branch.clone());
    let (w, path) = create(app, session_id, &p, &WorktreeOptions { base, ..Default::default() }, false, &s.title).await?;
    let sink = app.sessions.sink(app, session_id);
    sink.emit(EventBody::Notice { text: format!("Added a worktree for {} on branch {}: {path}", p.name, w.branch) }).await;
    let (_cancel, mut cancelled) = tokio::sync::watch::channel(false);
    if let Err(e) = run_setup(app, &p, &path, &sink, &mut cancelled).await {
        return Ok(format!("Worktree for {} on branch `{}`: {path}\nIts setup command failed; fix the environment yourself if needed:\n{e:#}", p.name, w.branch));
    }
    Ok(format!("Worktree for {} on branch `{}`: {path}\nEdit this folder, never the original checkout {}. Commit your work on this branch.", p.name, w.branch, p.path))
}

/// Restores the removed worktrees of a session before it resumes. Returns whether a folder was restored.
pub async fn restore_for_session(app: &App, session_id: &str) -> Result<bool> {
    let mut restored = false;
    for w in for_session(app, session_id).await? {
        if w.state == WorktreeState::Removed {
            restore(app, &w.id).await?;
            restored = true;
        }
    }
    Ok(restored)
}

/// Recreates a removed folder from its branch; a merged and deleted branch restarts from its base.
pub async fn restore(app: &App, id: &str) -> Result<TaskWorktree> {
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    if w.state == WorktreeState::Active && crate::hostfs::is_dir(&w.root).await {
        return Ok(w);
    }
    ensure!(w.state == WorktreeState::Removed, "this worktree cannot be restored");
    ensure!(crate::hostfs::is_dir(&w.repo).await, "the project checkout {} no longer exists", w.repo);
    ensure!(!crate::hostfs::exists(&w.root).await, "{} already exists", w.root);
    let _ = git(app, &w.repo, &["worktree", "prune"]).await;
    if git(app, &w.repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", w.branch)]).await.is_ok() {
        git(app, &w.repo, &["worktree", "add", &w.root, &w.branch]).await?;
    } else {
        let base = w.base_branch.clone().context("the branch was deleted and there is no base branch to restart from")?;
        git(app, &w.repo, &["worktree", "add", "-b", &w.branch, &w.root, &base]).await?;
    }
    copy_included(app, &w.repo, &w.root).await?;
    w.state = WorktreeState::Active;
    w.cleanup_note = None;
    save(app, &w).await?;
    notice(app, &w, &format!("Worktree restored from branch {}. Ignored files such as installed dependencies were not restored; run the project setup again if needed.", w.branch)).await;
    Ok(w)
}

/// Removes a clean folder; the branch keeps its commits. Fails with the reason when not safe.
async fn remove_clean(app: &App, w: &mut TaskWorktree) -> Result<()> {
    ensure!(w.state == WorktreeState::Active, "this worktree is not active");
    if let Some(reason) = idle_reason(app, w, true) {
        bail!("{reason}");
    }
    ensure!(w.merge.is_none(), "a merge is pending");
    if crate::hostfs::exists(&w.root).await {
        let dirty = dirty_files(app, &w.root).await?;
        ensure!(dirty.is_empty(), "{} uncommitted files", dirty.len());
        ensure!(!operation_in_progress(app, &w.root).await, "a Git operation is in progress");
        let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
        ensure!(branch == w.branch, "the worktree is on another branch ({branch})");
        for id in w.sessions.clone() {
            app.sessions.release(app, &id).await?;
        }
        git(app, &w.repo, &["worktree", "remove", &w.root]).await?;
    }
    w.state = WorktreeState::Removed;
    w.cleanup_note = None;
    // A branch fully merged into its base is not needed to restore: the folder restarts from the base.
    if let Some(base) = &w.base_branch
        && git(app, &w.repo, &["merge-base", "--is-ancestor", &format!("refs/heads/{}", w.branch), &format!("refs/heads/{base}")]).await.is_ok()
    {
        let _ = git(app, &w.repo, &["branch", "-d", &w.branch]).await;
    }
    save(app, w).await
}

/// Settings → Worktrees "Remove folder": a clean, non-destructive removal.
pub async fn remove(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    remove_clean(app, &mut w).await?;
    Ok(w)
}

/// Deletes the folder and branch, with uncommitted work, and archives the sessions. Owner confirmation only.
pub async fn discard(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    // Promotion holds this lock before changing the task record's ownership.
    let _predraft = app.predraft_lock.lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    ensure!(matches!(w.state, WorktreeState::Active | WorktreeState::Removed), "only an active or removed worktree can be discarded");
    for sid in &w.sessions {
        if let Some(s) = app.sessions.get(sid)
            && (app.sessions.is_busy(&s) || s.quota_pause.is_some() || s.status == SessionStatus::InTerminal || app.sessions.setup_running(sid))
        {
            bail!("stop the agents of this worktree first");
        }
    }
    if crate::hostfs::exists(&w.root).await {
        ensure!(w.state == WorktreeState::Active, "a folder appeared at the removed worktree path; inspect it before discarding");
        let (top, common) = repository(app, &w.root).await?;
        ensure!(top == canonical(&w.root).await? && common == w.common_dir, "the folder no longer belongs to this worktree");
        ensure!(git(app, &w.root, &["branch", "--show-current"]).await? == w.branch, "the worktree is on another branch; inspect it before discarding");
    }
    app.terminals.close_in(&w.root).await?;
    // Without its working directory an agent cannot continue: archive it. A discarded editable
    // context project only detaches idle processes.
    if w.primary {
        for sid in w.sessions.clone() {
            if app.sessions.get(&sid).is_some_and(|s| s.status != SessionStatus::Closed) {
                app.sessions.close_locked(app, &sid).await?;
            }
        }
    }
    if !w.primary {
        for sid in w.sessions.clone() {
            app.sessions.release(app, &sid).await?;
        }
    }
    if crate::hostfs::exists(&w.root).await {
        git(app, &w.repo, &["worktree", "remove", "--force", &w.root]).await?;
    }
    if git(app, &w.repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", w.branch)]).await.is_ok() {
        git(app, &w.repo, &["branch", "-D", &w.branch]).await?;
    }
    w.state = WorktreeState::Discarded;
    w.merge = None;
    save(app, &w).await?;
    Ok(w)
}

/// Check out locally: moves the branch into the project checkout so the owner can run it there.
pub async fn handoff(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    ensure!(w.state == WorktreeState::Active && w.primary, "only the active worktree of a session can be checked out locally");
    ensure!(w.merge.is_none(), "cancel the pending merge first");
    if let Some(reason) = idle_reason(app, &w, true) {
        bail!("{reason}");
    }
    let dirty = dirty_files(app, &w.root).await?;
    ensure!(dirty.is_empty(), "commit or discard the {} uncommitted files of the worktree first", dirty.len());
    let local = dirty_files(app, &w.repo).await?;
    ensure!(local.is_empty(), "the project checkout {} has {} uncommitted files; commit or stash them first", w.repo, local.len());
    ensure!(app.terminals.alive_in(&w.repo).is_empty(), "a terminal is open in the project checkout");
    let ignored = git(app, &w.root, &["ls-files", "--others", "--ignored", "--exclude-standard"]).await?;
    ensure!(ignored.is_empty(), "the worktree contains ignored files; move or remove them before checking out locally to avoid losing them");
    let busy_here = app.sessions.list().iter().any(|s| !w.sessions.contains(&s.id) && crate::terminals::inside(&s.cwd, &w.repo) && app.sessions.is_busy(s));
    ensure!(!busy_here, "an agent is running in the project checkout");
    let previous = match git(app, &w.repo, &["branch", "--show-current"]).await? {
        b if b.is_empty() => git(app, &w.repo, &["rev-parse", "HEAD"]).await?,
        b => b,
    };
    for sid in w.sessions.clone() {
        app.sessions.release(app, &sid).await?;
    }
    app.terminals.close_in(&w.root).await?;
    git(app, &w.repo, &["worktree", "remove", &w.root]).await?;
    if let Err(e) = git(app, &w.repo, &["switch", "--no-overwrite-ignore", &w.branch]).await {
        git(app, &w.repo, &["worktree", "add", &w.root, &w.branch]).await?;
        return Err(e);
    }
    move_sessions(app, &w, &w.root, &w.repo).await?;
    w.state = WorktreeState::HandedOff;
    w.handoff_from = Some(previous.clone());
    save(app, &w).await?;
    notice(app, &w, &format!("Branch {} is now checked out in {} (previously {previous}). Use Move back to worktree to return.", w.branch, w.repo)).await;
    Ok(w)
}

/// Returns a handed-off branch to its worktree and restores the checkout's previous branch.
pub async fn handback(app: &App, id: &str) -> Result<TaskWorktree> {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    ensure!(w.state == WorktreeState::HandedOff, "this branch is not checked out locally");
    if let Some(reason) = idle_reason(app, &w, true) {
        bail!("{reason}");
    }
    ensure!(app.terminals.alive_in(&w.repo).is_empty(), "a terminal is open in the project checkout");
    ensure!(!app.sessions.list().iter().any(|s| !w.sessions.contains(&s.id) && crate::terminals::inside(&s.cwd, &w.repo) && app.sessions.is_busy(s)), "an agent is running in the project checkout");
    let current = git(app, &w.repo, &["branch", "--show-current"]).await?;
    ensure!(current == w.branch, "the project checkout is on {current:?}, not {}", w.branch);
    let local = dirty_files(app, &w.repo).await?;
    ensure!(local.is_empty(), "the project checkout has {} uncommitted files; commit them on {} first", local.len(), w.branch);
    // Read the task branch's include rules before switching the project checkout away from it.
    let included = included_files(app, &w.repo).await?;
    let previous = w.handoff_from.clone().context("previous checkout unknown")?;
    let is_branch = git(app, &w.repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{previous}")]).await.is_ok();
    for sid in w.sessions.clone() {
        app.sessions.release(app, &sid).await?;
    }
    if is_branch {
        git(app, &w.repo, &["switch", "--no-overwrite-ignore", &previous]).await?;
    } else {
        git(app, &w.repo, &["switch", "--no-overwrite-ignore", "--detach", &previous]).await?;
    }
    git(app, &w.repo, &["worktree", "add", &w.root, &w.branch]).await?;
    let copied = copy_files(&w.repo, &w.root, &included).await;
    move_sessions(app, &w, &w.repo, &w.root).await?;
    w.state = WorktreeState::Active;
    w.handoff_from = None;
    save(app, &w).await?;
    copied.context("branch moved back to its worktree, but copying included files failed")?;
    notice(app, &w, &format!("Branch {} is back in its worktree {}.", w.branch, w.root)).await;
    Ok(w)
}

async fn move_sessions(app: &App, w: &TaskWorktree, from: &str, to: &str) -> Result<()> {
    for sid in &w.sessions {
        if let Some(s) = app.sessions.get(sid)
            && crate::terminals::inside(&s.cwd, from)
        {
            let rest = s.cwd[from.len()..].to_string();
            let cwd = format!("{to}{rest}");
            app.sessions.update(app, sid, move |s| s.cwd = cwd).await?;
        }
    }
    Ok(())
}

/// Merge into base: the owner's button, or `by_agent` from `launchpad work finish`. Returns at once; the
/// merge runs as soon as the agents are idle and the target checkout is clean.
pub async fn request_merge(app: &App, id: &str, by_agent: bool) -> Result<TaskWorktree> {
    let mut w = {
        let _lock = app.worktree_lock.lock().await;
        let mut w = get(app, id).await?;
        ensure!(w.state == WorktreeState::Active, "only an active worktree can be merged");
        let base = w.base_branch.clone().context("this worktree started from a detached checkout: there is no branch to merge into")?;
        ensure!(crate::hostfs::is_dir(&w.root).await, "the worktree folder is missing");
        let retries = w.merge.as_ref().filter(|m| m.by_agent && by_agent).map(|m| m.agent_retries).unwrap_or(0);
        w.merge = Some(MergeRequest { phase: MergePhase::Waiting, reason: format!("Waiting to merge into {base}"), by_agent, agent_retries: retries, requested_at: now_ms() });
        save(app, &w).await?;
        w
    };
    if !by_agent {
        attempt(app, &w.id).await;
        w = get(app, id).await?;
    } else {
        let (app, id) = (app.clone(), w.id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            attempt(&app, &id).await;
        });
    }
    Ok(w)
}

/// `launchpad work finish`: requests the merge of every worktree of the session.
pub async fn finish(app: &App, session_id: &str) -> Result<String> {
    let all = for_session(app, session_id).await?;
    let records: Vec<_> = all.into_iter().filter(|w| w.state == WorktreeState::Active && w.base_branch.is_some()).collect();
    ensure!(!records.is_empty(), "this session has no Launchpad worktree to merge");
    for w in &records {
        let dirty = dirty_files(app, &w.root).await?;
        ensure!(dirty.is_empty(), "{} has uncommitted changes ({}); commit or remove them, then run finish again", w.root, dirty.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
        ensure!(!operation_in_progress(app, &w.root).await, "{}: finish the Git operation in progress first", w.root);
        let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
        ensure!(branch == w.branch, "{} must stay on branch {} (it is on {branch:?})", w.root, w.branch);
    }
    let mut lines = vec![];
    for w in &records {
        request_merge(app, &w.id, true).await?;
        lines.push(format!("- {} into {}", w.branch, w.base_branch.as_deref().unwrap_or_default()));
    }
    Ok(format!(
        "Merge requested:\n{}\nLaunchpad merges after your turn ends, when the target checkout is clean. End your turn now and report the merge as requested, not as done. Do not merge yourself.",
        lines.join("\n")
    ))
}

pub async fn cancel_merge(app: &App, id: &str) -> Result<TaskWorktree> {
    let _lock = app.worktree_lock.lock().await;
    let mut w = get(app, id).await?;
    w.merge = None;
    save(app, &w).await?;
    notice(app, &w, &format!("Merge of {} cancelled.", w.branch)).await;
    Ok(w)
}

fn conflict_message(app: &App, w: &TaskWorktree, reason: &str, session_id: &str) -> String {
    format!(
        "Launchpad could not merge branch `{}` into `{}`: {reason}\nIn your worktree `{}`, rebase onto `{}` (for example `git rebase {}`), resolve the conflicts, re-run the relevant checks, and commit. Then run `{} work finish --session-id {session_id}` again. Do not merge into `{}` yourself.\nWhen you finish this recovery turn, make your final message a concise, self-contained summary of the original task and this recovery: lead with the overall outcome, preserve the substantive results from your previous task summary, and update the checks actually performed and their results, remaining work or limitations, and merge status. Report the merge as requested, not completed. The user sees this as the run's final message, so include the task results even though this is a follow-up turn.",
        w.branch,
        w.base_branch.as_deref().unwrap_or_default(),
        w.root,
        w.base_branch.as_deref().unwrap_or_default(),
        w.base_branch.as_deref().unwrap_or_default(),
        app.cli_command_in(&w.root),
        w.base_branch.as_deref().unwrap_or_default(),
    )
}

/// Fix with agent: sends the conflict to the worktree's latest session, under normal quota protection.
pub async fn fix_with_agent(app: &App, id: &str) -> Result<()> {
    let w = get(app, id).await?;
    let m = w.merge.clone().context("no merge problem to fix")?;
    ensure!(m.phase != MergePhase::Waiting, "the merge is still waiting");
    let session = w.sessions.last().context("this worktree has no session")?.clone();
    app.sessions.send(app, &session, &conflict_message(app, &w, &m.reason, &session), vec![]).await
}

/// One merge attempt. Never stashes, resets, cleans or commits anyone's changes, and never forces.
async fn attempt(app: &App, id: &str) {
    let outcome = {
        let _action = app.sessions.action_lock().await;
        let _lock = app.worktree_lock.lock().await;
        let Ok(mut w) = get(app, id).await else { return };
        let Some(m) = w.merge.clone() else { return };
        if m.phase != MergePhase::Waiting {
            return;
        }
        let result = try_merge(app, &w).await;
        let result = match result {
            Ok(Step::Merged(head)) => {
                let into = w.base_branch.clone().unwrap_or_default();
                w.merge = None;
                w.merged = Some(Merged { into: into.clone(), head: head.clone(), at: now_ms() });
                notice(app, &w, &format!("Launchpad merged {} into {into} ({}).", w.branch, &head[..12.min(head.len())])).await;
                None
            }
            Ok(Step::Wait(reason)) => {
                w.merge = Some(MergeRequest { reason, ..m });
                None
            }
            Ok(Step::Conflict(reason)) => {
                let send = m.by_agent && m.agent_retries < MAX_AGENT_RETRIES;
                w.merge = Some(MergeRequest { phase: MergePhase::Conflict, reason: reason.clone(), agent_retries: m.agent_retries + send as u32, ..m });
                notice(app, &w, &format!("Merge of {} stopped: {reason}", w.branch)).await;
                send.then(|| (w.sessions.last().cloned(), reason))
            }
            Err(e) => {
                w.merge = Some(MergeRequest { phase: MergePhase::Failed, reason: format!("{e:#}"), ..m });
                notice(app, &w, &format!("Merge of {} failed: {e:#}", w.branch)).await;
                None
            }
        };
        if let Err(e) = save(app, &w).await {
            tracing::warn!("worktree merge {id}: {e:#}");
        }
        result.map(|r| (w, r))
    };
    // The agent fixes conflicts itself, after the locks are released.
    if let Some((w, (Some(session), reason))) = outcome
        && let Err(e) = app.sessions.send_follow_up(app, &session, &conflict_message(app, &w, &reason, &session), false).await
    {
        tracing::warn!("worktree conflict message {session}: {e:#}");
    }
}

enum Step {
    Merged(String),
    Wait(String),
    Conflict(String),
}

async fn try_merge(app: &App, w: &TaskWorktree) -> Result<Step> {
    let base = w.base_branch.clone().context("no base branch")?;
    if let Some(reason) = idle_reason(app, w, false) {
        return Ok(Step::Wait(format!("Waiting: {reason}")));
    }
    ensure!(crate::hostfs::is_dir(&w.root).await, "the worktree folder is missing");
    let dirty = dirty_files(app, &w.root).await?;
    if !dirty.is_empty() {
        return Ok(Step::Conflict(format!("the worktree has {} uncommitted files ({}); commit them first", dirty.len(), dirty.iter().take(5).cloned().collect::<Vec<_>>().join(", "))));
    }
    ensure!(!operation_in_progress(app, &w.root).await, "a Git operation is in progress in the worktree");
    let branch = git(app, &w.root, &["branch", "--show-current"]).await?;
    ensure!(branch == w.branch, "the worktree is on {branch:?} instead of {}", w.branch);
    let Some(target) = checkout_of(app, &w.repo, &base).await? else {
        return Ok(Step::Wait(format!("Waiting: no checkout has {base} checked out. Check out {base} in {} to continue.", w.repo)));
    };
    let target_dirty = dirty_files(app, &target).await?;
    if !target_dirty.is_empty() {
        return Ok(Step::Wait(format!("Waiting: {target} has {} uncommitted files (another agent may be working there).", target_dirty.len())));
    }
    if operation_in_progress(app, &target).await {
        return Ok(Step::Wait(format!("Waiting: a Git operation is in progress in {target}.")));
    }
    if app.sessions.list().iter().any(|s| !w.sessions.contains(&s.id) && crate::terminals::inside(&s.cwd, &target) && !crate::terminals::inside(&s.cwd, &w.root) && app.sessions.is_busy(s)) {
        return Ok(Step::Wait(format!("Waiting: an agent is running in {target}.")));
    }
    if !app.terminals.alive_in(&target).is_empty() {
        return Ok(Step::Wait(format!("Waiting: a terminal is open in {target}.")));
    }
    let head = git(app, &w.root, &["rev-parse", "HEAD"]).await?;
    let base_head = git(app, &w.root, &["rev-parse", &format!("refs/heads/{base}")]).await?;
    if git(app, &w.root, &["merge-base", "--is-ancestor", &head, &base_head]).await.is_ok() {
        return Ok(Step::Merged(base_head));
    }
    let needs_rebase = git(app, &w.root, &["merge-base", "--is-ancestor", &base_head, &head]).await.is_err();
    if needs_rebase && !git(app, &w.root, &["rev-list", "--merges", &format!("{base_head}..{head}"), "--"]).await?.is_empty() {
        bail!("branch {} contains merge commits; integrate {base} manually while preserving merge resolutions, then request the merge again", w.branch);
    }
    if needs_rebase
        && let Err(e) = git(app, &w.root, &["rebase", &base_head]).await
    {
        let files = git(app, &w.root, &["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default();
        git(app, &w.root, &["rebase", "--abort"]).await.context("could not abort the failed rebase")?;
        let files = files.lines().take(10).collect::<Vec<_>>().join(", ");
        return Ok(Step::Conflict(if files.is_empty() { format!("rebasing onto {base} failed: {e:#}") } else { format!("rebasing onto {base} conflicts in {files}") }));
    }
    let head = git(app, &w.root, &["rev-parse", "HEAD"]).await?;
    git(app, &target, &["merge", "--ff-only", &head]).await.with_context(|| format!("fast-forward of {base} in {target}"))?;
    Ok(Step::Merged(head))
}

/// Promotes the session's worktree into a pre-draft, which then owns its folder and branch.
pub async fn mark_predraft(app: &App, root: &str, predraft_id: &str) -> Result<()> {
    let _lock = app.worktree_lock.lock().await;
    if let Some(mut w) = list(app).await?.into_iter().find(|w| w.root == root && w.state == WorktreeState::Active) {
        w.state = WorktreeState::Predraft;
        w.predraft_id = Some(predraft_id.to_string());
        w.merge = None;
        save(app, &w).await?;
    }
    Ok(())
}

/// After Archive: remove the clean worktrees whose sessions are all archived.
pub async fn after_archive(app: &App, session_id: &str) {
    let Ok(records) = for_session(app, session_id).await else { return };
    for w in records.into_iter().filter(|w| w.state == WorktreeState::Active) {
        let archived = w.sessions.iter().all(|id| app.sessions.get(id).is_none_or(|s| s.status == SessionStatus::Closed));
        if archived {
            cleanup(app, &w.id).await;
        }
    }
}

async fn cleanup(app: &App, id: &str) {
    let _action = app.sessions.action_lock().await;
    let _lock = app.worktree_lock.lock().await;
    let Ok(mut w) = get(app, id).await else { return };
    if let Err(e) = remove_clean(app, &mut w).await {
        let note = format!("Kept: {e:#}");
        if w.cleanup_note.as_deref() != Some(note.as_str()) {
            w.cleanup_note = Some(note);
            let _ = save(app, &w).await;
        }
    }
}

fn last_activity(app: &App, w: &TaskWorktree) -> i64 {
    w.sessions.iter().filter_map(|id| app.sessions.get(id)).map(|s: Session| s.status_since).max().unwrap_or(w.created_at)
}

/// Retention: keeps the newest `keep` clean worktrees of idle agents; older clean ones are removed (Resume
/// restores them). Dirty, busy and pending-merge worktrees always stay.
async fn retention(app: &App) {
    let keep = app.config.get().worktrees.keep as usize;
    let Ok(records) = list(app).await else { return };
    let mut active: Vec<_> = records.into_iter().filter(|w| w.state == WorktreeState::Active).collect();
    active.sort_by_key(|w| std::cmp::Reverse(last_activity(app, w)));
    let archived = |w: &TaskWorktree| w.sessions.iter().all(|id| app.sessions.get(id).is_none_or(|s| s.status == SessionStatus::Closed));
    for (i, w) in active.iter().enumerate() {
        if archived(w) || (keep > 0 && i >= keep) {
            cleanup(app, &w.id).await;
        }
    }
}

/// Retries waiting merges every 15 s and applies retention every hour (and at startup).
pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            if tick.is_multiple_of(240) {
                retention(&app).await;
            }
            if let Ok(records) = list(&app).await {
                for w in records.iter().filter(|w| w.merge.as_ref().is_some_and(|m| m.phase == MergePhase::Waiting)) {
                    attempt(&app, &w.id).await;
                }
            }
            tick += 1;
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
}

/// Disk usage of the worktree folders, in bytes (Settings → Worktrees).
pub async fn sizes(app: &App) -> Result<HashMap<String, u64>> {
    let records: Vec<TaskWorktree> = list(app).await?.into_iter().filter(|w| w.state == WorktreeState::Active).collect();
    let roots: Vec<String> = records.iter().map(|w| w.root.clone()).collect();
    let sizes = crate::hostfs::disk_usage(&roots).await;
    Ok(records.into_iter().zip(sizes).map(|(w, s)| (w.id, s)).collect())
}

#[cfg(all(test, target_os = "linux"))]
mod setup_tests {
    use super::*;

    // Use real shell descendants, including one that ignores TERM. No harness or daemon is started.
    async fn check_setup_shutdown(mode: &str) {
        let pid_file = std::env::temp_dir().join(format!("launchpad-setup-{}", uuid::Uuid::new_v4()));
        let env = crate::shellenv::UserEnv::resolve().await;
        let mut cmd = crate::procs::child_command(&env, "sh", None);
        let script = if mode == "escaped" {
            r#"setsid sh -c 'echo $$ > "$1"; while :; do sleep 1; done' child "$1" & wait"#
        } else if mode == "complete" {
            r#"sh -c 'trap "" TERM; echo $$ > "$1"; while :; do sleep 1; done' child "$1" >/dev/null 2>&1 & while ! test -s "$1"; do sleep 0.01; done"#
        } else {
            r#"sh -c 'trap "" TERM; echo $$ > "$1"; while :; do sleep 1; done' child "$1" & wait"#
        };
        cmd.process_tree().args(["-c", script, "setup"])
            .arg(&pid_file).stdin(crate::exec::Io::Null).stdout(crate::exec::Io::Piped).stderr(crate::exec::Io::Piped);
        let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
        let timeout = mode == "timeout";
        let task = tokio::spawn(async move {
            setup_output(&mut cmd, &mut cancelled, if timeout { Duration::from_millis(500) } else { Duration::from_secs(5) }, None).await
        });
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(s) = tokio::fs::read_to_string(&pid_file).await
                    && let Ok(pid) = s.trim().parse::<u32>() { break pid; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("setup descendant started");
        if matches!(mode, "cancel" | "escaped") { cancel.send(true).unwrap(); }
        let result = tokio::time::timeout(Duration::from_secs(3), task).await;
        // This descendant deliberately escaped our group; close it even if the assertion fails.
        if mode == "escaped" { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        let result = result.unwrap().unwrap();
        if mode == "complete" {
            assert!(result.unwrap().status.success());
        } else {
            assert!(result.unwrap_err().to_string().contains(if timeout { "30 minutes" } else { "cancelled" }));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await;
                // An orphan can remain a zombie until init reaps it; it cannot keep writing.
                if stat.is_err() || stat.unwrap().split(") ").nth(1).is_some_and(|s| s.starts_with('Z')) { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("setup descendant stopped before cleanup");
        let _ = tokio::fs::remove_file(pid_file).await;
    }

    #[tokio::test]
    async fn setup_cancellation_does_not_wait_for_escaped_pipe_owners() { check_setup_shutdown("escaped").await; }

    #[tokio::test]
    async fn setup_cancellation_stops_descendants() { check_setup_shutdown("cancel").await; }

    #[tokio::test]
    async fn setup_timeout_stops_descendants() { check_setup_shutdown("timeout").await; }

    #[tokio::test]
    async fn setup_exit_stops_detached_descendants() { check_setup_shutdown("complete").await; }
}

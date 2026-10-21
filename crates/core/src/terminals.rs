//! Integrated terminals (ARCHITECTURE.md, 6.5): shells and harness TUIs in a PTY owned by the daemon.
//!
//! Output stays in memory (1 MB per terminal) and goes to each viewer over its own WebSocket, never
//! through the global bus, which only carries list changes. Restarting the daemon ends every terminal.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tokio::sync::broadcast;
use ts_rs::TS;

use crate::App;
use crate::model::Session;
use crate::util::now_ms;

const BUFFER: usize = 1 << 20;
const MAX_OPEN: usize = 12;
pub const INPUT_CHUNK: usize = 64 * 1024;

// One writer per PTY, at most 256 KiB pending. No viewer waits on a blocking write.
fn input_writer(mut writer: Box<dyn Write + Send>) -> std::sync::mpsc::SyncSender<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
    std::thread::spawn(move || {
        for data in rx {
            if writer.write_all(&data).and_then(|_| writer.flush()).is_err() { break; }
        }
    });
    tx
}

#[derive(Default)]
pub(crate) struct PtyClose {
    #[cfg_attr(not(unix), allow(dead_code))]
    pub started: bool,
    pub worker: Option<std::thread::JoinHandle<()>>,
}

/// Capture job-control's foreground group before the shell exits and the PTY disappears.
/// Waiters join the escalation before announcing exit, even when the shell exits on TERM.
#[cfg(unix)]
pub(crate) fn stop_pty(pid: u32, master: Option<&(dyn MasterPty + Send)>) -> std::thread::JoinHandle<()> {
    let mut groups = vec![pid as i32];
    if let Some(fd) = master.and_then(|m| m.as_raw_fd()) {
        let foreground = unsafe { libc::tcgetpgrp(fd) };
        if foreground > 0 && !groups.contains(&foreground) { groups.push(foreground); }
    }
    for group in &groups {
        unsafe {
            libc::kill(-*group, libc::SIGHUP);
            libc::kill(-*group, libc::SIGTERM);
        }
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        for group in groups { unsafe { libc::kill(-group, libc::SIGKILL); } }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalKind {
    Shell,
    Command,
    Tui,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct TerminalInfo {
    pub id: String,
    pub kind: TerminalKind,
    pub title: String,
    pub cwd: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub alive: bool,
    pub exit_code: Option<u32>,
    pub created_at: i64,
}

/// What a viewer receives after its snapshot.
#[derive(Clone, Debug)]
pub enum Output {
    Data(Arc<[u8]>),
    Exit(Option<u32>),
    /// The effective PTY size or its owner changed. Viewers that don't own it render at this size.
    Size { rows: u16, cols: u16, owner: Option<u64> },
}

/// The viewers of one terminal. One PTY has one size: it follows the owner, the viewer that typed
/// last (or clicked Use this size). A viewer joining a terminal without owner becomes the owner.
#[derive(Default)]
struct Viewers {
    next: u64,
    /// The size each viewer would fit in its container, once it has reported one.
    wanted: HashMap<u64, Option<(u16, u16)>>,
    owner: Option<u64>,
}

struct Term {
    info: Mutex<TerminalInfo>,
    /// The last output (oldest bytes dropped first). Viewers subscribe while holding this lock,
    /// so the snapshot and the live stream neither overlap nor leave a gap.
    buffer: Mutex<Buffer>,
    tx: broadcast::Sender<Output>,
    writer: Mutex<Option<std::sync::mpsc::SyncSender<Vec<u8>>>>,
    #[cfg_attr(not(unix), allow(dead_code))]
    closing: Mutex<PtyClose>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    size: Mutex<PtySize>,
    viewers: Mutex<Viewers>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    /// A PTY owned by the helper of an SSH host: input, size and signals go through the connection.
    remote: Option<(Arc<crate::hosts::Conn>, Arc<crate::hosts::RemoteProc>)>,
}

/// The retained output and the absolute number of bytes the terminal ever produced, so a viewer
/// can resume from the byte offset it already has.
#[derive(Default)]
struct Buffer {
    bytes: VecDeque<u8>,
    /// Offset just after the last byte (bytes produced since the start).
    end: u64,
}

impl Buffer {
    fn start(&self) -> u64 {
        self.end - self.bytes.len() as u64
    }
}

#[derive(Clone, Default)]
pub struct Terminals {
    inner: Arc<Mutex<HashMap<String, Arc<Term>>>>,
}

/// A viewer: everything printed so far (or since its offset), then the live stream.
pub struct Attach {
    pub snapshot: Vec<u8>,
    /// The absolute offset of `snapshot[0]`; live output continues at `offset + snapshot.len()`.
    pub offset: u64,
    /// Whether `snapshot` continues from the requested offset (no reset needed).
    pub resumed: bool,
    pub rows: u16,
    pub cols: u16,
    pub exit: Option<Option<u32>>,
    /// The viewer whose size the PTY follows.
    pub owner: Option<u64>,
    pub rx: broadcast::Receiver<Output>,
}

impl Terminals {
    pub fn list(&self) -> Vec<TerminalInfo> {
        let mut v: Vec<TerminalInfo> = self.inner.lock().unwrap().values().map(|t| t.info.lock().unwrap().clone()).collect();
        v.sort_by_key(|t| t.created_at);
        v
    }

    fn get(&self, id: &str) -> Result<Arc<Term>> {
        self.inner.lock().unwrap().get(id).cloned().context("no such terminal")
    }

    /// Alive terminals whose folder is `path` or inside it.
    pub fn alive_in(&self, path: &str) -> Vec<String> {
        self.list().into_iter().filter(|t| t.alive && inside(&t.cwd, path)).map(|t| t.id).collect()
    }

    pub fn alive_count(&self) -> usize {
        self.list().iter().filter(|t| t.alive).count()
    }

    /// The running TUI of a session, if any.
    pub fn tui_of(&self, session_id: &str) -> Option<String> {
        self.list().into_iter().find(|t| t.alive && t.kind == TerminalKind::Tui && t.session_id.as_deref() == Some(session_id)).map(|t| t.id)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.get(id).is_ok()
    }

    /// Subscribes a viewer. With `from`, an offset the viewer already has, the snapshot holds only
    /// the bytes after it when the buffer still contains them; otherwise the whole buffer.
    pub fn attach(&self, id: &str, from: Option<u64>) -> Result<Attach> {
        let t = self.get(id)?;
        let buffer = t.buffer.lock().unwrap();
        let rx = t.tx.subscribe();
        let info = t.info.lock().unwrap();
        let exit = (!info.alive).then_some(info.exit_code);
        let resume = from.filter(|&o| o >= buffer.start() && o <= buffer.end);
        let offset = resume.unwrap_or(buffer.start());
        let snapshot = buffer.bytes.range((offset - buffer.start()) as usize..).copied().collect();
        let size = *t.size.lock().unwrap();
        let owner = t.viewers.lock().unwrap().owner;
        Ok(Attach { snapshot, offset, resumed: resume.is_some(), rows: size.rows.max(2), cols: size.cols.max(2), exit, owner, rx })
    }

    /// Registers a viewer. It owns the size when nobody else does (the first viewer, or after the
    /// owner left). Call `leave` when it disconnects.
    pub fn join(&self, id: &str) -> Result<u64> {
        let t = self.get(id)?;
        let _buffer = t.buffer.lock().unwrap();
        let mut v = t.viewers.lock().unwrap();
        v.next += 1;
        let me = v.next;
        v.wanted.insert(me, None);
        if v.owner.is_none() {
            v.owner = Some(me);
            announce(&t, &v);
        }
        Ok(me)
    }

    /// Releases a viewer and its ownership. A single remaining viewer takes over; with several,
    /// the size stays until one of them types or claims it.
    pub fn leave(&self, id: &str, viewer: u64) {
        let Ok(t) = self.get(id) else { return };
        let _buffer = t.buffer.lock().unwrap();
        let mut v = t.viewers.lock().unwrap();
        v.wanted.remove(&viewer);
        if v.owner != Some(viewer) {
            return;
        }
        v.owner = if v.wanted.len() == 1 { v.wanted.keys().next().copied() } else { None };
        take_size(&t, &v);
        announce(&t, &v);
    }

    /// Records the size a viewer would fit. The PTY only follows its owner.
    pub fn resize(&self, id: &str, viewer: u64, rows: u16, cols: u16) -> Result<()> {
        let t = self.get(id)?;
        let _buffer = t.buffer.lock().unwrap();
        let mut v = t.viewers.lock().unwrap();
        v.wanted.insert(viewer, Some((rows, cols)));
        if v.owner == Some(viewer) && take_size(&t, &v) {
            announce(&t, &v);
        }
        Ok(())
    }

    /// Makes a viewer the owner (it typed, pasted or clicked Use this size) and applies its size.
    pub fn claim(&self, id: &str, viewer: u64) -> Result<()> {
        let t = self.get(id)?;
        let _buffer = t.buffer.lock().unwrap();
        let mut v = t.viewers.lock().unwrap();
        if !v.wanted.contains_key(&viewer) {
            return Ok(());
        }
        // Announce even when it already owns the size: the viewer waits for an answer.
        v.owner = Some(viewer);
        take_size(&t, &v);
        announce(&t, &v);
        Ok(())
    }

    pub fn write(&self, id: &str, data: &[u8]) -> Result<()> {
        let t = self.get(id)?;
        anyhow::ensure!(data.len() <= INPUT_CHUNK, "terminal input frame is too large");
        if let Some((conn, p)) = &t.remote {
            anyhow::ensure!(t.info.lock().unwrap().alive, "the terminal has exited");
            conn.write(p, data);
            return Ok(());
        }
        let writer = t.writer.lock().unwrap();
        writer.as_ref().context("the terminal has exited")?
            .try_send(data.to_vec()).context("terminal input queue is full or closed")?;
        Ok(())
    }

    /// Asks the process group to stop (SIGHUP, as a closed terminal does, and SIGTERM), then kills it
    /// after 5 seconds. Interactive shells ignore SIGTERM alone.
    pub fn close(&self, id: &str) -> Result<()> {
        let t = self.get(id)?;
        if !t.info.lock().unwrap().alive {
            return Ok(());
        }
        #[cfg(unix)]
        if let Some((conn, p)) = &t.remote {
            conn.signal(p, libc::SIGHUP, true);
            conn.signal(p, libc::SIGTERM, true);
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(pid) = t.pid {
            let mut closing = t.closing.lock().unwrap();
            if !closing.started {
                closing.started = true;
                let master = t.master.lock().unwrap();
                closing.worker = Some(stop_pty(pid, master.as_deref()));
            }
            return Ok(());
        }
        if let Some(k) = t.killer.lock().unwrap().as_mut() { k.kill()?; }
        Ok(())
    }

    /// Closes the terminals of a folder and waits up to 6 seconds for them to exit.
    pub async fn close_in(&self, path: &str) -> Result<()> {
        let ids = self.alive_in(path);
        self.close_and_wait(&ids).await
    }

    pub async fn close_all(&self) -> Result<()> {
        let ids: Vec<String> = self.list().into_iter().filter(|t| t.alive).map(|t| t.id).collect();
        self.close_and_wait(&ids).await
    }

    pub async fn close_and_wait(&self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.close(id)?;
        }
        for _ in 0..60 {
            if !ids.iter().any(|id| self.get(id).is_ok_and(|t| t.info.lock().unwrap().alive)) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("a terminal did not exit")
    }

    /// Forgets an exited terminal.
    pub fn remove(&self, app: &App, id: &str) -> Result<()> {
        let t = self.get(id)?;
        if t.info.lock().unwrap().alive {
            bail!("close the terminal first");
        }
        self.inner.lock().unwrap().remove(id);
        app.bus.publish("terminals", "removed", serde_json::json!({"id": id}));
        Ok(())
    }

    async fn spawn(&self, app: &App, mut info: TerminalInfo, argv: Vec<String>, env: Vec<(String, String)>) -> Result<TerminalInfo> {
        // The PTY would otherwise start the process in another folder.
        if !crate::hostfs::is_dir(&info.cwd).await {
            bail!("the folder {} no longer exists", info.cwd);
        }
        if self.alive_count() >= MAX_OPEN {
            bail!("{MAX_OPEN} terminals are already open; close one first");
        }
        if let Some(host) = crate::hosts::loc::host_of(&info.cwd).map(str::to_string) {
            return self.spawn_remote(app, info, &host, argv, env).await;
        }
        let pair = portable_pty::native_pty_system().openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::from_argv(argv.iter().map(Into::into).collect());
        cmd.env_clear();
        for (k, v) in app.env.vars() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("LAUNCHPAD_ENV", app.paths.app_env.name());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.cwd(&info.cwd);
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("start {}", argv[0]))?;
        // The reader sees EOF only once no process holds the slave side.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = input_writer(pair.master.take_writer()?);
        let (tx, _) = broadcast::channel(1024);
        let term = Arc::new(Term {
            info: Mutex::new(info.clone()),
            buffer: Mutex::new(Buffer::default()),
            tx,
            writer: Mutex::new(Some(writer)),
            closing: Mutex::new(PtyClose::default()),
            master: Mutex::new(Some(pair.master)),
            size: Mutex::new(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }),
            viewers: Mutex::new(Viewers::default()),
            killer: Mutex::new(Some(child.clone_killer())),
            pid: child.process_id(),
            remote: None,
        });
        self.inner.lock().unwrap().insert(info.id.clone(), term.clone());
        tracing::info!("terminal {} started in {}: {}", info.id, info.cwd, argv.join(" "));
        app.bus.publish("terminals", "opened", serde_json::json!(info));

        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let t = term.clone();
        std::thread::spawn(move || {
            let mut chunk = vec![0u8; 16 * 1024];
            loop {
                let n = match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut buffer = t.buffer.lock().unwrap();
                buffer.bytes.extend(&chunk[..n]);
                buffer.end += n as u64;
                let extra = buffer.bytes.len().saturating_sub(BUFFER);
                buffer.bytes.drain(..extra);
                let _ = t.tx.send(Output::Data(chunk[..n].into()));
            }
            let _ = done_tx.send(());
        });
        let t = term.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code());
            let close = t.closing.lock().unwrap().worker.take();
            if let Some(close) = close { let _ = close.join(); }
            // Lets the reader drain the last output (ConPTY only ends it when the master is dropped).
            let _ = done_rx.recv_timeout(Duration::from_secs(1));
            finished(&app, &t, code);
        });
        info.alive = true;
        Ok(info)
    }

    /// A PTY on an SSH host: the helper owns it, so it survives a lost connection and the output resumes.
    async fn spawn_remote(&self, app: &App, mut info: TerminalInfo, host: &str, argv: Vec<String>, env: Vec<(String, String)>) -> Result<TerminalInfo> {
        let conn = crate::hosts::conn(host)?;
        let mut vars = vec![("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".into(), "truecolor".into()), ("LAUNCHPAD_ENV".into(), app.paths.app_env.name().into())];
        vars.extend(env.into_iter().map(|(k, v)| (k, crate::hosts::loc::localize_arg(host, &v))));
        let argv: Vec<String> = argv.iter().map(|a| crate::hosts::loc::localize_arg(host, a)).collect();
        let cwd = crate::hosts::loc::native(&info.cwd).to_string();
        let (p, [mut out, _err]) = conn.spawn(argv.clone(), Some(cwd), vars, Some((24, 80)), true).await?;
        let (tx, _) = broadcast::channel(1024);
        let term = Arc::new(Term {
            info: Mutex::new(info.clone()),
            buffer: Mutex::new(Buffer::default()),
            tx,
            writer: Mutex::new(None),
            closing: Mutex::new(PtyClose::default()),
            master: Mutex::new(None),
            size: Mutex::new(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }),
            viewers: Mutex::new(Viewers::default()),
            killer: Mutex::new(None),
            pid: None,
            remote: Some((conn.clone(), p.clone())),
        });
        self.inner.lock().unwrap().insert(info.id.clone(), term.clone());
        tracing::info!("terminal {} started on {host} in {}: {}", info.id, info.cwd, argv.join(" "));
        app.bus.publish("terminals", "opened", serde_json::json!(info));
        let (t, app) = (term.clone(), app.clone());
        tokio::spawn(async move {
            while let Some(chunk) = out.recv().await {
                let mut buffer = t.buffer.lock().unwrap();
                buffer.bytes.extend(&chunk);
                buffer.end += chunk.len() as u64;
                let extra = buffer.bytes.len().saturating_sub(BUFFER);
                buffer.bytes.drain(..extra);
                let _ = t.tx.send(Output::Data(chunk.into()));
            }
            let code = match conn.wait(&p).await {
                Ok(s) => s.code.map(|c| c as u32),
                Err(e) => {
                    let note = format!("\r\n[{e}]\r\n");
                    let mut buffer = t.buffer.lock().unwrap();
                    buffer.bytes.extend(note.as_bytes());
                    buffer.end += note.len() as u64;
                    let _ = t.tx.send(Output::Data(note.as_bytes().into()));
                    None
                }
            };
            finished(&app, &t, code);
        });
        info.alive = true;
        Ok(info)
    }

    /// Opens a login shell or runs an explicit snippet in an allowed folder.
    pub async fn open_shell(&self, app: &App, cwd: &str, command: Option<&str>, language: Option<&str>) -> Result<TerminalInfo> {
        if command.is_some_and(|c| c.trim().is_empty()) {
            bail!("the command is empty");
        }
        // Serialize terminal startup with task Git operations and their idle checks.
        let _action = app.sessions.action_lock().await;
        let _guard = app.sessions.allow_work().await;
        let (cwd, project) = check_cwd(app, cwd).await?;
        let cwd_s = cwd.to_string_lossy().to_string();
        crate::predrafts::check_session_work(app, &cwd_s).await?;
        let branch = crate::procs::output(&app.env, "git", &["rev-parse", "--abbrev-ref", "HEAD"], Some(&cwd)).await.ok();
        let name = project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| cwd.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(cwd_s.clone()));
        let title = match branch.filter(|b| !b.is_empty() && b != "HEAD") {
            Some(b) => format!("{name} · {b}"),
            None => name,
        };
        let info = TerminalInfo {
            id: uuid::Uuid::new_v4().to_string(),
            kind: if command.is_some() { TerminalKind::Command } else { TerminalKind::Shell },
            title,
            cwd: cwd_s,
            project_id: project.map(|p| p.id),
            session_id: None,
            alive: true,
            exit_code: None,
            created_at: now_ms(),
        };
        let remote = crate::hosts::loc::host_of(&info.cwd).is_some();
        let argv = match command {
            Some(command) => command_shell(app, command, language.unwrap_or("shell"), remote)?,
            None if remote => vec!["@shell".into(), "-l".into()],
            None => shell(app),
        };
        self.spawn(app, info, argv, vec![]).await
    }

    /// Opens the harness TUI of a session. The caller detaches the headless harness first.
    pub(crate) async fn open_tui(&self, app: &App, s: &Session) -> Result<TerminalInfo> {
        let is_temp = match &s.project_id {
            Some(id) => crate::projects::get(app, id).await?.is_some_and(|p| p.is_temp),
            None => false,
        };
        let argv = tui_command(app, s, is_temp)?;
        // The TUI uses the same variant environment as the headless process (account, API key).
        let mut env = crate::variants::env(app, s.harness, &s.launch_spec, s.model.as_deref(), crate::hosts::loc::host_of(&s.cwd))?;
        env.extend(app.harnesses.get(s.harness).terminal_env(is_temp).iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let info = TerminalInfo {
            id: uuid::Uuid::new_v4().to_string(),
            kind: TerminalKind::Tui,
            title: if s.title.trim().is_empty() { format!("{} session", s.harness.as_str()) } else { s.title.clone() },
            cwd: s.cwd.clone(),
            project_id: s.project_id.clone(),
            session_id: Some(s.id.clone()),
            alive: true,
            exit_code: None,
            created_at: now_ms(),
        };
        self.spawn(app, info, argv, env).await
    }
}

/// Marks a terminal exited and tells its viewers; a clean shell exit closes its tab.
fn finished(app: &App, t: &Arc<Term>, code: Option<u32>) {
    let info = {
        let _buffer = t.buffer.lock().unwrap();
        t.writer.lock().unwrap().take();
        t.master.lock().unwrap().take();
        let info = {
            let mut info = t.info.lock().unwrap();
            info.alive = false;
            info.exit_code = code;
            info.clone()
        };
        let _ = t.tx.send(Output::Exit(code));
        info
    };
    tracing::info!("terminal {} exited ({code:?})", info.id);
    app.bus.publish("terminals", "exited", serde_json::json!(info));
    // A normal shell exit (Ctrl+D or `exit`) closes its tab in every viewer. Keep failed
    // shells, explicit snippet commands and harness TUIs so their final output can still be inspected.
    if info.kind == TerminalKind::Shell && code == Some(0) {
        let _ = app.terminals.remove(app, &info.id);
    }
}

/// Resizes the PTY to the owner's reported size. Returns whether the size changed.
/// Callers hold the buffer lock, so viewers attaching meanwhile see either the old or the new size.
fn take_size(t: &Term, v: &Viewers) -> bool {
    let Some((rows, cols)) = v.owner.and_then(|o| v.wanted.get(&o).copied().flatten()) else { return false };
    let size = PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 };
    let mut current = t.size.lock().unwrap();
    if (current.rows, current.cols) == (size.rows, size.cols) {
        return false;
    }
    if let Some((conn, p)) = &t.remote {
        if !t.info.lock().unwrap().alive {
            return false;
        }
        conn.resize(p, size.rows, size.cols);
        *current = size;
        return true;
    }
    // An exited terminal keeps its last size.
    let Some(m) = t.master.lock().unwrap().as_ref().map(|m| m.resize(size)) else { return false };
    if let Err(e) = m {
        tracing::debug!("terminal resize: {e:#}");
        return false;
    }
    *current = size;
    true
}

/// Tells every viewer the effective size and owner, in order with the output (buffer lock held).
fn announce(t: &Term, v: &Viewers) {
    let size = *t.size.lock().unwrap();
    let _ = t.tx.send(Output::Size { rows: size.rows.max(2), cols: size.cols.max(2), owner: v.owner });
}

pub fn inside(cwd: &str, path: &str) -> bool {
    let (cwd, path) = (Path::new(cwd), Path::new(path));
    cwd.starts_with(path) || std::fs::canonicalize(path).is_ok_and(|p| cwd.starts_with(p))
}

/// The user's login shell (`pwsh.exe`, or `powershell.exe`, on Windows).
fn shell(app: &App) -> Vec<String> {
    if cfg!(windows) {
        let p = app.env.which("pwsh.exe").or_else(|| app.env.which("powershell.exe")).map(|p| p.to_string_lossy().to_string());
        return vec![p.unwrap_or_else(|| "powershell.exe".into()), "-NoLogo".into()];
    }
    let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
    vec![app.env.get("SHELL").filter(|s| !s.is_empty()).unwrap_or(fallback).to_string(), "-l".into()]
}

fn command_shell(app: &App, command: &str, language: &str, remote: bool) -> Result<Vec<String>> {
    // On a host the helper resolves programs on the host's login-shell PATH.
    let mut argv = match language {
        "shell" if remote => vec!["@shell".into()],
        "sh" | "bash" | "zsh" | "fish" if remote => vec![language.to_string()],
        "powershell" | "pwsh" if remote => vec!["pwsh".into(), "-NoLogo".into()],
        "shell" => shell(app),
        "sh" | "bash" | "zsh" | "fish" => vec![app.env.which(language).with_context(|| format!("{language} is not installed"))?.to_string_lossy().into_owned()],
        "powershell" | "pwsh" => {
            let path = app.env.which("pwsh").or_else(|| app.env.which("powershell"));
            vec![path.context("PowerShell is not installed")?.to_string_lossy().into_owned(), "-NoLogo".into()]
        }
        _ => bail!("unsupported shell language: {language}"),
    };
    let powershell = matches!(language, "powershell" | "pwsh") || (language == "shell" && cfg!(windows) && !remote);
    // Pass the whole snippet as one argument, never interpolate it into a wrapper command.
    argv.push(if powershell { "-Command" } else { "-c" }.into());
    argv.push(command.into());
    Ok(argv)
}

/// Like `code::open`: a registered project, one of its git worktrees, the folder of a session, or the home
/// folder of an SSH host (setup terminals: install and sign-in commands).
async fn check_cwd(app: &App, cwd: &str) -> Result<(PathBuf, Option<crate::model::Project>)> {
    let root = crate::hostfs::canonicalize(cwd).await.with_context(|| format!("{cwd} does not exist"))?;
    if let Some(host) = crate::hosts::loc::host_of_path(&root)
        && app.hosts.home(host).is_some_and(|h| crate::hosts::loc::join(Some(host), &h) == root.to_string_lossy())
    {
        return Ok((root, None));
    }
    let projects = crate::projects::list(app).await?;
    if let Some(p) = crate::code::project_for_root(&projects, &root).await {
        return Ok((root, Some(p)));
    }
    let mut session = None;
    for s in app.sessions.list() {
        if Path::new(&s.cwd) == root || crate::hostfs::canonicalize(&s.cwd).await.is_ok_and(|c| c == root) {
            session = Some(s);
            break;
        }
    }
    match session {
        Some(s) => {
            let project = projects.into_iter().find(|p| Some(&p.id) == s.project_id.as_ref());
            Ok((root, project))
        }
        None => bail!("{} is not a registered project, one of its worktrees, or a session folder", root.display()),
    }
}

/// The command that resumes the session in a TUI.
fn tui_command(app: &App, s: &Session, is_temp: bool) -> Result<Vec<String>> {
    let mut command = app.harnesses.get(s.harness).terminal_command(s, is_temp)?;
    anyhow::ensure!(!command.is_empty(), "empty harness terminal command");
    if crate::hosts::loc::host_of(&s.cwd).is_none()
        && let Some(path) = app.env.which(&command[0]) {
        command[0] = path.to_string_lossy().to_string();
    }
    Ok(command)
}

/// Whether a TUI process still runs this session (checked before re-attaching), including one
/// opened in another client.
pub fn tui_running(app: &App, s: &Session) -> bool {
    let Some(id) = &s.harness_session_id else {
        return false;
    };
    // On a host, an integrated terminal is the only TUI Launchpad can see.
    if crate::hosts::loc::host_of(&s.cwd).is_some() {
        return app.terminals.tui_of(&s.id).is_some();
    }
    let markers = app.harnesses.get(s.harness).terminal_resume_markers();
    #[cfg(not(target_os = "linux"))]
    {
        let output = if cfg!(windows) {
            std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", "Get-CimInstance Win32_Process | Select-Object -ExpandProperty CommandLine"]).output()
        } else {
            std::process::Command::new("ps").args(["-axo", "command="]).output()
        };
        return output
            .map(|out| {
                String::from_utf8_lossy(&out.stdout).lines().any(|line| {
                    line.contains(id) && markers.iter().any(|marker| line.contains(marker))
                })
            })
            .unwrap_or(true);
    }
    #[cfg(target_os = "linux")]
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    #[cfg(target_os = "linux")]
    for entry in dir.flatten() {
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
        let has_id = args.contains(&id.as_bytes());
        let is_resume = args
            .iter()
            .any(|arg| markers.iter().any(|marker| *arg == marker.as_bytes()));
        if has_id && is_resume {
            return true;
        }
    }
    #[cfg(target_os = "linux")]
    { false }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn blocked_input_is_bounded_without_waiting_on_the_writer() {
        struct Blocked(std::sync::mpsc::Receiver<()>);
        impl Write for Blocked {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let (release, gate) = std::sync::mpsc::channel();
        let input = input_writer(Box::new(Blocked(gate)));
        let mut rejected = false;
        for _ in 0..6 {
            if input.try_send(vec![0; INPUT_CHUNK]).is_err() { rejected = true; }
        }
        assert!(rejected);
        drop(input);
        drop(release);
    }

    #[test]
    fn closing_kills_a_foreground_job_that_ignores_hup_and_term() {
        use std::io::BufRead;
        let pair = portable_pty::native_pty_system().openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }).unwrap();
        let mut command = CommandBuilder::new("/bin/bash");
        command.args(["--noprofile", "--norc", "-ic", "sh -c 'trap \"\" HUP TERM; echo READY; exec sleep 60'; :"]);
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = std::io::BufReader::new(pair.master.try_clone_reader().unwrap());
        let mut line = String::new();
        while !line.contains("READY") {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
        }
        let pid = child.process_id().unwrap();
        let foreground = unsafe { libc::tcgetpgrp(pair.master.as_raw_fd().unwrap()) };
        if foreground <= 0 || foreground == pid as i32 {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
            let _ = child.wait();
            panic!("test shell did not create a separate foreground process group");
        }
        let stop = stop_pty(pid, Some(pair.master.as_ref()));
        stop.join().unwrap();
        let _ = child.wait().unwrap();
        // The slave remains open until the resistant foreground job has actually died.
        let (done, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = reader.read_to_end(&mut output);
            let _ = done.send(());
        });
        if rx.recv_timeout(Duration::from_secs(3)).is_err() {
            unsafe { libc::kill(-foreground, libc::SIGKILL); }
            panic!("foreground job survived terminal close");
        }
    }

    #[test]
    fn native_pty_preserves_progress_control_bytes_and_reports_resize() {
        let pair = portable_pty::native_pty_system().openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }).unwrap();
        pair.master.resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 }).unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf 'progress 1\\r\\033[2Kprogress 2\\n'; stty size"]);
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert!(child.wait().unwrap().success());
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("progress 1\r\x1b[2Kprogress 2"), "{text:?}");
        assert!(text.contains("40 120"), "{text:?}");
    }
}

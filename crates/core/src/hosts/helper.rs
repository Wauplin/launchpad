//! The Launchpad helper on an SSH host (`launchpad host-helper serve`). It is the daemon's binary, uploaded
//! by the daemon, and runs detached from SSH so harness turns and terminals survive a lost connection.
//!
//! It owns the processes it starts, keeps their output until the daemon acknowledges it, and writes their
//! input exactly once (offsets on both sides). One daemon is connected at a time, through
//! `launchpad host-helper bridge`, which `ssh` runs and which pipes its stdio to the helper socket.
//! Agents' `launchpad` CLI calls reach the daemon through a second socket (`cli.sock`).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::proto::*;

/// Output kept per process stream while the daemon is away (oldest bytes dropped first).
const HARNESS_BUFFER: usize = 64 << 20;
const PTY_BUFFER: usize = 2 << 20;
/// The helper exits after this long without a daemon or retained processes/CLI calls.
const IDLE_EXIT: Duration = Duration::from_secs(15 * 60);
/// How long an agent's `launchpad` call waits for the daemon to reconnect.
const CLI_WAIT: Duration = Duration::from_secs(30 * 60);
/// Processes are stopped and buffered results discarded when no daemon connected for this long.
const ABANDONED: Duration = Duration::from_secs(30 * 24 * 3600);

/// `~/.local/share/launchpad-host/<env>`: sockets, log and attachments of one daemon env.
pub fn dir(env: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".local/share/launchpad-host").join(env)
}

/// Folder of the uploaded helper binaries (`launchpad-<commit>`).
pub const BIN_DIR: &str = ".local/share/launchpad-host/bin";

/// Environment variable set for every process the helper starts: the CLI then talks to the daemon
/// through the helper instead of a local daemon port.
pub const HELPER_ENV: &str = "LAUNCHPAD_HOST_HELPER";

#[cfg(unix)]
pub use unix::{bridge, cli_request, serve};

#[cfg(not(unix))]
pub async fn serve(_env: &str) -> Result<()> {
    bail!("the Launchpad host helper runs on Linux and macOS hosts")
}
#[cfg(not(unix))]
pub async fn bridge(_env: &str) -> Result<()> {
    bail!("the Launchpad host helper runs on Linux and macOS hosts")
}
#[cfg(not(unix))]
pub async fn cli_request(_dir: &Path, _method: &str, _path: &str, _body: Option<String>) -> Result<(u16, String)> {
    bail!("the Launchpad host helper runs on Linux and macOS hosts")
}

/// Runs a file system operation locally (on the host, for the helper).
pub(crate) fn fs_op(op: FsOp) -> std::result::Result<serde_json::Value, ErrorInfo> {
    use serde_json::json;
    fn err(e: std::io::Error, what: &str) -> ErrorInfo {
        let kind = match e.kind() {
            std::io::ErrorKind::NotFound => "not_found",
            std::io::ErrorKind::AlreadyExists => "exists",
            _ => "other",
        };
        ErrorInfo { message: format!("{what}: {e}"), kind: kind.into() }
    }
    fn kind(t: std::fs::FileType) -> &'static str {
        if t.is_symlink() { "symlink" } else if t.is_dir() { "dir" } else if t.is_file() { "file" } else { "other" }
    }
    match op {
        FsOp::Canonicalize { path } => std::fs::canonicalize(&path).map(|p| json!(p.to_string_lossy())).map_err(|e| err(e, &path)),
        FsOp::Metadata { path, follow } => {
            let m = if follow { std::fs::metadata(&path) } else { std::fs::symlink_metadata(&path) };
            m.map(|m| {
                let modified_ms = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
                json!(Meta { kind: kind(m.file_type()).into(), len: m.len(), modified_ms })
            }).map_err(|e| err(e, &path))
        }
        FsOp::Read { path, max } => (|| {
            let m = std::fs::metadata(&path)?;
            if m.len() > max {
                return Err(std::io::Error::other(format!("larger than {max} bytes ({} bytes)", m.len())));
            }
            std::fs::read(&path)
        })().map(|b| json!(B64(b))).map_err(|e| err(e, &path)),
        FsOp::Tail { path, len } => (|| {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(&path)?;
            let size = f.metadata()?.len();
            f.seek(SeekFrom::Start(size.saturating_sub(len)))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(buf)
        })().map(|b| json!(B64(b))).map_err(|e| err(e, &path)),
        FsOp::Write { path, data, mode } => (|| {
            let tmp = format!("{path}.lp-tmp-{}", uuid::Uuid::new_v4());
            std::fs::write(&tmp, &data.0)?;
            #[cfg(unix)]
            if let Some(mode) = mode {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
            }
            #[cfg(not(unix))]
            let _ = mode;
            std::fs::rename(&tmp, &path)
        })().map(|_| json!(null)).map_err(|e| err(e, &path)),
        FsOp::CreateDirAll { path, mode } => (|| {
            std::fs::create_dir_all(&path)?;
            #[cfg(unix)]
            if let Some(mode) = mode {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            }
            #[cfg(not(unix))]
            let _ = mode;
            Ok(())
        })().map(|_| json!(null)).map_err(|e| err(e, &path)),
        FsOp::Remove { path, recursive } => (|| {
            let m = std::fs::symlink_metadata(&path)?;
            if m.is_dir() {
                if recursive { std::fs::remove_dir_all(&path) } else { std::fs::remove_dir(&path) }
            } else {
                std::fs::remove_file(&path)
            }
        })().map(|_| json!(null)).map_err(|e| err(e, &path)),
        FsOp::Rename { from, to } => std::fs::rename(&from, &to).map(|_| json!(null)).map_err(|e| err(e, &from)),
        FsOp::ReadDir { path } => std::fs::read_dir(&path).map(|d| {
            let entries: Vec<DirEntry> = d.flatten().map(|e| DirEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                kind: e.file_type().map(kind).unwrap_or("other").into(),
            }).collect();
            json!(entries)
        }).map_err(|e| err(e, &path)),
        FsOp::Copy { from, to } => std::fs::copy(&from, &to).map(|n| json!(n)).map_err(|e| err(e, &from)),
        FsOp::Walk { root } => Ok(json!(crate::code::walk(Path::new(&root)))),
        FsOp::DiskUsage { paths } => {
            fn walk(p: &Path) -> u64 {
                let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
                if meta.is_dir() { std::fs::read_dir(p).map(|d| d.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0) } else { meta.len() }
            }
            Ok(json!(paths.iter().map(|p| walk(Path::new(p))).collect::<Vec<u64>>()))
        }
        FsOp::InstallSkill => install_skill().map(|_| json!(null)).map_err(|e| ErrorInfo { message: format!("{e:#}"), kind: "other".into() }),
    }
}

/// Installs the Launchpad skill for agents on this machine as a managed copy, like the Windows daemon. A real
/// folder without the marker (the owner's own copy) and a symlink (a source checkout) are kept.
fn install_skill() -> Result<()> {
    let dest = dirs::home_dir().context("home directory")?.join(".agents/skills/launchpad");
    if let Ok(meta) = std::fs::symlink_metadata(&dest)
        && (meta.file_type().is_symlink() || !dest.join(".launchpad-managed").exists())
    {
        return Ok(());
    }
    std::fs::create_dir_all(&dest)?;
    std::fs::write(dest.join("SKILL.md"), crate::predrafts::SKILL)?;
    std::fs::write(dest.join(".launchpad-managed"), "")?;
    Ok(())
}

#[derive(Default)]
struct Stream {
    buf: VecDeque<u8>,
    /// Offset just after the last byte.
    end: u64,
}

impl Stream {
    fn start(&self) -> u64 {
        self.end - self.buf.len() as u64
    }
}

#[derive(Default)]
struct Streams {
    s: [Stream; 2],
    /// Client generation that receives live output (0: none).
    attached: u64,
    exit: Option<(Option<i32>, Option<i32>)>,
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::shellenv::UserEnv;
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::{mpsc, oneshot};

    enum Input {
        Pipe(tokio::process::ChildStdin),
        /// Blocking PTY writer, fed by a thread.
        Pty(std::sync::mpsc::Sender<Vec<u8>>),
    }

    struct Proc {
        key: String,
        boot: String,
        cap: usize,
        streams: Mutex<Streams>,
        input: tokio::sync::Mutex<(Option<Input>, u64)>,
        pid: Option<u32>,
        pty: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
        closing: Mutex<crate::terminals::PtyClose>,
    }

    struct Client {
        generation: u64,
        tx: Option<mpsc::UnboundedSender<String>>,
        since: Instant,
    }

    struct Helper {
        /// The login-shell environment, read again when the daemon connects or checks the host.
        env: std::sync::RwLock<UserEnv>,
        dir: PathBuf,
        info: HostInfo,
        procs: Mutex<HashMap<String, Arc<Proc>>>,
        client: Mutex<Client>,
        boot: Mutex<Option<String>>,
        cli: Mutex<HashMap<u64, oneshot::Sender<(u16, String)>>>,
        next_cli: AtomicU64,
        /// Agent calls accepted while no daemon was connected, sent when one connects.
        queued: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Helper {
        fn send(&self, generation: u64, msg: &FromHelper) {
            let client = self.client.lock().unwrap();
            if client.generation == generation
                && let Some(tx) = &client.tx
            {
                let _ = tx.send(serde_json::to_string(msg).unwrap_or_default());
            }
        }

        fn env(&self) -> UserEnv {
            self.env.read().unwrap().clone()
        }

        async fn refresh_env(&self) {
            let env = UserEnv::resolve().await;
            *self.env.write().unwrap() = env;
        }

        fn current(&self) -> u64 {
            self.client.lock().unwrap().generation
        }

        /// Appends output and forwards it when a daemon follows the process.
        fn push(&self, p: &Proc, stream: u8, data: &[u8]) {
            let mut st = p.streams.lock().unwrap();
            let s = &mut st.s[stream as usize];
            let off = s.end;
            s.buf.extend(data);
            s.end += data.len() as u64;
            let extra = s.buf.len().saturating_sub(p.cap);
            s.buf.drain(..extra);
            if st.attached != 0 {
                self.send(st.attached, &FromHelper::Out { key: p.key.clone(), s: stream, off, data: B64(data.to_vec()) });
            }
        }

        fn exited(&self, p: &Proc, code: Option<i32>, signal: Option<i32>) {
            let mut st = p.streams.lock().unwrap();
            st.exit = Some((code, signal));
            if st.attached != 0 {
                self.send(st.attached, &FromHelper::Exit { key: p.key.clone(), code, signal });
            }
        }

        fn kill(&self, p: &Proc) {
            if let Some(pid) = p.pid {
                let pid = pid as i32;
                unsafe {
                    libc::kill(-pid, libc::SIGHUP);
                    libc::kill(-pid, libc::SIGTERM);
                    libc::kill(pid, libc::SIGTERM);
                }
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                        libc::kill(pid, libc::SIGKILL);
                    }
                });
            }
        }
    }

    fn resolve(env: &UserEnv, argv: &[String]) -> Result<Vec<String>> {
        let mut argv = argv.to_vec();
        let first = argv.first().context("empty command")?.clone();
        if first == "@shell" {
            argv[0] = env.get("SHELL").filter(|s| !s.is_empty()).unwrap_or(if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" }).to_string();
        } else if let Some(p) = env.which(&first) {
            argv[0] = p.to_string_lossy().into_owned();
        } else {
            bail!("{first} was not found on the PATH of the login shell");
        }
        Ok(argv)
    }

    fn command(h: &Helper, argv: &[String], cwd: Option<&str>, env: &[(String, String)]) -> Result<tokio::process::Command> {
        let login = h.env();
        let argv = resolve(&login, argv)?;
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]).env_clear().envs(login.vars().iter()).env(HELPER_ENV, &h.dir).envs(env.iter().cloned());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        Ok(cmd)
    }

    async fn exec(h: &Helper, argv: Vec<String>, cwd: Option<String>, env: Vec<(String, String)>, stdin: Option<B64>) -> Result<Value> {
        let mut cmd = command(h, &argv, cwd.as_deref(), &env)?;
        cmd.kill_on_drop(true).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        cmd.stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
        let mut child = cmd.spawn().with_context(|| format!("start {}", argv[0]))?;
        if let (Some(data), Some(mut w)) = (stdin, child.stdin.take()) {
            tokio::spawn(async move {
                let _ = w.write_all(&data.0).await;
            });
        }
        let out = child.wait_with_output().await?;
        use std::os::unix::process::ExitStatusExt;
        Ok(json!(ExecResult { code: out.status.code(), signal: out.status.signal(), stdout: B64(out.stdout), stderr: B64(out.stderr) }))
    }

    fn spawn(h: &Arc<Helper>, key: String, argv: Vec<String>, cwd: Option<String>, env: Vec<(String, String)>, pty: Option<(u16, u16)>, stdin: bool) -> Result<Value> {
        if h.procs.lock().unwrap().contains_key(&key) {
            return Ok(Value::Null);
        }
        let boot = h.boot.lock().unwrap().clone().unwrap_or_default();
        let generation = h.current();
        match pty {
            None => {
                let mut cmd = command(h, &argv, cwd.as_deref(), &env)?;
                cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
                cmd.stdin(if stdin { std::process::Stdio::piped() } else { std::process::Stdio::null() });
                // Its own process group, so Stop and a new daemon boot reach the commands it started.
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setpgid(0, 0) == -1 { return Err(std::io::Error::last_os_error()); }
                        Ok(())
                    });
                }
                let mut child = cmd.spawn().with_context(|| format!("start {}", argv[0]))?;
                let p = Arc::new(Proc {
                    key: key.clone(),
                    boot,
                    cap: HARNESS_BUFFER,
                    streams: Mutex::new(Streams { attached: generation, ..Default::default() }),
                    input: tokio::sync::Mutex::new((child.stdin.take().map(Input::Pipe), 0)),
                    pid: child.id(),
                    pty: Mutex::new(None),
                    closing: Mutex::new(crate::terminals::PtyClose::default()),
                });
                h.procs.lock().unwrap().insert(key, p.clone());
                let readers: Vec<_> = [child.stdout.take().map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Send + Unpin>), child.stderr.take().map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)]
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, r)| r.map(|r| (i as u8, r)))
                    .map(|(i, mut r)| {
                        let (h, p) = (h.clone(), p.clone());
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 64 * 1024];
                            loop {
                                match r.read(&mut buf).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => h.push(&p, i, &buf[..n]),
                                }
                            }
                        })
                    })
                    .collect();
                let h = h.clone();
                tokio::spawn(async move {
                    for r in readers {
                        let _ = r.await;
                    }
                    let status = child.wait().await;
                    // A completed shell/harness must not leave descendants writing into a folder
                    // that the daemon can now archive or remove.
                    if let Some(pid) = p.pid { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
                    use std::os::unix::process::ExitStatusExt;
                    let (code, signal) = status.map(|s| (s.code(), s.signal())).unwrap_or((None, None));
                    p.input.lock().await.0.take();
                    h.exited(&p, code, signal);
                });
            }
            Some((rows, cols)) => {
                let env_now = h.env();
                let argv = resolve(&env_now, &argv)?;
                let size = portable_pty::PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 };
                let pair = portable_pty::native_pty_system().openpty(size)?;
                let mut cmd = portable_pty::CommandBuilder::from_argv(argv.iter().map(Into::into).collect());
                cmd.env_clear();
                for (k, v) in env_now.vars() {
                    cmd.env(k, v);
                }
                cmd.env(HELPER_ENV, &h.dir);
                for (k, v) in &env {
                    cmd.env(k, v);
                }
                if let Some(cwd) = &cwd {
                    cmd.cwd(cwd);
                }
                let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("start {}", argv[0]))?;
                drop(pair.slave);
                let mut reader = pair.master.try_clone_reader()?;
                let mut writer = pair.master.take_writer()?;
                let (wtx, wrx) = std::sync::mpsc::channel::<Vec<u8>>();
                std::thread::spawn(move || {
                    for data in wrx {
                        if writer.write_all(&data).and_then(|_| writer.flush()).is_err() {
                            break;
                        }
                    }
                });
                let p = Arc::new(Proc {
                    key: key.clone(),
                    boot,
                    cap: PTY_BUFFER,
                    streams: Mutex::new(Streams { attached: generation, ..Default::default() }),
                    input: tokio::sync::Mutex::new((Some(Input::Pty(wtx)), 0)),
                    pid: child.process_id(),
                    pty: Mutex::new(Some(pair.master)),
                    closing: Mutex::new(crate::terminals::PtyClose::default()),
                });
                h.procs.lock().unwrap().insert(key, p.clone());
                let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                let (hr, pr) = (h.clone(), p.clone());
                std::thread::spawn(move || {
                    let mut chunk = vec![0u8; 16 * 1024];
                    loop {
                        match std::io::Read::read(&mut reader, &mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => hr.push(&pr, 0, &chunk[..n]),
                        }
                    }
                    let _ = done_tx.send(());
                });
                let h = h.clone();
                std::thread::spawn(move || {
                    let code = child.wait().ok().map(|s| s.exit_code() as i32);
                    let close = p.closing.lock().unwrap().worker.take();
                    if let Some(close) = close { let _ = close.join(); }
                    let _ = done_rx.recv_timeout(Duration::from_secs(1));
                    p.pty.lock().unwrap().take();
                    h.exited(&p, code, None);
                });
            }
        }
        Ok(Value::Null)
    }

    async fn handle(h: &Arc<Helper>, op: Op) -> std::result::Result<Value, ErrorInfo> {
        let other = |e: anyhow::Error| ErrorInfo { message: format!("{e:#}"), kind: "other".into() };
        match op {
            Op::Exec { argv, cwd, env, stdin } => exec(h, argv, cwd, env, stdin).await.map_err(other),
            Op::Spawn { key, argv, cwd, env, pty, stdin } => spawn(h, key, argv, cwd, env, pty, stdin).map_err(other),
            Op::Signal { key, signal, group } => {
                let proc = h.procs.lock().unwrap().get(&key).cloned();
                if let Some(p) = &proc
                    && group && signal == libc::SIGHUP
                    && let Some(pid) = p.pid
                {
                    let mut closing = p.closing.lock().unwrap();
                    let pty = p.pty.lock().unwrap();
                    if pty.is_some() {
                        if !closing.started {
                            closing.started = true;
                            closing.worker = Some(crate::terminals::stop_pty(pid, pty.as_deref()));
                        }
                        return Ok(Value::Null);
                    }
                }
                if let Some(pid) = proc.and_then(|p| p.pid) {
                    unsafe { libc::kill(if group { -(pid as i32) } else { pid as i32 }, signal) };
                }
                Ok(Value::Null)
            }
            Op::Resize { key, rows, cols } => {
                let p = h.procs.lock().unwrap().get(&key).cloned();
                if let Some(p) = p
                    && let Some(m) = p.pty.lock().unwrap().as_ref()
                {
                    m.resize(portable_pty::PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 }).map_err(other)?;
                }
                Ok(Value::Null)
            }
            Op::Which { programs } => Ok(json!(programs.iter().map(|p| h.env().which(p).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())).collect::<Vec<_>>())),
            Op::Fs(op) => tokio::task::spawn_blocking(move || fs_op(op)).await.map_err(|e| other(e.into()))?,
            Op::Ping => Ok(Value::Null),
            Op::RefreshEnv => {
                h.refresh_env().await;
                Ok(Value::Null)
            }
        }
    }

    async fn on_message(h: &Arc<Helper>, generation: u64, msg: ToHelper) {
        match msg {
            ToHelper::Hello { .. } => {}
            ToHelper::Req { id, op } => {
                let h = h.clone();
                tokio::spawn(async move {
                    let res = handle(&h, op).await;
                    let msg = match res {
                        Ok(v) => FromHelper::Res { id, ok: Some(v), err: None },
                        Err(e) => FromHelper::Res { id, ok: None, err: Some(e) },
                    };
                    h.send(generation, &msg);
                });
            }
            ToHelper::Input { key, off, data } => {
                let Some(p) = h.procs.lock().unwrap().get(&key).cloned() else { return };
                let mut input = p.input.lock().await;
                let written = input.1;
                let end = off + data.0.len() as u64;
                if end <= written || off > written {
                    return;
                }
                let bytes = &data.0[(written - off) as usize..];
                let ok = match &mut input.0 {
                    Some(Input::Pipe(w)) => w.write_all(bytes).await.is_ok() && w.flush().await.is_ok(),
                    Some(Input::Pty(tx)) => tx.send(bytes.to_vec()).is_ok(),
                    None => false,
                };
                if !ok {
                    input.0 = None;
                }
                input.1 = end;
                drop(input);
                h.send(generation, &FromHelper::InputAck { key, off: end });
            }
            ToHelper::CloseInput { key, off } => {
                let Some(p) = h.procs.lock().unwrap().get(&key).cloned() else { return };
                let mut input = p.input.lock().await;
                if input.1 >= off {
                    input.0 = None;
                }
            }
            ToHelper::Ack { key, out, err } => {
                let Some(p) = h.procs.lock().unwrap().get(&key).cloned() else { return };
                let mut st = p.streams.lock().unwrap();
                for (s, off) in st.s.iter_mut().zip([out, err]) {
                    let drop = off.saturating_sub(s.start()).min(s.buf.len() as u64) as usize;
                    s.buf.drain(..drop);
                }
            }
            ToHelper::Release { key } => {
                let p = h.procs.lock().unwrap().remove(&key);
                if let Some(p) = p
                    && p.streams.lock().unwrap().exit.is_none()
                {
                    h.kill(&p);
                }
            }
            ToHelper::CliResponse { id, status, body } => {
                if let Some(tx) = h.cli.lock().unwrap().remove(&id) {
                    let _ = tx.send((status, body));
                }
            }
        }
    }

    /// A daemon connection: `Hello`, then requests until the bridge closes.
    async fn client(h: Arc<Helper>, reader: BufReader<tokio::net::unix::OwnedReadHalf>, mut writer: tokio::net::unix::OwnedWriteHalf) -> Result<()> {
        let mut lines = reader.lines();
        let first = lines.next_line().await?.context("no hello")?;
        let ToHelper::Hello { boot, host, env: _, known } = serde_json::from_str(&first)? else { bail!("expected hello") };
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let generation = {
            let mut c = h.client.lock().unwrap();
            c.generation += 1;
            c.tx = Some(tx);
            c.since = Instant::now();
            c.generation
        };
        tracing::info!("daemon connected for host {host} (boot {boot})");
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() || writer.write_all(b"\n").await.is_err() {
                    break;
                }
                if rx.is_empty() && writer.flush().await.is_err() {
                    break;
                }
            }
        });
        // Processes of another daemon boot, or unknown to the daemon, have no owner any more.
        *h.boot.lock().unwrap() = Some(boot.clone());
        let stale: Vec<Arc<Proc>> = {
            let mut procs = h.procs.lock().unwrap();
            let keys: Vec<String> = procs.iter().filter(|(k, p)| p.boot != boot || !known.iter().any(|kn| &kn.key == *k)).map(|(k, _)| k.clone()).collect();
            keys.iter().filter_map(|k| procs.remove(k)).collect()
        };
        for p in stale {
            if p.streams.lock().unwrap().exit.is_none() {
                tracing::info!("stopping process {} of an earlier daemon connection", p.key);
                h.kill(&p);
            }
        }
        let mut states = vec![];
        for k in &known {
            let p = h.procs.lock().unwrap().get(&k.key).cloned();
            if let Some(p) = p {
                states.push(ProcState { key: k.key.clone(), input: p.input.lock().await.1 });
            }
        }
        h.send(generation, &FromHelper::Hello { version: super::super::build_id().into(), procs: states, info: h.info.clone() });
        for k in &known {
            let Some(p) = h.procs.lock().unwrap().get(&k.key).cloned() else { continue };
            let mut st = p.streams.lock().unwrap();
            for (i, from) in [k.out, k.err].into_iter().enumerate() {
                let s = &st.s[i];
                let from = from.max(s.start()).min(s.end);
                if from < s.end {
                    let data: Vec<u8> = s.buf.range((from - s.start()) as usize..).copied().collect();
                    h.send(generation, &FromHelper::Out { key: p.key.clone(), s: i as u8, off: from, data: B64(data) });
                }
            }
            st.attached = generation;
            if let Some((code, signal)) = st.exit {
                h.send(generation, &FromHelper::Exit { key: p.key.clone(), code, signal });
            }
        }
        let queued: Vec<_> = h.queued.lock().unwrap().drain(..).collect();
        for (method, path, body) in queued {
            let id = h.next_cli.fetch_add(1, Ordering::Relaxed) + 1;
            h.send(generation, &FromHelper::Cli { id, method, path, body, queued: true });
        }
        let result = loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break Ok(()),
                Err(e) => break Err(e.into()),
            };
            match serde_json::from_str::<ToHelper>(&line) {
                Ok(msg) => on_message(&h, generation, msg).await,
                Err(e) => tracing::warn!("invalid message: {e}"),
            }
        };
        let mut c = h.client.lock().unwrap();
        if c.generation == generation {
            c.tx = None;
            c.since = Instant::now();
        }
        tracing::info!("daemon disconnected");
        result
    }

    async fn cli_client(h: Arc<Helper>, stream: UnixStream) -> Result<()> {
        let (r, mut w) = stream.into_split();
        let line = BufReader::new(r).lines().next_line().await?.context("empty request")?;
        let req: Value = serde_json::from_str(&line)?;
        let (tx, rx) = oneshot::channel();
        let id = h.next_cli.fetch_add(1, Ordering::Relaxed) + 1;
        let connected = || {
            let c = h.client.lock().unwrap();
            c.tx.is_some().then_some(c.generation)
        };
        // The daemon's computer may be asleep: hold the call until it reconnects rather than failing the agent's step.
        let (method, path, body) = (req["method"].as_str().unwrap_or("GET").to_string(), req["path"].as_str().unwrap_or("/").to_string(), req["body"].as_str().map(str::to_string));
        // Merge requests and suggestions only record local state: accept them now, deliver them on reconnect.
        // Hold the client lock through enqueueing, so a reconnect cannot drain the queue first.
        let (mut generation, queued) = {
            let c = h.client.lock().unwrap();
            let generation = c.tx.is_some().then_some(c.generation);
            let queued = generation.is_none() && method == "POST" && matches!(path.as_str(), "/api/worktrees/finish" | "/api/suggestions");
            if queued {
                h.queued.lock().unwrap().push((method.clone(), path.clone(), body.clone()));
            }
            (generation, queued)
        };
        if queued {
            let message = if path.ends_with("finish") { "Merge request queued: the Launchpad daemon is away (its computer may be asleep). Launchpad receives it when it reconnects and merges after your turn ends. End your turn and report the merge as requested, not as done." } else { "Suggestion queued: the Launchpad daemon is away (its computer may be asleep) and saves it locally when it reconnects." };
            w.write_all(json!({"status": 202, "body": json!({"message": message, "queued": true}).to_string()}).to_string().as_bytes()).await?;
            w.write_all(b"\n").await?;
            return Ok(());
        }
        if generation.is_none() {
            w.write_all(b"{\"waiting\":true}\n").await?;
            let deadline = Instant::now() + CLI_WAIT;
            while generation.is_none() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_secs(1)).await;
                generation = connected();
            }
        }
        let response = match generation {
            None => (503, json!({"error": "The Launchpad daemon did not reconnect to this host within 30 minutes. Run the command again later."}).to_string()),
            Some(generation) => {
                h.cli.lock().unwrap().insert(id, tx);
                h.send(generation, &FromHelper::Cli { id, method, path, body, queued: false });
                match tokio::time::timeout(Duration::from_secs(180), rx).await {
                    Ok(Ok(r)) => r,
                    _ => {
                        h.cli.lock().unwrap().remove(&id);
                        (504, json!({"error": "The Launchpad daemon did not answer."}).to_string())
                    }
                }
            }
        };
        w.write_all(json!({"status": response.0, "body": response.1}).to_string().as_bytes()).await?;
        w.write_all(b"\n").await?;
        Ok(())
    }

    fn lock(dir: &Path) -> Result<Option<std::fs::File>> {
        use std::os::fd::AsRawFd;
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("helper.lock"))?;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Ok(None);
        }
        Ok(Some(f))
    }

    pub async fn serve(env: &str) -> Result<()> {
        let dir = dir(env);
        std::fs::create_dir_all(&dir)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let Some(_lock) = lock(&dir)? else {
            tracing::info!("a helper is already running");
            return Ok(());
        };
        let env = UserEnv::resolve().await;
        let _ = install_skill();
        prune_binaries();
        link_cli();
        let info = HostInfo {
            os: match std::env::consts::OS { "linux" => "Linux", "macos" => "Darwin", o => o }.into(),
            arch: std::env::consts::ARCH.into(),
            home: dirs::home_dir().unwrap_or_default().to_string_lossy().into_owned(),
            user: env.get("USER").or_else(|| env.get("LOGNAME")).map(str::to_string).unwrap_or_else(|| if unsafe { libc::geteuid() } == 0 { "root".into() } else { String::new() }),
            shell: env.get("SHELL").unwrap_or("").into(),
            cli: std::env::current_exe().map(|p| p.to_string_lossy().trim_end_matches(" (deleted)").to_string()).unwrap_or_default(),
        };
        let h = Arc::new(Helper {
            env: std::sync::RwLock::new(env),
            dir: dir.clone(),
            info,
            procs: Mutex::default(),
            client: Mutex::new(Client { generation: 0, tx: None, since: Instant::now() }),
            boot: Mutex::default(),
            cli: Mutex::default(),
            next_cli: AtomicU64::new(0),
            queued: Mutex::default(),
        });
        for name in ["helper.sock", "cli.sock"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let daemon = UnixListener::bind(dir.join("helper.sock"))?;
        let cli = UnixListener::bind(dir.join("cli.sock"))?;
        tracing::info!("helper {} listening in {}", super::super::build_id(), dir.display());
        let (stop_tx, mut stop) = mpsc::unbounded_channel::<()>();
        {
            let h = h.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let (connected, since) = {
                        let c = h.client.lock().unwrap();
                        (c.tx.is_some(), c.since.elapsed())
                    };
                    if connected {
                        continue;
                    }
                    if since > ABANDONED {
                        let procs: Vec<_> = h.procs.lock().unwrap().drain().map(|(_, p)| p).collect();
                        for p in procs {
                            h.kill(&p);
                        }
                        h.queued.lock().unwrap().clear();
                    }
                    // An exited process still owns output the sleeping daemon has not received.
                    // Keep it and queued agent calls until reconnect, release or abandonment.
                    if since > IDLE_EXIT && h.procs.lock().unwrap().is_empty()
                        && h.queued.lock().unwrap().is_empty() && h.cli.lock().unwrap().is_empty()
                    {
                        let _ = stop_tx.send(());
                        return;
                    }
                }
            });
        }
        loop {
            tokio::select! {
                conn = daemon.accept() => {
                    let Ok((stream, _)) = conn else { continue };
                    let h = h.clone();
                    tokio::spawn(async move {
                        let (r, mut w) = stream.into_split();
                        let mut reader = BufReader::new(r);
                        let mut first = String::new();
                        if reader.read_line(&mut first).await.unwrap_or(0) == 0 { return; }
                        match serde_json::from_str::<BridgeHello>(&first) {
                            Ok(BridgeHello::Bridge { version }) if version == super::super::build_id() => {
                                let ok = serde_json::to_string(&BridgeHello::Ok).unwrap() + "\n";
                                if w.write_all(ok.as_bytes()).await.is_err() { return; }
                                if let Err(e) = client(h, reader, w).await { tracing::warn!("daemon connection: {e:#}"); }
                            }
                            Ok(BridgeHello::Bridge { .. }) => {
                                let reply = serde_json::to_string(&BridgeHello::Mismatch { version: super::super::build_id().into() }).unwrap() + "\n";
                                let _ = w.write_all(reply.as_bytes()).await;
                                let mut next = String::new();
                                if reader.read_line(&mut next).await.is_ok() && matches!(serde_json::from_str(&next), Ok(BridgeHello::Shutdown)) {
                                    tracing::info!("replaced by another helper build");
                                    let procs: Vec<_> = h.procs.lock().unwrap().drain().map(|(_, p)| p).collect();
                                    for p in &procs { h.kill(p); }
                                    let _ = std::fs::remove_file(h.dir.join("helper.sock"));
                                    std::process::exit(0);
                                }
                            }
                            _ => {}
                        }
                    });
                }
                conn = cli.accept() => {
                    let Ok((stream, _)) = conn else { continue };
                    let h = h.clone();
                    tokio::spawn(async move { if let Err(e) = cli_client(h, stream).await { tracing::warn!("cli request: {e:#}"); } });
                }
                _ = stop.recv() => {
                    tracing::info!("idle; exiting");
                    let _ = std::fs::remove_file(dir.join("helper.sock"));
                    let _ = std::fs::remove_file(dir.join("cli.sock"));
                    return Ok(());
                }
            }
        }
    }

    /// `~/.local/share/launchpad-host/bin/launchpad` points to the running build: agents' prompts use this path.
    fn link_cli() {
        let (Some(bin), Ok(me)) = (dirs::home_dir().map(|h| h.join(BIN_DIR)), std::env::current_exe()) else { return };
        if me.parent() != Some(bin.as_path()) {
            return;
        }
        let link = bin.join("launchpad");
        let tmp = bin.join(format!(".launchpad-{}", std::process::id()));
        if std::os::unix::fs::symlink(&me, &tmp).is_ok() && std::fs::rename(&tmp, &link).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Removes helper builds other than this one that were not used for two weeks.
    fn prune_binaries() {
        let Some(bin) = dirs::home_dir().map(|h| h.join(BIN_DIR)) else { return };
        let me = std::env::current_exe().ok();
        let Ok(entries) = std::fs::read_dir(&bin) else { return };
        let cutoff = std::time::SystemTime::now() - Duration::from_secs(14 * 24 * 3600);
        for e in entries.flatten() {
            let path = e.path();
            if Some(&path) == me.as_ref() || !e.file_name().to_string_lossy().starts_with("launchpad-") || e.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            if e.metadata().and_then(|m| m.accessed().or_else(|_| m.modified())).is_ok_and(|t| t < cutoff) {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn start_server(env: &str) -> Result<()> {
        let dir = dir(env);
        std::fs::create_dir_all(&dir)?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("helper.log"))?;
        let mut cmd = std::process::Command::new(std::env::current_exe()?);
        cmd.args(["host-helper", "serve", "--env", env]).stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log);
        // Detached from the SSH session: a lost connection does not end the helper or its processes.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn().context("start the host helper")?;
        Ok(())
    }

    async fn connect(env: &str) -> Result<UnixStream> {
        let sock = dir(env).join("helper.sock");
        let mut started = false;
        for _ in 0..100 {
            if let Ok(s) = UnixStream::connect(&sock).await {
                return Ok(s);
            }
            if !started {
                start_server(env)?;
                started = true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("the host helper did not start; see {}", dir(env).join("helper.log").display())
    }

    /// Pipes stdio (the SSH channel) to the helper server, starting or replacing it first.
    pub async fn bridge(env: &str) -> Result<()> {
        let mut replaced = false;
        let stream = loop {
            let stream = connect(env).await?;
            let (r, mut w) = stream.into_split();
            let hello = serde_json::to_string(&BridgeHello::Bridge { version: super::super::build_id().into() })? + "\n";
            w.write_all(hello.as_bytes()).await?;
            let mut reader = BufReader::new(r);
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            match serde_json::from_str::<BridgeHello>(&line)? {
                BridgeHello::Ok => break (reader, w),
                BridgeHello::Mismatch { .. } if !replaced => {
                    let stop = serde_json::to_string(&BridgeHello::Shutdown)? + "\n";
                    w.write_all(stop.as_bytes()).await?;
                    replaced = true;
                    let lock_path = dir(env);
                    for _ in 0..100 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        if lock(&lock_path)?.is_some() {
                            break;
                        }
                    }
                }
                other => bail!("unexpected helper answer: {other:?}"),
            }
        };
        let (mut reader, mut w) = stream;
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let up = async {
            let _ = tokio::io::copy(&mut stdin, &mut w).await;
        };
        let down = async {
            let _ = tokio::io::copy_buf(&mut reader, &mut stdout).await;
        };
        tokio::select! {
            _ = up => {},
            _ = down => {},
        }
        Ok(())
    }

    /// One `launchpad` CLI call from the host, through the helper to the daemon.
    pub async fn cli_request(dir: &Path, method: &str, path: &str, body: Option<String>) -> Result<(u16, String)> {
        let stream = UnixStream::connect(dir.join("cli.sock")).await.context("the Launchpad host helper is not running")?;
        let (r, mut w) = stream.into_split();
        w.write_all(json!({"method": method, "path": path, "body": body}).to_string().as_bytes()).await?;
        w.write_all(b"\n").await?;
        let mut lines = BufReader::new(r).lines();
        let mut line = lines.next_line().await?.context("no answer from the host helper")?;
        if serde_json::from_str::<Value>(&line).is_ok_and(|v| v["waiting"] == true) {
            eprintln!("Waiting for the Launchpad daemon to reconnect to this host (its computer may be asleep)…");
            line = lines.next_line().await?.context("no answer from the host helper")?;
        }
        let v: Value = serde_json::from_str(&line)?;
        Ok((v["status"].as_u64().unwrap_or(500) as u16, v["body"].as_str().unwrap_or("").to_string()))
    }
}

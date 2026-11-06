//! Commands started by the daemon, with the user environment and, for contained sessions, in a jail.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::shellenv::UserEnv;

pub type Stdin = Box<dyn AsyncWrite + Send + Unpin>;
pub type Reader = Box<dyn AsyncRead + Send + Unpin>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Io {
    Null,
    Piped,
    Inherit,
}

impl Io {
    fn std(self) -> std::process::Stdio {
        match self {
            Io::Null => std::process::Stdio::null(),
            Io::Piped => std::process::Stdio::piped(),
            Io::Inherit => std::process::Stdio::inherit(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
    pub fn code(&self) -> Option<i32> {
        self.code
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(s: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&s);
        #[cfg(not(unix))]
        let signal = None;
        ExitStatus { code: s.code(), signal }
    }
}

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.code, self.signal) {
            (Some(c), _) => write!(f, "exit status: {c}"),
            (None, Some(s)) => write!(f, "signal: {s}"),
            _ => f.write_str("unknown exit status"),
        }
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct Cmd {
    program: String,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    /// The user environment.
    env: UserEnv,
    io: [Option<Io>; 3],
    kill_on_drop: bool,
    child: bool,
    process_tree: bool,
    stdin_data: Option<Vec<u8>>,
}

impl Cmd {
    pub fn new(env: &UserEnv, program: &str, cwd: Option<&Path>) -> Cmd {
        Cmd {
            program: program.to_string(),
            args: vec![],
            envs: vec![],
            cwd: cwd.map(Path::to_path_buf),
            env: env.clone(),
            io: [None; 3],
            kill_on_drop: true,
            child: false,
            process_tree: false,
            stdin_data: None,
        }
    }

    /// Harness children also die with the daemon (`PR_SET_PDEATHSIG`).
    pub(crate) fn child(mut self) -> Cmd {
        self.child = true;
        self
    }

    /// Own a separate process group, including descendants, for cancellable setup/harness work.
    pub fn process_tree(&mut self) -> &mut Self {
        self.process_tree = true;
        self
    }

    pub fn arg(&mut self, a: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(a.as_ref().to_os_string());
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }
    pub fn env(&mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> &mut Self {
        self.envs.push((k.as_ref().to_os_string(), v.as_ref().to_os_string()));
        self
    }
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (k, v) in vars {
            self.env(k, v);
        }
        self
    }
    /// Runs the command in a jail.
    pub fn jail(&mut self, jail: Option<Arc<crate::jail::Jail>>) -> &mut Self {
        self.env = self.env.jailed(jail);
        self
    }
    pub fn current_dir(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.cwd = Some(p.as_ref().to_path_buf());
        self
    }
    pub fn stdin(&mut self, io: Io) -> &mut Self {
        self.io[0] = Some(io);
        self
    }
    pub fn stdout(&mut self, io: Io) -> &mut Self {
        self.io[1] = Some(io);
        self
    }
    pub fn stderr(&mut self, io: Io) -> &mut Self {
        self.io[2] = Some(io);
        self
    }
    /// Bytes written to stdin by `output()`.
    pub fn stdin_bytes(&mut self, data: Vec<u8>) -> &mut Self {
        self.stdin_data = Some(data);
        self
    }
    pub fn kill_on_drop(&mut self, kill: bool) -> &mut Self {
        self.kill_on_drop = kill;
        self
    }

    fn local(&self) -> tokio::process::Command {
        let resolved = self.env.which(&self.program).unwrap_or_else(|| self.program.clone().into());
        let mut cmd = match self.env.jail() {
            // A contained process: in its sandbox (bubblewrap, then the network capture), with only the kept
            // machine variables.
            Some(jail) => {
                // Symlinks on the way (`~/.local/bin/x` → …/current/…) may not exist in the jail: run the target.
                let resolved = std::fs::canonicalize(&resolved).unwrap_or(resolved);
                let (program, args) = jail.command(resolved.into_os_string(), &self.args, self.cwd.as_deref());
                let mut cmd = tokio::process::Command::new(program);
                cmd.env_clear().envs(jail.environment(self.env.vars())).args(args);
                cmd
            }
            None => {
                let mut cmd = tokio::process::Command::new(resolved);
                cmd.env_clear().envs(self.env.vars().iter()).args(&self.args);
                cmd
            }
        };
        #[cfg(windows)]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW: pipes carry harness output.
        cmd.envs(self.envs.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        for (i, io) in self.io.iter().enumerate() {
            if let Some(io) = io {
                match i {
                    0 => cmd.stdin(io.std()),
                    1 => cmd.stdout(io.std()),
                    _ => cmd.stderr(io.std()),
                };
            }
        }
        cmd.kill_on_drop(self.kill_on_drop);
        #[cfg(unix)]
        if self.process_tree { cmd.process_group(0); }
        // SAFETY: prctl is async-signal-safe and only affects the child.
        #[cfg(target_os = "linux")]
        if self.child {
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        cmd
    }

    pub async fn output(&mut self) -> std::io::Result<Output> {
        if self.process_tree {
            if self.io[1].is_none() { self.stdout(Io::Piped); }
            if self.io[2].is_none() { self.stderr(Io::Piped); }
            if self.stdin_data.is_some() { self.stdin(Io::Piped); }
            let mut child = self.spawn().await?;
            if let (Some(data), Some(mut w)) = (self.stdin_data.clone(), child.stdin.take()) {
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let _ = w.write_all(&data).await;
                });
            }
            return child.wait_with_output().await;
        }
        let mut cmd = self.local();
        if self.io[1].is_none() {
            cmd.stdout(std::process::Stdio::piped());
        }
        if self.io[2].is_none() {
            cmd.stderr(std::process::Stdio::piped());
        }
        let Some(data) = self.stdin_data.clone() else {
            let out = cmd.output().await?;
            return Ok(Output { status: out.status.into(), stdout: out.stdout, stderr: out.stderr });
        };
        cmd.stdin(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;
        if let Some(mut w) = child.stdin.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = w.write_all(&data).await;
            });
        }
        let out = child.wait_with_output().await?;
        Ok(Output { status: out.status.into(), stdout: out.stdout, stderr: out.stderr })
    }

    pub async fn status(&mut self) -> std::io::Result<ExitStatus> {
        Ok(self.output().await?.status)
    }

    pub async fn spawn(&mut self) -> std::io::Result<Proc> {
        let mut child = self.local().spawn()?;
        #[cfg(target_os = "macos")]
        let group_guard = if self.process_tree {
            match MacGroupGuard::start(child.id().expect("spawned process has a PID")) {
                Ok(guard) => Some(guard),
                Err(e) => {
                    if let Some(pid) = child.id() { kill_tree(pid, true); }
                    let _ = child.wait().await;
                    return Err(e);
                }
            }
        } else { None };
        Ok(Proc {
            stdin: child.stdin.take().map(|w| Box::new(w) as Stdin),
            stdout: child.stdout.take().map(|r| Box::new(r) as Reader),
            stderr: child.stderr.take().map(|r| Box::new(r) as Reader),
            tree_pid: if self.process_tree { child.id() } else { None },
            child,
            #[cfg(target_os = "macos")]
            group_guard,
        })
    }
}

pub struct Proc {
    pub stdin: Option<Stdin>,
    pub stdout: Option<Reader>,
    pub stderr: Option<Reader>,
    child: tokio::process::Child,
    tree_pid: Option<u32>,
    #[cfg(target_os = "macos")]
    group_guard: Option<MacGroupGuard>,
}

/// macOS has no parent-death signal. A separate, small watchdog owns only this group's
/// lifetime: EOF on its private stdin kills the group, including after daemon SIGKILL.
/// It shares no agent pipes and runs outside launchd's daemon group so it can finish cleanup.
#[cfg(target_os = "macos")]
struct MacGroupGuard {
    input: Option<tokio::process::ChildStdin>,
    child: Option<tokio::process::Child>,
}

#[cfg(target_os = "macos")]
impl MacGroupGuard {
    fn start(pid: u32) -> std::io::Result<Self> {
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", "read -r line; /bin/kill -KILL -- -\"$1\"", "launchpad-group-guard", &pid.to_string()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()?;
        Ok(Self { input: child.stdin.take(), child: Some(child) })
    }

    async fn finish(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take() { let _ = child.wait().await; }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacGroupGuard {
    fn drop(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            tokio::spawn(async move { let _ = child.wait().await; });
        }
    }
}

/// Stops a process from another task (Stop of a harness).
#[derive(Clone)]
pub enum Killer {
    Local(Option<u32>),
    LocalTree(u32),
}

impl Killer {
    pub async fn terminate(&self) { self.signal(false).await; }

    /// Force the process (and its owned group) to exit before cleanup.
    pub async fn kill(&self) { self.signal(true).await; }

    async fn signal(&self, force: bool) {
        match self {
            Killer::LocalTree(pid) => kill_tree(*pid, force),
            Killer::Local(Some(pid)) => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(*pid as i32, if force { libc::SIGKILL } else { libc::SIGTERM });
                }
                #[cfg(windows)]
                {
                    let _ = tokio::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status().await;
                }
            }
            Killer::Local(None) => {}
        }
    }
}

fn kill_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    unsafe { libc::kill(-(pid as i32), if force { libc::SIGKILL } else { libc::SIGTERM }); }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = force;
        let _ = std::process::Command::new("taskkill").creation_flags(0x08000000).args(["/PID", &pid.to_string(), "/T", "/F"]).status();
    }
}

impl Proc {
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn killer(&self) -> Killer {
        self.tree_pid.map(Killer::LocalTree).unwrap_or_else(|| Killer::Local(self.child.id()))
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        // A shell can exit while detached descendants still write into its folder.
        if let Some(pid) = self.tree_pid.take() { kill_tree(pid, true); }
        #[cfg(target_os = "macos")]
        if let Some(mut guard) = self.group_guard.take() { guard.finish().await; }
        Ok(status.into())
    }

    pub fn start_kill(&mut self) -> std::io::Result<()> {
        if let Some(pid) = self.tree_pid { kill_tree(pid, true); }
        self.child.start_kill()
    }

    pub async fn kill(&mut self) -> std::io::Result<()> {
        self.start_kill()?;
        let _ = self.wait().await;
        Ok(())
    }

    pub async fn wait_with_output(mut self) -> std::io::Result<Output> {
        use tokio::io::AsyncReadExt;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut o, mut e) = (self.stdout.take(), self.stderr.take());
        let read_out = async {
            if let Some(r) = o.as_mut() {
                let _ = r.read_to_end(&mut out).await;
            }
        };
        let read_err = async {
            if let Some(r) = e.as_mut() {
                let _ = r.read_to_end(&mut err).await;
            }
        };
        let status = {
            let read = async { tokio::join!(read_out, read_err); };
            tokio::pin!(read);
            tokio::select! {
                _ = &mut read => self.wait().await?,
                status = self.wait() => {
                    // A detached descendant can retain the pipes after the owned process exits.
                    // Drain available output, then drop the readers instead of waiting for EOF forever.
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), &mut read).await;
                    status?
                }
            }
        };
        Ok(Output { status, stdout: out, stderr: err })
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if let Some(pid) = self.tree_pid { kill_tree(pid, true); }
    }
}

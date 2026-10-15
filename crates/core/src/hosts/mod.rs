//! SSH hosts (docs/ssh-hosts-plan.md): machines reached with the system `ssh` where projects live and
//! agents run. The daemon stays the only brain; a Launchpad helper on each host (the same binary,
//! uploaded and version-matched by the daemon) owns the harness and terminal processes there.
//!
//! A folder on a host is written `<host>:<path>` (`loc.rs`); `exec.rs` and `hostfs.rs` route commands
//! and file operations by that prefix, so most features need no host-specific code.

pub mod conn;
pub mod helper;
mod install;
pub mod loc;
pub mod preflight;
pub mod proto;
mod relay;
pub mod sshconfig;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use conn::{Conn, LinkState, RemoteProc};

/// A machine reached with `ssh`, as registered in Settings → Hosts.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Host {
    /// Short stable name, used in host locations (`devbox:/home/…`) and labels.
    pub id: String,
    /// What `ssh` connects to: an alias of `~/.ssh/config` or `user@host`.
    pub destination: String,
    pub port: Option<u16>,
    /// Where the host's projects live (Scan, Clone, task worktrees). `~` is the host's home.
    pub projects_root: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// A host and its live state, part of `Status`.
#[derive(Clone, Debug, Serialize, TS)]
pub struct HostStatus {
    pub host: Host,
    pub state: LinkState,
    pub error: Option<String>,
    /// When `state` last changed (Unix ms).
    #[ts(type = "number")]
    pub since: i64,
    pub info: Option<proto::HostInfo>,
    #[ts(type = "number | null")]
    pub latency_ms: Option<u64>,
    /// A password, passphrase or host key question from `ssh`, answered in a dialog and never stored.
    pub prompt: Option<HostPrompt>,
    pub checks: Vec<preflight::Check>,
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct HostPrompt {
    pub id: String,
    pub text: String,
    /// A password or passphrase (masked input), rather than a yes/no question.
    pub secret: bool,
}

/// The host needs an owner action; it is not retried as if the network were down.
#[derive(Debug)]
pub struct NeedsSetup(pub String);
impl std::fmt::Display for NeedsSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for NeedsSetup {}

type Prompt = (HostPrompt, tokio::sync::oneshot::Sender<Option<String>>);

pub struct Inner {
    pub(crate) boot: String,
    pub(crate) profile: String,
    paths: Arc<crate::paths::Paths>,
    env: crate::shellenv::UserEnv,
    db: crate::db::Db,
    status: crate::status::StatusHandle,
    records: RwLock<HashMap<String, Host>>,
    conns: RwLock<HashMap<String, Arc<Conn>>>,
    prompts: Mutex<HashMap<String, Prompt>>,
    checks: Mutex<HashMap<String, (Vec<preflight::Check>, i64)>>,
    /// The running daemon, set once it is assembled (session notices from relayed agent calls).
    app: OnceLock<crate::App>,
}

#[derive(Clone)]
pub struct Hosts(Arc<Inner>);

impl std::ops::Deref for Hosts {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

static HOSTS: OnceLock<Hosts> = OnceLock::new();

/// The registry of the running daemon (None in the CLI and the helper).
pub fn global() -> Option<&'static Hosts> {
    HOSTS.get()
}

/// The connection to a host, for commands and file operations on its folders.
pub fn conn(id: &str) -> Result<Arc<Conn>> {
    let hosts = global().context("SSH hosts are only available in the daemon")?;
    hosts.conns.read().unwrap().get(id).cloned().with_context(|| format!("unknown host {id}; add it in Settings → Hosts"))
}

/// The helper build: a hash of the binary, so a rebuilt development binary replaces the helper too.
pub fn build_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let exe = std::env::current_exe().ok().and_then(|p| std::fs::File::open(p).ok());
        let Some(mut f) = exe else { return crate::COMMIT.chars().take(16).collect() };
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            match std::io::Read::read(&mut f, &mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(_) => return crate::COMMIT.chars().take(16).collect(),
            }
        }
        hex::encode(hasher.finalize())[..16].to_string()
    })
}

/// `"$HOME/<rel>"`, for a command run by the remote login shell.
pub(crate) fn shell_quote_home(rel: &str) -> String {
    format!("\"$HOME/{}\"", rel.replace(['"', '\\', '$', '`'], ""))
}

/// The system `ssh`, with the user's own configuration (jump hosts, certificates, agent keys) and a
/// shared master connection. Agent, X11 and port forwarding are disabled on Launchpad's connection.
/// Password and host key questions go to the UI through `launchpad` as `SSH_ASKPASS`.
pub(crate) fn ssh_command(hosts: &Hosts, host: &Host) -> tokio::process::Command {
    let control = hosts.paths.data_dir.join("ssh");
    let _ = std::fs::create_dir_all(&control);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700));
    }
    let program = hosts.env.which("ssh").unwrap_or_else(|| "ssh".into());
    let mut cmd = tokio::process::Command::new(program);
    cmd.env_clear().envs(hosts.env.vars().iter());
    cmd.args(["-T", "-x", "-a", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", "-o", "ConnectTimeout=20", "-o", "ClearAllForwardings=yes"]);
    // OpenSSH appends a dot and 16 random characters when creating the socket. Allow for the
    // 40-character %C hash and the smaller Unix socket limit on macOS. Long data paths still
    // work without connection sharing; also override a ControlPath inherited from ssh config.
    if control.as_os_str().len() + 1 + 40 + 17 < 104 {
        cmd.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=600"]);
        cmd.arg("-o").arg(format!("ControlPath={}/%C", control.display()));
    } else {
        cmd.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    }
    if let Some(port) = host.port {
        cmd.args(["-p", &port.to_string()]);
    }
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("SSH_ASKPASS", exe).env("SSH_ASKPASS_REQUIRE", "force").env("LAUNCHPAD_ASKPASS", &host.id);
        cmd.env("LAUNCHPAD_PROFILE", &hosts.profile).env("LAUNCHPAD_PORT", hosts.paths.port.to_string());
        if hosts.env.get("DISPLAY").is_none() {
            cmd.env("DISPLAY", "launchpad");
        }
    }
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    cmd.arg("--").arg(&host.destination);
    cmd
}

/// Runs one short command on the host over SSH (outside the helper: install and probes).
pub(crate) async fn ssh_run(hosts: &Hosts, host: &Host, command: &str, stdin: Option<std::path::PathBuf>, timeout: std::time::Duration) -> Result<(bool, String, String)> {
    let mut cmd = ssh_command(hosts, host);
    cmd.arg(command).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    let mut child = cmd.spawn().context("start ssh")?;
    if let Some(path) = stdin {
        let mut w = child.stdin.take().context("ssh stdin")?;
        tokio::spawn(async move {
            if let Ok(mut f) = tokio::fs::File::open(&path).await {
                let _ = tokio::io::copy(&mut f, &mut w).await;
            }
        });
    }
    let out = tokio::time::timeout(timeout, child.wait_with_output()).await.map_err(|_| anyhow::anyhow!("ssh did not finish within {} s", timeout.as_secs()))??;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).trim().to_string()))
}

impl Hosts {
    /// Loads the registered hosts and starts their connections.
    pub async fn start(paths: Arc<crate::paths::Paths>, env: crate::shellenv::UserEnv, db: crate::db::Db, status: crate::status::StatusHandle) -> Result<Hosts> {
        let rows: Vec<Host> = db
            .call(|c| {
                let mut st = c.prepare("SELECT json FROM host")?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?.filter_map(|r| r.ok()).filter_map(|j| serde_json::from_str(&j).ok()).collect();
                Ok(rows)
            })
            .await?;
        let hosts = Hosts(Arc::new(Inner {
            boot: uuid::Uuid::new_v4().to_string(),
            profile: paths.profile.name().to_string(),
            paths,
            env,
            db,
            status,
            records: RwLock::default(),
            conns: RwLock::default(),
            prompts: Mutex::default(),
            checks: Mutex::default(),
            app: OnceLock::new(),
        }));
        let _ = HOSTS.set(hosts.clone());
        for h in rows {
            hosts.open(h);
        }
        hosts.publish();
        Ok(hosts)
    }

    fn open(&self, h: Host) {
        if cfg!(windows) {
            tracing::warn!("SSH hosts are not supported on Windows yet; {} stays offline", h.id);
        }
        let conn = Conn::new(&h.id, self.clone());
        self.records.write().unwrap().insert(h.id.clone(), h.clone());
        self.conns.write().unwrap().insert(h.id.clone(), conn.clone());
        if !cfg!(windows) {
            tokio::spawn(conn.run());
        }
    }

    pub(crate) fn set_app(&self, app: &crate::App) {
        let _ = self.app.set(app.clone());
    }

    pub(crate) fn app(&self) -> Option<crate::App> {
        self.app.get().cloned()
    }

    pub fn record(&self, id: &str) -> Option<Host> {
        self.records.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Host> {
        let mut v: Vec<Host> = self.records.read().unwrap().values().cloned().collect();
        v.sort_by_key(|h| h.created_at);
        v
    }

    pub fn snapshot(&self) -> Vec<HostStatus> {
        let prompts = self.prompts.lock().unwrap();
        let checks = self.checks.lock().unwrap();
        self.list()
            .into_iter()
            .map(|host| {
                let conn = self.conns.read().unwrap().get(&host.id).cloned();
                let (state, error, since) = conn.as_ref().map(|c| c.state.lock().unwrap().clone()).unwrap_or((LinkState::Unreachable, None, 0));
                let (checks, checked_at) = checks.get(&host.id).cloned().map(|(c, at)| (c, Some(at))).unwrap_or_default();
                HostStatus {
                    info: conn.as_ref().and_then(|c| c.info.lock().unwrap().clone()),
                    latency_ms: conn.as_ref().and_then(|c| *c.latency_ms.lock().unwrap()),
                    prompt: prompts.get(&host.id).map(|(p, _)| p.clone()),
                    state,
                    error,
                    since,
                    checks,
                    checked_at,
                    host,
                }
            })
            .collect()
    }

    pub(crate) fn publish(&self) {
        let snapshot = self.snapshot();
        self.status.update(|s| s.hosts = snapshot);
    }

    pub fn status(&self, id: &str) -> Option<HostStatus> {
        self.snapshot().into_iter().find(|h| h.host.id == id)
    }

    /// The host's home folder, once connected.
    pub fn home(&self, id: &str) -> Option<String> {
        conn(id).ok()?.info.lock().unwrap().as_ref().map(|i| i.home.clone())
    }

    /// The projects root of a host as a location (`devbox:/home/u/projects`).
    pub fn projects_root(&self, id: &str) -> Result<String> {
        let h = self.record(id).with_context(|| format!("unknown host {id}"))?;
        let home = self.home(id).with_context(|| format!("{id} is not connected yet"))?;
        let root = h.projects_root.trim();
        let native = if root == "~" { home } else if let Some(rest) = root.strip_prefix("~/") { format!("{}/{rest}", home.trim_end_matches('/')) } else { root.to_string() };
        Ok(loc::join(Some(id), &native))
    }

    async fn save(&self, h: &Host) -> Result<()> {
        let (id, json) = (h.id.clone(), serde_json::to_string(h)?);
        self.db.call(move |c| Ok(c.execute("INSERT INTO host (id, json) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET json = excluded.json", rusqlite::params![id, json])?)).await?;
        Ok(())
    }

    pub async fn add(&self, destination: &str, id: Option<&str>, port: Option<u16>, projects_root: Option<&str>) -> Result<Host> {
        ensure!(!cfg!(windows), "SSH hosts are not supported on Windows yet");
        let destination = destination.trim();
        ensure!(!destination.is_empty() && !destination.starts_with('-') && !destination.contains(char::is_whitespace), "enter an ssh destination: an alias of ~/.ssh/config or user@host");
        let id = match id.map(str::trim).filter(|i| !i.is_empty()) {
            Some(i) => i.to_string(),
            None => default_id(destination),
        };
        ensure!(loc::valid_id(&id), "use a host name of 2 to 63 lowercase letters, digits, dots, dashes or underscores");
        ensure!(self.record(&id).is_none(), "a host named {id} already exists");
        let h = Host {
            id,
            destination: destination.to_string(),
            port,
            projects_root: projects_root.map(str::trim).filter(|r| !r.is_empty()).unwrap_or("~/projects").to_string(),
            created_at: crate::util::now_ms(),
        };
        self.save(&h).await?;
        self.open(h.clone());
        self.publish();
        Ok(h)
    }

    pub async fn update(&self, id: &str, projects_root: Option<&str>) -> Result<Host> {
        let mut h = self.record(id).context("unknown host")?;
        if let Some(root) = projects_root.map(str::trim).filter(|r| !r.is_empty()) {
            h.projects_root = root.to_string();
        }
        self.save(&h).await?;
        self.records.write().unwrap().insert(h.id.clone(), h.clone());
        self.publish();
        Ok(h)
    }

    /// Removes a host without projects. Processes on it are stopped by the helper.
    pub async fn remove(&self, app: &crate::App, id: &str) -> Result<()> {
        let projects = crate::projects::list(app).await?;
        let on_host: Vec<String> = projects.iter().filter(|p| loc::host_of(&p.path) == Some(id)).map(|p| p.name.clone()).collect();
        ensure!(on_host.is_empty(), "remove the projects of {id} first: {}", on_host.join(", "));
        if let Some(c) = self.conns.write().unwrap().remove(id) {
            c.stop();
        }
        self.records.write().unwrap().remove(id);
        let id2 = id.to_string();
        self.db.call(move |c| Ok(c.execute("DELETE FROM host WHERE id = ?1", [id2])?)).await?;
        self.publish();
        Ok(())
    }

    /// Reconnects now instead of waiting for the next attempt.
    pub fn retry(&self, id: &str) -> Result<()> {
        let c = conn(id)?;
        if c.link() == LinkState::Connected {
            c.disconnect();
        }
        c.retry.notify_one();
        Ok(())
    }

    /// An `ssh` question (`SSH_ASKPASS`): waits for the owner's answer in the UI, or None.
    pub async fn ask(&self, id: &str, text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        let secret = !(lower.contains("yes/no") || lower.contains("continue connecting"));
        let prompt = HostPrompt { id: uuid::Uuid::new_v4().to_string(), text: text.trim().to_string(), secret };
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.prompts.lock().unwrap().insert(id.to_string(), (prompt.clone(), tx));
        self.publish();
        let answer = tokio::time::timeout(std::time::Duration::from_secs(180), rx).await.ok().and_then(|r| r.ok()).flatten();
        {
            let mut prompts = self.prompts.lock().unwrap();
            if prompts.get(id).is_some_and(|(p, _)| p.id == prompt.id) {
                prompts.remove(id);
            }
        }
        self.publish();
        answer
    }

    pub fn answer(&self, id: &str, prompt_id: &str, answer: Option<String>) -> Result<()> {
        let mut prompts = self.prompts.lock().unwrap();
        let Some((p, _)) = prompts.get(id) else { bail!("this question was already answered") };
        ensure!(p.id == prompt_id, "this question was already answered");
        let (_, tx) = prompts.remove(id).unwrap();
        let _ = tx.send(answer);
        drop(prompts);
        self.publish();
        Ok(())
    }

    pub(crate) fn set_checks(&self, id: &str, checks: Vec<preflight::Check>) {
        self.checks.lock().unwrap().insert(id.to_string(), (checks, crate::util::now_ms()));
        self.publish();
    }
}

/// `devbox` from `devbox`, `user@devbox.lan` or `devbox.example.com`.
fn default_id(destination: &str) -> String {
    let host = destination.rsplit('@').next().unwrap_or(destination);
    let host = host.split(':').next().unwrap_or(host);
    let first = if host.parse::<std::net::IpAddr>().is_ok() { host.replace('.', "-") } else { host.split('.').next().unwrap_or(host).to_string() };
    let id: String = first.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '-' }).collect();
    let id = id.trim_matches('-').to_string();
    if id.len() < 2 { format!("host-{id}") } else { id }
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_ids_from_destinations() {
        assert_eq!(super::default_id("devbox"), "devbox");
        assert_eq!(super::default_id("wauplin@devbox.lan"), "devbox");
        assert_eq!(super::default_id("root@10.0.0.4"), "10-0-0-4");
        assert_eq!(super::default_id("GPU_01"), "gpu_01");
        assert_eq!(super::default_id("x"), "host-x");
    }
}

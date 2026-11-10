//! Runner self-update, started by an owner click (on Home or on the runner): when no agent of this runner is in a
//! turn, the runner downloads Home's own executable through its tunnel (`GET /api/runners/build`), checks its
//! SHA-256, OS and architecture, replaces its executable and restarts itself in place (same process ID, so a
//! systemd service keeps running). Terminals end with the restart; sessions resume as after any daemon restart.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::App;

/// Response headers of `GET /api/runners/build`.
pub const SHA256_HEADER: &str = "x-launchpad-sha256";
pub const COMMIT_HEADER: &str = "x-launchpad-commit";
pub const OS_HEADER: &str = "x-launchpad-os";
pub const ARCH_HEADER: &str = "x-launchpad-arch";

/// Seconds without any agent in a turn before the update starts (the turn-end push and sync finish first).
const QUIET: u32 = 4;
const CHECK: Duration = Duration::from_secs(5);

/// A runner update in progress, in the runner's status.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunnerUpdate {
    /// Waits until no agent of this runner is in a turn.
    Waiting { running: u32 },
    Downloading,
    /// The new build is in place; the daemon restarts.
    Restarting { commit: String },
    Failed { error: String },
}

static STATE: Mutex<Option<RunnerUpdate>> = Mutex::new(None);
/// The executable to start after the update, once set.
static RESTART: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
static SHUTDOWN: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Whether this daemon can update itself: a paired runner on Unix (not a Burst job, which runs Home's image).
pub fn supported(app: &App) -> bool {
    cfg!(unix) && app.paths.role == crate::role::Role::Runner && crate::home_link::burst().is_none()
}

fn set(app: &App, state: Option<RunnerUpdate>) {
    *STATE.lock().unwrap() = state.clone();
    app.status.update(|s| s.runner_update = state);
}

/// Agents of this runner in a turn.
pub fn running(app: &App) -> u32 {
    app.sessions.list().iter().filter(|s| s.status.is_running()).count() as u32
}

/// Starts an update. It waits for running agents first; a second click while one is pending changes nothing.
pub fn start(app: &App) -> Result<RunnerUpdate> {
    anyhow::ensure!(supported(app), "only a paired runner on Linux or macOS updates itself; reinstall this one");
    anyhow::ensure!(crate::home_link::load(&app.paths).is_some(), "this runner is not paired with a Home: run launchpad connect");
    {
        let current = STATE.lock().unwrap().clone();
        if let Some(s) = current
            && !matches!(s, RunnerUpdate::Failed { .. })
        {
            return Ok(s);
        }
    }
    let first = RunnerUpdate::Waiting { running: running(app) };
    set(app, Some(first.clone()));
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = run(&app).await {
            tracing::warn!("runner update: {e:#}");
            if STATE.lock().unwrap().is_some() {
                set(&app, Some(RunnerUpdate::Failed { error: format!("{e:#}") }));
            }
        }
    });
    Ok(first)
}

/// Cancels an update that still waits for agents; clears a failed one.
pub fn cancel(app: &App) -> Result<()> {
    let current = STATE.lock().unwrap().clone();
    anyhow::ensure!(matches!(current, None | Some(RunnerUpdate::Waiting { .. } | RunnerUpdate::Failed { .. })), "the update is already downloading or restarting");
    set(app, None);
    Ok(())
}

async fn run(app: &App) -> Result<()> {
    let mut quiet = 0;
    while quiet < QUIET {
        tokio::time::sleep(CHECK).await;
        if !matches!(*STATE.lock().unwrap(), Some(RunnerUpdate::Waiting { .. })) {
            return Ok(()); // cancelled
        }
        let n = running(app);
        quiet = if n == 0 { quiet + 1 } else { 0 };
        set(app, Some(RunnerUpdate::Waiting { running: n }));
    }
    set(app, Some(RunnerUpdate::Downloading));
    let exe = std::env::current_exe().context("find this executable")?;
    let commit = download(app, &exe).await?;
    tracing::info!("runner updated to {commit}: restarting");
    set(app, Some(RunnerUpdate::Restarting { commit }));
    let _ = RESTART.set(exe);
    // A graceful stop first; the restart happens anyway if open connections keep the server up.
    SHUTDOWN.notify_waiters();
    tokio::time::sleep(Duration::from_secs(10)).await;
    restart_now();
    Ok(())
}

/// Downloads Home's build next to `exe` and moves it over `exe`. Returns Home's commit.
async fn download(app: &App, exe: &std::path::Path) -> Result<String> {
    let link = crate::home_link::load(&app.paths).context("this runner is not paired with a Home")?;
    let headers = vec![("x-launchpad-runner".to_string(), link.credential())];
    let empty: crate::home_link::ByteStream = Box::pin(futures_util::stream::empty());
    let reply = crate::home_link::http(crate::runners::proto::HttpTarget::Home, "GET", "/api/runners/build", headers, empty).await?;
    let header = |name: &str| reply.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default();
    if reply.status != 200 {
        let mut body = reply.body;
        let mut text = Vec::new();
        while let Some(Ok(b)) = body.next().await {
            text.extend_from_slice(&b);
        }
        let value: serde_json::Value = serde_json::from_slice(&text).unwrap_or_default();
        anyhow::bail!("Home did not send its build: {}", value["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("HTTP {}", reply.status)));
    }
    let (sha, commit, os, arch) = (header(SHA256_HEADER), header(COMMIT_HEADER), header(OS_HEADER), header(ARCH_HEADER));
    anyhow::ensure!(os == std::env::consts::OS && arch == std::env::consts::ARCH, "Home runs on {os} {arch} and this runner on {} {}: reinstall it with a build for this machine", std::env::consts::OS, std::env::consts::ARCH);
    anyhow::ensure!(sha.len() == 64, "Home sent no checksum");
    let dir = exe.parent().context("the executable has no folder")?;
    let tmp = dir.join(format!(".launchpad-update-{}", std::process::id()));
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await.with_context(|| format!("write in {}", dir.display()))?;
        let mut hasher = Sha256::new();
        let mut body = reply.body;
        while let Some(part) = body.next().await {
            let part = part.context("download Home's build")?;
            hasher.update(&part);
            tokio::io::AsyncWriteExt::write_all(&mut file, &part).await?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        file.sync_all().await?;
        anyhow::ensure!(hex::encode(hasher.finalize()) == sha, "the download does not match Home's build; try again");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
        }
        tokio::fs::rename(&tmp, exe).await.with_context(|| format!("replace {}", exe.display()))?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result.map(|_| commit)
}

/// Resolves when an update asks the server to stop.
pub async fn shutdown_requested() {
    SHUTDOWN.notified().await;
}

/// After the server stopped: starts the new build in this process when an update asked for it.
pub fn restart_if_updated() {
    if RESTART.get().is_some() {
        restart_now();
    }
}

fn restart_now() {
    let Some(exe) = RESTART.get() else { return };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
        tracing::error!("restart after the update: {err}");
    }
    // The new build is in place: a service manager restarts it.
    std::process::exit(75);
}

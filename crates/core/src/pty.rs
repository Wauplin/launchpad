//! Linux PTY for the optional embedded harness TUI. It lives in the session worker.
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::io::unix::AsyncFd;

use crate::{App, model::Session};

const BUFFER_LIMIT: usize = 1024 * 1024;

struct Pty {
    fd: AsyncFd<OwnedFd>,
    pid: i32,
    alive: AtomicBool,
    // A byte buffer, not text: UTF-8 and escape sequences may span read boundaries.
    output: Mutex<Output>,
    input: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    end: u64,
    truncated: bool,
}

#[derive(Clone, Default)]
pub struct Terminal {
    inner: Arc<Mutex<Option<Arc<Pty>>>>,
}

impl Terminal {
    pub fn running(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|p| p.alive.load(Ordering::SeqCst))
    }

    pub fn clear(&self) -> Result<()> {
        let mut owner = self.inner.lock().unwrap();
        anyhow::ensure!(
            !owner
                .as_ref()
                .is_some_and(|p| p.alive.load(Ordering::SeqCst)),
            "terminal is still running"
        );
        *owner = None;
        Ok(())
    }

    pub fn snapshot(&self) -> serde_json::Value {
        match self.inner.lock().unwrap().as_ref() {
            Some(p) => {
                let output = p.output.lock().unwrap();
                serde_json::json!({"running":p.alive.load(Ordering::SeqCst),"output":STANDARD.encode(&output.bytes),"truncated":output.truncated,"end":output.end})
            }
            None => serde_json::json!({"running":false,"output":"","truncated":false,"end":0}),
        }
    }

    pub async fn open(&self, app: &App, s: &Session) -> Result<()> {
        anyhow::ensure!(!self.running(), "terminal is already running");
        let is_temp = match &s.project_id {
            Some(id) => crate::projects::get(app, id)
                .await?
                .is_some_and(|p| p.is_temp),
            None => false,
        };
        let argv = crate::terminal::tui_command(app, s, is_temp)?;
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty initializes the two file descriptors; no name buffer is passed.
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        anyhow::ensure!(result == 0, "open PTY: {}", std::io::Error::last_os_error());
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            anyhow::ensure!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == 0,
                "set PTY close-on-exec"
            );
        }
        anyhow::ensure!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == 0,
            "set PTY nonblocking"
        );
        let slave = std::fs::File::from(slave);
        let mut cmd = crate::procs::command(&app.env, &argv[0], Some(std::path::Path::new(&s.cwd)));
        cmd.args(&argv[1..])
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("LAUNCHPAD_PROFILE", app.paths.profile.name())
            .env("LAUNCHPAD_SESSION_ID", &s.id)
            .stdin(slave.try_clone()?)
            .stdout(slave.try_clone()?)
            .stderr(slave);
        // SAFETY: these libc calls are async-signal-safe, performed only in the child before exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().context("start embedded TUI")?;
        let p = Arc::new(Pty {
            fd: AsyncFd::new(master)?,
            pid: child.id().context("TUI pid")? as i32,
            alive: AtomicBool::new(true),
            output: Mutex::new(Output::default()),
            input: tokio::sync::Mutex::new(()),
        });
        *self.inner.lock().unwrap() = Some(p.clone());
        let (bus, topic, reader) = (app.bus.clone(), format!("terminal/{}", s.id), p.clone());
        tokio::spawn(async move {
            let mut bytes = [0u8; 8192];
            loop {
                let Ok(mut ready) = reader.fd.readable().await else {
                    break;
                };
                let result = ready.try_io(|fd| {
                    let n = unsafe {
                        libc::read(fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len())
                    };
                    if n < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                });
                match result {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Ok(Ok(n)) => {
                        let mut output = reader.output.lock().unwrap();
                        let start = output.end;
                        output.end += n as u64;
                        let end = output.end;
                        output.bytes.extend_from_slice(&bytes[..n]);
                        if output.bytes.len() > BUFFER_LIMIT {
                            let excess = output.bytes.len() - BUFFER_LIMIT;
                            output.bytes.drain(..excess);
                            output.truncated = true;
                        }
                        drop(output);
                        bus.publish(&topic, "output", serde_json::json!({"data":STANDARD.encode(&bytes[..n]),"start":start,"end":end}));
                    }
                    Err(_) => continue,
                }
            }
        });
        let (bus, topic) = (app.bus.clone(), format!("terminal/{}", s.id));
        tokio::spawn(async move {
            let status = child.wait().await;
            p.alive.store(false, Ordering::SeqCst);
            bus.publish(
                &topic,
                "exit",
                serde_json::json!({"status":format!("{status:?}")}),
            );
        });
        Ok(())
    }

    pub async fn input(&self, data: &[u8]) -> Result<()> {
        anyhow::ensure!(data.len() <= 64 * 1024, "terminal input is too large");
        let p = self
            .inner
            .lock()
            .unwrap()
            .clone()
            .context("no embedded terminal")?;
        anyhow::ensure!(p.alive.load(Ordering::SeqCst), "terminal has exited");
        let _input = p.input.lock().await;
        let mut remaining = data;
        while !remaining.is_empty() {
            let mut ready = p.fd.writable().await?;
            match ready.try_io(|fd| {
                let n = unsafe {
                    libc::write(fd.as_raw_fd(), remaining.as_ptr().cast(), remaining.len())
                };
                if n < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(result) => {
                    let n = result?;
                    anyhow::ensure!(n > 0, "terminal closed");
                    remaining = &remaining[n..];
                }
                Err(_) => continue,
            }
        }
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(
            (2..=500).contains(&rows) && (2..=500).contains(&cols),
            "invalid terminal size"
        );
        let p = self
            .inner
            .lock()
            .unwrap()
            .clone()
            .context("no embedded terminal")?;
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        anyhow::ensure!(
            unsafe { libc::ioctl(p.fd.as_raw_fd(), libc::TIOCSWINSZ, &size) } == 0,
            "resize terminal: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let p = self.inner.lock().unwrap().clone();
        if let Some(p) = p
            && p.alive.load(Ordering::SeqCst)
        {
            unsafe {
                libc::kill(-p.pid, libc::SIGTERM);
            }
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while p.alive.load(Ordering::SeqCst) && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if p.alive.load(Ordering::SeqCst) {
                unsafe {
                    libc::kill(-p.pid, libc::SIGKILL);
                }
            }
            while p.alive.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        Ok(())
    }
}

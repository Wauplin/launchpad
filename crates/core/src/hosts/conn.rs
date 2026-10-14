//! The daemon side of a host connection: one `ssh … host-helper bridge` process per host, reconnected
//! every 15 seconds while the host is unreachable. Remote processes survive reconnects: their output
//! continues from the last received offset and unacknowledged input is sent again.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, mpsc, oneshot, watch};

use super::proto::*;
use super::{Host, Hosts};
use crate::exec::{ExitStatus, ProcessLost};

/// Acknowledge output after this many new bytes, so the helper can drop it.
const ACK_EVERY: u64 = 256 * 1024;

/// What the UI shows for a host.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Connecting,
    Connected,
    /// The connection failed or dropped; retried every 15 s. Remote turns keep running.
    Unreachable,
    /// The host needs an owner action (unsupported platform, helper install failed).
    NeedsSetup,
}

pub(super) struct Stream {
    got: u64,
    acked: u64,
    tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

pub(super) struct Input {
    /// Unacknowledged bytes, starting at offset `start`.
    buf: VecDeque<u8>,
    start: u64,
    closed: bool,
}

impl Input {
    fn end(&self) -> u64 {
        self.start + self.buf.len() as u64
    }
}

/// A process the helper runs for this daemon.
pub struct RemoteProc {
    pub(super) key: String,
    pub(super) streams: Mutex<[Stream; 2]>,
    pub(super) input: Mutex<Input>,
    exit: watch::Sender<Option<Result<ExitStatus, String>>>,
    /// Signals and the PTY size, sent again when the connection returns.
    pending: Mutex<Vec<Op>>,
    size: Mutex<Option<(u16, u16)>>,
}

impl RemoteProc {
    fn finish(&self, result: Result<ExitStatus, String>) {
        for s in self.streams.lock().unwrap().iter_mut() {
            s.tx.take();
        }
        self.exit.send_replace(Some(result));
    }

    pub fn exited(&self) -> bool {
        self.exit.borrow().is_some()
    }
}

pub struct Conn {
    pub id: String,
    hosts: Hosts,
    pub(super) state: Mutex<(LinkState, Option<String>, i64)>,
    out: Mutex<Option<mpsc::UnboundedSender<String>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, ErrorInfo>>>>,
    next_id: AtomicU64,
    procs: Mutex<HashMap<String, Arc<RemoteProc>>>,
    connected: watch::Sender<bool>,
    pub(super) retry: Notify,
    pub(super) info: Mutex<Option<HostInfo>>,
    pub(super) latency_ms: Mutex<Option<u64>>,
    stopped: std::sync::atomic::AtomicBool,
    child: Mutex<Option<u32>>,
}

impl Conn {
    pub(super) fn new(id: &str, hosts: Hosts) -> Arc<Conn> {
        Arc::new(Conn {
            id: id.to_string(),
            hosts,
            state: Mutex::new((LinkState::Connecting, None, crate::util::now_ms())),
            out: Mutex::default(),
            pending: Mutex::default(),
            next_id: AtomicU64::new(0),
            procs: Mutex::default(),
            connected: watch::channel(false).0,
            retry: Notify::new(),
            info: Mutex::default(),
            latency_ms: Mutex::default(),
            stopped: Default::default(),
            child: Mutex::default(),
        })
    }

    pub(super) fn set_state(&self, state: LinkState, error: Option<String>) {
        {
            let mut s = self.state.lock().unwrap();
            if s.0 == state && s.1 == error {
                return;
            }
            let since = if s.0 == state { s.2 } else { crate::util::now_ms() };
            *s = (state, error, since);
        }
        self.hosts.publish();
    }

    pub fn link(&self) -> LinkState {
        self.state.lock().unwrap().0.clone()
    }

    fn send(&self, msg: &ToHelper) -> bool {
        let out = self.out.lock().unwrap();
        match &*out {
            Some(tx) => tx.send(serde_json::to_string(msg).unwrap_or_default()).is_ok(),
            None => false,
        }
    }

    /// Waits a little for a connection in progress; an unreachable host fails at once.
    async fn wait_connected(&self) -> Result<()> {
        let mut rx = self.connected.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            if *rx.borrow_and_update() {
                return Ok(());
            }
            let (state, error, _) = self.state.lock().unwrap().clone();
            if state != LinkState::Connecting {
                bail!("{} is {}{}", self.id, if state == LinkState::NeedsSetup { "not set up" } else { "unreachable" }, error.map(|e| format!(": {e}")).unwrap_or_default());
            }
            if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
                bail!("{} is not connected yet", self.id);
            }
        }
    }

    pub async fn request(&self, op: Op) -> Result<Value> {
        self.request_info(op).await.map_err(|e| anyhow!("{}", e.message))
    }

    /// Like `request`, with the error kind (`not_found`).
    pub async fn request_info(&self, op: Op) -> Result<Value, ErrorInfo> {
        let other = |e: anyhow::Error| ErrorInfo { message: format!("{e:#}"), kind: "other".into() };
        self.wait_connected().await.map_err(other)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if !self.send(&ToHelper::Req { id, op }) {
            self.pending.lock().unwrap().remove(&id);
            return Err(other(anyhow!("{} disconnected", self.id)));
        }
        rx.await.map_err(|_| other(anyhow!("the connection to {} was lost during the request", self.id)))?
    }

    /// Starts a process owned by the helper.
    pub async fn spawn(self: &Arc<Self>, argv: Vec<String>, cwd: Option<String>, env: Vec<(String, String)>, pty: Option<(u16, u16)>, stdin: bool) -> Result<(Arc<RemoteProc>, [mpsc::UnboundedReceiver<Vec<u8>>; 2])> {
        let key = uuid::Uuid::new_v4().to_string();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (err_tx, err_rx) = mpsc::unbounded_channel();
        let p = Arc::new(RemoteProc {
            key: key.clone(),
            streams: Mutex::new([Stream { got: 0, acked: 0, tx: Some(out_tx) }, Stream { got: 0, acked: 0, tx: Some(err_tx) }]),
            input: Mutex::new(Input { buf: VecDeque::new(), start: 0, closed: !stdin }),
            exit: watch::channel(None).0,
            pending: Mutex::default(),
            size: Mutex::new(pty),
        });
        // Registered first: output may arrive before the response.
        self.procs.lock().unwrap().insert(key.clone(), p.clone());
        if let Err(e) = self.request(Op::Spawn { key: key.clone(), argv, cwd, env, pty, stdin }).await {
            self.procs.lock().unwrap().remove(&key);
            return Err(e);
        }
        Ok((p, [out_rx, err_rx]))
    }

    /// Input is sent while holding its lock, so it reaches the helper in offset order.
    pub fn write(&self, p: &RemoteProc, data: &[u8]) {
        let mut input = p.input.lock().unwrap();
        if input.closed {
            return;
        }
        let off = input.end();
        input.buf.extend(data);
        self.send(&ToHelper::Input { key: p.key.clone(), off, data: B64(data.to_vec()) });
    }

    pub fn close_input(&self, p: &RemoteProc) {
        let mut input = p.input.lock().unwrap();
        if input.closed {
            return;
        }
        input.closed = true;
        let off = input.end();
        self.send(&ToHelper::CloseInput { key: p.key.clone(), off });
    }

    /// Sends a signal now, or as soon as the connection returns.
    pub fn signal(&self, p: &RemoteProc, signal: i32, group: bool) {
        if p.exited() {
            return;
        }
        let op = Op::Signal { key: p.key.clone(), signal, group };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        if !self.send(&ToHelper::Req { id, op: op.clone() }) {
            p.pending.lock().unwrap().push(op);
        }
    }

    pub fn resize(&self, p: &RemoteProc, rows: u16, cols: u16) {
        *p.size.lock().unwrap() = Some((rows, cols));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(&ToHelper::Req { id, op: Op::Resize { key: p.key.clone(), rows, cols } });
    }

    /// Forgets a process; the helper stops it if it still runs.
    pub fn release(&self, p: &RemoteProc) {
        self.procs.lock().unwrap().remove(&p.key);
        self.send(&ToHelper::Release { key: p.key.clone() });
        if !p.exited() {
            p.finish(Err("released".into()));
        }
    }

    pub async fn wait(&self, p: &RemoteProc) -> std::io::Result<ExitStatus> {
        let mut rx = p.exit.subscribe();
        loop {
            if let Some(r) = rx.borrow_and_update().clone() {
                return r.map_err(|e| std::io::Error::other(ProcessLost(e)));
            }
            if rx.changed().await.is_err() {
                return Err(std::io::Error::other(ProcessLost("the host connection closed".into())));
            }
        }
    }

    /// Ends the connection; the loop reconnects unless the host was removed.
    pub(super) fn disconnect(&self) {
        if let Some(pid) = *self.child.lock().unwrap() {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            #[cfg(not(unix))]
            let _ = pid;
        }
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.disconnect();
        self.retry.notify_one();
        for p in self.procs.lock().unwrap().drain().map(|(_, p)| p) {
            p.finish(Err("the host was removed".into()));
        }
    }

    /// Connects, follows the connection until it drops, and starts again.
    pub(super) async fn run(self: Arc<Self>) {
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return;
            }
            let Some(host) = self.hosts.record(&self.id) else { return };
            self.set_state(LinkState::Connecting, None);
            let started = std::time::Instant::now();
            let mut delay = Duration::from_secs(15);
            match self.connect(&host).await {
                Ok(()) => {
                    self.set_state(LinkState::Unreachable, Some("the connection was lost".into()));
                    if started.elapsed() > Duration::from_secs(60) { delay = Duration::from_secs(2); }
                }
                Err(e) => {
                    let setup = e.chain().any(|c| c.is::<super::NeedsSetup>());
                    tracing::info!("host {}: {e:#}", self.id);
                    self.set_state(if setup { LinkState::NeedsSetup } else { LinkState::Unreachable }, Some(format!("{e:#}")));
                }
            }
            if self.stopped.load(Ordering::SeqCst) {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {},
                _ = self.retry.notified() => {},
            }
        }
    }

    async fn connect(self: &Arc<Self>, host: &Host) -> Result<()> {
        let bin = super::install::ensure(&self.hosts, host).await?;
        let mut cmd = super::ssh_command(&self.hosts, host);
        cmd.arg(format!("{} host-helper bridge --profile {}", super::shell_quote_home(&bin), self.hosts.profile))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().context("start ssh")?;
        *self.child.lock().unwrap() = child.id();
        let mut stdin = child.stdin.take().context("ssh stdin")?;
        let stdout = child.stdout.take().context("ssh stdout")?;
        let stderr = child.stderr.take().context("ssh stderr")?;
        let tail = Arc::new(Mutex::new(Vec::<String>::new()));
        {
            let tail = tail.clone();
            let id = self.id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("ssh {id}: {line}");
                    let mut t = tail.lock().unwrap();
                    t.push(line);
                    if t.len() > 5 {
                        t.remove(0);
                    }
                }
            });
        }
        let known: Vec<Known> = self.procs.lock().unwrap().values().map(|p| {
            let s = p.streams.lock().unwrap();
            Known { key: p.key.clone(), out: s[0].got, err: s[1].got }
        }).collect();
        let hello = ToHelper::Hello { boot: self.hosts.boot.clone(), host: self.id.clone(), profile: self.hosts.profile.clone(), known };
        stdin.write_all((serde_json::to_string(&hello)? + "\n").as_bytes()).await?;
        stdin.flush().await?;
        let mut lines = BufReader::with_capacity(1 << 20, stdout).lines();
        let started = std::time::Instant::now();
        let first = tokio::time::timeout(Duration::from_secs(60), lines.next_line()).await.map_err(|_| anyhow!("the host helper did not answer"))??;
        let Some(first) = first else {
            let _ = child.wait().await;
            bail!("{}", ssh_error(&tail.lock().unwrap()));
        };
        let FromHelper::Hello { version, procs, info } = serde_json::from_str(&first).context("unexpected helper answer")? else { bail!("unexpected helper answer") };
        anyhow::ensure!(version == super::build_id(), "the host runs helper build {version}");
        *self.latency_ms.lock().unwrap() = Some(started.elapsed().as_millis() as u64);
        *self.info.lock().unwrap() = Some(info);
        // Processes the helper no longer has ended with the helper (host restart): Interrupted, never Done.
        let lost: Vec<Arc<RemoteProc>> = {
            let mut mine = self.procs.lock().unwrap();
            let gone: Vec<String> = mine.keys().filter(|k| !procs.iter().any(|p| &p.key == *k)).cloned().collect();
            gone.iter().filter_map(|k| mine.remove(k)).collect()
        };
        for p in lost {
            p.finish(Err(format!("the process on {} ended while it was unreachable (the host or its helper restarted)", self.id)));
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                if rx.is_empty() && stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        *self.out.lock().unwrap() = Some(tx);
        // Input the helper did not write yet, then held signals and sizes.
        for state in &procs {
            let Some(p) = self.procs.lock().unwrap().get(&state.key).cloned() else { continue };
            {
                let mut input = p.input.lock().unwrap();
                let drop = state.input.saturating_sub(input.start).min(input.buf.len() as u64) as usize;
                input.buf.drain(..drop);
                input.start += drop as u64;
                if !input.buf.is_empty() {
                    self.send(&ToHelper::Input { key: p.key.clone(), off: input.start, data: B64(input.buf.iter().copied().collect()) });
                }
                if input.closed {
                    self.send(&ToHelper::CloseInput { key: p.key.clone(), off: input.end() });
                }
            }
            let ops: Vec<Op> = p.pending.lock().unwrap().drain(..).collect();
            let size = *p.size.lock().unwrap();
            for op in ops.into_iter().chain(size.map(|(rows, cols)| Op::Resize { key: p.key.clone(), rows, cols })) {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
                self.send(&ToHelper::Req { id, op });
            }
        }
        self.connected.send_replace(true);
        self.set_state(LinkState::Connected, None);
        tracing::info!("host {} connected", self.id);
        let me = self.clone();
        let pinger = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let t = std::time::Instant::now();
                if me.request(Op::Ping).await.is_ok() {
                    *me.latency_ms.lock().unwrap() = Some(t.elapsed().as_millis() as u64);
                }
            }
        });
        let result = loop {
            match lines.next_line().await {
                Ok(Some(line)) => match serde_json::from_str::<FromHelper>(&line) {
                    Ok(msg) => self.dispatch(msg),
                    Err(e) => tracing::warn!("host {}: invalid message: {e}", self.id),
                },
                Ok(None) => break Ok(()),
                Err(e) => break Err(e.into()),
            }
        };
        pinger.abort();
        self.connected.send_replace(false);
        self.out.lock().unwrap().take();
        writer.abort();
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err(ErrorInfo { message: format!("the connection to {} was lost", self.id), kind: "other".into() }));
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
        *self.child.lock().unwrap() = None;
        tracing::info!("host {} disconnected", self.id);
        result
    }

    fn dispatch(self: &Arc<Self>, msg: FromHelper) {
        match msg {
            FromHelper::Hello { .. } => {}
            FromHelper::Res { id, ok, err } => {
                if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(match err {
                        Some(e) => Err(e),
                        None => Ok(ok.unwrap_or(Value::Null)),
                    });
                }
            }
            FromHelper::Out { key, s, off, data } => {
                let Some(p) = self.procs.lock().unwrap().get(&key).cloned() else { return };
                let ack = {
                    let mut streams = p.streams.lock().unwrap();
                    let st = &mut streams[(s as usize).min(1)];
                    let end = off + data.0.len() as u64;
                    if end <= st.got {
                        None
                    } else {
                        if off > st.got {
                            tracing::warn!("host {}: {} bytes of output of {key} were dropped by the helper buffer", self.id, off - st.got);
                        }
                        let skip = st.got.saturating_sub(off) as usize;
                        if let Some(tx) = &st.tx {
                            let _ = tx.send(data.0[skip..].to_vec());
                        }
                        st.got = end;
                        if st.got - st.acked >= ACK_EVERY {
                            st.acked = st.got;
                            Some((streams[0].got, streams[1].got))
                        } else {
                            None
                        }
                    }
                };
                if let Some((out, err)) = ack {
                    self.send(&ToHelper::Ack { key, out, err });
                }
            }
            FromHelper::Exit { key, code, signal } => {
                let p = self.procs.lock().unwrap().remove(&key);
                if let Some(p) = p {
                    p.finish(Ok(ExitStatus { code, signal }));
                }
                self.send(&ToHelper::Release { key });
            }
            FromHelper::InputAck { key, off } => {
                let Some(p) = self.procs.lock().unwrap().get(&key).cloned() else { return };
                let mut input = p.input.lock().unwrap();
                let drop = off.saturating_sub(input.start).min(input.buf.len() as u64) as usize;
                input.buf.drain(..drop);
                input.start += drop as u64;
            }
            FromHelper::Cli { id, method, path, body } => {
                let me = self.clone();
                tokio::spawn(async move {
                    let (status, body) = match super::relay::call(&me.hosts, &me.id, &method, &path, body).await {
                        Ok(r) => r,
                        Err(e) => (502, serde_json::json!({"error": format!("{e:#}")}).to_string()),
                    };
                    me.send(&ToHelper::CliResponse { id, status, body });
                });
            }
        }
    }
}

fn ssh_error(tail: &[String]) -> String {
    let text = tail.iter().filter(|l| !l.trim().is_empty()).cloned().collect::<Vec<_>>().join(" ");
    if text.is_empty() { "ssh closed the connection".into() } else { text }
}

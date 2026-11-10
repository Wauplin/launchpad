//! Client for `codex app-server`: JSON-RPC over stdio, one JSON object per line, no `"jsonrpc"` field.
//!
//! A server is started on first use and started again after it exits, until [`CodexServer::shutdown`].
//! Contained sessions each have their own; the quota monitor has one per login.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use launchpad_core::shellenv::UserEnv;

/// A message from the app-server that is not a response to one of our requests.
#[derive(Clone, Debug)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// A request from the server (approvals, user input). Must be answered with [`CodexServer::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The process exited. Running turns are lost.
    Exited,
}

/// A server rejection is distinct from a transport error or timeout: only an explicit
/// no-active-turn rejection proves that steering did not deliver input.
#[derive(Debug)]
pub(super) struct RpcError(pub Value);
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "codex: {}", self.0) }
}
impl std::error::Error for RpcError {}

struct Proc {
    stdin: tokio::sync::Mutex<launchpad_core::exec::Stdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>,
    alive: std::sync::atomic::AtomicBool,
    /// Asks the reader to kill the process (it owns the child, so a reaped PID is never signalled).
    kill: tokio::sync::Notify,
}

/// Starts `codex app-server` on demand and routes its messages.
#[derive(Clone)]
pub struct CodexServer {
    env: UserEnv,
    proc: Arc<tokio::sync::Mutex<Option<Arc<Proc>>>>,
    next_id: Arc<AtomicI64>,
    /// Every message reaches every subscriber, in order and without loss: the queues are unbounded, so
    /// a slow consumer (the router waiting on the database) delays messages but never drops a turn end.
    subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<Incoming>>>>,
    /// Set by [`CodexServer::shutdown`]: no new process starts.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl CodexServer {
    pub fn new(env: UserEnv) -> CodexServer {
        CodexServer {
            env,
            proc: Arc::new(tokio::sync::Mutex::new(None)),
            next_id: Arc::new(AtomicI64::new(0)),
            subscribers: Arc::default(),
            closed: Arc::default(),
        }
    }

    /// Ends the running process, if any: the next request starts a new one, which reads its login again.
    pub async fn restart(&self) {
        let old = self.proc.lock().await.take();
        if let Some(p) = old {
            p.alive.store(false, Ordering::Relaxed);
            // The app-server exits at the end of its input.
            let _ = p.stdin.lock().await.shutdown().await;
        }
    }

    /// Ends the process for good and closes every subscription (their receivers then end).
    /// The process exits at the end of its input; it is killed when it has not after 5 seconds.
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.subscribers.lock().unwrap().clear();
        let old = self.proc.lock().await.take();
        if let Some(p) = old {
            p.alive.store(false, Ordering::Relaxed);
            let _ = p.stdin.lock().await.shutdown().await;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                // Harmless when the process already exited.
                p.kill.notify_one();
            });
        }
    }

    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<Incoming> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    fn publish(subscribers: &Mutex<Vec<mpsc::UnboundedSender<Incoming>>>, msg: Incoming) {
        subscribers.lock().unwrap().retain(|tx| tx.send(msg.clone()).is_ok());
    }

    /// Returns the running process, starting it (and sending `initialize`) when needed.
    async fn get(&self) -> Result<Arc<Proc>> {
        let mut guard = self.proc.lock().await;
        anyhow::ensure!(!self.closed.load(Ordering::Relaxed), "codex app-server was shut down");
        if let Some(p) = guard.as_ref()
            && p.alive.load(Ordering::Relaxed)
        {
            return Ok(p.clone());
        }
        // The lock stays held until `initialize` is done, so no request goes out before it.
        let p = self.spawn().await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let init = self
            .send_request(
                &p,
                id,
                "initialize",
                json!({"clientInfo": {"name": "launchpad", "title": "Launchpad", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"experimentalApi": true}}),
            )
            .await
            .context("codex app-server initialize")?;
        tracing::info!(
            "codex app-server started: {}",
            init.get("userAgent")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
        );
        write_line(&p, &json!({"method": "initialized"})).await?;
        *guard = Some(p.clone());
        Ok(p)
    }

    async fn spawn(&self) -> Result<Arc<Proc>> {
        let mut child = launchpad_core::procs::child_command(&self.env, "codex", None)
            .args(["app-server", "-c", "check_for_update_on_startup=false"])
            .stdin(launchpad_core::exec::Io::Piped)
            .stdout(launchpad_core::exec::Io::Piped)
            .stderr(launchpad_core::exec::Io::Piped)
            .spawn()
            .await
            .context("start codex app-server")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let p = Arc::new(Proc {
            stdin: tokio::sync::Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            alive: std::sync::atomic::AtomicBool::new(true),
            kill: tokio::sync::Notify::new(),
        });

        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "codex", "{line}");
            }
        });

        let reader_proc = p.clone();
        let incoming = self.subscribers.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                let line = tokio::select! {
                    line = lines.next_line() => line,
                    _ = reader_proc.kill.notified() => {
                        let _ = child.start_kill();
                        continue;
                    }
                };
                let Ok(Some(line)) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    tracing::warn!("codex app-server: invalid line: {line}");
                    continue;
                };
                let method = msg
                    .get("method")
                    .and_then(|m| m.as_str())
                    .map(str::to_string);
                let id = msg.get("id").cloned();
                match (method, id) {
                    (Some(method), Some(id)) => {
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        Self::publish(&incoming, Incoming::Request { id, method, params });
                    }
                    (Some(method), None) => {
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        Self::publish(&incoming, Incoming::Notification { method, params });
                    }
                    (None, Some(id)) => {
                        let Some(id) = id.as_i64() else { continue };
                        let waiter = reader_proc.pending.lock().unwrap().remove(&id);
                        if let Some(tx) = waiter {
                            let res = match msg.get("error") {
                                Some(err) => Err(RpcError(err.clone()).into()),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = tx.send(res);
                        }
                    }
                    (None, None) => {}
                }
            }
            reader_proc.alive.store(false, Ordering::Relaxed);
            for (_, tx) in reader_proc.pending.lock().unwrap().drain() {
                let _ = tx.send(Err(anyhow!("codex app-server exited")));
            }
            let status = child.wait().await;
            tracing::warn!("codex app-server exited: {status:?}");
            Self::publish(&incoming, Incoming::Exited);
        });
        Ok(p)
    }

    async fn send_request(&self, p: &Proc, id: i64, method: &str, params: Value) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        p.pending.lock().unwrap().insert(id, tx);
        write_line(p, &json!({"method": method, "id": id, "params": params})).await?;
        match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => bail!("codex app-server exited"),
            Err(_) => {
                p.pending.lock().unwrap().remove(&id);
                bail!("codex {method}: timeout")
            }
        }
    }

    /// Sends a request and waits for its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let p = self.get().await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.send_request(&p, id, method, params).await
    }

    /// Answers a server request.
    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        let p = self.get().await?;
        write_line(&p, &json!({"id": id, "result": result})).await
    }
}

async fn write_line(p: &Proc, msg: &Value) -> Result<()> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    let mut stdin = p.stdin.lock().await;
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write to codex app-server")?;
    stdin.flush().await?;
    Ok(())
}

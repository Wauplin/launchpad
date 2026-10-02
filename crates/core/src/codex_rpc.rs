//! Client for `codex app-server`: JSON-RPC over stdio, one JSON object per line, no `"jsonrpc"` field.
//!
//! One process is shared by every Codex session and by the quota monitor. It is started on first use
//! and started again after it exits. See docs/spikes/3-codex-app-server.md.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{broadcast, oneshot};

use crate::shellenv::UserEnv;

/// A message from the app-server that is not a response to one of our requests.
#[derive(Clone, Debug)]
pub enum Incoming {
    Notification { method: String, params: Value },
    /// A request from the server (approvals, user input). Must be answered with [`CodexServer::respond`].
    Request { id: Value, method: String, params: Value },
    /// The process exited. Running turns are lost.
    Exited,
}

struct Proc {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>,
    alive: std::sync::atomic::AtomicBool,
}

/// Starts `codex app-server` on demand and routes its messages.
#[derive(Clone)]
pub struct CodexServer {
    env: UserEnv,
    proc: Arc<tokio::sync::Mutex<Option<Arc<Proc>>>>,
    next_id: Arc<AtomicI64>,
    incoming: broadcast::Sender<Incoming>,
}

impl CodexServer {
    pub fn new(env: UserEnv) -> CodexServer {
        let (incoming, _) = broadcast::channel(8192);
        CodexServer {
            env,
            proc: Arc::new(tokio::sync::Mutex::new(None)),
            next_id: Arc::new(AtomicI64::new(0)),
            incoming,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Incoming> {
        self.incoming.subscribe()
    }

    /// Returns the running process, starting it (and sending `initialize`) when needed.
    async fn get(&self) -> Result<Arc<Proc>> {
        let mut guard = self.proc.lock().await;
        if let Some(p) = guard.as_ref() {
            if p.alive.load(Ordering::Relaxed) {
                return Ok(p.clone());
            }
        }
        // The lock stays held until `initialize` is done, so no request goes out before it.
        let p = self.spawn().await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let init = self
            .send_request(
                &p,
                id,
                "initialize",
                json!({"clientInfo": {"name": "launchpad", "title": "Launchpad", "version": env!("CARGO_PKG_VERSION")}}),
            )
            .await
            .context("codex app-server initialize")?;
        tracing::info!("codex app-server started: {}", init.get("userAgent").and_then(|v| v.as_str()).unwrap_or("?"));
        write_line(&p, &json!({"method": "initialized"})).await?;
        *guard = Some(p.clone());
        Ok(p)
    }

    async fn spawn(&self) -> Result<Arc<Proc>> {
        let mut child = crate::procs::child_command(&self.env, "codex", None)
            .args(["app-server", "-c", "check_for_update_on_startup=false"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("start codex app-server")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let p = Arc::new(Proc {
            stdin: tokio::sync::Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            alive: std::sync::atomic::AtomicBool::new(true),
        });

        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "codex", "{line}");
            }
        });

        let reader_proc = p.clone();
        let incoming = self.incoming.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    tracing::warn!("codex app-server: invalid line: {line}");
                    continue;
                };
                let method = msg.get("method").and_then(|m| m.as_str()).map(str::to_string);
                let id = msg.get("id").cloned();
                match (method, id) {
                    (Some(method), Some(id)) => {
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        let _ = incoming.send(Incoming::Request { id, method, params });
                    }
                    (Some(method), None) => {
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        let _ = incoming.send(Incoming::Notification { method, params });
                    }
                    (None, Some(id)) => {
                        let Some(id) = id.as_i64() else { continue };
                        let waiter = reader_proc.pending.lock().unwrap().remove(&id);
                        if let Some(tx) = waiter {
                            let res = match msg.get("error") {
                                Some(err) => Err(anyhow!("codex: {}", err)),
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
            let _ = incoming.send(Incoming::Exited);
            let status = child.wait().await;
            tracing::warn!("codex app-server exited: {status:?}");
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
    stdin.write_all(line.as_bytes()).await.context("write to codex app-server")?;
    stdin.flush().await?;
    Ok(())
}

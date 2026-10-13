//! Integrated terminals. Output goes over one WebSocket per viewer (`/api/terminals/{id}/ws`), never
//! over the global one: binary frames carry output and keystrokes, text frames carry JSON control messages.
//!
//! Several viewers share one PTY size. `reset` and `size` messages carry the effective rows/columns and
//! `owner` (`self`, `other` or `none`). Every viewer reports the size it would fit (`resize`); only the
//! owner's applies. A viewer becomes the owner when it is the first one, when it sends `claim` (the user
//! typed, pasted or clicked Use this size) or when it is the only one left after the owner disconnected.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use crate::auth::Access;
use futures_util::{SinkExt, StreamExt};
use launchpad_core::terminals::{Output, TerminalInfo};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::error::{ApiError, ApiResult};
use crate::Server;

pub fn router() -> Router<Server> {
    Router::new()
        .route("/api/terminals", get(list).post(open))
        .route("/api/terminals/close-all", post(close_all))
        .route("/api/terminals/{id}", axum::routing::delete(remove))
        .route("/api/terminals/{id}/close", post(close))
        .route("/api/terminals/{id}/ws", get(ws))
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{e:#}"))
}

async fn list(State(s): State<Server>) -> Json<Vec<TerminalInfo>> {
    Json(s.app.terminals.list())
}

#[derive(Deserialize)]
struct Open {
    cwd: String,
    command: Option<String>,
    language: Option<String>,
}

async fn open(State(s): State<Server>, Json(b): Json<Open>) -> ApiResult<TerminalInfo> {
    Ok(Json(s.app.terminals.open_shell(&s.app, &b.cwd, b.command.as_deref(), b.language.as_deref()).await.map_err(bad)?))
}

async fn close(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.terminals.close(&id).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn close_all(State(s): State<Server>) -> ApiResult<Value> {
    s.app.terminals.close_all().await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn remove(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.app.terminals.remove(&s.app, &id).map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

async fn ws(State(s): State<Server>, Path(id): Path<String>, Extension(access): Extension<Access>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    s.app.terminals.attach(&id).map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(ws.on_upgrade(move |socket| serve(s, id, socket, access)))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { rows: u16, cols: u16 },
    Claim,
    Ack { epoch: u64, bytes: usize },
}

// Bounds data queued across the WebSocket and xterm parser, even when socket sends are fast.
// This is per viewer: a hidden/slow window must never stop the shared PTY or another viewer.
const OUTPUT_WINDOW: usize = 256 * 1024;
const OUTPUT_CHUNK: usize = 16 * 1024;

#[derive(Default)]
struct OutputWindow {
    epoch: u64,
    pending: usize,
}

impl OutputWindow {
    fn reset(&mut self) {
        self.epoch += 1;
        self.pending = 0;
    }

    fn available(&self) -> bool {
        self.pending <= OUTPUT_WINDOW - OUTPUT_CHUNK
    }

    fn acknowledge(&mut self, epoch: u64, bytes: usize) {
        // Old callbacks after a resync cannot grant credit for the new snapshot.
        if epoch == self.epoch && bytes <= self.pending {
            self.pending -= bytes;
        }
    }
}

fn owner_label(owner: Option<u64>, me: u64) -> &'static str {
    match owner {
        Some(o) if o == me => "self",
        Some(_) => "other",
        None => "none",
    }
}

/// Releases the viewer's size ownership however the connection ends.
struct Viewer {
    terminals: launchpad_core::terminals::Terminals,
    id: String,
    me: u64,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.terminals.leave(&self.id, self.me);
    }
}

async fn serve(s: Server, id: String, socket: WebSocket, mut access: Access) {
    let (mut tx, mut rx) = socket.split();
    let Ok(me) = s.app.terminals.join(&id) else { return };
    let _viewer = Viewer { terminals: s.app.terminals.clone(), id: id.clone(), me };
    let mut window = OutputWindow::default();
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    // A viewer that falls behind gets a fresh snapshot ("reset") instead of a gap.
    'attach: loop {
        if !access.valid() { return }
        let Ok(mut a) = s.app.terminals.attach(&id) else { return };
        window.reset();
        let reset = json!({"type": "reset", "epoch": window.epoch, "rows": a.rows, "cols": a.cols, "owner": owner_label(a.owner, me)}).to_string();
        if tx.send(Message::Text(reset.into())).await.is_err() {
            return;
        }
        let exit = |code: Option<u32>| Message::Text(json!({"type": "exit", "code": code}).to_string().into());
        let mut snapshot = a.snapshot.chunks(OUTPUT_CHUNK);
        let mut replaying = true;
        loop {
            if !access.valid() { return }
            // Snapshot chunks obey the same credit limit as live output. Keep receiving input,
            // resize and ACK controls while either stream is waiting for renderer capacity.
            if replaying && window.available() {
                if let Some(chunk) = snapshot.next() {
                    window.pending += chunk.len();
                    if tx.send(Message::Binary(chunk.to_vec().into())).await.is_err() {
                        return;
                    }
                    continue;
                }
                replaying = false;
                if tx.send(Message::Text(json!({"type": "ready"}).to_string().into())).await.is_err() {
                    return;
                }
                if let Some(code) = a.exit
                    && tx.send(exit(code)).await.is_err() {
                    return;
                }
            }
            tokio::select! {
                _ = access.changes.changed() => { if !access.valid() { return } }
                _ = expiry.tick() => { if !access.valid() { return } }
                out = a.rx.recv(), if !replaying && window.available() => {
                    let msg = match out {
                        Ok(Output::Data(d)) => {
                            window.pending += d.len();
                            Message::Binary(d.to_vec().into())
                        }
                        Ok(Output::Exit(code)) => exit(code),
                        Ok(Output::Size { rows, cols, owner }) => Message::Text(json!({"type": "size", "rows": rows, "cols": cols, "owner": owner_label(owner, me)}).to_string().into()),
                        Err(RecvError::Lagged(_)) => continue 'attach,
                        // The terminal was removed.
                        Err(RecvError::Closed) => return,
                    };
                    if tx.send(msg).await.is_err() {
                        return;
                    }
                }
                msg = rx.next() => match msg {
                    Some(Ok(Message::Binary(b))) => {
                        if !access.valid() { return }
                        // A process that stops reading its input fills the PTY: block a pool thread, not the runtime.
                        let (terminals, id) = (s.app.terminals.clone(), id.clone());
                        let _ = tokio::task::spawn_blocking(move || terminals.write(&id, &b)).await;
                    }
                    Some(Ok(Message::Text(t))) => {
                        if !access.valid() { return }
                        match serde_json::from_str::<Control>(&t) {
                        Ok(Control::Resize { rows, cols }) => { let _ = s.app.terminals.resize(&id, me, rows, cols); }
                        Ok(Control::Claim) => { let _ = s.app.terminals.claim(&id, me); }
                        Ok(Control::Ack { epoch, bytes }) => window.acknowledge(epoch, bytes),
                        Err(e) => tracing::debug!("terminal control message: {e}"),
                        }
                    },
                    Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_waits_for_parser_credit_and_ignores_stale_or_invalid_acks() {
        let mut window = OutputWindow::default();
        window.reset();
        window.pending = OUTPUT_WINDOW;
        assert!(!window.available());
        window.acknowledge(window.epoch, OUTPUT_CHUNK);
        assert!(window.available());
        window.reset();
        window.pending = OUTPUT_CHUNK;
        window.acknowledge(window.epoch - 1, OUTPUT_CHUNK);
        window.acknowledge(window.epoch, OUTPUT_CHUNK + 1);
        assert_eq!(window.pending, OUTPUT_CHUNK);
        window.acknowledge(window.epoch, OUTPUT_CHUNK);
        assert_eq!(window.pending, 0);
    }
}

//! Integrated terminals. Output goes over one WebSocket per viewer (`/api/terminals/{id}/ws`), never
//! over the global one: binary frames carry output and keystrokes, text frames carry JSON control messages.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
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

async fn ws(State(s): State<Server>, Path(id): Path<String>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    s.app.terminals.attach(&id).map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(ws.on_upgrade(move |socket| serve(s, id, socket)))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { rows: u16, cols: u16 },
}

async fn serve(s: Server, id: String, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    // A viewer that falls behind gets a fresh snapshot ("reset") instead of a gap.
    'attach: loop {
        let Ok(mut a) = s.app.terminals.attach(&id) else { return };
        let reset = json!({"type": "reset"}).to_string();
        if tx.send(Message::Text(reset.into())).await.is_err() || tx.send(Message::Binary(a.snapshot.into())).await.is_err() {
            return;
        }
        let exit = |code: Option<u32>| Message::Text(json!({"type": "exit", "code": code}).to_string().into());
        if let Some(code) = a.exit
            && tx.send(exit(code)).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                out = a.rx.recv() => {
                    let msg = match out {
                        Ok(Output::Data(d)) => Message::Binary(d.to_vec().into()),
                        Ok(Output::Exit(code)) => exit(code),
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
                        // A process that stops reading its input fills the PTY: block a pool thread, not the runtime.
                        let (terminals, id) = (s.app.terminals.clone(), id.clone());
                        let _ = tokio::task::spawn_blocking(move || terminals.write(&id, &b)).await;
                    }
                    Some(Ok(Message::Text(t))) => match serde_json::from_str::<Control>(&t) {
                        Ok(Control::Resize { rows, cols }) => { let _ = s.app.terminals.resize(&id, rows, cols); }
                        Err(e) => tracing::debug!("terminal control message: {e}"),
                    },
                    Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return,
                },
            }
        }
    }
}

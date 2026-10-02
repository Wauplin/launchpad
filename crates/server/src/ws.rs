//! One WebSocket per window. The daemon pushes every bus event as `{seq, topic, type, payload}`.
//! If the client falls behind, it gets `{"type": "resync"}` and reloads what it shows.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::Server;

pub async fn handler(State(s): State<Server>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| run(s, socket))
}

async fn run(s: Server, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut events = s.app.bus.subscribe();
    let hello = serde_json::json!({"type": "hello", "profile": s.app.paths.profile.name(), "build": crate::ui::build_id()});
    if tx.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ev = events.recv() => {
                let text = match ev {
                    Ok(ev) => serde_json::to_string(&ev).unwrap_or_default(),
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!("websocket client lagged by {n} events");
                        serde_json::json!({"type": "resync"}).to_string()
                    }
                    Err(RecvError::Closed) => break,
                };
                if tx.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

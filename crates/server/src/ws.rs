//! One WebSocket per window. The daemon pushes every bus event as `{seq, topic, type, payload}`.
//! If the client falls behind, it gets `{"type": "resync"}` and reloads what it shows.

use axum::extract::State;
use axum::Extension;
use crate::auth::Access;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::Server;

pub async fn handler(State(s): State<Server>, Extension(access): Extension<Access>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| run(s, socket, access))
}

static NEXT_WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn run(s: Server, socket: WebSocket, access: Access) {
    let window = NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    serve_socket(&s, socket, window, access).await;
    // The window is gone: it watches nothing any more.
    s.app.sessions.set_watched(window, None);
}

async fn serve_socket(s: &Server, socket: WebSocket, window: u64, mut access: Access) {
    let (mut tx, mut rx) = socket.split();
    let mut events = s.app.bus.subscribe();
    let hello = serde_json::json!({"type": "hello", "profile": s.app.paths.profile.name(), "build": crate::ui::build_id()});
    if tx.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        if !access.valid() { break }
        tokio::select! {
            _ = access.changes.changed() => { if !access.valid() { break } }
            _ = expiry.tick() => { if !access.valid() { break } }
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
                    Some(Ok(Message::Text(t))) => { if !access.valid() { break } client_message(s, window, &t); }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

/// Messages from the UI. `presence`: whether the window has the focus, which sessions it shows and
/// how long ago it last received user input (`idle_ms`, null before any input), so the daemon does
/// not notify about a session you are looking at. Windows re-send it on input, throttled.
fn client_message(s: &Server, window: u64, text: &str) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else { return };
    if msg["type"] == "presence" {
        let sessions = msg["sessions"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        s.app.sessions.set_watched(window, Some((msg["focused"].as_bool().unwrap_or(false), sessions, msg["idle_ms"].as_i64())));
    }
}

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
    serve_socket(&s, socket).await;
    // The window is gone: nothing is watched any more.
    presence(&s.app, false, vec![]);
}

async fn serve_socket(s: &Server, socket: WebSocket) {
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
                    Some(Ok(Message::Text(t))) => client_message(s, &t),
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

/// Messages from the UI. `presence`: whether the window has the focus and which sessions it shows,
/// so the daemon does not notify about a session you are looking at.
fn client_message(s: &Server, text: &str) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else { return };
    if msg["type"] == "presence" {
        let sessions = msg["sessions"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        presence(&s.app, msg["focused"].as_bool().unwrap_or(false), sessions);
    }
}

fn presence(app: &launchpad_core::App, focused: bool, sessions: Vec<String>) {
    app.sessions.set_watched(focused, sessions.clone());
    for s in app.sessions.list().into_iter().filter(|s| s.live) {
        let (paths, sessions) = (app.paths.clone(), sessions.clone());
        tokio::spawn(async move {
            let _ = launchpad_core::worker::call(&paths, &s.id, "presence", serde_json::json!({"focused":focused,"sessions":sessions})).await;
        });
    }
}

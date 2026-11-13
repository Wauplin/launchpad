//! One WebSocket per window. The daemon pushes every bus event as `{bus, seq, topic, type, payload}`.
//!
//! The socket opens with `hello` (`env`, `build`, `boot`, `bus`: the last delivery number). A window
//! that reconnects passes `?boot=<id>&since=<last delivery number it received>`: when the bus still holds
//! every event after it, the daemon sends them, then `{"type": "replayed"}`. Otherwise (first connection,
//! another daemon run, events dropped from the ring) it sends `{"type": "resync"}` and the window reloads
//! what it shows; so does a client that falls behind the live stream. `{"type": "ping"}` arrives every 25 s
//! so proxies keep the idle socket open and the window can detect a dead connection. A window may send
//! `{"type": "ping"}` too; the daemon answers `{"type": "pong"}`.
//!
//! `GET /api/events` is the same stream as server-sent events, for clients that cannot open
//! a WebSocket with their headers (the Chrome extension through Home's Space). Each message is `data: <the same
//! JSON text>`; bus events also carry `id: <boot>:<bus>`, so `Last-Event-ID` resumes like `?boot=&since=`. A
//! `: keepalive` comment comes every 15 s. It is one-way: no presence.

use axum::extract::{Query, State};
use axum::Extension;
use crate::auth::Access;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::Server;

#[derive(serde::Deserialize, Default, Clone)]
pub struct Resume {
    boot: Option<String>,
    since: Option<u64>,
}

pub async fn handler(State(s): State<Server>, Extension(access): Extension<Access>, Query(resume): Query<Resume>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| run(s, socket, access, resume))
}

const PING: std::time::Duration = std::time::Duration::from_secs(25);

static NEXT_WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn run(s: Server, socket: WebSocket, access: Access, resume: Resume) {
    let window = NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    serve_socket(&s, socket, window, access, resume).await;
    // The window is gone: it watches nothing any more.
    s.app.sessions.set_watched(window, None);
}

async fn serve_socket(s: &Server, socket: WebSocket, window: u64, mut access: Access, resume: Resume) {
    let (mut tx, mut rx) = socket.split();
    let bus = &s.app.bus;
    let since = resume.since.filter(|_| resume.boot.as_deref() == Some(bus.boot()));
    let replay = bus.subscribe(since);
    let mut events = replay.rx;
    let hello = serde_json::json!({"type": "hello", "env": s.app.paths.app_env.name(), "build": crate::ui::build_id(), "boot": bus.boot(), "bus": replay.last});
    if tx.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    let mut first = Vec::new();
    match replay.missed {
        Some(missed) => {
            if !missed.is_empty() {
                tracing::debug!("websocket replays {} events", missed.len());
            }
            first.extend(missed.into_iter().map(|e| e.text.to_string()));
            first.push(r#"{"type":"replayed"}"#.to_string());
        }
        None => first.push(r#"{"type":"resync"}"#.to_string()),
    }
    for text in first {
        if !access.valid() || tx.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    loop {
        if !access.valid() { break }
        tokio::select! {
            _ = access.changes.changed() => { if !access.valid() { break } }
            _ = expiry.tick() => { if !access.valid() { break } }
            _ = ping.tick() => {
                if tx.send(Message::Text(r#"{"type":"ping"}"#.into())).await.is_err() {
                    break;
                }
            }
            ev = events.recv() => {
                let text = match ev {
                    Ok(ev) => ev.text.to_string(),
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
                    Some(Ok(Message::Text(t))) => {
                        if !access.valid() { break }
                        if t.as_str() == r#"{"type":"ping"}"# {
                            if tx.send(Message::Text(r#"{"type":"pong"}"#.into())).await.is_err() { break }
                        } else {
                            client_message(s, window, &t);
                        }
                    }
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

const KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(15);

/// `GET /api/events`: the bus as server-sent events (see the module comment).
pub async fn events(State(s): State<Server>, Extension(access): Extension<Access>, Query(mut resume): Query<Resume>, headers: axum::http::HeaderMap) -> Response {
    use axum::http::header;
    if resume.since.is_none()
        && let Some((boot, since)) = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.split_once(':'))
    {
        resume = Resume { boot: Some(boot.to_string()), since: since.parse().ok() };
    }
    let bus = &s.app.bus;
    let boot = bus.boot().to_string();
    let since = resume.since.filter(|_| resume.boot.as_deref() == Some(bus.boot()));
    let replay = bus.subscribe(since);
    let hello = serde_json::json!({"type": "hello", "env": s.app.paths.app_env.name(), "build": crate::ui::build_id(), "boot": boot, "bus": replay.last});
    let mut first = std::collections::VecDeque::from([format!("retry: 3000\ndata: {hello}\n\n")]);
    match replay.missed {
        Some(missed) => {
            first.extend(missed.into_iter().map(|e| format!("id: {boot}:{}\ndata: {}\n\n", e.bus, e.text)));
            first.push_back("data: {\"type\":\"replayed\"}\n\n".into());
        }
        None => first.push_back("data: {\"type\":\"resync\"}\n\n".into()),
    }
    struct Stream {
        first: std::collections::VecDeque<String>,
        events: tokio::sync::broadcast::Receiver<launchpad_core::events::Delivered>,
        access: Access,
        boot: String,
        keepalive: tokio::time::Interval,
        expiry: tokio::time::Interval,
    }
    let state = Stream {
        first,
        events: replay.rx,
        access,
        boot,
        keepalive: tokio::time::interval_at(tokio::time::Instant::now() + KEEPALIVE, KEEPALIVE),
        expiry: tokio::time::interval(std::time::Duration::from_secs(30)),
    };
    let stream = futures_util::stream::unfold(state, |mut st| async move {
        if !st.access.valid() {
            return None;
        }
        if let Some(text) = st.first.pop_front() {
            return Some((Ok::<_, std::convert::Infallible>(bytes::Bytes::from(text)), st));
        }
        loop {
            let text = tokio::select! {
                _ = st.access.changes.changed() => { if !st.access.valid() { return None } continue }
                _ = st.expiry.tick() => { if !st.access.valid() { return None } continue }
                _ = st.keepalive.tick() => ": keepalive\n\n".to_string(),
                ev = st.events.recv() => match ev {
                    Ok(ev) => format!("id: {}:{}\ndata: {}\n\n", st.boot, ev.bus, ev.text),
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!("event stream client lagged by {n} events");
                        "data: {\"type\":\"resync\"}\n\n".to_string()
                    }
                    Err(RecvError::Closed) => return None,
                },
            };
            return Some((Ok(bytes::Bytes::from(text)), st));
        }
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}

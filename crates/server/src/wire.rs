//! The tunnel's transport: text frames over a WebSocket, whichever side dialed.
//!
//! A runner normally dials Home (`/api/runners/tunnel`). A Burst runner cannot reach a private Home, so Home
//! dials it instead (`/api/runner/reverse`, through the HF Jobs proxy). Both sides then speak the same protocol
//! over a `Wire`: liveness uses the runner's `Ping` messages, not WebSocket pings.

use std::pin::Pin;

use futures_util::{Sink, SinkExt, Stream, StreamExt};

pub type WireTx = Pin<Box<dyn Sink<String, Error = anyhow::Error> + Send>>;
pub type WireRx = Pin<Box<dyn Stream<Item = anyhow::Result<String>> + Send>>;

/// A WebSocket accepted by this daemon's router.
pub fn axum(socket: axum::extract::ws::WebSocket) -> (WireTx, WireRx) {
    use axum::extract::ws::Message;
    let (tx, rx) = socket.split();
    let tx = tx.sink_map_err(anyhow::Error::from).with(|t: String| async move { Ok::<_, anyhow::Error>(Message::Text(t.into())) });
    let rx = rx.filter_map(|m| async move {
        match m {
            Ok(Message::Text(t)) => Some(Ok(t.to_string())),
            Ok(Message::Close(_)) => None,
            Ok(_) => None,
            Err(e) => Some(Err(anyhow::Error::from(e))),
        }
    });
    (Box::pin(tx), Box::pin(rx))
}

/// A WebSocket this daemon dialed.
pub fn tungstenite<S>(socket: tokio_tungstenite::WebSocketStream<S>) -> (WireTx, WireRx)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use tokio_tungstenite::tungstenite::Message;
    let (tx, rx) = socket.split();
    let tx = tx.sink_map_err(anyhow::Error::from).with(|t: String| async move { Ok::<_, anyhow::Error>(Message::Text(t.into())) });
    let rx = rx.filter_map(|m| async move {
        match m {
            Ok(Message::Text(t)) => Some(Ok(t.to_string())),
            Ok(_) => None,
            Err(e) => Some(Err(anyhow::Error::from(e))),
        }
    });
    (Box::pin(tx), Box::pin(rx))
}

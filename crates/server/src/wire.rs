//! The tunnel's transport: text frames over a WebSocket, whichever side dialed.
//!
//! A runner normally dials Home (`/api/runners/tunnel`). A Burst runner cannot reach a private Home, so Home
//! dials it instead (`/api/runner/reverse`, through the HF Jobs proxy). Both sides then speak the same protocol
//! over a `Wire`: liveness uses the runner's `Ping` messages, not WebSocket pings.

use std::pin::Pin;

use futures_util::{Sink, SinkExt, Stream, StreamExt};

pub type WireTx = Pin<Box<dyn Sink<String, Error = anyhow::Error> + Send>>;
pub type WireRx = Pin<Box<dyn Stream<Item = anyhow::Result<String>> + Send>>;

/// The largest tunnel message: a file download in an envelope, or a replica item (`replica::MAX_ITEM_BYTES`;
/// attachments travel as references). Each message is one frame, so frames get the same limit (the default is
/// 16 MiB). The Space gateway uses the same limit (`WS_MAX_MESSAGE` in deploy/hf-home/gateway/proxy.py).
const MAX_MESSAGE: usize = 64 << 20;

/// Accepts a tunnel WebSocket with the tunnel's message limit.
pub fn accept(ws: axum::extract::WebSocketUpgrade) -> axum::extract::WebSocketUpgrade {
    ws.max_message_size(MAX_MESSAGE).max_frame_size(MAX_MESSAGE)
}

/// Dials a tunnel WebSocket with the tunnel's message limit.
pub async fn dial<R: tokio_tungstenite::tungstenite::client::IntoClientRequest + Unpin>(
    request: R,
) -> Result<
    (tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, tokio_tungstenite::tungstenite::handshake::client::Response),
    tokio_tungstenite::tungstenite::Error,
> {
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    tokio_tungstenite::connect_async_with_config(request, Some(config), false).await
}

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

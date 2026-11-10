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

// ---------------------------------------------------------------- Lanes and flow control

/// Parts of exchange bodies queued for the socket at most, per connection (`lanes`).
pub const BULK_FRAMES: usize = 16;

/// Bytes of one exchange body in flight at most: the sender waits for the receiver's `HttpAck` before sending
/// more, so a slow reader (a Burst agent's download, a slow Git client) holds at most this much on either side.
pub const WINDOW: usize = 4 << 20;

/// Largest body part per frame, before base64.
pub const CHUNK: usize = 256 * 1024;

/// One writer per connection, with two lanes: the control lane (unbounded: pings, requests and answers,
/// acknowledgements, replica batches, snapshots, terminal frames, which their own acks bound) always goes
/// first; the bulk lane (bounded: exchange body parts) goes when the control lane is empty. The writer ends when
/// the socket fails or every sender is gone; the caller watches its handle.
pub fn lanes<T: serde::Serialize + Send + 'static>(mut tx: WireTx) -> (tokio::sync::mpsc::UnboundedSender<T>, tokio::sync::mpsc::Sender<T>, tokio::task::JoinHandle<()>) {
    let (control, mut control_rx) = tokio::sync::mpsc::unbounded_channel::<T>();
    let (bulk, mut bulk_rx) = tokio::sync::mpsc::channel::<T>(BULK_FRAMES);
    let writer = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                biased;
                m = control_rx.recv() => m,
                Some(m) = bulk_rx.recv() => Some(m),
            };
            let Some(msg) = msg else { break };
            let Ok(text) = serde_json::to_string(&msg) else { continue };
            if tx.send(text).await.is_err() {
                break;
            }
        }
    });
    (control, bulk, writer)
}

/// The receiving end of an exchange body: parts arrive from the tunnel (at most `WINDOW` bytes ahead), parts
/// read are acknowledged (`ack`, in batches, and always once nothing is left to read, so the sender never waits
/// on bytes already consumed), and dropping it before the end cancels the exchange (`cancel`).
pub struct Inbound {
    rx: tokio::sync::mpsc::UnboundedReceiver<Result<bytes::Bytes, std::io::Error>>,
    queued: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    unacked: usize,
    ack: Box<dyn Fn(u32) + Send + Sync>,
    cancel: Option<Box<dyn FnOnce() + Send + Sync>>,
}

/// Where the tunnel loop puts the parts of an `Inbound` body. It never waits: the sender keeps to the window,
/// and a sender that does not is refused (`push` returns false, and the exchange ends).
#[derive(Clone)]
pub struct InboundTx {
    tx: tokio::sync::mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>,
    queued: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl InboundTx {
    /// False: the reader is gone, or the part would pass the window.
    pub fn push(&self, part: Result<bytes::Bytes, std::io::Error>) -> bool {
        use std::sync::atomic::Ordering;
        let len = part.as_ref().map(|b| b.len()).unwrap_or(0);
        if self.queued.fetch_add(len, Ordering::AcqRel) + len > WINDOW {
            return false;
        }
        self.tx.send(part).is_ok()
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

impl Inbound {
    pub fn new(ack: impl Fn(u32) + Send + Sync + 'static, cancel: impl FnOnce() + Send + Sync + 'static) -> (InboundTx, Self) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let queued = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (InboundTx { tx, queued: queued.clone() }, Inbound { rx, queued, unacked: 0, ack: Box::new(ack), cancel: Some(Box::new(cancel)) })
    }
}

impl Stream for Inbound {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        use std::sync::atomic::Ordering;
        let next = self.rx.poll_recv(cx);
        match &next {
            std::task::Poll::Ready(Some(Ok(b))) => {
                let len = b.len();
                let left = self.queued.fetch_sub(len, Ordering::AcqRel) - len;
                self.unacked += len;
                if self.unacked >= WINDOW / 8 || left == 0 {
                    let n = std::mem::take(&mut self.unacked);
                    (self.ack)(n as u32);
                }
            }
            // Ended (or failed) on the other side: nothing to cancel.
            std::task::Poll::Ready(_) => self.cancel = None,
            std::task::Poll::Pending => {}
        }
        next
    }
}

impl Drop for Inbound {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

/// The sending end's window of an exchange body: one permit per byte in flight.
pub fn window() -> std::sync::Arc<tokio::sync::Semaphore> {
    std::sync::Arc::new(tokio::sync::Semaphore::new(WINDOW))
}

/// Waits for room to send a part of `len` bytes (at most `CHUNK`). False: the exchange was cancelled (the window
/// closed).
pub async fn take_part(window: &tokio::sync::Semaphore, len: usize) -> bool {
    match window.acquire_many(len.min(WINDOW) as u32).await {
        Ok(p) => {
            p.forget();
            true
        }
        Err(_) => false,
    }
}

/// The receiver acknowledged `n` bytes (never more than the window, whatever it says).
pub fn acked(window: &tokio::sync::Semaphore, n: u32) {
    let room = WINDOW.saturating_sub(window.available_permits());
    window.add_permits((n as usize).min(room));
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    #[tokio::test]
    async fn control_messages_pass_ahead_of_queued_bulk_parts() {
        let (sink_tx, mut sink_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let wire: super::WireTx = Box::pin(futures_util::sink::unfold(sink_tx, |tx, text: String| async move {
            tx.send(text).map_err(|_| anyhow::anyhow!("closed"))?;
            Ok::<_, anyhow::Error>(tx)
        }));
        let (control, bulk, writer) = super::lanes::<String>(wire);
        // Fill the bulk lane before the writer runs, then add one control message.
        for i in 0..super::BULK_FRAMES {
            bulk.try_send(format!("bulk {i}")).unwrap();
        }
        assert!(bulk.try_send("one too many".into()).is_err());
        control.send("ping".into()).unwrap();
        let first: String = serde_json::from_str(&sink_rx.recv().await.unwrap()).unwrap();
        assert_eq!(first, "ping");
        let second: String = serde_json::from_str(&sink_rx.recv().await.unwrap()).unwrap();
        assert_eq!(second, "bulk 0");
        drop((control, bulk));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn an_inbound_body_acknowledges_each_part_and_cancels_when_dropped_early() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (acks, cancels) = (std::sync::Arc::new(AtomicUsize::new(0)), std::sync::Arc::new(AtomicUsize::new(0)));
        let (a, c) = (acks.clone(), cancels.clone());
        let (tx, mut body) = super::Inbound::new(move |n| { a.fetch_add(n as usize, Ordering::Relaxed); }, move || { c.fetch_add(1, Ordering::Relaxed); });
        assert!(tx.push(Ok(bytes::Bytes::from_static(b"ab"))));
        assert!(tx.push(Ok(bytes::Bytes::from_static(b"cde"))));
        // A small part read while more waits is acknowledged later, in a batch; the last one read flushes.
        body.next().await.unwrap().unwrap();
        assert_eq!(acks.load(Ordering::Relaxed), 0);
        body.next().await.unwrap().unwrap();
        assert_eq!(acks.load(Ordering::Relaxed), 5);
        // A sender past the window is refused.
        assert!(!tx.push(Ok(bytes::Bytes::from(vec![0u8; super::WINDOW + 1]))));
        drop(body);
        assert_eq!(cancels.load(Ordering::Relaxed), 1);
        // A body read to its end cancels nothing.
        let c2 = std::sync::Arc::new(AtomicUsize::new(0));
        let c = c2.clone();
        let (tx, mut body) = super::Inbound::new(|_| {}, move || { c.fetch_add(1, Ordering::Relaxed); });
        drop(tx);
        assert!(body.next().await.is_none());
        drop(body);
        assert_eq!(c2.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn acknowledgements_never_open_the_window_wider() {
        let w = super::window();
        for _ in 0..3 {
            assert!(super::take_part(&w, 1000).await);
        }
        assert_eq!(w.available_permits(), super::WINDOW - 3000);
        super::acked(&w, 1_000_000);
        assert_eq!(w.available_permits(), super::WINDOW);
        w.close();
        assert!(!super::take_part(&w, 1).await);
    }
}

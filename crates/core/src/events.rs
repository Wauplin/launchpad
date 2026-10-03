//! In-process event bus. Every state change is published here and forwarded to WebSocket clients.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tokio::sync::broadcast;

/// One message on the bus, sent as is over the WebSocket.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    /// Per-session sequence number for `session/{id}` topics (replayable from `state.db`),
    /// a global counter otherwise.
    pub seq: u64,
    pub topic: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Event>,
    seq: Arc<AtomicU64>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Bus {
        let (tx, _) = broadcast::channel(4096);
        Bus { tx, seq: Arc::new(AtomicU64::new(0)) }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Publishes an event with a global sequence number.
    pub fn publish(&self, topic: &str, kind: &str, payload: serde_json::Value) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Event { seq, topic: topic.to_string(), kind: kind.to_string(), payload });
    }

    /// Publishes an event that already has its own sequence number (session events).
    pub fn send(&self, event: Event) {
        // No receiver is fine: nobody is connected.
        let _ = self.tx.send(event);
    }
}

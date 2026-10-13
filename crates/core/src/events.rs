//! In-process event bus. Every state change is published here and forwarded to WebSocket clients.
//!
//! Each delivered event gets a delivery number (`bus`), increasing by one per event for the whole
//! daemon, separate from `Event.seq`. The bus keeps the most recent deliveries in a bounded ring, so
//! a window that reconnects after a short network loss receives what it missed instead of reloading
//! everything. Delivery numbers restart with the daemon; `boot` tells windows which daemon run they
//! belong to.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// An event as delivered to windows: its delivery number and its JSON text
/// (`{"bus": n, "seq": …, "topic": …, "type": …, "payload": …}`), serialized once.
#[derive(Clone, Debug)]
pub struct Delivered {
    pub bus: u64,
    pub text: Arc<str>,
}

/// Recent deliveries kept for replay: at most this many events and this many bytes of JSON.
const RING_EVENTS: usize = 4096;
const RING_BYTES: usize = 8 << 20;

struct Ring {
    /// The delivery number of the last event (0 before the first one).
    last: u64,
    events: VecDeque<Delivered>,
    bytes: usize,
}

/// A subscription that resumes after delivery number `since`.
pub struct Replay {
    /// The missed events in order, or None when they are not all in the ring any more.
    pub missed: Option<Vec<Delivered>>,
    /// The delivery number of the last event before the live stream starts.
    pub last: u64,
    pub rx: broadcast::Receiver<Delivered>,
}

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Delivered>,
    seq: Arc<AtomicU64>,
    ring: Arc<Mutex<Ring>>,
    boot: Arc<str>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Bus {
        let (tx, _) = broadcast::channel(4096);
        let ring = Ring { last: 0, events: VecDeque::new(), bytes: 0 };
        Bus { tx, seq: Arc::new(AtomicU64::new(0)), ring: Arc::new(Mutex::new(ring)), boot: uuid::Uuid::new_v4().simple().to_string().into() }
    }

    /// Random per daemon start: delivery numbers of another boot mean nothing.
    pub fn boot(&self) -> &str {
        &self.boot
    }

    /// Subscribes to the live stream. With `since`, also returns the events delivered after it when
    /// the ring still has all of them. Runs under the ring lock, so there is neither a gap nor a
    /// duplicate between the missed events and the live stream.
    pub fn subscribe(&self, since: Option<u64>) -> Replay {
        let ring = self.ring.lock().unwrap();
        let rx = self.tx.subscribe();
        let missed = since.filter(|&n| n <= ring.last).and_then(|n| {
            if n == ring.last {
                return Some(Vec::new());
            }
            let first = ring.events.front()?.bus;
            (first <= n + 1).then(|| ring.events.iter().filter(|e| e.bus > n).cloned().collect())
        });
        Replay { missed, last: ring.last, rx }
    }

    /// Publishes an event with a global sequence number.
    pub fn publish(&self, topic: &str, kind: &str, payload: serde_json::Value) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Event { seq, topic: topic.to_string(), kind: kind.to_string(), payload });
    }

    /// Publishes an event that already has its own sequence number (session events).
    pub fn send(&self, event: Event) {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
        let mut ring = self.ring.lock().unwrap();
        ring.last += 1;
        let bus = ring.last;
        // `json` is an object: insert the delivery number as its first field.
        let rest = json.strip_prefix('{').unwrap_or("}");
        let sep = if rest.starts_with('}') { "" } else { "," };
        let delivered = Delivered { bus, text: format!("{{\"bus\":{bus}{sep}{rest}").into() };
        ring.bytes += delivered.text.len();
        ring.events.push_back(delivered.clone());
        while ring.events.len() > RING_EVENTS || (ring.bytes > RING_BYTES && ring.events.len() > 1) {
            let old = ring.events.pop_front().expect("not empty");
            ring.bytes -= old.text.len();
        }
        // No receiver is fine: nobody is connected.
        let _ = self.tx.send(delivered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_missed_events_without_gap_or_duplicate() {
        let bus = Bus::new();
        for i in 0..3 {
            bus.publish("t", "k", serde_json::json!(i));
        }
        let mut r = bus.subscribe(Some(1));
        let missed: Vec<u64> = r.missed.unwrap().iter().map(|e| e.bus).collect();
        assert_eq!(missed, [2, 3]);
        assert_eq!(r.last, 3);
        bus.publish("t", "k", serde_json::json!(3));
        let live = r.rx.try_recv().unwrap();
        assert_eq!(live.bus, 4);
        assert!(live.text.starts_with("{\"bus\":4,\"seq\":4,"), "{}", live.text);
        assert_eq!(bus.subscribe(Some(4)).missed.unwrap().len(), 0);
        // Unknown future numbers (another boot) and nothing to resume from.
        assert!(bus.subscribe(Some(9)).missed.is_none());
        assert!(bus.subscribe(None).missed.is_none());
        for i in 0..RING_EVENTS {
            bus.publish("t", "k", serde_json::json!(i));
        }
        assert!(bus.subscribe(Some(1)).missed.is_none());
        assert_eq!(bus.subscribe(Some(5)).missed.unwrap().len(), RING_EVENTS - 1);
    }
}

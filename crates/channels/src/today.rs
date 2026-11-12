//! This crate's part of Today (`launchpad_core::today`): Outbox drafts that wait for Send, uncertain sends to
//! check, and messages sent or discarded (Done).
use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line};

use crate::connections::{self, OutboxStatus};

pub async fn items(app: &App) -> Vec<Item> {
    let list = match connections::outbox(app).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!("Today: Outbox: {e:#}");
            return vec![];
        }
    };
    list.into_iter().map(|m| {
        // Connections are shared: a message without a session shows in every workspace.
        let place = m.session_id.clone().map(Place::Session).unwrap_or_default();
        let title = m.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| line(&m.body));
        let item = |at: i64| Item::new(Kind::Outbox, Filter::Send, &m.id, title.clone(), place.clone(), at).session(m.session_id.clone()).detail(format!("{} · drafted by {}", m.connection, m.author));
        match m.status {
            OutboxStatus::Draft => item(m.created_at).action("outbox.send"),
            // Never sent again: the owner checks the destination, then marks it sent or discards it.
            OutboxStatus::Uncertain | OutboxStatus::Sending => item(m.created_at).detail(format!("Uncertain: check {} before anything else", m.connection)).action("outbox.discard"),
            OutboxStatus::Sent => item(m.sent_at.unwrap_or(m.created_at)).handled(format!("Sent to {}", m.connection)),
            OutboxStatus::Discarded => item(m.created_at).handled("Discarded"),
        }
    }).collect()
}

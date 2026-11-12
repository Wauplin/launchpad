//! This crate's part of Today (`launchpad_core::today`): Outbox drafts that wait for Send, uncertain sends to
//! check, and messages sent or discarded (Done); and bot channels where a bot asks the owner (CHN-07).
use std::collections::HashMap;

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line};

use crate::channel::{self, Kind as ChannelKind};
use crate::connections::{self, OutboxStatus};

pub async fn items(app: &App) -> Vec<Item> {
    let mut out = outbox(app).await;
    match asks(app).await {
        Ok(list) => out.extend(list),
        Err(e) => tracing::warn!("Today: channel asks: {e:#}"),
    }
    out
}

/// An unanswered message of a bot to the owner: newer than the owner's read mark, by a bot, with `--ask-owner` or
/// a mention of the owner (the channel list's amber `attention` count, the same rule).
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub channel: String,
    pub id: i64,
    pub author: String,
    pub text: String,
    pub at: i64,
    pub ask_owner: bool,
}

/// Pure: one item per channel, from its asks (oldest first): the newest ask, and how many wait.
pub fn per_channel(asks: Vec<Ask>) -> Vec<(Ask, u32)> {
    let mut by: Vec<(Ask, u32)> = vec![];
    for a in asks {
        match by.iter_mut().find(|(b, _)| b.channel == a.channel) {
            Some(slot) => *slot = (a, slot.1 + 1),
            None => by.push((a, 1)),
        }
    }
    by
}

/// The row's title: "Release asks you in #release-train", "Release mentions you in the DM with Docs".
pub fn ask_title(author: &str, ask_owner: bool, place: &str) -> String {
    format!("{author} {} you in {place}", if ask_owner { "asks" } else { "mentions" })
}

/// Channels (not archived) where a bot waits for the owner: one item each, with Reply (`channel.post`, answering the
/// newest ask) as its main action. It leaves when the owner posts there or reads past it (Mark read).
async fn asks(app: &App) -> Result<Vec<Item>> {
    if !launchpad_core::vault::here(app) {
        return Ok(vec![]);
    }
    let all: Vec<Ask> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare(
                "SELECT m.channel_id, m.id, m.author_name, m.text, m.created_at, m.ask_owner FROM channel_message m JOIN channel c ON c.id = m.channel_id
                 WHERE c.archived = 0 AND m.id > c.owner_read AND m.author_kind != 'owner' AND (m.ask_owner = 1 OR EXISTS (SELECT 1 FROM json_each(m.mentions) WHERE value = 'owner'))
                 ORDER BY m.id",
            )?;
            let rows = st
                .query_map([], |r| Ok(Ask { channel: r.get(0)?, id: r.get(1)?, author: r.get(2)?, text: r.get(3)?, at: r.get(4)?, ask_owner: r.get(5)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    if all.is_empty() {
        return Ok(vec![]);
    }
    let bots = channel::bots(app).await?;
    let mut out = vec![];
    for (a, n) in per_channel(all) {
        let Ok(ch) = channel::load(app, &a.channel).await else { continue };
        let place = match ch.kind {
            ChannelKind::Channel => format!("#{}", ch.name),
            ChannelKind::Dm => {
                let members = channel::members_of(app, &ch.id).await.unwrap_or_default();
                let names: HashMap<&str, &str> = members.iter().filter_map(|m| bots.get(&m.bot_id).map(|b| (m.bot_id.as_str(), b.name.as_str()))).collect();
                let other = names.iter().find(|(_, name)| **name != a.author).map_or("another bot", |(_, name)| name);
                format!("the DM with {other}")
            }
        };
        let detail = if n > 1 { format!("{n} messages for you · {}", line(&a.text)) } else { line(&a.text) };
        let mut item = Item::new(Kind::ChannelAsk, Filter::Answer, &ch.id, ask_title(&a.author, a.ask_owner, &place), Place::Workspace(Some(ch.workspace_id.clone())), a.at).detail(detail).action("channel.post");
        item.message = Some(a.id);
        item.count = Some(n);
        out.push(item);
    }
    Ok(out)
}

async fn outbox(app: &App) -> Vec<Item> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(channel: &str, id: i64, ask_owner: bool) -> Ask {
        Ask { channel: channel.into(), id, author: "Release".into(), text: format!("t{id}"), at: id, ask_owner }
    }

    #[test]
    fn one_row_per_channel_with_the_newest_ask_and_the_count() {
        let got = per_channel(vec![ask("a", 1, true), ask("b", 2, false), ask("a", 3, false)]);
        assert_eq!(got, [(ask("a", 3, false), 2), (ask("b", 2, false), 1)]);
        assert_eq!(ask_title("Release", true, "#release-train"), "Release asks you in #release-train");
        assert_eq!(ask_title("Release", false, "the DM with Docs"), "Release mentions you in the DM with Docs");
    }
}

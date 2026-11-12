//! Channel cards (`launchpad_automation::proposals`: `ProposalBody::Channel` from `launchpad propose channel`, and
//! `ProposalBody::ChannelJoin` from `launchpad channel join <name>`), checked and applied here through
//! `ext::Extensions::channel_card` (the automation crate does not depend on this one). A card names only records of
//! its workspace; a join card always names its author's bot. Accepting a channel card runs what `channel.create`
//! does, a join card what `channel.add-member` does.

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use launchpad_core::App;

use crate::channel::{self, Kind, Mode, NewChannel, NewMember};

fn mode_of(v: &Value) -> Result<Mode> {
    match v {
        Value::Null => Ok(Mode::Mentions),
        v => serde_json::from_value(v.clone()).map_err(|_| anyhow::anyhow!("mode is all, mentions, digest or muted")),
    }
}

/// A named channel of the workspace, by name (`release-train`, `#release-train`) or ID.
async fn named(app: &App, workspace: &str, key: &str) -> Result<channel::Row> {
    let key = key.trim().trim_start_matches('#');
    ensure!(!key.is_empty(), "name the channel to join (release-train)");
    let (ws, k) = (workspace.to_string(), key.to_string());
    let id: Option<String> = app
        .state_db
        .call(move |c| Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT id FROM channel WHERE workspace_id = ?1 AND (name = ?2 OR id = ?2) AND kind = 'channel'", rusqlite::params![ws, k], |r| r.get(0)))?))
        .await?;
    channel::load(app, &id.with_context(|| format!("no channel #{key} in this workspace (`launchpad channel list` shows them)"))?).await
}

/// Checks a channel card of `workspace` (`apply` false: returns the card with names and IDs filled in) or applies it
/// (returns `{"id": <channel ID>}`). `bot`: the author's bot, which a join card always names.
pub async fn card(app: &App, workspace: &str, card: Value, bot: Option<&str>, apply: bool, by: &str) -> Result<Value> {
    channel::ensure_home(app)?;
    let bots = channel::bots(app).await?;
    match card["kind"].as_str() {
        Some("channel") => {
            let mut members = vec![];
            for m in card["members"].as_array().cloned().unwrap_or_default() {
                let (key, mode) = match &m {
                    Value::String(s) => (s.clone(), Mode::Mentions),
                    v => (v["bot"].as_str().context("each member is a bot slug or {\"bot\": \"release\", \"mode\": \"all\"}")?.to_string(), mode_of(&v["mode"])?),
                };
                members.push(NewMember { bot: key, mode });
            }
            let n = NewChannel { workspace_id: workspace.to_string(), name: card["name"].as_str().unwrap_or_default().to_string(), purpose: card["purpose"].as_str().unwrap_or_default().to_string(), members };
            if apply {
                return Ok(json!({"id": channel::create(app, n, by).await?.id}));
            }
            let (name, members) = channel::check_new(app, &n).await?;
            let members: Vec<Value> = members.iter().map(|(id, mode)| json!({"bot": id, "name": bots.get(id).map_or("", |b| b.name.as_str()), "mode": mode})).collect();
            Ok(json!({"kind": "channel", "name": name, "purpose": card["purpose"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" "), "members": members}))
        }
        Some("channel_join") => {
            let bot_id = match bot {
                Some(b) => b.to_string(),
                None => card["bot_id"].as_str().filter(|b| !b.is_empty()).context("only a bot joins a channel: this agent is not a bot's session")?.to_string(),
            };
            let info = bots.get(&bot_id).context("no such bot")?;
            ensure!(info.workspace_id == workspace, "{} belongs to another workspace: agents propose only in their own workspace", info.name);
            let ch = named(app, workspace, card["channel"].as_str().unwrap_or_default()).await?;
            ensure!(ch.kind == Kind::Channel, "a DM's members are its two bots");
            let mode = mode_of(&card["mode"])?;
            let member = channel::members_of(app, &ch.id).await?.into_iter().any(|m| m.bot_id == bot_id);
            if apply {
                ensure!(!ch.archived, "#{} is archived: bring it back first", ch.name);
                // Joined since (the owner added it): the card changes nothing, its mode included.
                if !member {
                    channel::set_member(app, &ch.id, &bot_id, Some(mode)).await?;
                }
                return Ok(json!({"id": ch.id}));
            }
            if member {
                bail!("{} is already a member of #{}", info.name, ch.name);
            }
            ensure!(!ch.archived, "#{} is archived", ch.name);
            Ok(json!({"kind": "channel_join", "channel": ch.id, "channel_name": ch.name, "bot_id": bot_id, "bot_name": info.name, "mode": mode}))
        }
        _ => bail!("not a channel card"),
    }
}

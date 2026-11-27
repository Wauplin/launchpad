//! `launchpad recall …`: search the past sessions of this session's workspace (`/api/recall/{workspace}…` on Home).
//! Results are derived evidence: the knowledge base, bot memory, BOT.md and repository instructions win every
//! conflict. Reads need read access (a bot's session also `recall = true` in its bot.toml); `backfill`, `rebuild`,
//! `enable` and `disable` need admin access.
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use launchpad_core::paths::Paths;
use serde_json::{Value, json};

use crate::{query, url_segment};

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RecallArgs {
    /// Workspace ID (default: this session's workspace).
    #[arg(long, global = true)]
    workspace: Option<String>,
    #[command(subcommand)]
    command: Option<RecallCommand>,
    #[command(flatten)]
    search: SearchArgs,
}

#[derive(Args)]
struct SearchArgs {
    /// What to look for, in free text (quote it).
    query: Vec<String>,
    /// How many hits (default 8, at most 20). Each is a passage to read.
    #[arg(short, long)]
    k: Option<usize>,
    /// Only turns on or after this date (YYYY-MM-DD): for the latest word on a topic, since ranking ignores age.
    #[arg(long)]
    since: Option<String>,
    /// Only turns on or before this date (YYYY-MM-DD).
    #[arg(long)]
    until: Option<String>,
    /// Only one kind of block: text, thinking, tool_use or tool_result.
    #[arg(long = "type")]
    block_type: Option<String>,
    /// Only sessions of this bot (slug, name or ID).
    #[arg(long, conflicts_with = "mine")]
    bot: Option<String>,
    /// Only your own bot's sessions.
    #[arg(long)]
    mine: bool,
    /// One line per hit, no passage: to choose what to read with `recall get`.
    #[arg(long)]
    oneline: bool,
}

#[derive(Subcommand)]
enum RecallCommand {
    /// Read the turns of a session around an event: `recall get <session-id> --seq <seq>` (the two parts of a hit's
    /// `lp:session/<session-id>/<seq>`), or `--from <seq> [--to <seq>]`.
    Get {
        session: String,
        #[arg(long, conflicts_with = "from")]
        seq: Option<i64>,
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
    },
    /// Whether recall works, how far this workspace's index is (sessions and turns indexed, newest indexed turn,
    /// size), what it left out and why, and the last backfill's report (JSON).
    Status,
    /// Index every stored session of the workspace, in the background on Home (admin access). Skips what is already
    /// indexed and continues after an interruption; `recall status` shows its progress.
    Backfill {
        /// Only count what it would do.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete the workspace's recall memory and build it again from the stored sessions (admin access): the only way
    /// it forgets archived or hidden sessions.
    Rebuild,
    /// Turn recall on for Home (admin access).
    Enable,
    /// Turn recall off for Home (admin access): nothing is indexed or answered; the memories stay.
    Disable,
}

pub async fn command(paths: &Paths, args: RecallArgs) -> Result<()> {
    let RecallArgs { workspace, command, search } = args;
    let print = |v: &Value| println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    if let Some(RecallCommand::Enable | RecallCommand::Disable) = &command {
        let client = crate::Client::new(paths)?;
        let on = matches!(command, Some(RecallCommand::Enable));
        print(&client.json("PATCH", "/api/recall/config", Some(json!({ "enabled": on }))).await?);
        return Ok(());
    }
    let (client, ws) = crate::oracle::scoped(paths, workspace).await?;
    let base = format!("/api/recall/{}", url_segment(&ws));
    let text = |v: Value| println!("{}", v["text"].as_str().unwrap_or_default().trim_end());
    match command {
        None => {
            let q = search.query.join(" ");
            if q.trim().is_empty() {
                bail!("give what to look for: launchpad recall \"<query>\" (or recall get, recall status)");
            }
            let k = search.k.map(|k| k.to_string());
            let mut pairs: Vec<(&str, &str)> = vec![("q", q.as_str())];
            for (name, value) in [("k", k.as_deref()), ("since", search.since.as_deref()), ("until", search.until.as_deref()), ("type", search.block_type.as_deref()), ("bot", search.bot.as_deref())] {
                if let Some(v) = value {
                    pairs.push((name, v));
                }
            }
            if search.mine {
                pairs.push(("mine", "true"));
            }
            if search.oneline {
                pairs.push(("oneline", "true"));
            }
            text(client.json("GET", &query(&format!("{base}/search"), &pairs), None).await?);
        }
        Some(RecallCommand::Get { session, seq, from, to }) => {
            let numbers: Vec<(&str, String)> = [("seq", seq), ("from", from), ("to", to)].into_iter().filter_map(|(n, v)| v.map(|v| (n, v.to_string()))).collect();
            let mut pairs: Vec<(&str, &str)> = vec![("session", session.as_str())];
            pairs.extend(numbers.iter().map(|(n, v)| (*n, v.as_str())));
            text(client.json("GET", &query(&format!("{base}/get"), &pairs), None).await?);
        }
        Some(RecallCommand::Status) => print(&client.json("GET", &format!("{base}/status"), None).await?),
        Some(RecallCommand::Backfill { dry_run }) => print(&client.json("POST", &format!("{base}/backfill"), Some(json!({ "dry_run": dry_run }))).await?),
        Some(RecallCommand::Rebuild) => print(&client.json("POST", &format!("{base}/rebuild"), Some(json!({}))).await?),
        Some(RecallCommand::Enable | RecallCommand::Disable) => unreachable!("handled above"),
    }
    Ok(())
}

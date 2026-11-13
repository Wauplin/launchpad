//! Proposals: inert cards written by agents with the Launchpad CLI tool (`launchpad propose`), accepted by an
//! owner click, or at once for one-time starts when the author is a bot in auto mode (`bot.toml` `proposals`).
//! Ready cards name an owner action of the action list (merge, publish, post, send, run CI, Inbox done): the owner's click
//! runs that action's own API call from the card, then marks the card started (`ran`); accepting never runs them.
//! Like agent suggestions, the agent drafts and the owner publishes. Accepting claims the proposal
//! durably before acting; a claim interrupted by a crash becomes Uncertain and is never retried
//! automatically. Launches use the session ID chosen at proposal time, so a finished launch is found
//! again by that ID.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{crons::CronCondition, schedules::LaunchDelay};
use launchpad_core::{App, model::LaunchSpec, util::now_ms};

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalBody {
    /// A new agent.
    Launch { spec: LaunchSpec },
    /// A message to an existing session.
    Message { session_id: String, text: String },
    /// A new agent that starts when every condition is met.
    Schedule { spec: LaunchSpec, conditions: Vec<LaunchDelay> },
    /// A repeating agent (cron, in the time zone of the browser that accepts it, else `[home] time_zone`).
    Cron { spec: LaunchSpec, schedule: String, conditions: Vec<CronCondition> },
    /// A list of sessions to look at; nothing to start.
    Status { session_ids: Vec<String> },
    /// A new preset, or new values for the preset `preset_id`. Proposals never delete presets.
    Preset {
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        preset_id: Option<String>,
        label: String,
        text: String,
        #[serde(default = "default_true")]
        agent: bool,
    },
    /// A new bot of the proposal's workspace. Creating it starts nothing: the owner sends its first task.
    Bot {
        name: String,
        /// BOT.md.
        instructions: String,
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
        /// A folder of the workspace. None: each session starts in a temporary folder.
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
    },
    /// A new routine of a bot of the workspace: its runs are messages to the bot's thread. A shorthand: Home stores
    /// it as a `BotFiles` card with the new `routines/<name>.toml` (older cards still start as they are).
    Routine {
        bot_id: String,
        title: String,
        /// What the bot does at each run.
        instructions: String,
        #[serde(default)]
        trigger: crate::crons::Trigger,
        #[serde(default)]
        schedule: String,
        #[serde(default)]
        conditions: Vec<CronCondition>,
    },
    /// A new BOT.md for a bot of the workspace. A shorthand: Home stores it as a `BotFiles` card (older cards still
    /// start as they are). Accepting fails when BOT.md changed since.
    BotInstructions {
        bot_id: String,
        instructions: String,
        /// BOT.md when the proposal was made (filled by Home).
        #[serde(default)]
        before: String,
        /// The unified diff from `before` (filled by Home).
        #[serde(default)]
        patch: String,
    },
    /// Changes to the files of a bot of the workspace, often written by the bot itself: BOT.md and its routines
    /// (`routines/<name>.toml`, null deletes one), one diff per file. Accepting writes them all at once (the routines
    /// follow their files) and fails when one of them changed since. bot.toml stays the owner's, `memory/` the bot's.
    BotFiles {
        bot_id: String,
        /// Path → its whole new content, or null to delete a routine file.
        #[ts(type = "Record<string, string | null>")]
        files: std::collections::BTreeMap<String, Option<String>>,
        /// Each file's SHA-256 when the card was made, null when it did not exist (filled by Home).
        #[serde(default)]
        #[ts(type = "Record<string, string | null>")]
        before: std::collections::BTreeMap<String, Option<String>>,
        /// The unified diff of each file (filled by Home).
        #[serde(default)]
        #[ts(type = "Record<string, string>")]
        patches: std::collections::BTreeMap<String, String>,
    },
    /// A ready click: an owner action of the action list (`actions::Action::ready`) with its route parameters and
    /// body, run only by the owner's click on the card (the same button, confirmation and API call as its page).
    Ready {
        action: String,
        args: Vec<String>,
        #[serde(default)]
        #[ts(type = "Record<string, unknown>")]
        input: serde_json::Value,
        /// The agent session the click is about; a merge names the session whose branch it merges.
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// A report about what an agent did (often on a GitHub pull request or issue), or a question about it: nothing
    /// to start. The owner marks it read, or replies to its author's thread from the card (`reply`). One waiting
    /// report per GitHub item and author (the bot, else the session): a new report about the same item replaces it.
    Report {
        outcome: ReportOutcome,
        /// `owner/repo#number`: the GitHub issue or pull request it is about.
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        github: Option<String>,
        /// Markdown: what was done and why, or the question and the text the agent would post.
        body: String,
    },
    /// A new named bot channel of the proposal's workspace with its member bots (`launchpad propose channel`).
    /// Accepting creates it, like `channel.create`; it starts nothing.
    Channel {
        /// Lowercase letters, digits and dashes (`release-train`).
        name: String,
        #[serde(default)]
        purpose: String,
        /// Member bots: a slug or `{"bot": "release", "mode": "all"}` when filed; Home stores the bot IDs and names.
        #[serde(default, deserialize_with = "card_members")]
        members: Vec<ChannelCardMember>,
    },
    /// A bot asks to join a named channel of its workspace (`launchpad channel join <name>`). Accepting adds it, like
    /// `channel.add-member`. Home fills in the channel ID and names, and the bot: always the author's.
    ChannelJoin {
        /// The channel's name when filed; its ID once stored.
        channel: String,
        #[serde(default)]
        channel_name: String,
        #[serde(default)]
        bot_id: String,
        #[serde(default)]
        bot_name: String,
        #[serde(default = "mentions")]
        #[ts(type = "\"all\" | \"mentions\" | \"digest\" | \"muted\"")]
        mode: String,
    },
    /// A new tool in Home's registry. Adding it asks for nothing secret; the owner pastes the token after.
    /// Its token is always a new vault entry named like the tool, never an existing one.
    Tool {
        tool: launchpad_core::vault::Tool,
        /// Where the owner creates the token.
        #[serde(default)]
        #[ts(optional)]
        #[serde(skip_serializing_if = "Option::is_none")]
        token_url: Option<String>,
    },
}

/// A member of a proposed channel: the bot (its ID once stored), its name and its mode.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct ChannelCardMember {
    pub bot: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "mentions")]
    #[ts(type = "\"all\" | \"mentions\" | \"digest\" | \"muted\"")]
    pub mode: String,
}

fn mentions() -> String {
    "mentions".into()
}

/// `["release", {"bot": "docs", "mode": "all"}]`.
fn card_members<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<ChannelCardMember>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum One {
        Key(String),
        Full(ChannelCardMember),
    }
    Ok(Vec::<One>::deserialize(d)?.into_iter().map(|o| match o { One::Key(bot) => ChannelCardMember { bot, name: String::new(), mode: mentions() }, One::Full(m) => m }).collect())
}

/// What a report says happened. `NeedsYou` waits for an answer; the others are for the owner to read.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportOutcome {
    NeedsYou,
    Merged,
    Approved,
    Commented,
    Waiting,
    Closed,
    Info,
}

/// `owner/repo#number`, as Today and GitHub links read it.
fn valid_github_ref(r: &str) -> bool {
    let Some((repo, number)) = r.split_once('#') else { return false };
    let Some((owner, name)) = repo.split_once('/') else { return false };
    let part = |x: &str| !x.is_empty() && x.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    part(owner) && part(name) && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

/// Who a report speaks for: its bot (every session of the thread), else its session.
fn author_key(app: &App, session_id: Option<&str>) -> Option<String> {
    let s = app.sessions.get(session_id?)?;
    Some(match &s.launch_spec.bot { Some(b) => format!("bot:{}", b.id), None => format!("session:{}", s.id) })
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Proposed,
    /// Claimed by an owner click; the action is in progress.
    Starting,
    /// Done: `result_id` is the session, schedule or cron.
    Started,
    Dismissed,
    /// Superseded by another proposal before starting, or edited in the composer (no `replaced_by`).
    Replaced,
    /// The action failed before doing anything; the owner may start it again.
    Failed,
    /// A claim whose outcome is unknown (daemon restart while starting). Never retried automatically.
    Uncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Proposal {
    pub id: String,
    /// The workspace of the session that wrote it (None: no workspace).
    pub workspace_id: Option<String>,
    /// The session that wrote it.
    pub session_id: Option<String>,
    pub title: String,
    /// Why the agent proposes it, one or two sentences.
    pub note: Option<String>,
    pub body: ProposalBody,
    pub state: ProposalState,
    /// For launches and new presets: the ID reserved at proposal time. Then the started session, schedule, cron or preset.
    pub result_id: Option<String>,
    /// Successor card when this proposal was replaced.
    #[serde(default)]
    pub replaced_by: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize)]
pub struct Create {
    #[serde(default)]
    pub replaces: Option<String>,
    pub title: String,
    #[serde(default)]
    pub note: Option<String>,
    pub body: ProposalBody,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

async fn save(app: &App, p: &Proposal) -> Result<()> {
    let (id, json) = (p.id.clone(), serde_json::to_string(p)?);
    app.state_db.call(move |c| c.execute("INSERT OR REPLACE INTO launch_proposal (id, json) VALUES (?1, ?2)", params![id, json]).map(|_| ()).map_err(Into::into)).await?;
    app.bus.publish("proposals", "changed", serde_json::to_value(p)?);
    Ok(())
}

pub async fn list(app: &App) -> Result<Vec<Proposal>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM launch_proposal ORDER BY rowid DESC LIMIT 500")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
        })
        .await
}

pub async fn get(app: &App, id: &str) -> Result<Proposal> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM launch_proposal WHERE id = ?1", params![id], |r| r.get(0)).ok())).await?;
    Ok(serde_json::from_str(&json.context("no such proposal")?)?)
}

/// Paid usage is always off on proposals; the owner changes it in the composer.
fn sanitize(spec: &mut LaunchSpec, workspace_id: &Option<String>) -> Result<()> {
    ensure!(spec.bot.is_none(), "proposals cannot start bot sessions: propose a message to the bot instead");
    spec.allow_paid_usage = false;
    spec.workspace_id = workspace_id.clone();
    Ok(())
}

/// Stores an inert proposal after the same validation as the launch preview.
pub async fn create(app: &App, c: Create) -> Result<Proposal> {
    ensure!(!c.title.trim().is_empty(), "give the proposal a title");
    let mut body = c.body;
    let author = c.session_id.as_deref().and_then(|sid| app.sessions.get(sid));
    // An agent proposes only for its own workspace (`launchpad_core::scope`); the owner names any workspace.
    let author_ws = match &author {
        Some(s) => Some(launchpad_core::scope::of_session(app, &s.id).await.context("this session has no workspace, so it proposes nothing")?),
        None => None,
    };
    if let (Some(own), Some(w)) = (&author_ws, &c.workspace_id) {
        launchpad_core::scope::ensure_inside(own, Some(w), &format!("workspace {w}"))?;
    }
    let mut workspace_id = author_ws.clone().or(c.workspace_id.clone());
    let same_workspace = |bot: &launchpad_core::bots::Bot| author.is_none() || author_ws.as_deref() == Some(bot.workspace_id.as_str());
    let mut result_id = None;
    // The shorthands become one bot-files card, so every change to a bot's files reads as a diff.
    match &body {
        ProposalBody::Routine { bot_id, title, instructions, trigger, schedule, conditions } => {
            ensure!(!title.trim().is_empty(), "give the routine a name");
            // A channel trigger may name the channel (`#name`, `@bot` for a DM): the file keeps its ID.
            let mut trigger = trigger.clone();
            if let Ok(bot) = launchpad_core::bots::get(app, bot_id).await
                && same_workspace(&bot)
            {
                crate::channel_trigger::resolve_trigger(app, &bot, &mut trigger).await?;
            }
            let f = crate::crons::RoutineFile { title: title.trim().into(), paused: false, schedule: schedule.clone(), time_zone: None, instructions: instructions.clone(), trigger, conditions: conditions.clone() };
            let taken: Vec<String> = launchpad_core::bots::routine_files(app, bot_id).await.with_context(|| format!("no bot {bot_id} (see `bots list`)"))?.into_iter().map(|(p, _)| p).collect();
            let path = crate::crons::file_path_for(&f.title, &taken);
            body = ProposalBody::BotFiles { bot_id: bot_id.clone(), files: [(path, Some(crate::crons::render_file(&f)?))].into(), before: Default::default(), patches: Default::default() };
        }
        ProposalBody::BotInstructions { bot_id, instructions, .. } => {
            body = ProposalBody::BotFiles { bot_id: bot_id.clone(), files: [(launchpad_core::bots::INSTRUCTIONS.to_string(), Some(instructions.clone()))].into(), before: Default::default(), patches: Default::default() };
        }
        _ => {}
    }
    if let Some(own) = &author_ws {
        check_scope(app, own, &mut body).await?;
    }
    match &mut body {
        ProposalBody::Launch { spec } | ProposalBody::Schedule { spec, .. } | ProposalBody::Cron { spec, .. } => {
            sanitize(spec, &workspace_id)?;
            if spec.profile.is_some() && spec.harness.is_none() {
                let config = app.config.get();
                spec.harness = config.profiles.iter().find(|p| Some(&p.id) == spec.profile.as_ref()).map(|p| p.harness);
            }
            ensure!(spec.harness.is_some(), "set a profile (see `profiles list`)");
            ensure!(!spec.text.trim().is_empty(), "write a prompt in spec.text");
            app.sessions.preview(app, spec).await.context("invalid launch spec")?;
            if matches!(body, ProposalBody::Launch { .. }) {
                result_id = Some(uuid::Uuid::new_v4().to_string());
            }
        }
        ProposalBody::Message { session_id, text } => {
            ensure!(app.sessions.get(session_id).is_some(), "no session {session_id}");
            ensure!(!text.trim().is_empty(), "the message is empty");
        }
        ProposalBody::Status { session_ids } => ensure!(!session_ids.is_empty(), "list at least one session"),
        ProposalBody::Report { github, body, .. } => {
            ensure!(!body.trim().is_empty(), "write the report in body (Markdown)");
            if let Some(g) = github {
                *g = g.trim().to_string();
                ensure!(valid_github_ref(g), "github is owner/repo#number, such as huggingface/huggingface.js#2716");
            }
        }
        ProposalBody::Tool { tool, token_url } => {
            tool.credential = tool.credential.take().map(|_| tool.id.clone());
            launchpad_core::vault::check_tool(tool)?;
            if let Some(u) = token_url {
                ensure!(u.starts_with("https://"), "token_url must be an https address");
            }
        }
        ProposalBody::Bot { name, instructions, profile, folder } => {
            let ws = workspace_id.as_deref().context("bots belong to a workspace: propose from a session of that workspace")?;
            ensure!(!name.trim().is_empty(), "give the bot a name");
            ensure!(!instructions.trim().is_empty(), "write the bot's instructions (its BOT.md)");
            if let Some(p) = profile {
                ensure!(app.config.get().profiles.iter().any(|x| &x.id == p), "no profile {p} (see `profiles list`)");
            }
            if let Some(f) = folder {
                let p = launchpad_core::projects::get(app, f).await?.with_context(|| format!("no folder {f} (see `folders list`)"))?;
                ensure!(p.workspace_id.as_deref() == Some(ws), "{} belongs to another workspace", p.name);
                ensure!(!p.is_temp, "{} is a temporary folder: use null instead", p.name);
            }
        }
        ProposalBody::Routine { bot_id, title, instructions, trigger, schedule, .. } => {
            let bot = launchpad_core::bots::get(app, bot_id).await.with_context(|| format!("no bot {bot_id} (see `bots list`)"))?;
            ensure!(same_workspace(&bot), "{} belongs to another workspace", bot.settings.name);
            ensure!(!title.trim().is_empty(), "give the routine a name");
            ensure!(!instructions.trim().is_empty(), "write what the bot does at each run");
            if *trigger == crate::crons::Trigger::Schedule {
                crate::crons::next_after(schedule, None, now_ms()).context("invalid cron schedule")?;
            } else {
                crate::crons::validate_trigger(trigger, schedule, None)?;
            }
            workspace_id = Some(bot.workspace_id);
        }
        ProposalBody::BotInstructions { bot_id, instructions, before, patch } => {
            let bot = launchpad_core::bots::get(app, bot_id).await.with_context(|| format!("no bot {bot_id} (see `bots list`)"))?;
            ensure!(same_workspace(&bot), "{} belongs to another workspace", bot.settings.name);
            ensure!(!instructions.trim().is_empty(), "write the whole new BOT.md");
            if !instructions.ends_with('\n') {
                instructions.push('\n');
            }
            ensure!(*instructions != bot.instructions, "this is already the bot's BOT.md");
            *before = bot.instructions.clone();
            *patch = launchpad_core::kb::unified(&bot.instructions, instructions);
            workspace_id = Some(bot.workspace_id);
        }
        ProposalBody::BotFiles { bot_id, files, before, patches } => {
            let bot = launchpad_core::bots::get(app, bot_id).await.with_context(|| format!("no bot {bot_id} (see `bots list`)"))?;
            ensure!(same_workspace(&bot), "{} belongs to another workspace", bot.settings.name);
            let current = current_files(app, &bot.id).await?;
            before.clear();
            patches.clear();
            for (path, text) in files.iter_mut() {
                ensure!(path == launchpad_core::bots::INSTRUCTIONS || launchpad_core::bots::routine_path_ok(path), "a card changes BOT.md and routines/<name>.toml (lowercase letters, digits, - and _), not {path}: bot.toml is the owner's, memory/ yours (`bot memory write`)");
                let old = current.get(path.as_str());
                match text {
                    Some(t) if path == launchpad_core::bots::INSTRUCTIONS => {
                        ensure!(!t.trim().is_empty(), "write the whole new BOT.md");
                        if !t.ends_with('\n') {
                            t.push('\n');
                        }
                    }
                    Some(t) => {
                        crate::crons::check_file(app, &bot, t).await.with_context(|| path.clone())?;
                    }
                    None => {
                        ensure!(path != launchpad_core::bots::INSTRUCTIONS, "BOT.md cannot be deleted");
                        ensure!(old.is_some(), "{path} does not exist");
                    }
                }
                before.insert(path.clone(), old.map(|(sha, _)| sha.clone()));
                patches.insert(path.clone(), launchpad_core::kb::unified(old.map_or("", |(_, t)| t.as_str()), text.as_deref().unwrap_or("")));
            }
            // A file the card would leave as it is changes nothing.
            files.retain(|path, text| current.get(path.as_str()).map(|(_, t)| t) != text.as_ref());
            before.retain(|path, _| files.contains_key(path));
            patches.retain(|path, _| files.contains_key(path));
            ensure!(!files.is_empty(), "these files already have this content");
            workspace_id = Some(bot.workspace_id);
        }
        ProposalBody::Ready { action, args, input, session_id } => {
            use launchpad_core::actions;
            if let Some(sid) = session_id {
                ensure!(app.sessions.get(sid).is_some(), "no session {sid}");
            }
            ensure!(action != "worktree.merge" || session_id.is_some(), "a merge card names the session whose branch it merges (session_id)");
            let a = actions::get(action).filter(|a| a.ready).with_context(|| format!("{action} is not a ready action: use one of {}", actions::ACTIONS.iter().filter(|a| a.ready).map(|a| a.name).collect::<Vec<_>>().join(", ")))?;
            actions::path(a, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
            if input.is_null() {
                *input = serde_json::json!({});
            }
            let obj = input.as_object().context("input is a JSON object")?;
            for f in a.input {
                ensure!(f.optional || obj.contains_key(f.name), "{} needs input.{}", a.name, f.name);
            }
            if let Some(k) = obj.keys().find(|k| !a.input.iter().any(|f| f.name == k.as_str())) {
                bail!("{} takes no input.{k}", a.name);
            }
        }
        ProposalBody::Channel { .. } | ProposalBody::ChannelJoin { .. } => {
            let ws = workspace_id.as_deref().context("channels belong to a workspace: propose from a session of that workspace")?;
            let bot = author.as_ref().and_then(|s| s.launch_spec.bot.as_ref()).map(|b| b.id.clone());
            ensure!(author.is_none() || bot.is_some() || matches!(body, ProposalBody::Channel { .. }), "only a bot joins a channel: this agent is not a bot's session");
            let checked = launchpad_core::ext::extensions().channel_card(app, ws, serde_json::to_value(&body)?, bot.as_deref(), false, "").await?;
            body = serde_json::from_value(checked)?;
        }
        ProposalBody::Preset { preset_id, label, text, .. } => {
            ensure!(!label.trim().is_empty(), "give the preset a label");
            ensure!(!text.trim().is_empty(), "write the preset text");
            match preset_id {
                Some(id) => ensure!(app.config.get().presets.iter().any(|p| &p.id == id), "no preset {id} (see `presets list`)"),
                None => result_id = Some(format!("preset-{}", uuid::Uuid::new_v4().simple())),
            }
        }
    }
    if let ProposalBody::Cron { schedule, .. } = &body {
        crate::crons::next_after(schedule, None, now_ms()).context("invalid cron schedule")?;
    }
    // Only proposals authored by a bot in auto mode are eligible, and only one-time starts and messages. Anything
    // that repeats or lasts (crons, routines, bots, BOT.md, presets, tools) and ready cards always wait for a click
    // (presets are shared settings: a session with admin access changes them directly, with Undo).
    // A message to a session with admin access always waits for the owner: an auto-mode bot never steers an
    // admin session without a click.
    let to_admin = match &body {
        ProposalBody::Message { session_id, .. } => launchpad_core::vault::session_access(app, session_id).await.ok().flatten() == Some(launchpad_core::vault::LaunchpadAccess::Admin),
        _ => false,
    };
    let auto = matches!(body, ProposalBody::Launch { .. } | ProposalBody::Message { .. } | ProposalBody::Schedule { .. }) && !to_admin && auto_mode(app, author.as_ref()).await;
    // Automatic work needs confirmed quota: an automatic launch holds its prompt in its start and a schedule waits
    // before it runs, but a message is sent at once, so without confirmed quota it waits for the owner's click and
    // its note says why.
    let held = match (&body, auto) {
        (ProposalBody::Message { session_id, .. }, true) => app.sessions.get(session_id).and_then(|s| launchpad_core::quota::hold_reason(app, s.harness, &s.launch_spec)),
        _ => None,
    };
    let mut note = c.note.filter(|n| !n.trim().is_empty());
    if let Some(why) = &held {
        let line = format!("Not sent automatically: automatic work needs confirmed quota ({why}). Start sends it now.");
        note = Some(note.map_or_else(|| line.clone(), |n| format!("{n}\n\n{line}")));
    }
    // A join card always says who asks and for what.
    let title = match &body {
        ProposalBody::ChannelJoin { channel_name, bot_name, .. } => format!("{bot_name} asks to join #{channel_name}"),
        _ => c.title.trim().to_string(),
    };
    let now = now_ms();
    let p = Proposal { id: uuid::Uuid::new_v4().to_string(), workspace_id, session_id: c.session_id, title, note, body, state: ProposalState::Proposed, result_id, replaced_by: None, error: None, created_at: now, updated_at: now };
    // Share the claim lock with accept/dismiss: a replacement cannot race an owner click.
    let _lock = ACCEPT.lock().await;
    let replaces = match (&c.replaces, &p.body) {
        (None, ProposalBody::Report { github: Some(g), .. }) => waiting_report(app, &p, g).await?,
        (None, ProposalBody::ChannelJoin { channel, bot_id, .. }) => waiting_join(app, &p, channel, bot_id).await?,
        _ => c.replaces.clone(),
    };
    let stored = p.clone();
    app.state_db.call(move |db| store_created(db, &stored, replaces.as_deref())).await?;
    app.bus.publish("proposals", "changed", serde_json::to_value(&p)?);
    drop(_lock);
    if auto && held.is_none() {
        return accept_inner(app, &p.id, false, None, None).await;
    }
    Ok(p)
}

/// Every record an agent's card names is in its workspace `ws`: the folders, sessions, Local PRs and bots of
/// launch specs, the session a message or a status card names, the sessions a schedule waits for, the item a
/// ready card or a report is about (a repository of another workspace's folders; repositories without a folder
/// belong to no workspace). Bot cards are checked where they are read (their bot's workspace).
async fn check_scope(app: &App, ws: &str, body: &mut ProposalBody) -> Result<()> {
    use launchpad_core::scope;
    let session = |id: String| async move { scope::ensure_inside(ws, scope::of_session(app, &id).await.as_deref(), &format!("session {id}")) };
    match body {
        ProposalBody::Launch { spec } | ProposalBody::Cron { spec, .. } => scope::check_spec(app, ws, spec).await?,
        ProposalBody::Schedule { spec, conditions } => {
            scope::check_spec(app, ws, spec).await?;
            for c in conditions.iter() {
                if let crate::schedules::LaunchDelay::Agents { session_ids } = c {
                    for id in session_ids {
                        session(id.clone()).await?;
                    }
                }
            }
        }
        ProposalBody::Message { session_id, .. } => session(session_id.clone()).await?,
        ProposalBody::Status { session_ids } => {
            for id in session_ids.iter() {
                session(id.clone()).await?;
            }
        }
        ProposalBody::Report { github: Some(g), .. } => {
            let repo = g.trim().split('#').next().unwrap_or_default();
            ensure!(!scope::repo_outside(app, ws, repo).await, "{repo} belongs to another workspace's folder: agents report only in their own workspace");
        }
        ProposalBody::Ready { action, args, session_id, .. } => {
            if let Some(id) = session_id {
                session(id.clone()).await?;
            }
            let first = args.first().cloned().unwrap_or_default();
            match action.as_str() {
                a if a.starts_with("local-pr.") => scope::ensure_inside(ws, scope::of_predraft(app, &first).await.as_deref(), &format!("Local PR {first}"))?,
                "worktree.merge" => scope::ensure_inside(ws, scope::of_worktree(app, &first).await.as_deref(), &format!("worktree {first}"))?,
                "github.run-ci" => {
                    let repo = args.iter().take(2).cloned().collect::<Vec<_>>().join("/");
                    ensure!(!scope::repo_outside(app, ws, &repo).await, "{repo} belongs to another workspace's folder: agents prepare clicks only in their own workspace");
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

/// The waiting report of the same author and workspace about the same GitHub item, which a new report replaces.
async fn waiting_report(app: &App, new: &Proposal, github: &str) -> Result<Option<String>> {
    let Some(author) = author_key(app, new.session_id.as_deref()) else { return Ok(None) };
    Ok(list(app).await?.into_iter().find(|p| {
        matches!(p.state, ProposalState::Proposed | ProposalState::Failed)
            && p.workspace_id == new.workspace_id
            && matches!(&p.body, ProposalBody::Report { github: Some(g), .. } if g == github)
            && author_key(app, p.session_id.as_deref()).as_deref() == Some(author.as_str())
    }).map(|p| p.id))
}

/// The waiting join card of the same bot for the same channel, which a new one replaces.
async fn waiting_join(app: &App, new: &Proposal, channel: &str, bot: &str) -> Result<Option<String>> {
    Ok(list(app).await?.into_iter().find(|p| {
        matches!(p.state, ProposalState::Proposed | ProposalState::Failed) && p.workspace_id == new.workspace_id && matches!(&p.body, ProposalBody::ChannelJoin { channel: c, bot_id: b, .. } if c == channel && b == bot)
    }).map(|p| p.id))
}

/// The author is a session of a bot whose `bot.toml` says `proposals = "auto"`.
async fn auto_mode(app: &App, author: Option<&launchpad_core::model::Session>) -> bool {
    let Some(b) = author.and_then(|s| s.launch_spec.bot.as_ref()) else { return false };
    launchpad_core::bots::get(app, &b.id).await.is_ok_and(|bot| bot.settings_error.is_none() && bot.settings.proposals == launchpad_core::bots::ProposalMode::Auto)
}

/// Atomically retire the old card and insert its successor, or leave both untouched on error.
fn store_created(db: &mut rusqlite::Connection, p: &Proposal, replaces: Option<&str>) -> Result<()> {
    let tx = db.transaction()?;
    if let Some(id) = replaces {
        let json: Option<String> = tx.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let mut old: Proposal = serde_json::from_str(&json.context("no such proposal to replace")?)?;
        ensure!(matches!(old.state, ProposalState::Proposed | ProposalState::Failed), "cannot replace proposal '{}' ({:?}); only waiting, unstarted proposals can be replaced", old.title, old.state);
        ensure!(old.workspace_id == p.workspace_id, "cannot replace a proposal from another workspace");
        old.state = ProposalState::Replaced;
        old.replaced_by = Some(p.id.clone());
        old.updated_at = p.updated_at;
        tx.execute("UPDATE launch_proposal SET json = ?2 WHERE id = ?1", params![id, serde_json::to_string(&old)?])?;
    }
    tx.execute("INSERT INTO launch_proposal (id, json) VALUES (?1, ?2)", params![p.id, serde_json::to_string(p)?])?;
    tx.commit()?;
    Ok(())
}

/// Serializes claims, dismissals and replacements so only one can retire a waiting card.
static ACCEPT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Owner click: claim, act, record the result. `body` may replace the proposal's spec (edited in the card).
pub async fn accept(app: &App, id: &str) -> Result<Proposal> {
    accept_inner(app, id, true, None, None).await
}

/// Accept from a browser: a routine or cron keeps the browser's time zone. `agent`: a session with admin access
/// accepted it, not the owner.
pub async fn accept_in(app: &App, id: &str, time_zone: Option<String>, agent: Option<String>) -> Result<Proposal> {
    accept_inner(app, id, true, time_zone, agent).await
}

/// Who a message card's text comes from: its author (its bot, else its session). The owner's click resets the
/// hop count; a card accepted automatically (auto mode) or by an agent counts one hop after the author's turn, or the
/// accepting agent's.
async fn card_sender(app: &App, p: &Proposal, owner_click: bool, agent: Option<&str>) -> Result<Option<launchpad_core::model::MessageFrom>> {
    let Some(sid) = p.session_id.as_deref() else { return Ok(None) };
    let hops = match (owner_click, agent) {
        (true, None) => 0,
        (_, Some(a)) => launchpad_core::senders::hops_of(app, a).await + 1,
        (false, None) => launchpad_core::senders::hops_of(app, sid).await + 1,
    };
    launchpad_core::senders::guard(hops)?;
    // An author whose record is gone is still not the owner: "an agent".
    let from = match app.sessions.get(sid) {
        Some(author) => launchpad_core::senders::sender_of(&author, hops),
        None => launchpad_core::model::MessageFrom { kind: launchpad_core::model::SenderKind::Session, id: sid.to_string(), hops, ..Default::default() },
    };
    Ok(Some(launchpad_core::model::MessageFrom { card: Some(p.id.clone()), accepted: owner_click && agent.is_none(), ..from }))
}

/// Who a launch card's first message comes from: nobody when the owner clicked it (the launch is the owner's), else
/// its author, one hop into the chain (`card_sender`), without a header (the new agent's task is its own). Any sender
/// the card's spec named is replaced.
async fn launch_sender(app: &App, p: &Proposal, owner_click: bool, agent: Option<&str>) -> Result<Option<launchpad_core::model::MessageFrom>> {
    if owner_click && agent.is_none() {
        return Ok(None);
    }
    card_sender(app, p, owner_click, agent).await
}

/// `manual`: an owner click (or an admin session's `agent` accept), which may also retry a failed card.
async fn accept_inner(app: &App, id: &str, manual: bool, time_zone: Option<String>, agent: Option<String>) -> Result<Proposal> {
    let mut p = {
        let _lock = ACCEPT.lock().await;
        let mut p = get(app, id).await?;
        match p.state {
            ProposalState::Proposed => {}
            ProposalState::Failed if manual => {}
            ProposalState::Failed => return Ok(p),
            ProposalState::Started => return Ok(p),
            ProposalState::Uncertain => bail!("Delivery not confirmed. Check Agents before starting a new attempt."),
            ProposalState::Starting => bail!("This proposal is already starting"),
            ProposalState::Dismissed => bail!("This proposal was dismissed"),
            ProposalState::Replaced => bail!("This proposal was replaced; open its replacement card"),
        }
        p.state = ProposalState::Starting;
        p.error = None;
        p.updated_at = now_ms();
        save(app, &p).await?;
        p
    };
    let result: Result<String> = async {
        // An admin session starts only cards whose records are in its own workspace.
        if let Some(a) = agent.as_deref() {
            let own = launchpad_core::scope::of_session(app, a).await.context("this session has no workspace")?;
            launchpad_core::scope::ensure_inside(&own, p.workspace_id.as_deref(), "this card")?;
            check_scope(app, &own, &mut p.body.clone()).await?;
        }
        let mut body = p.body.clone();
        // A card is an agent's text: its launch may open a Local PR, never a pull request on GitHub by itself.
        if let ProposalBody::Launch { spec } | ProposalBody::Schedule { spec, .. } | ProposalBody::Cron { spec, .. } = &mut body
            && let Some(w) = spec.worktree.as_mut()
        {
            w.when_done = w.when_done.without_github();
        }
        match body {
            ProposalBody::Launch { mut spec } => {
                let id = p.result_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if app.sessions.get(&id).is_some() {
                    return Ok(id);
                }
                spec.from = launch_sender(app, &p, manual, agent.as_deref()).await?;
                // Auto mode is automatic work: the start waits for confirmed quota.
                Ok(launchpad_core::runners::launch_anywhere(app, spec, id, !manual).await?.id)
            }
            ProposalBody::Message { session_id, text } => {
                let from = card_sender(app, &p, manual, agent.as_deref()).await?;
                let context = launchpad_core::model::MessageContext { from, ..Default::default() };
                launchpad_core::runners::send_anywhere_with(app, &session_id, &text, vec![], context, Some(format!("proposal-{}", p.id))).await?;
                Ok(session_id)
            }
            ProposalBody::Schedule { mut spec, conditions } => {
                spec.from = launch_sender(app, &p, manual, agent.as_deref()).await?;
                Ok(crate::schedules::create(app, spec, conditions).await?.id)
            }
            ProposalBody::Cron { spec, schedule, conditions } => Ok(crate::crons::create(app, spec, schedule, conditions, Default::default(), None, time_zone).await?.id),
            ProposalBody::Routine { bot_id, title, instructions, trigger, schedule, conditions } => Ok(crate::crons::create_for_bot(app, &bot_id, crate::crons::BotRoutine { title, instructions, trigger, schedule, conditions, time_zone }).await?.id),
            ProposalBody::BotInstructions { bot_id, instructions, before, .. } => {
                let bot = launchpad_core::bots::get(app, &bot_id).await?;
                ensure!(bot.instructions == before, "BOT.md changed since this proposal: ask {} again", bot.settings.name);
                launchpad_core::bots::update(app, &bot_id, launchpad_core::bots::Patch { instructions: Some(instructions), ..Default::default() }).await?;
                Ok(bot_id)
            }
            ProposalBody::BotFiles { bot_id, files, before, .. } => {
                let bot = launchpad_core::bots::get(app, &bot_id).await?;
                let current = current_files(app, &bot_id).await?;
                let mut writes = vec![];
                for (path, text) in files {
                    let text = match text {
                        Some(t) if launchpad_core::bots::routine_path_ok(&path) => {
                            let f = crate::crons::check_file(app, &bot, &t).await.with_context(|| path.clone())?;
                            Some(with_zone(t, &f, time_zone.as_deref()))
                        }
                        t => t,
                    };
                    let now = current.get(path.as_str());
                    // Already as proposed (an earlier attempt wrote it before failing, or the same change was made by
                    // hand): this card overwrites nothing.
                    let applied = now.map(|(_, t)| t) == text.as_ref();
                    ensure!(applied || before.get(&path).cloned().flatten() == now.map(|(sha, _)| sha.clone()), "{path} changed since this proposal: ask {} again", bot.settings.name);
                    writes.push((path, text));
                }
                launchpad_core::bots::write_files(app, &bot_id, writes).await?;
                Ok(bot_id)
            }
            ProposalBody::Channel { .. } | ProposalBody::ChannelJoin { .. } => {
                let ws = p.workspace_id.clone().context("this proposal has no workspace")?;
                let bot = match &p.body { ProposalBody::ChannelJoin { bot_id, .. } => Some(bot_id.clone()), _ => None };
                let by = p.session_id.as_deref().map_or("owner".to_string(), |s| format!("session:{s}"));
                let done = launchpad_core::ext::extensions().channel_card(app, &ws, serde_json::to_value(&p.body)?, bot.as_deref(), true, &by).await?;
                Ok(done["id"].as_str().context("no channel")?.to_string())
            }
            ProposalBody::Status { .. } => bail!("a status card has nothing to start"),
            ProposalBody::Report { .. } => bail!("a report has nothing to start: mark it read or reply"),
            ProposalBody::Ready { action, .. } => bail!("a ready card runs {action} from its own button"),
            ProposalBody::Tool { mut tool, .. } => {
                let tools = launchpad_core::vault::tools(app).await?;
                if tools.iter().any(|t| t.id == tool.id) {
                    return Ok(tool.id);
                }
                // Never lend an existing token to a proposed tool.
                tool.credential = tool.credential.map(|_| tool.id.clone());
                if let Some(c) = &tool.credential {
                    ensure!(!launchpad_core::vault::entries(app).await?.iter().any(|e| &e.id == c), "a token named {c} already exists; add this tool from Settings → Credentials and tools");
                }
                Ok(launchpad_core::vault::put_tool(app, tool).await?.id)
            }
            ProposalBody::Bot { name, instructions, profile, folder } => {
                let workspace_id = p.workspace_id.clone().context("this proposal has no workspace")?;
                Ok(launchpad_core::bots::create(app, launchpad_core::bots::Create { workspace_id, name, instructions, profile, folder, permission_mode: Default::default(), proposals: Default::default(), launchpad: Default::default(), starters: vec![] }).await?.id)
            }
            ProposalBody::Preset { preset_id, label, text, agent } => {
                let create = preset_id.is_none();
                let id = preset_id.or(p.result_id.clone()).context("the preset has no ID")?;
                app.config.update(|config| apply_preset(&mut config.presets, &id, create, &label, &text, agent))?;
                Ok(id)
            }
        }
    }
    .await;
    match result {
        Ok(id) => {
            p.state = ProposalState::Started;
            p.result_id = Some(id);
        }
        Err(e) => {
            p.state = ProposalState::Failed;
            p.error = Some(format!("{e:#}"));
        }
    }
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// A bot's BOT.md and routine files: path → (SHA-256, text).
async fn current_files(app: &App, bot_id: &str) -> Result<std::collections::BTreeMap<String, (String, String)>> {
    let tree = launchpad_core::plain::tree(app, &launchpad_core::bots::store(bot_id)).await?;
    let mut out = std::collections::BTreeMap::new();
    for e in tree.files.into_iter().filter(|e| e.path == launchpad_core::bots::INSTRUCTIONS || launchpad_core::bots::routine_path_ok(&e.path)) {
        let text = String::from_utf8_lossy(&launchpad_core::plain::read_blob(app, &e.sha256).await?).into_owned();
        out.insert(e.path, (e.sha256, text));
    }
    Ok(out)
}

/// A proposed schedule without a time zone runs in the zone of the browser that accepts it, like a routine card.
fn with_zone(text: String, f: &crate::crons::RoutineFile, zone: Option<&str>) -> String {
    match zone.filter(|z| !z.is_empty()) {
        Some(z) if f.time_zone.is_none() && f.trigger == crate::crons::Trigger::Schedule => format!("time_zone = {}\n{text}", toml::Value::String(z.to_string())),
        _ => text,
    }
}

/// Creates the preset `id` (appended, Ask Launchpad off) or updates its label, text and agent flag in place.
/// A create whose preset already exists (an earlier attempt) succeeds without changing it.
fn apply_preset(presets: &mut Vec<launchpad_core::config::Preset>, id: &str, create: bool, label: &str, text: &str, agent: bool) -> Result<()> {
    let (label, text) = (label.trim().to_string(), text.trim().to_string());
    match presets.iter_mut().find(|p| p.id == id) {
        Some(_) if create => {}
        Some(p) => {
            p.label = label;
            p.text = text;
            p.agent = agent;
        }
        None if create => presets.push(launchpad_core::config::Preset { id: id.to_string(), label, text, bots: false, agent }),
        None => bail!("the preset {id} no longer exists"),
    }
    Ok(())
}

pub async fn dismiss(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    ensure!(matches!(p.state, ProposalState::Proposed | ProposalState::Failed | ProposalState::Uncertain), "only waiting proposals can be dismissed");
    p.state = ProposalState::Dismissed;
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// The owner's reply to a report: a message to its author's thread (the bot's current session, else the session
/// that wrote it), naming the card. The report then counts as answered (Started, `result_id` the session). Claimed
/// first like a start: a reply interrupted by a restart becomes Uncertain and is never sent again automatically.
pub async fn reply(app: &App, id: &str, text: &str) -> Result<Proposal> {
    ensure!(!text.trim().is_empty(), "write a reply");
    let mut p = {
        let _lock = ACCEPT.lock().await;
        let mut p = get(app, id).await?;
        ensure!(matches!(p.body, ProposalBody::Report { .. }), "only reports take a reply");
        ensure!(matches!(p.state, ProposalState::Proposed | ProposalState::Failed), "this report is not waiting");
        p.state = ProposalState::Starting;
        p.error = None;
        p.updated_at = now_ms();
        save(app, &p).await?;
        p
    };
    let ProposalBody::Report { github, .. } = &p.body else { unreachable!() };
    let about = github.as_deref().map(|g| format!(" about {g}")).unwrap_or_default();
    let message = format!("The owner replied to your report “{}”{about} (card {}):\n\n{}", p.title, p.id, text.trim());
    let result: Result<String> = async {
        let author = p.session_id.as_deref().and_then(|sid| app.sessions.get(sid)).context("the agent that wrote this report no longer exists")?;
        match &author.launch_spec.bot {
            Some(b) => Ok(launchpad_core::bots::message(app, &b.id, message, vec![], Default::default(), false).await?.id),
            None => {
                launchpad_core::runners::send_anywhere(app, &author.id, &message, vec![], Some(format!("proposal-reply-{}", p.id))).await?;
                Ok(author.id.clone())
            }
        }
    }
    .await;
    match result {
        Ok(sid) => {
            p.state = ProposalState::Started;
            p.result_id = Some(sid);
        }
        Err(e) => {
            p.state = ProposalState::Failed;
            p.error = Some(format!("{e:#}"));
        }
    }
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// A ready card whose action the owner just ran from the card: Started.
pub async fn ran(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    let ProposalBody::Ready { action, .. } = &p.body else { bail!("only ready cards run from their button") };
    ensure!(matches!(p.state, ProposalState::Proposed | ProposalState::Failed), "this card is not waiting");
    p.result_id = Some(action.clone());
    p.state = ProposalState::Started;
    p.error = None;
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// Edit in the composer: the card is replaced by what the owner launches from there, so it cannot start twice.
pub async fn edited(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    ensure!(matches!(p.state, ProposalState::Proposed | ProposalState::Failed), "only waiting, unstarted proposals can be edited");
    p.state = ProposalState::Replaced;
    p.replaced_by = None;
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// After the owner checked Agents: an uncertain launch becomes Started when its reserved session exists,
/// otherwise it can be started again (with a new reserved session ID).
pub async fn recheck(app: &App, id: &str) -> Result<Proposal> {
    let _lock = ACCEPT.lock().await;
    let mut p = get(app, id).await?;
    ensure!(p.state == ProposalState::Uncertain, "only uncertain proposals need a check");
    let created = match &p.body {
        ProposalBody::Launch { .. } => p.result_id.as_deref().is_some_and(|id| app.sessions.get(id).is_some()),
        ProposalBody::Preset { preset_id: None, .. } => p.result_id.as_deref().is_some_and(|id| app.config.get().presets.iter().any(|x| x.id == id)),
        _ => false,
    };
    if created {
        p.state = ProposalState::Started;
    } else {
        p.state = ProposalState::Failed;
        p.error = Some("Not confirmed after a restart; checked by the owner.".into());
        if matches!(p.body, ProposalBody::Launch { .. }) {
            p.result_id = Some(uuid::Uuid::new_v4().to_string());
        }
    }
    p.updated_at = now_ms();
    save(app, &p).await?;
    Ok(p)
}

/// At startup: a claim still Starting was interrupted; its outcome is unknown.
pub async fn recover(app: &App) -> Result<()> {
    for mut p in list(app).await? {
        if p.state == ProposalState::Starting {
            p.state = ProposalState::Uncertain;
            p.error = Some("The daemon restarted while this was starting. Check Agents before starting a new attempt.".into());
            p.updated_at = now_ms();
            save(app, &p).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(id: &str, state: ProposalState) -> Proposal {
        Proposal {
            id: id.into(), workspace_id: None, session_id: None, title: format!("Card {id}"), note: None,
            body: ProposalBody::Status { session_ids: vec!["session".into()] }, state, result_id: None,
            replaced_by: None, error: None, created_at: 1, updated_at: 2,
        }
    }

    fn database() -> rusqlite::Connection {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE launch_proposal (id TEXT PRIMARY KEY, json TEXT NOT NULL)").unwrap();
        db
    }

    fn read(db: &rusqlite::Connection, id: &str) -> Proposal {
        let json: String = db.query_row("SELECT json FROM launch_proposal WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn accepted_schedules_take_the_browser_zone_once() {
        let text = crate::crons::render_file(&crate::crons::RoutineFile { title: "Daily".into(), paused: false, schedule: "0 9 * * *".into(), time_zone: None, instructions: "Check".into(), trigger: Default::default(), conditions: vec![] }).unwrap();
        let f = crate::crons::parse_file(&text).unwrap();
        let zoned = with_zone(text.clone(), &f, Some("Europe/Paris"));
        assert_eq!(crate::crons::parse_file(&zoned).unwrap().time_zone.as_deref(), Some("Europe/Paris"));
        let g = crate::crons::parse_file(&zoned).unwrap();
        assert_eq!(with_zone(zoned.clone(), &g, Some("UTC")), zoned);
        assert_eq!(with_zone(text.clone(), &f, None), text);
    }

    #[test]
    fn github_refs() {
        assert!(valid_github_ref("huggingface/huggingface.js#2716"));
        for bad in ["huggingface.js#2716", "a/b#", "a/b#12x", "a/b/c#1", "/b#1", "a/b 1"] {
            assert!(!valid_github_ref(bad), "{bad}");
        }
    }

    #[test]
    fn preset_proposals_create_or_update_without_deleting() {
        let mut presets = vec![launchpad_core::config::Preset { id: "a".into(), label: "A".into(), text: "old".into(), bots: true, agent: true }];
        apply_preset(&mut presets, "new", true, " New ", " Do it. ", false).unwrap();
        assert_eq!(presets[1], launchpad_core::config::Preset { id: "new".into(), label: "New".into(), text: "Do it.".into(), bots: false, agent: false });
        // A repeated create keeps the existing preset.
        apply_preset(&mut presets, "new", true, "Other", "Other", true).unwrap();
        assert_eq!(presets[1].label, "New");
        apply_preset(&mut presets, "a", false, "A2", "new", false).unwrap();
        assert_eq!(presets[0], launchpad_core::config::Preset { id: "a".into(), label: "A2".into(), text: "new".into(), bots: true, agent: false });
        assert!(apply_preset(&mut presets, "gone", false, "X", "Y", true).is_err());
        assert_eq!(presets.len(), 2);
    }

    #[test]
    fn replaces_waiting_cards_and_keeps_successor_links() {
        for state in [ProposalState::Proposed, ProposalState::Failed] {
            let mut db = database();
            store_created(&mut db, &proposal("old", state), None).unwrap();
            store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap();
            let old = read(&db, "old");
            assert_eq!(old.state, ProposalState::Replaced);
            assert_eq!(old.replaced_by.as_deref(), Some("new"));
            assert_eq!(old.title, "Card old");
            assert_eq!(read(&db, "new").state, ProposalState::Proposed);
            assert!(read(&db, "new").note.is_none());
            store_created(&mut db, &proposal("newer", ProposalState::Proposed), Some("new")).unwrap();
            assert_eq!(read(&db, "new").replaced_by.as_deref(), Some("newer"));
        }
    }

    #[test]
    fn rejects_nonwaiting_missing_and_cross_workspace_targets_without_creating_a_card() {
        for state in [ProposalState::Starting, ProposalState::Started, ProposalState::Dismissed, ProposalState::Uncertain, ProposalState::Replaced] {
            let mut db = database();
            store_created(&mut db, &proposal("old", state.clone()), None).unwrap();
            let error = store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap_err();
            assert!(error.to_string().contains("only waiting, unstarted proposals"));
            assert_eq!(read(&db, "old").state, state);
            assert_eq!(db.query_row("SELECT count(*) FROM launch_proposal", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
        let mut db = database();
        assert!(store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("missing")).unwrap_err().to_string().contains("no such proposal"));
        let mut old = proposal("old", ProposalState::Proposed);
        old.workspace_id = Some("other".into());
        store_created(&mut db, &old, None).unwrap();
        assert!(store_created(&mut db, &proposal("new", ProposalState::Proposed), Some("old")).unwrap_err().to_string().contains("another workspace"));
        assert_eq!(read(&db, "old").state, ProposalState::Proposed);
    }

    #[test]
    fn insertion_failure_rolls_back_replacement_and_legacy_cards_still_load() {
        let mut db = database();
        let old = proposal("old", ProposalState::Proposed);
        store_created(&mut db, &old, None).unwrap();
        assert!(store_created(&mut db, &old, Some("old")).is_err());
        assert_eq!(read(&db, "old").state, ProposalState::Proposed);
        let mut json = serde_json::to_value(old).unwrap();
        json.as_object_mut().unwrap().remove("replaced_by");
        assert!(serde_json::from_value::<Proposal>(json).unwrap().replaced_by.is_none());
    }
}

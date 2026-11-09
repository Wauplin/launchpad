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
    /// A new routine of a bot of the workspace: its runs are messages to the bot's thread.
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
    /// A new BOT.md for a bot of the workspace, often written by the bot itself; the card shows the diff. Accepting
    /// fails when BOT.md changed since.
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
    ensure!(!spec.target.is_browser() && spec.bot.is_none(), "proposals cannot start browser or bot sessions: propose a message to the bot instead");
    spec.allow_paid_usage = false;
    spec.workspace_id = workspace_id.clone();
    Ok(())
}

/// Stores an inert proposal after the same validation as the launch preview.
pub async fn create(app: &App, c: Create) -> Result<Proposal> {
    ensure!(!c.title.trim().is_empty(), "give the proposal a title");
    let mut body = c.body;
    let author = c.session_id.as_deref().and_then(|sid| app.sessions.get(sid));
    let mut workspace_id = match (&c.workspace_id, &author) {
        (Some(w), _) => Some(w.clone()),
        (None, Some(s)) => launchpad_core::kb::workspace_of_session(app, &s.id).await,
        _ => None,
    };
    // An agent proposes changes to bots of its own workspace only.
    let author_ws = match &author {
        Some(s) => launchpad_core::kb::workspace_of_session(app, &s.id).await,
        None => None,
    };
    let same_workspace = |bot: &launchpad_core::bots::Bot| author.is_none() || author_ws.as_deref() == Some(bot.workspace_id.as_str());
    let mut result_id = None;
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
    let now = now_ms();
    let p = Proposal { id: uuid::Uuid::new_v4().to_string(), workspace_id, session_id: c.session_id, title: c.title.trim().to_string(), note: c.note.filter(|n| !n.trim().is_empty()), body, state: ProposalState::Proposed, result_id, replaced_by: None, error: None, created_at: now, updated_at: now };
    // Share the claim lock with accept/dismiss: a replacement cannot race an owner click.
    let _lock = ACCEPT.lock().await;
    let replaces = match (&c.replaces, &p.body) {
        (None, ProposalBody::Report { github: Some(g), .. }) => waiting_report(app, &p, g).await?,
        _ => c.replaces.clone(),
    };
    let stored = p.clone();
    app.state_db.call(move |db| store_created(db, &stored, replaces.as_deref())).await?;
    app.bus.publish("proposals", "changed", serde_json::to_value(&p)?);
    drop(_lock);
    // Only proposals authored by a bot in auto mode are eligible, and only one-time starts and messages. Anything
    // that repeats or lasts (crons, routines, bots, BOT.md, presets, tools) and ready cards always wait for a click
    // (presets are shared settings: a session with admin access changes them directly, with Undo).
    if matches!(p.body, ProposalBody::Launch { .. } | ProposalBody::Message { .. } | ProposalBody::Schedule { .. })
        && auto_mode(app, author.as_ref()).await
    {
        return accept_inner(app, &p.id, false, None).await;
    }
    Ok(p)
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
    accept_inner(app, id, true, None).await
}

/// Accept from a browser: a routine or cron keeps the browser's time zone.
pub async fn accept_in(app: &App, id: &str, time_zone: Option<String>) -> Result<Proposal> {
    accept_inner(app, id, true, time_zone).await
}

/// `manual`: an owner click, which may also retry a failed card.
async fn accept_inner(app: &App, id: &str, manual: bool, time_zone: Option<String>) -> Result<Proposal> {
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
        match p.body.clone() {
            ProposalBody::Launch { spec } => {
                let id = p.result_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if app.sessions.get(&id).is_some() {
                    return Ok(id);
                }
                Ok(launchpad_core::runners::launch_anywhere(app, spec, id, false).await?.id)
            }
            ProposalBody::Message { session_id, text } => {
                launchpad_core::runners::send_anywhere(app, &session_id, &text, vec![], Some(format!("proposal-{}", p.id))).await?;
                Ok(session_id)
            }
            ProposalBody::Schedule { spec, conditions } => Ok(crate::schedules::create(app, spec, conditions).await?.id),
            ProposalBody::Cron { spec, schedule, conditions } => Ok(crate::crons::create(app, spec, schedule, conditions, Default::default(), None, time_zone).await?.id),
            ProposalBody::Routine { bot_id, title, instructions, trigger, schedule, conditions } => Ok(crate::crons::create_for_bot(app, &bot_id, crate::crons::BotRoutine { title, instructions, trigger, schedule, conditions, time_zone }).await?.id),
            ProposalBody::BotInstructions { bot_id, instructions, before, .. } => {
                let bot = launchpad_core::bots::get(app, &bot_id).await?;
                ensure!(bot.instructions == before, "BOT.md changed since this proposal: ask {} again", bot.settings.name);
                launchpad_core::bots::update(app, &bot_id, launchpad_core::bots::Patch { instructions: Some(instructions), ..Default::default() }).await?;
                Ok(bot_id)
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
                Ok(launchpad_core::bots::create(app, launchpad_core::bots::Create { workspace_id, name, instructions, profile, folder, permission_mode: Default::default(), proposals: Default::default(), launchpad: Default::default() }).await?.id)
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
            Some(b) => Ok(launchpad_core::bots::message(app, &b.id, message, vec![], false).await?.id),
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

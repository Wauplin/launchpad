//! Bots: named, persistent agents of a workspace. A bot is a set of files Home stores with versions, like a
//! plain folder (store `bot-<id>`): `BOT.md` (the owner's instructions) and `bot.toml` (its profile, folder,
//! permission mode and proposal mode). Its notes go in the workspace's knowledge base under `bots/<slug>/`.
//! Ask Launchpad is a bot like the others, created once per workspace (`ask_bot`).
//!
//! A bot has one thread: a chain of sessions with `LaunchSpec.bot` set. A message goes to the newest open
//! session; a new session starts when there is none, when the bot's profile or folder changed, or on the
//! owner's click. It references the previous session's trace, never an inlined transcript.

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{BotHandoff, BotLaunch, FileAttachment, LaunchSpec, PermissionMode, Session, SessionStatus, Target, WorktreeOptions};
use crate::plain::{self, Meta, Update};
use crate::{App, util::now_ms};

pub const INSTRUCTIONS: &str = "BOT.md";
pub const SETTINGS: &str = "bot.toml";

pub fn store(id: &str) -> String {
    format!("bot-{id}")
}

pub fn id_of(store: &str) -> Option<&str> {
    store.strip_prefix("bot-")
}

/// What happens to the proposals the bot's sessions write (`launchpad propose`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProposalMode {
    /// Every proposal waits for the owner's click.
    #[default]
    Manual,
    /// One-time launches, messages and schedules start at once; routines, crons, bots, presets, tools, BOT.md
    /// changes and ready cards still wait.
    Auto,
}

/// `bot.toml`.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
pub struct BotSettings {
    pub name: String,
    /// A profile ID from Settings → Profiles. None: the first subscription profile (else the first profile).
    /// A removed profile: the bot asks you to choose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The folder it starts in: a plain folder, or a Git folder (a fresh worktree per session). None: each
    /// session starts in a temporary folder and adds worktrees of the folders it needs with the CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub proposals: ProposalMode,
    /// What it may do with the Launchpad CLI tool: read, propose (the default) or admin.
    #[serde(default)]
    pub launchpad: crate::vault::LaunchpadAccess,
    /// Default tools of its sessions (`tool_defaults`): tool ID or `mcp:<server>` → on or off, over the
    /// workspace's and the folder's defaults. Only tools granted to its workspace apply.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[ts(optional, as = "Option<crate::tool_defaults::ToolChoice>")]
    pub tools: crate::tool_defaults::ToolChoice,
}

/// The profile a bot runs with: its own, or the default when bot.toml names none.
pub fn profile_of<'a>(config: &'a crate::config::Config, s: &BotSettings) -> Option<&'a crate::config::Profile> {
    match &s.profile {
        Some(p) => config.profiles.iter().find(|x| &x.id == p),
        None => config.profiles.iter().find(|p| p.billing == crate::config::Billing::Subscription).or(config.profiles.first()),
    }
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct Bot {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    #[ts(type = "number")]
    pub created_at: i64,
    pub settings: BotSettings,
    /// BOT.md.
    pub instructions: String,
    /// `bot.toml` as written, and why it cannot be used (a hand edit with an error).
    pub settings_text: String,
    pub settings_error: Option<String>,
    /// The folder was created for this bot (deleting the bot keeps it).
    pub own_folder: bool,
    /// Paused: its routines and incoming hooks start nothing; your own messages still go through.
    pub paused: bool,
    #[ts(type = "number")]
    pub version: i64,
}

#[derive(Debug, Deserialize)]
pub struct Create {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub profile: Option<String>,
    /// None: a temporary folder per session.
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub proposals: ProposalMode,
    #[serde(default)]
    pub launchpad: crate::vault::LaunchpadAccess,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Patch {
    pub settings: Option<BotSettings>,
    pub instructions: Option<String>,
    /// A hand edit of `bot.toml`, checked before it is saved.
    pub settings_text: Option<String>,
}

fn ensure_home(app: &App) -> Result<()> {
    ensure!(crate::home_git::here(app), "bots are stored by Home");
    Ok(())
}

/// A slug for `bots/<slug>/`: lowercase ASCII words of the name, unique in the workspace.
pub fn slug_for(name: &str, taken: &[String]) -> String {
    let base: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = base.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    let base: String = base.chars().take(32).collect::<String>().trim_end_matches('-').to_string();
    let base = if base.is_empty() { "bot".to_string() } else { base };
    let mut slug = base.clone();
    let mut n = 2;
    while taken.contains(&slug) {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

pub fn parse_settings(text: &str) -> Result<BotSettings> {
    let mut s: BotSettings = toml::from_str(text).context("bot.toml")?;
    ensure!(!s.name.trim().is_empty(), "bot.toml: name is empty");
    s.folder = s.folder.filter(|f| !f.trim().is_empty());
    Ok(s)
}

fn render_settings(s: &BotSettings) -> Result<String> {
    Ok(format!("# Edited from the bot's Settings tab. folder and profile are IDs; see Folders and Settings → Profiles.\n# Without folder, each session starts in a temporary folder.\n# proposals: \"manual\" (every proposal waits for your click) or \"auto\" (one-time starts apply at once).\n# launchpad: what it may do with the Launchpad CLI: \"read\", \"propose\" or \"admin\" (change Launchpad's settings).\n{}", toml::to_string(s)?))
}

type Row = (String, String, String, i64, Option<String>, bool);

async fn rows(app: &App) -> Result<Vec<Row>> {
    app.state_db.call(|c| {
        let mut st = c.prepare("SELECT id, workspace_id, slug, created_at, own_folder, paused FROM bot ORDER BY created_at")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get::<_, i64>(5)? != 0)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

pub(crate) async fn text_of(app: &App, files: &[plain::Entry], path: &str) -> String {
    match files.iter().find(|f| f.path == path) {
        Some(f) => plain::read_blob(app, &f.sha256).await.ok().map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default(),
        None => String::new(),
    }
}

async fn load(app: &App, (id, workspace_id, slug, created_at, own, paused): Row) -> Result<Bot> {
    let tree = plain::tree(app, &store(&id)).await?;
    let instructions = text_of(app, &tree.files, INSTRUCTIONS).await;
    let settings_text = text_of(app, &tree.files, SETTINGS).await;
    let (settings, settings_error) = match parse_settings(&settings_text) {
        Ok(s) => (s, None),
        Err(e) => (BotSettings { name: slug.clone(), ..Default::default() }, Some(format!("{e:#}"))),
    };
    let own_folder = own.is_some() && own == settings.folder;
    Ok(Bot { id, workspace_id, slug, created_at, settings, instructions, settings_text, settings_error, own_folder, paused, version: tree.version })
}

pub async fn list(app: &App) -> Result<Vec<Bot>> {
    ensure_home(app)?;
    let mut out = vec![];
    for row in rows(app).await? {
        out.push(load(app, row).await?);
    }
    Ok(out)
}

pub async fn get(app: &App, id: &str) -> Result<Bot> {
    ensure_home(app)?;
    let row = rows(app).await?.into_iter().find(|r| r.0 == id).context("no such bot")?;
    load(app, row).await
}

pub(crate) async fn exists(app: &App, id: &str) -> Result<bool> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT 1 FROM bot WHERE id = ?1", [id], |_| Ok(true)).optional()?.unwrap_or(false))).await
}

/// Writes files as the owner, with their history.
async fn write(app: &App, id: &str, files: Vec<(&str, String)>) -> Result<()> {
    write_store(app, &store(id), files).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": id}));
    Ok(())
}

/// Writes the files of a bot's store as the owner, each change with its history. Unchanged files are skipped.
pub(crate) async fn write_store(app: &App, store_id: &str, files: Vec<(&str, String)>) -> Result<()> {
    let tree = plain::tree(app, store_id).await?;
    let mut updates = vec![];
    for (path, body) in files {
        let old = text_of(app, &tree.files, path).await;
        if old == body && tree.files.iter().any(|f| f.path == path) {
            continue;
        }
        let data = body.into_bytes();
        let sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&data));
        plain::write_blob(app, &sha, &data).await?;
        let (added, removed) = crate::kb::counts(&old, &String::from_utf8_lossy(&data));
        updates.push((Update { path: path.into(), sha256: Some(sha), size: data.len() as u64, executable: false }, Some(Meta { author: crate::kb::Author::owner(None), added: Some(added), removed: Some(removed) })));
    }
    if !updates.is_empty() {
        plain::record(app, store_id, None, updates).await?;
    }
    Ok(())
}

async fn check_settings(app: &App, workspace_id: &str, s: &BotSettings) -> Result<()> {
    ensure!(!s.name.trim().is_empty(), "give the bot a name");
    if let Some(f) = &s.folder {
        let folder = crate::projects::get(app, f).await?.context("the bot's folder no longer exists: choose another one")?;
        ensure!(folder.workspace_id.as_deref() == Some(workspace_id), "{} belongs to another workspace", folder.name);
        ensure!(!folder.is_temp, "{} is a temporary folder: choose a registered folder, or none", folder.name);
    }
    if let Some(p) = &s.profile {
        ensure!(app.config.get().profiles.iter().any(|x| &x.id == p), "no profile {p}: choose one in the bot's Settings");
    }
    crate::tool_defaults::check(&s.tools)?;
    Ok(())
}

pub async fn create(app: &App, c: Create) -> Result<Bot> {
    ensure_home(app)?;
    let name = c.name.trim().to_string();
    ensure!(!name.is_empty(), "give the bot a name");
    crate::workspace::get(app, &c.workspace_id).await.context("choose a workspace")?;
    let taken: Vec<String> = rows(app).await?.into_iter().filter(|r| r.1 == c.workspace_id).map(|r| r.2).collect();
    let slug = slug_for(&name, &taken);
    let id = uuid::Uuid::new_v4().to_string();
    let profile = c.profile.filter(|p| !p.is_empty());
    let folder = c.folder.filter(|f| !f.is_empty());
    let own: Option<String> = None;
    let settings = BotSettings { name: name.clone(), profile, model: None, effort: None, folder, permission_mode: c.permission_mode, proposals: c.proposals, launchpad: c.launchpad, tools: Default::default() };
    check_settings(app, &c.workspace_id, &settings).await?;
    let instructions = if c.instructions.trim().is_empty() { format!("# {name}\n") } else { format!("{}\n", c.instructions.trim_end()) };
    let (id2, ws, slug2, at) = (id.clone(), c.workspace_id.clone(), slug.clone(), now_ms());
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO bot (id, workspace_id, slug, created_at, own_folder) VALUES (?1, ?2, ?3, ?4, ?5)", params![id2, ws, slug2, at, own])?)).await?;
    write(app, &id, vec![(INSTRUCTIONS, instructions), (SETTINGS, render_settings(&settings)?)]).await?;
    get(app, &id).await
}

pub async fn update(app: &App, id: &str, p: Patch) -> Result<Bot> {
    let bot = get(app, id).await?;
    let mut files = vec![];
    if let Some(text) = p.settings_text {
        let s = parse_settings(&text)?;
        check_settings(app, &bot.workspace_id, &s).await?;
        files.push((SETTINGS, text));
    } else if let Some(s) = p.settings {
        check_settings(app, &bot.workspace_id, &s).await?;
        files.push((SETTINGS, render_settings(&s)?));
    }
    if let Some(i) = p.instructions {
        files.push((INSTRUCTIONS, i));
    }
    write(app, id, files).await?;
    get(app, id).await
}

/// Pause or Resume: a paused bot's routines and incoming hooks start nothing. Stop ends the current run.
pub async fn set_paused(app: &App, id: &str, paused: bool) -> Result<Bot> {
    get(app, id).await?;
    let id2 = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot SET paused = ?2 WHERE id = ?1", params![id2, paused])?)).await?;
    crate::ext::extensions().bot_paused(app, id, paused).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": id}));
    get(app, id).await
}

/// Deletes the bot, its files and its routines. Its notes in the knowledge base, its folder and its sessions stay.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    ensure!(thread(app, id).iter().all(|s| !s.status.is_running()), "the bot is working: stop it first");
    crate::ext::extensions().bot_deleted(app, id).await?;
    let (id2, st) = (id.to_string(), store(id));
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM bot WHERE id = ?1", [&id2])?;
        tx.execute("DELETE FROM queued_message WHERE target = 'bot:' || ?1", [&id2])?;
        tx.execute("DELETE FROM plain_file WHERE store = ?1", [&st])?;
        tx.execute("DELETE FROM plain_state WHERE store = ?1", [&st])?;
        tx.commit()?;
        Ok(())
    }).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": id}));
    Ok(())
}

/// Changes of the bot's files in the last 30 days, newest first.
pub async fn history(app: &App, id: &str) -> Result<Vec<plain::Change>> {
    get(app, id).await?;
    plain::history(app, &store(id)).await
}

pub(crate) async fn blob_string(app: &App, sha: Option<&str>) -> String {
    match sha {
        Some(h) => plain::read_blob(app, h).await.ok().map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default(),
        None => String::new(),
    }
}

/// The unified diff of one change.
pub async fn diff(app: &App, id: &str, change: i64) -> Result<String> {
    let c = history(app, id).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    Ok(crate::kb::unified(&blob_string(app, c.sha256.as_deref()).await, &blob_string(app, c.new_sha256.as_deref()).await))
}

/// Restore previous: puts back a file's content from before a change, as a new change by you.
pub async fn restore(app: &App, id: &str, change: i64) -> Result<Bot> {
    let c = history(app, id).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    ensure!(c.sha256.is_some(), "this change created the file: there is no earlier version");
    let text = blob_string(app, c.sha256.as_deref()).await;
    match c.path.as_str() {
        SETTINGS => update(app, id, Patch { settings_text: Some(text), ..Default::default() }).await,
        INSTRUCTIONS => update(app, id, Patch { instructions: Some(text), ..Default::default() }).await,
        _ => bail!("unknown bot file {}", c.path),
    }
}

/// The bot's sessions, oldest first.
pub fn thread(app: &App, id: &str) -> Vec<Session> {
    let mut list: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.launch_spec.bot.as_ref().is_some_and(|b| b.id == id)).collect();
    list.sort_by_key(|s| s.started_at);
    list
}

/// Why the next message starts a new session, or None to continue the current one (its status, profile and
/// folder, None for a temporary folder).
pub fn rotation(bot: &BotSettings, current: Option<(SessionStatus, Option<&str>, Option<&str>)>) -> Option<&'static str> {
    let Some((status, profile, folder)) = current else { return Some("first") };
    if status == SessionStatus::Closed {
        Some("archived")
    } else if profile != bot.profile.as_deref() {
        Some("profile")
    } else if folder != bot.folder.as_deref() {
        Some("folder")
    } else {
        None
    }
}

/// Sends the owner's message to the bot's thread: to its open session, or to a new one.
pub async fn message(app: &App, id: &str, text: String, attachments: Vec<FileAttachment>, new_session: bool) -> Result<Session> {
    message_with(app, id, text, attachments, new_session, false).await
}

/// A message to the bot's thread. `automatic` (routines, incoming hooks): a paused bot refuses it, and a new
/// session needs confirmed quota. Hooks and Telegram go through `hooks::deliver`, which waits for the end of a
/// running turn.
pub async fn message_with(app: &App, id: &str, text: String, attachments: Vec<FileAttachment>, new_session: bool, automatic: bool) -> Result<Session> {
    ensure!(!text.trim().is_empty() || !attachments.is_empty(), "the message is empty");
    let bot = get(app, id).await?;
    ensure!(!automatic || !bot.paused, "{} is paused", bot.settings.name);
    if let Some(e) = &bot.settings_error {
        bail!("bot.toml cannot be used ({e}); fix it in the bot's Settings tab");
    }
    let s = &bot.settings;
    let config = app.config.get();
    let profile = profile_of(&config, s).context("choose a profile for this bot in its Settings tab")?;
    let sessions = thread(app, id);
    let current = sessions.iter().rev().find(|x| x.status != SessionStatus::Closed);
    let resolved = BotSettings { profile: Some(profile.id.clone()), ..s.clone() };
    let folder_of = |c: &Session| if matches!(c.launch_spec.target, Target::Temp) { None } else { c.project_id.as_deref().map(str::to_string) };
    let current_folder = current.and_then(folder_of);
    match (new_session, rotation(&resolved, current.map(|c| (c.status, c.launch_spec.profile.as_deref(), current_folder.as_deref())))) {
        (false, None) => {
            let c = current.expect("a current session");
            crate::runners::send_anywhere(app, &c.id, &text, attachments, None).await?;
            return app.sessions.get(&c.id).context("the bot's session disappeared");
        }
        _ => {
            if let Some(c) = current {
                ensure!(!c.status.is_running(), "{} is working: wait for its turn to end, or Stop it, before a new session", s.name);
                crate::runners::session_action(app, &c.id, "close").await?;
            }
        }
    }
    let folder = match &s.folder {
        Some(f) => Some(crate::projects::get(app, f).await?.context("the bot's folder no longer exists: choose another one in its Settings tab")?),
        None => None,
    };
    let previous = sessions.last().map(|p| BotHandoff { session_id: p.id.clone(), title: p.title.clone(), through_seq: p.last_seq });
    let spec = LaunchSpec {
        text,
        attachments,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        model: s.model.clone().or_else(|| profile.model.clone()),
        effort: s.effort.clone().or_else(|| profile.effort.clone()),
        permission_mode: s.permission_mode,
        target: folder.as_ref().map_or(Target::Temp, |f| Target::Project { project_id: f.id.clone() }),
        // Git folders: a fresh worktree per session, never the owner's checkout.
        worktree: folder.as_ref().filter(|f| !f.plain).map(|f| WorktreeOptions { when_done: f.when_done, ..Default::default() }),
        workspace_id: Some(bot.workspace_id.clone()),
        bot: Some(BotLaunch { id: bot.id.clone(), name: s.name.clone(), slug: bot.slug.clone(), instructions: bot.instructions.clone(), previous }),
        ..Default::default()
    };
    crate::runners::launch_anywhere(app, spec, uuid::Uuid::new_v4().to_string(), automatic).await
}

/// The bot part of a session's first prompt.
pub fn instruction(bot: &BotLaunch, cli: &str) -> String {
    let mut out = format!(
        "You are {name}, a Launchpad bot: a persistent agent with one thread that continues across sessions. The owner's instructions for you (BOT.md):\n\n<bot-instructions>\n{instructions}\n</bot-instructions>\n\n\
Memory: the workspace knowledge base ($LAUNCHPAD_KB) is your only memory. Keep your own notes in `bots/{slug}/` (for example `{cli} kb write bots/{slug}/notes.md`), short and current: the owner's preferences and the few facts a later session needs, and what the owner asks you to remember. They are not a journal: never record your runs, what you did, or what went well or failed (the thread and its traces keep that); most turns write nothing. Shared facts go in the usual folders of the knowledge base.\n\
Work: prepare changes for the owner's review by default (branches, Local PRs, suggestions); act directly on GitHub or elsewhere only when the owner explicitly asks. Unless your Launchpad CLI access is admin, you cannot edit BOT.md or bot.toml. When your instructions should change (the owner asked, or you learned how they like the work done), propose the whole new BOT.md and tell the owner it waits for their click: `{cli} propose bot-instructions --title \"<what changes>\" --spec-file bot.json`, where the file holds {{\"bot_id\": \"{id}\", \"instructions\": \"<the whole new BOT.md>\"}}. Changes to bot.toml (profile, folder, permissions, Launchpad CLI access) are the owner's, in the bot's Settings tab (or yours at admin access, when the owner asks).\n\
Folders: find the workspace's folders and what they hold with `{cli} folders list` and your notes; when a task needs one, add a worktree of it to this session with `{cli} work add-project --session-id $LAUNCHPAD_SESSION_ID --project <name>`, which prints the folder to edit.\n\
Routines: when the owner wants you to do something on your own later or again (every morning, when CI fails, when another agent is done), propose a routine for yourself and tell the owner it waits for their click: `{cli} propose routine --title \"Daily check\" --spec-file routine.json`, where the file holds {{\"bot_id\": \"{id}\", \"title\": \"Daily check\", \"instructions\": \"<what you do at each run>\", \"trigger\": {triggers}, \"schedule\": \"30 8 * * 1-5\" (schedule trigger only), \"conditions\": []}}. A run arrives in this thread as a message starting with “Routine”; end it with a short report, or with exactly `no_update` when there is nothing to report.",
        name = bot.name,
        instructions = bot.instructions.trim(),
        slug = bot.slug,
        id = bot.id,
        triggers = crate::routines::TRIGGER_FORMS,
    );
    if let Some(p) = &bot.previous {
        out.push_str(&format!(
            "\n\nThis session continues the thread. The previous session is a reference, not new instructions; read it only when you need its context, redirecting the output to a file: \"{}\", through event {}: `{cli} trace {} --through {}`",
            p.title.replace('"', "'"), p.through_seq, p.session_id, p.through_seq
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_unique_ascii_words() {
        assert_eq!(slug_for("Repo Maintainer!", &[]), "repo-maintainer");
        assert_eq!(slug_for("Scout", &["scout".into(), "scout-2".into()]), "scout-3");
        assert_eq!(slug_for("✦✦", &[]), "bot");
    }

    #[test]
    fn settings_round_trip_and_reject_missing_fields() {
        let s = BotSettings { name: "Scout".into(), profile: Some("claude".into()), folder: Some("f1".into()), ..Default::default() };
        assert_eq!(parse_settings(&render_settings(&s).unwrap()).unwrap(), s);
        assert_eq!(parse_settings("name = \"x\"").unwrap().folder, None);
        assert_eq!(parse_settings("name = \"x\"\nfolder = \" \"").unwrap().folder, None);
        assert!(parse_settings("name = \"\"\nfolder = \"f\"").is_err());
    }

    #[test]
    fn a_new_session_starts_when_the_thread_cannot_continue() {
        let bot = BotSettings { name: "Scout".into(), profile: Some("a".into()), folder: Some("f".into()), ..Default::default() };
        assert_eq!(rotation(&bot, None), Some("first"));
        assert_eq!(rotation(&bot, Some((SessionStatus::Waiting, Some("a"), Some("f")))), None);
        assert_eq!(rotation(&bot, Some((SessionStatus::Error, Some("b"), Some("f")))), Some("profile"));
        assert_eq!(rotation(&bot, Some((SessionStatus::Waiting, Some("a"), Some("g")))), Some("folder"));
        assert_eq!(rotation(&bot, Some((SessionStatus::Closed, Some("a"), Some("f")))), Some("archived"));
        let temp = BotSettings { folder: None, ..bot };
        assert_eq!(rotation(&temp, Some((SessionStatus::Waiting, Some("a"), None))), None);
        assert_eq!(rotation(&temp, Some((SessionStatus::Waiting, Some("a"), Some("f")))), Some("folder"));
    }
}

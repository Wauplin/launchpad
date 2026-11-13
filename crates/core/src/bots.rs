//! Bots: named, persistent agents of a workspace. A bot is a set of files Home stores with versions, like a
//! plain folder (store `bot-<id>`): `BOT.md` (the owner's instructions), `bot.toml` (its profile, folder,
//! permission mode and proposal mode), `routines/<name>.toml` (the definitions of its routines; their runs and
//! watermarks are records of the automation crate, synced after each write through `ext::Extensions`) and
//! `memory/` (its own notes, `bot_memory`). Runners mirror the store read-only into its sessions (`$LAUNCHPAD_BOT`).
//! Ask Launchpad is a bot like the others, created once per workspace (`ask_bot`).
//!
//! A bot has one thread: a chain of sessions with `LaunchSpec.bot` set. A message goes to the newest open
//! session; a new session starts when there is none, when the bot's profile or folder changed, or on the
//! owner's click. It references the previous session's trace, never an inlined transcript.

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{BotHandoff, BotLaunch, FileAttachment, ContextProject, LaunchSpec, MessageContext, PermissionMode, Session, SessionStatus, Target, WorktreeOptions};
use crate::plain::{self, Meta, Update};
use crate::{App, util::now_ms};

pub const INSTRUCTIONS: &str = "BOT.md";
pub const SETTINGS: &str = "bot.toml";
/// The folder of its routine files, `routines/<name>.toml`.
pub const ROUTINES: &str = "routines/";

/// A routine file's path: `routines/<name>.toml`, the name in lowercase ASCII letters, digits, `-` and `_`.
pub fn routine_path_ok(path: &str) -> bool {
    path.strip_prefix(ROUTINES).and_then(|p| p.strip_suffix(".toml")).is_some_and(|n| {
        (1..=64).contains(&n.len()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') && !n.starts_with(['-', '_'])
    })
}

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
    /// It may mark the owner's Inbox threads done itself, here and on GitHub (`launchpad inbox done`), without a
    /// click. Off by default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub inbox_done: bool,
    /// Default tools of its sessions (`tool_defaults`): tool ID or `mcp:<server>` → on or off, over the
    /// workspace's and the folder's defaults. Only tools granted to its workspace apply.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[ts(optional, as = "Option<crate::tool_defaults::ToolChoice>")]
    pub tools: crate::tool_defaults::ToolChoice,
    /// Ready-made messages shown over the bot's message box: a message sends at once; one ending in "…" fills the
    /// box so you finish it. None: no chips.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<String>>")]
    pub starters: Vec<String>,
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
    /// Its version (`version`): a session that started from or last received another one gets the new BOT.md
    /// with the next message.
    pub instructions_version: String,
    /// `bot.toml` as written, and why it cannot be used (a hand edit with an error).
    pub settings_text: String,
    pub settings_error: Option<String>,
    /// The folder was created for this bot (deleting the bot keeps it).
    pub own_folder: bool,
    /// Paused: its routines and incoming hooks start nothing; your own messages still go through.
    pub paused: bool,
    #[ts(type = "number")]
    pub version: i64,
    /// Its notes: the files under `memory/`, with their store paths.
    pub memory: Vec<plain::Entry>,
    /// How many lines of its notes a new session gets in its instructions (`bot_memory::compose`).
    pub memory_loaded: crate::bot_memory::LoadedLines,
    /// Its routine files (`routines/<name>.toml`) as written, by path: what an Undo of a file change puts back.
    #[ts(type = "Record<string, string>")]
    pub routines: std::collections::BTreeMap<String, String>,
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
    #[serde(default)]
    pub starters: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Patch {
    pub settings: Option<BotSettings>,
    pub instructions: Option<String>,
    /// A hand edit of `bot.toml`, checked before it is saved.
    pub settings_text: Option<String>,
    /// Routine files (`routines/<name>.toml`) to write, or to delete (null). The API checks their content first.
    pub files: Option<std::collections::BTreeMap<String, Option<String>>>,
    /// Only these fields of `bot.toml` (`set_fields`), the others kept: `{"inbox_done": true}`. Null puts a field
    /// back to its default.
    pub set: Option<serde_json::Map<String, serde_json::Value>>,
    /// An agent's change (set by the API from the session CLI token, never from the body): checked with
    /// `agent_refusal`.
    #[serde(skip)]
    pub by_agent: bool,
}

/// Pure: why an agent may not save `new` over the current settings (None: it may). Only the owner raises a bot's
/// Launchpad CLI access, lets it mark Inbox threads done or loosens its permission mode; an agent may lower or
/// tighten them. `current` None: the current bot.toml has an error, read as the strictest settings (read access,
/// no Inbox done, Ask for approval).
pub fn agent_refusal(current: Option<&BotSettings>, new: &BotSettings) -> Option<&'static str> {
    let (access, inbox, mode) = current.map_or((crate::vault::LaunchpadAccess::Read, false, PermissionMode::Approvals), |c| (c.launchpad, c.inbox_done, c.permission_mode));
    if new.launchpad > access {
        Some("only the owner raises a bot's Launchpad CLI access (`launchpad` in bot.toml, on the bot's page: About → Access); an agent may only lower it")
    } else if new.inbox_done && !inbox {
        Some("only the owner lets a bot mark Inbox threads done (`inbox_done = true` in bot.toml, on the bot's page: About → Access); an agent may only turn it off")
    } else if new.permission_mode.strictness() < mode.strictness() {
        Some("only the owner loosens a bot's permission mode (YOLO → Auto → Ask for approval, on the bot's page: About → Access); an agent may only make it stricter")
    } else {
        None
    }
}

/// The fields of `bot.toml`, as `Patch::set` names them.
pub const SETTING_FIELDS: &[&str] = &["name", "profile", "model", "effort", "folder", "permission_mode", "proposals", "launchpad", "inbox_done", "tools", "starters"];

/// Every field of the settings with its value, defaults included (bot.toml leaves defaults out): the values a
/// `set` puts back.
pub fn setting_values(s: &BotSettings) -> serde_json::Map<String, serde_json::Value> {
    let mut v = match serde_json::to_value(s) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => Default::default(),
    };
    for k in SETTING_FIELDS {
        v.entry(k.to_string()).or_insert_with(|| match *k {
            "inbox_done" => serde_json::Value::Bool(false),
            "tools" => serde_json::json!({}),
            "starters" => serde_json::json!([]),
            _ => serde_json::Value::Null,
        });
    }
    v
}

/// The settings with only the fields of `set` changed (null: the field's default).
pub fn set_fields(current: &BotSettings, set: &serde_json::Map<String, serde_json::Value>) -> Result<BotSettings> {
    ensure!(!set.is_empty(), "name at least one bot.toml field to set");
    let mut v = serde_json::to_value(current)?;
    let obj = v.as_object_mut().context("bot settings")?;
    for (k, val) in set {
        ensure!(SETTING_FIELDS.contains(&k.as_str()), "bot.toml has no field {k} (fields: {})", SETTING_FIELDS.join(", "));
        if val.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), val.clone());
        }
    }
    let mut s: BotSettings = serde_json::from_value(v).context("bot.toml")?;
    s.folder = s.folder.filter(|f| !f.trim().is_empty());
    Ok(s)
}

fn ensure_home(app: &App) -> Result<()> {
    ensure!(crate::home_git::here(app), "bots are stored by Home");
    Ok(())
}

/// A slug (its Telegram command): lowercase ASCII words of the name, unique in the workspace.
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
    Ok(format!("# Edited from the bot's page (About → Access, or ⋯ → Edit as file). folder and profile are IDs; see Folders and Settings → Profiles.\n# Without folder, each session starts in a temporary folder.\n# proposals: \"manual\" (every proposal waits for your click) or \"auto\" (one-time starts apply at once).\n# launchpad: what it may do with the Launchpad CLI: \"read\", \"propose\" or \"admin\" (change Launchpad's settings).\n# inbox_done = true: it may mark your Inbox threads done (or unsubscribe from them) itself, here and on GitHub.\n# starters: ready-made messages over the message box; one ending in \"…\" fills the box so you finish it.\n{}", toml::to_string(s)?))
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
    let memory = tree.files.iter().filter(|f| f.path.starts_with(crate::bot_memory::DIR)).cloned().collect();
    let loaded = crate::bot_memory::compose(&crate::bot_memory::store_notes(app, &tree.files).await, "launchpad");
    let memory_loaded = crate::bot_memory::LoadedLines { loaded: loaded.loaded as u32, total: loaded.total as u32 };
    let instructions_version = version(&instructions);
    let mut routines = std::collections::BTreeMap::new();
    for f in tree.files.iter().filter(|f| routine_path_ok(&f.path)) {
        routines.insert(f.path.clone(), text_of(app, &tree.files, &f.path).await);
    }
    Ok(Bot { id, workspace_id, slug, created_at, settings, instructions, instructions_version, settings_text, settings_error, own_folder, paused, version: tree.version, memory, memory_loaded, routines })
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

/// The workspace of a bot, None when there is no such bot.
pub async fn workspace_of(app: &App, id: &str) -> Result<Option<String>> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT workspace_id FROM bot WHERE id = ?1", [id], |r| r.get(0)).optional()?)).await
}

/// A bot's name and access, read from bot.toml only (no BOT.md, memory or routines): for quick lookups such as the
/// extension's default bot.
#[derive(Clone, Debug)]
pub struct Brief {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    pub name: String,
    pub created_at: i64,
    pub launchpad: crate::vault::LaunchpadAccess,
}

/// The bots of a workspace, oldest first.
pub async fn briefs(app: &App, workspace_id: &str) -> Result<Vec<Brief>> {
    ensure_home(app)?;
    let mut out = vec![];
    for (id, ws, slug, created_at, _, _) in rows(app).await?.into_iter().filter(|r| r.1 == workspace_id) {
        let tree = plain::tree(app, &store(&id)).await?;
        let settings = parse_settings(&text_of(app, &tree.files, SETTINGS).await).unwrap_or_else(|_| BotSettings { name: slug.clone(), ..Default::default() });
        out.push(Brief { id, workspace_id: ws, slug, name: settings.name, created_at, launchpad: settings.launchpad });
    }
    Ok(out)
}

/// The bot the Ask field talks to by default (`askTarget` of the web app): the oldest bot with admin access, else
/// the oldest bot.
pub fn ask_target(bots: &[Brief]) -> Option<&Brief> {
    bots.iter().find(|b| b.launchpad == crate::vault::LaunchpadAccess::Admin).or_else(|| bots.first())
}

pub(crate) async fn exists(app: &App, id: &str) -> Result<bool> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT 1 FROM bot WHERE id = ?1", [id], |_| Ok(true)).optional()?.unwrap_or(false))).await
}

/// Writes (or deletes, None) files as the owner, with their history, then lets the routines follow their files.
async fn write(app: &App, id: &str, files: Vec<(String, Option<String>)>) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    write_store(app, &store(id), files).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": id}));
    crate::ext::extensions().bot_files_changed(app, id).await
}

/// Writes BOT.md and routine files (`routines/<name>.toml`, None deletes one) as the owner, all in one change;
/// the routines follow. bot.toml goes through `update` (checked), `memory/` through `bot_memory`.
pub async fn write_files(app: &App, id: &str, files: Vec<(String, Option<String>)>) -> Result<()> {
    get(app, id).await?;
    for (path, body) in &files {
        ensure!(path == INSTRUCTIONS || routine_path_ok(path), "a bot's files here are BOT.md and routines/<name>.toml (lowercase letters, digits, - and _), not {path}");
        ensure!(path != INSTRUCTIONS || body.is_some(), "BOT.md cannot be deleted");
    }
    write(app, id, files).await
}

/// The routine files of a bot (`routines/<name>.toml`), with their text, by path.
pub async fn routine_files(app: &App, id: &str) -> Result<Vec<(String, String)>> {
    let tree = plain::tree(app, &store(id)).await?;
    let mut out = vec![];
    for f in tree.files.iter().filter(|f| routine_path_ok(&f.path)) {
        out.push((f.path.clone(), text_of(app, &tree.files, &f.path).await));
    }
    Ok(out)
}

/// Writes (or deletes, None) the files of a bot's store as the owner, each change with its history. Unchanged
/// files are skipped. Nothing else follows: callers publish and sync.
pub async fn write_store(app: &App, store_id: &str, files: Vec<(String, Option<String>)>) -> Result<()> {
    let tree = plain::tree(app, store_id).await?;
    let mut updates = vec![];
    for (path, body) in files {
        let exists = tree.files.iter().any(|f| f.path == path);
        let old = text_of(app, &tree.files, &path).await;
        let Some(body) = body else {
            if exists {
                let (added, removed) = crate::kb::counts(&old, "");
                updates.push((Update { path, sha256: None, size: 0, executable: false }, Some(Meta { author: crate::kb::Author::owner(None), added: Some(added), removed: Some(removed) })));
            }
            continue;
        };
        if old == body && exists {
            continue;
        }
        let data = body.into_bytes();
        let sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&data));
        plain::write_blob(app, &sha, &data).await?;
        let (added, removed) = crate::kb::counts(&old, &String::from_utf8_lossy(&data));
        updates.push((Update { path, sha256: Some(sha), size: data.len() as u64, executable: false }, Some(Meta { author: crate::kb::Author::owner(None), added: Some(added), removed: Some(removed) })));
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
        ensure!(app.config.get().profiles.iter().any(|x| &x.id == p), "no profile {p}: choose one on the bot's page (About → Access)");
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
    let settings = BotSettings { name: name.clone(), profile, model: None, effort: None, folder, permission_mode: c.permission_mode, proposals: c.proposals, launchpad: c.launchpad, inbox_done: false, tools: Default::default(), starters: c.starters };
    check_settings(app, &c.workspace_id, &settings).await?;
    let instructions = if c.instructions.trim().is_empty() { format!("# {name}\n") } else { format!("{}\n", c.instructions.trim_end()) };
    let (id2, ws, slug2, at) = (id.clone(), c.workspace_id.clone(), slug.clone(), now_ms());
    // A new bot has nothing to move out of the knowledge base: a `bots/<slug>/` folder left there by a deleted bot
    // of the same name stays where it is (`bot_memory::move_from_kb` runs once, for the bots made before it).
    let moved = crate::bot_memory::moved_key(&id);
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute("INSERT INTO bot (id, workspace_id, slug, created_at, own_folder) VALUES (?1, ?2, ?3, ?4, ?5)", params![id2, ws, slug2, at, own])?;
        tx.execute("INSERT OR IGNORE INTO seeded (key, at) VALUES (?1, ?2)", params![moved, at])?;
        tx.commit()?;
        Ok(())
    }).await?;
    write(app, &id, vec![(INSTRUCTIONS.into(), Some(instructions)), (SETTINGS.into(), Some(render_settings(&settings)?))]).await?;
    get(app, &id).await
}

pub async fn update(app: &App, id: &str, p: Patch) -> Result<Bot> {
    let bot = get(app, id).await?;
    let mut files = vec![];
    let mut name = None;
    let agent = |s: &BotSettings| match p.by_agent.then(|| agent_refusal(bot.settings_error.is_none().then_some(&bot.settings), s)).flatten() {
        Some(why) => bail!("{why}"),
        None => Ok(()),
    };
    if let Some(text) = &p.settings_text {
        let s = parse_settings(text)?;
        agent(&s)?;
        check_settings(app, &bot.workspace_id, &s).await?;
        name = Some(s.name.clone());
        files.push((SETTINGS.to_string(), Some(text.clone())));
    } else if let Some(s) = &p.settings {
        agent(s)?;
        check_settings(app, &bot.workspace_id, s).await?;
        name = Some(s.name.clone());
        files.push((SETTINGS.to_string(), Some(render_settings(s)?)));
    } else if let Some(set) = &p.set {
        ensure!(bot.settings_error.is_none(), "{}'s bot.toml has an error: fix it on its page first (⋯ → Edit as file)", bot.settings.name);
        let s = set_fields(&bot.settings, set)?;
        agent(&s)?;
        check_settings(app, &bot.workspace_id, &s).await?;
        name = Some(s.name.clone());
        files.push((SETTINGS.to_string(), Some(render_settings(&s)?)));
    }
    if let Some(i) = p.instructions {
        files.push((INSTRUCTIONS.to_string(), Some(i)));
    }
    for (path, body) in p.files.unwrap_or_default() {
        ensure!(routine_path_ok(&path), "only routine files (routines/<name>.toml: lowercase letters, digits, - and _) go in files, not {path}");
        files.push((path, body));
    }
    write(app, id, files).await?;
    if let Some(name) = name.filter(|n| n.trim() != bot.settings.name.trim()) {
        rename(app, &bot, &name).await?;
    }
    get(app, id).await
}

/// A new name gives the bot a new slug (unique in its workspace): its Telegram command follows. Its files, memory
/// included, stay in its store.
async fn rename(app: &App, bot: &Bot, name: &str) -> Result<()> {
    let taken: Vec<String> = rows(app).await?.into_iter().filter(|r| r.1 == bot.workspace_id && r.0 != bot.id).map(|r| r.2).collect();
    let slug = slug_for(name.trim(), &taken);
    if slug == bot.slug {
        return Ok(());
    }
    let (id, s) = (bot.id.clone(), slug.clone());
    app.state_db.call(move |c| Ok(c.execute("UPDATE bot SET slug = ?2 WHERE id = ?1", params![id, s])?)).await?;
    app.bus.publish("bots", "changed", serde_json::json!({"id": bot.id}));
    Ok(())
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

/// Deletes the bot, its files (memory included) and its routines. Its folder and its sessions stay.
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

/// Restore previous: puts back a file's content from before a change, as a new change by you. A memory note or a
/// routine file comes back even when it was removed since; one that the change created is removed. An agent's
/// Restore of bot.toml follows `agent_refusal` like its other changes.
pub async fn restore(app: &App, id: &str, change: i64, by_agent: bool) -> Result<Bot> {
    let c = history(app, id).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    if c.path.starts_with(crate::bot_memory::DIR) {
        let current = plain::tree(app, &store(id)).await?.files.into_iter().find(|e| e.path == c.path).map(|e| e.sha256);
        let op = match c.sha256 {
            Some(h) => {
                let size = tokio::fs::metadata(plain::blob_path(app, &h)?).await.map(|m| m.len()).unwrap_or(0);
                crate::kb::Op::Write { path: c.path, sha256: h, size, base: current }
            }
            None => crate::kb::Op::Remove { path: c.path, base: current },
        };
        crate::bot_memory::apply(app, id, crate::kb::Author::owner(None), vec![op]).await?;
        return get(app, id).await;
    }
    if routine_path_ok(&c.path) {
        let text = match c.sha256.as_deref() {
            Some(h) => Some(blob_string(app, Some(h)).await),
            None => None,
        };
        write(app, id, vec![(c.path, text)]).await?;
        return get(app, id).await;
    }
    ensure!(c.sha256.is_some(), "this change created the file: there is no earlier version");
    let text = blob_string(app, c.sha256.as_deref()).await;
    match c.path.as_str() {
        SETTINGS => update(app, id, Patch { settings_text: Some(text), by_agent, ..Default::default() }).await,
        INSTRUCTIONS => update(app, id, Patch { instructions: Some(text), ..Default::default() }).await,
        _ => bail!("unknown bot file {}", c.path),
    }
}

/// The content of one of the bot's files (`memory/…`, `routines/…`, BOT.md, bot.toml).
pub async fn read_file(app: &App, id: &str, path: &str) -> Result<Vec<u8>> {
    get(app, id).await?;
    let e = plain::tree(app, &store(id)).await?.files.into_iter().find(|e| e.path == path).with_context(|| format!("there is no {path}"))?;
    plain::read_blob(app, &e.sha256).await
}

/// The bot's sessions, oldest first.
pub fn thread(app: &App, id: &str) -> Vec<Session> {
    let mut list: Vec<Session> = app.sessions.list().into_iter().filter(|s| s.launch_spec.bot.as_ref().is_some_and(|b| b.id == id)).collect();
    list.sort_by_key(|s| s.started_at);
    list
}

/// The current session of a thread, as `rotation` compares it with the bot's settings: its status, and the
/// profile, folder (None for a temporary folder), model, effort and permission mode bot.toml gave it at start.
#[derive(Clone, Copy, Debug)]
pub struct Current<'a> {
    pub status: SessionStatus,
    pub profile: Option<&'a str>,
    pub folder: Option<&'a str>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub permission_mode: PermissionMode,
}

impl<'a> Current<'a> {
    pub fn of(s: &'a Session) -> Current<'a> {
        Current {
            status: s.status,
            profile: s.launch_spec.profile.as_deref(),
            folder: if matches!(s.launch_spec.target, Target::Temp) { None } else { s.project_id.as_deref() },
            model: s.launch_spec.model.as_deref(),
            effort: s.launch_spec.effort.as_deref(),
            // The mode bot.toml gave it, not one the owner set on the session in Agents.
            permission_mode: s.launch_spec.bot.as_ref().and_then(|b| b.permission_mode).unwrap_or(s.launch_spec.permission_mode),
        }
    }
}

/// Why the next message starts a new session, or None to continue the current one. `bot` holds the values a new
/// session would get (its profile, and the model and effort of bot.toml or else of the profile).
pub fn rotation(bot: &BotSettings, current: Option<Current>) -> Option<&'static str> {
    let Some(c) = current else { return Some("first") };
    if c.status == SessionStatus::Closed {
        Some("archived")
    } else if c.profile != bot.profile.as_deref() {
        Some("profile")
    } else if c.folder != bot.folder.as_deref() {
        Some("folder")
    } else if c.model != bot.model.as_deref() {
        Some("model")
    } else if c.effort != bot.effort.as_deref() {
        Some("effort")
    } else if c.permission_mode != bot.permission_mode {
        Some("permissions")
    } else {
        None
    }
}

/// A short version of BOT.md: sessions record the one they run (`BotLaunch.version`).
pub fn version(instructions: &str) -> String {
    crate::kb::sha(instructions.as_bytes())[..16].to_string()
}

/// The block Home adds after the typed text of the first message a session gets after BOT.md changed.
pub fn changed_block(instructions: &str) -> String {
    format!("{CHANGED_MARK} The owner edited BOT.md since this session started or last heard of it. From now on, follow this version instead of the earlier one:\n\n<bot-instructions>\n{}\n</bot-instructions>", instructions.trim())
}

/// How the block starts (the chat shows it as one line).
pub const CHANGED_MARK: &str = "Your instructions changed.";

/// Home: a message to a bot session whose BOT.md changed since it started or last received it gets the new BOT.md
/// after its typed text (`changed_block`). Returns the body to send, and the version and text to record in the
/// session once it is delivered (`record_instructions`).
pub async fn with_new_instructions(app: &App, session_id: &str, body: Option<String>) -> (Option<String>, Option<(String, String)>) {
    let unchanged = (body.clone(), None);
    let Some(launch) = app.sessions.get(session_id).and_then(|s| s.launch_spec.bot) else { return unchanged };
    let Ok(bot) = get(app, &launch.id).await else { return unchanged };
    let now = bot.instructions_version.clone();
    // Sessions from before versions were recorded: the BOT.md they started from.
    let theirs = if launch.version.is_empty() { version(&launch.instructions) } else { launch.version.clone() };
    if theirs == now {
        return (body, launch.version.is_empty().then_some((now, bot.instructions)));
    }
    match body.as_deref().and_then(|b| add_block(b, &bot.instructions)) {
        Some(body) => (Some(body), Some((now, bot.instructions))),
        None => unchanged,
    }
}

/// A session input body (`{"text": …}`) with `changed_block` after its typed text; None for another body.
fn add_block(body: &str, instructions: &str) -> Option<String> {
    let mut v = serde_json::from_str::<serde_json::Value>(body).ok().filter(serde_json::Value::is_object)?;
    let typed = v["text"].as_str().unwrap_or_default().trim_end().to_string();
    let block = changed_block(instructions);
    v["text"] = serde_json::Value::String(if typed.is_empty() { block } else { format!("{typed}\n\n{block}") });
    Some(v.to_string())
}

/// Home: records the BOT.md a session received (`with_new_instructions`).
pub async fn record_instructions(app: &App, session_id: &str, (version, instructions): (String, String)) {
    let done = app.sessions.update(app, session_id, |s| {
        if let Some(b) = &mut s.launch_spec.bot {
            b.version = version;
            b.instructions = instructions;
        }
    }).await;
    if let Err(e) = done {
        tracing::warn!("instructions version of session {session_id}: {e:#}");
    }
}

/// Why an automatic message to the bot's thread waits for its quota now (`quota::hold_reason` of its open session
/// when it runs on the bot's profile, else of a new session with that profile), or None when it may go.
pub fn quota_hold(app: &App, bot: &Bot) -> Option<String> {
    let config = app.config.get();
    let profile = profile_of(&config, &bot.settings)?;
    if let Some(c) = thread(app, &bot.id).into_iter().rev().find(|x| x.status != SessionStatus::Closed)
        && c.launch_spec.profile.as_deref() == Some(profile.id.as_str())
    {
        return crate::quota::hold_reason(app, c.harness, &c.launch_spec);
    }
    let spec = LaunchSpec { harness: Some(profile.harness), profile: Some(profile.id.clone()), ..Default::default() };
    crate::quota::hold_reason(app, profile.harness, &spec)
}

/// Sends the owner's message to the bot's thread: to its open session, or to a new one.
pub async fn message(app: &App, id: &str, text: String, attachments: Vec<FileAttachment>, context: MessageContext, new_session: bool) -> Result<Session> {
    message_with(app, id, text, attachments, context, new_session.then(String::new), false, None).await
}

/// One lock per bot: two messages to the same bot never both decide to start a new session of its thread.
static MESSAGES: std::sync::LazyLock<crate::locks::Locks> = std::sync::LazyLock::new(Default::default);

/// A message to the bot's thread. `automatic` (routines, incoming hooks, other agents): a paused bot refuses it, and
/// it needs confirmed quota (`quota_hold`: refused for the open session, held by the start of a new one). Hooks and
/// Telegram go through `hooks::deliver`, which waits for the end of a running turn. `context` (folders, GitHub links, agent traces, as in an agent's message box) goes with the
/// message to the current session, or into the launch of a new one. `context.from`, set only by Home's own code
/// (`senders`), names a sender other than the owner: the message arrives under its header.
/// `new_session`: the owner asked for a new session, with their reason (may be empty), shown in the chat's divider.
/// `key`: the request's Idempotency-Key (the window's retry protection), passed on as the message's or the new
/// session's id. The bot's lock (`MESSAGES`) covers the choice of session and a new session's launch, so concurrent
/// messages never start two sessions; a message to the open session is sent after the lock is released.
#[allow(clippy::too_many_arguments)]
pub async fn message_with(app: &App, id: &str, text: String, attachments: Vec<FileAttachment>, context: MessageContext, new_session: Option<String>, automatic: bool, key: Option<String>) -> Result<Session> {
    ensure!(!text.trim().is_empty() || !attachments.is_empty() || !context.is_empty(), "the message is empty");
    if let Some(f) = &context.from {
        crate::senders::guard_from(f)?;
    }
    let lock = MESSAGES.get(id);
    let held = lock.lock().await;
    let bot = get(app, id).await?;
    ensure!(!automatic || !bot.paused, "{} is paused", bot.settings.name);
    if let Some(e) = &bot.settings_error {
        bail!("bot.toml cannot be used ({e}); fix it on the bot's page (⋯ → Edit as file)");
    }
    let s = &bot.settings;
    let config = app.config.get();
    let profile = profile_of(&config, s).context("choose a profile for this bot on its page (About → Access)")?;
    let sessions = thread(app, id);
    let current = sessions.iter().rev().find(|x| x.status != SessionStatus::Closed);
    let resolved = BotSettings { profile: Some(profile.id.clone()), model: s.model.clone().or_else(|| profile.model.clone()), effort: s.effort.clone().or_else(|| profile.effort.clone()), ..s.clone() };
    let reason = match current {
        None if !sessions.is_empty() => Some("archived"),
        c => rotation(&resolved, c.map(Current::of)),
    };
    match (new_session.is_some(), reason) {
        (false, None) => {
            let c = current.expect("a current session");
            // Automatic work needs confirmed quota (`quota::hold_reason`); a new session waits for it in its start.
            if automatic && let Some(why) = crate::quota::hold_reason(app, c.harness, &c.launch_spec) {
                bail!("{} waits for its quota: {why}", s.name);
            }
            let c = c.id.clone();
            drop(held);
            let refs = context.github_refs.clone();
            crate::runners::send_anywhere_with(app, &c, &text, attachments, context, key).await?;
            crate::ref_use::record(app, &c, Some(id), &refs).await;
            return app.sessions.get(&c).context("the bot's session disappeared");
        }
        _ => {
            if let Some(c) = current {
                ensure!(!c.status.is_running(), "{} is working: wait for its turn to end, or Stop it, before a new session", s.name);
                crate::runners::session_action(app, &c.id, "close").await?;
            }
        }
    }
    let from = context.from.clone();
    let mut spec = next_spec(app, &bot, &sessions, text, attachments, context).await?;
    // The first message of the new session, from another bot or agent: under its header, like `runners::deliver`.
    if let Some(f) = from {
        spec.text = crate::senders::wrap(&f, &spec.text);
        spec.from = Some(f);
    }
    if let Some(b) = &mut spec.bot {
        b.reason = Some(if new_session.is_some() && reason.is_none() { "new" } else { reason.unwrap_or("first") }.to_string());
        b.note = note_of(new_session);
    }
    let session = key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let started = crate::runners::launch_anywhere(app, spec, session, automatic).await;
    drop(held);
    started
}

/// The owner's reason for a new session: one line, at most 120 characters, None when empty.
fn note_of(note: Option<String>) -> Option<String> {
    let line = note?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then(|| line.chars().take(120).collect())
}

/// The launch of a new session of the thread: the bot's settings, BOT.md and its version, and the previous
/// session to reference. `thread` is the bot's sessions, oldest first.
pub async fn next_spec(app: &App, bot: &Bot, thread: &[Session], text: String, attachments: Vec<FileAttachment>, context: MessageContext) -> Result<LaunchSpec> {
    let s = &bot.settings;
    let config = app.config.get();
    let profile = profile_of(&config, s).context("choose a profile for this bot on its page (About → Access)")?;
    let folder = match &s.folder {
        Some(f) => Some(crate::projects::get(app, f).await?.context("the bot's folder no longer exists: choose another one on its page (About → Access)")?),
        None => None,
    };
    let previous = thread.last().map(|p| BotHandoff { session_id: p.id.clone(), title: p.title.clone(), through_seq: p.last_seq });
    Ok(LaunchSpec {
        text,
        attachments,
        harness: Some(profile.harness),
        profile: Some(profile.id.clone()),
        model: s.model.clone().or_else(|| profile.model.clone()),
        effort: s.effort.clone().or_else(|| profile.effort.clone()),
        permission_mode: s.permission_mode,
        target: folder.as_ref().map_or(Target::Temp, |f| Target::Project { project_id: f.id.clone() }),
        // Git folders: a fresh worktree per session, never the owner's checkout.
        worktree: folder.as_ref().filter(|f| !f.plain).map(|f| WorktreeOptions { when_done: f.when_done.without_github(), ..Default::default() }),
        workspace_id: Some(bot.workspace_id.clone()),
        bot: Some(BotLaunch {
            id: bot.id.clone(),
            name: s.name.clone(),
            slug: bot.slug.clone(),
            instructions: bot.instructions.clone(),
            version: bot.instructions_version.clone(),
            reason: None,
            note: None,
            permission_mode: Some(s.permission_mode),
            previous,
            inbox_done: s.inbox_done && s.launchpad != crate::vault::LaunchpadAccess::Read,
        }),
        mentions: context.mentions,
        skills: context.skills,
        context_projects: context.context_projects.into_iter().map(|c| ContextProject { editable: false, ..c }).collect(),
        github_refs: context.github_refs,
        agent_traces: context.agent_traces,
        topics: context.topics,
        selection: context.selection,
        ..Default::default()
    })
}

/// The bot part of a session's first prompt.
pub fn instruction(bot: &BotLaunch, cli: &str) -> String {
    let mut out = format!(
        "You are {name}, a Launchpad bot: a persistent agent with one thread that continues across sessions. The owner's instructions for you (BOT.md):\n\n<bot-instructions>\n{instructions}\n</bot-instructions>\n\n\
Your folder: `$LAUNCHPAD_BOT` is a read-only copy of your files: BOT.md, bot.toml, `routines/<name>.toml` (your routines) and `memory/`, your own notes. Your instructions hold these notes as they were at start (\"Your memory\", up to {loaded} lines; when they say it was cut, read the rest before you answer anything it may cover). Write them with `{cli} bot memory write <file>` (content on stdin, or `--file <file>`; paths are relative to `memory/`, for example `notes.md`), `{cli} bot memory append <file> <text>` and `{cli} bot memory rm <file>`; `{cli} bot memory list` and `{cli} bot memory read <file>` read them. Keep them short and current: the owner's preferences and the few facts a later session needs, and what the owner asks you to remember. They are not a journal: never record your runs, what you did, or what went well or failed (the thread and its traces keep that); most turns write nothing. Facts other agents need (shared facts about people, projects, how-tos) go in the workspace knowledge base ($LAUNCHPAD_KB) instead.\n\
Work: prepare changes for the owner's review by default (branches, Local PRs, suggestions); act directly on GitHub or elsewhere only when the owner explicitly asks. Unless your Launchpad CLI access is admin, you cannot edit BOT.md, bot.toml or your routines. When your instructions or routines should change (the owner asked, or you learned how they like the work done), propose the whole new files as one card and tell the owner it waits for their click: `{cli} propose bot-files --title \"<what changes>\" --spec-file files.json`, where the file holds {{\"bot_id\": \"{id}\", \"files\": {{\"BOT.md\": \"<the whole new BOT.md>\", \"routines/<name>.toml\": \"<the whole new routine file>\"}}}} (only the files that change; null deletes a routine; start from your copies in `$LAUNCHPAD_BOT`). The owner sees one diff per file. Changes to bot.toml (profile, folder, permissions, Launchpad CLI access) are the owner's, on the bot's page (About → Access) (or yours at admin access, when the owner asks).\n\
Folders: find the workspace's folders and what they hold with `{cli} folders list` and your notes; when a task needs one, add a worktree of it to this session with `{cli} work add-project --session-id $LAUNCHPAD_SESSION_ID --project <name>`, which prints the folder to edit.\n\
Routines: when the owner wants you to do something on your own later or again (every morning, when CI fails, when another agent is done), propose a routine for yourself and tell the owner it waits for their click: `{cli} propose routine --title \"Daily check\" --spec-file routine.json`, where the file holds {{\"bot_id\": \"{id}\", \"title\": \"Daily check\", \"instructions\": \"<what you do at each run>\", \"trigger\": {triggers}, \"schedule\": \"30 8 * * 1-5\" (schedule trigger only), \"conditions\": []}}; it becomes a card with your new file `routines/<name>.toml`. To change or delete routines, propose their files with `bot-files` as above (the same fields in TOML; your copies in `$LAUNCHPAD_BOT/routines/` show the format). A run arrives in this thread as a message starting with “Routine”; end it with a short report, or with exactly `no_update` when there is nothing to report.",
        name = bot.name,
        instructions = bot.instructions.trim(),
        id = bot.id,
        triggers = crate::routines::TRIGGER_FORMS,
        loaded = crate::bot_memory::LOADED_LINES,
    );
    if bot.inbox_done {
        out.push_str(&format!("\nInbox: the owner lets you mark their Inbox threads done yourself, in Launchpad and on GitHub (Done, not just read): `{cli} inbox done <thread-id>...`, with the ids from `{cli} inbox list`; `{cli} inbox unsubscribe <thread-id>...` also mutes them on GitHub, so later activity does not notify the owner. Only threads your instructions name; never mark them done with `gh` (Launchpad's Inbox would keep them)."));
    }
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
    fn agents_only_lower_a_bots_access_and_tighten_its_mode() {
        use crate::vault::LaunchpadAccess::*;
        let cur = BotSettings { name: "Scout".into(), launchpad: Propose, permission_mode: PermissionMode::Auto, ..Default::default() };
        let with = |f: fn(&mut BotSettings)| { let mut s = cur.clone(); f(&mut s); s };
        assert!(agent_refusal(Some(&cur), &cur).is_none());
        assert!(agent_refusal(Some(&cur), &with(|s| { s.launchpad = Read; s.permission_mode = PermissionMode::Approvals; s.name = "Other".into() })).is_none());
        assert!(agent_refusal(Some(&cur), &with(|s| s.launchpad = Admin)).is_some());
        assert!(agent_refusal(Some(&cur), &with(|s| s.inbox_done = true)).is_some());
        assert!(agent_refusal(Some(&cur), &with(|s| s.permission_mode = PermissionMode::Yolo)).is_some());
        assert!(agent_refusal(Some(&BotSettings { inbox_done: true, ..cur.clone() }), &cur).is_none());
        // A broken bot.toml reads as the strictest settings.
        assert!(agent_refusal(None, &with(|s| { s.launchpad = Read; s.permission_mode = PermissionMode::Approvals })).is_none());
        assert!(agent_refusal(None, &cur).is_some());
    }

    #[test]
    fn set_changes_only_the_named_fields() {
        let cur = BotSettings { name: "Scout".into(), profile: Some("p1".into()), model: Some("m".into()), proposals: ProposalMode::Auto, ..Default::default() };
        let set = |v: serde_json::Value| v.as_object().unwrap().clone();
        let s = set_fields(&cur, &set(serde_json::json!({"inbox_done": true}))).unwrap();
        assert_eq!(s, BotSettings { inbox_done: true, ..cur.clone() });
        // Null puts back the default; the undo values of `setting_values` restore the settings.
        let s2 = set_fields(&s, &set(serde_json::json!({"model": null, "launchpad": "admin"}))).unwrap();
        assert_eq!((s2.model.as_deref(), s2.launchpad, s2.profile.as_deref()), (None, crate::vault::LaunchpadAccess::Admin, Some("p1")));
        let old = setting_values(&cur);
        let back: serde_json::Map<_, _> = ["model", "launchpad", "inbox_done"].iter().map(|k| (k.to_string(), old[*k].clone())).collect();
        assert_eq!(set_fields(&BotSettings { inbox_done: true, ..s2 }, &back).unwrap(), cur);
        assert!(set_fields(&cur, &set(serde_json::json!({"inbox-done": true}))).is_err());
        assert!(set_fields(&cur, &set(serde_json::json!({"inbox_done": "yes"}))).is_err());
        assert!(set_fields(&cur, &serde_json::Map::new()).is_err());
        // Every field of bot.toml is named.
        assert_eq!(setting_values(&cur).keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>(), SETTING_FIELDS.iter().copied().collect());
    }

    #[test]
    fn slugs_are_unique_ascii_words() {
        assert_eq!(slug_for("Repo Maintainer!", &[]), "repo-maintainer");
        assert_eq!(slug_for("Scout", &["scout".into(), "scout-2".into()]), "scout-3");
        assert_eq!(slug_for("✦✦", &[]), "bot");
    }

    #[test]
    fn routine_paths_are_simple_toml_names() {
        for ok in ["routines/daily-check.toml", "routines/a.toml", "routines/ci_2.toml"] {
            assert!(routine_path_ok(ok), "{ok}");
        }
        for bad in ["routines/.toml", "routines/Daily.toml", "routines/a/b.toml", "routines/../x.toml", "routines/-a.toml", "routines/a.md", "bot.toml", "memory/a.toml", "routines/a.toml/"] {
            assert!(!routine_path_ok(bad), "{bad}");
        }
    }

    #[test]
    fn settings_round_trip_and_reject_missing_fields() {
        let s = BotSettings { name: "Scout".into(), profile: Some("claude".into()), folder: Some("f1".into()), ..Default::default() };
        assert_eq!(parse_settings(&render_settings(&s).unwrap()).unwrap(), s);
        let s = BotSettings { tools: [("gh".to_string(), false)].into_iter().collect(), starters: vec!["What needs me?".into(), "Status of …".into()], ..s };
        assert_eq!(parse_settings(&render_settings(&s).unwrap()).unwrap(), s);
        assert_eq!(parse_settings("name = \"x\"").unwrap().folder, None);
        assert_eq!(parse_settings("name = \"x\"\nfolder = \" \"").unwrap().folder, None);
        assert!(parse_settings("name = \"\"\nfolder = \"f\"").is_err());
    }

    #[test]
    fn the_reason_for_a_new_session_is_one_short_line() {
        assert_eq!(note_of(None), None);
        assert_eq!(note_of(Some("  \n ".into())), None);
        assert_eq!(note_of(Some(" Fresh\n context  ".into())).as_deref(), Some("Fresh context"));
        assert_eq!(note_of(Some("é".repeat(200))).map(|n| n.chars().count()), Some(120));
    }

    #[test]
    fn a_new_session_starts_when_the_thread_cannot_continue() {
        let bot = BotSettings { profile: Some("a".into()), folder: Some("f".into()), model: Some("m".into()), ..Default::default() };
        let cur = |status, profile, folder| Current { status, profile, folder, model: Some("m"), effort: None, permission_mode: PermissionMode::default() };
        assert_eq!(rotation(&bot, None), Some("first"));
        assert_eq!(rotation(&bot, Some(cur(SessionStatus::Waiting, Some("a"), Some("f")))), None);
        assert_eq!(rotation(&bot, Some(cur(SessionStatus::Error, Some("b"), Some("f")))), Some("profile"));
        assert_eq!(rotation(&bot, Some(cur(SessionStatus::Waiting, Some("a"), Some("g")))), Some("folder"));
        assert_eq!(rotation(&bot, Some(cur(SessionStatus::Closed, Some("a"), Some("f")))), Some("archived"));
        assert_eq!(rotation(&bot, Some(Current { model: Some("other"), ..cur(SessionStatus::Waiting, Some("a"), Some("f")) })), Some("model"));
        assert_eq!(rotation(&bot, Some(Current { effort: Some("high"), ..cur(SessionStatus::Waiting, Some("a"), Some("f")) })), Some("effort"));
        assert_eq!(rotation(&bot, Some(Current { permission_mode: PermissionMode::Approvals, ..cur(SessionStatus::Waiting, Some("a"), Some("f")) })), Some("permissions"));
        let temp = BotSettings { profile: Some("a".into()), model: Some("m".into()), ..Default::default() };
        assert_eq!(rotation(&temp, Some(cur(SessionStatus::Waiting, Some("a"), None))), None);
        assert_eq!(rotation(&temp, Some(cur(SessionStatus::Waiting, Some("a"), Some("f")))), Some("folder"));
    }

    #[test]
    fn changed_instructions_follow_the_typed_text() {
        let block = changed_block("  # Scout\n\nWatch CI.\n");
        assert!(block.starts_with(CHANGED_MARK));
        assert!(block.ends_with("<bot-instructions>\n# Scout\n\nWatch CI.\n</bot-instructions>"));
        assert_eq!(version("a"), version("a"));
        assert_ne!(version("a"), version("b"));
        assert_eq!(version("a").len(), 16);
        let body = add_block(r#"{"text":"Check CI\n","files":[],"context":{"github_refs":[]}}"#, "New job").unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["text"].as_str().unwrap(), format!("Check CI\n\n{}", changed_block("New job")));
        assert_eq!(v["context"], serde_json::json!({"github_refs": []}));
        let only: serde_json::Value = serde_json::from_str(&add_block(r#"{"files":[{"name":"a"}]}"#, "J").unwrap()).unwrap();
        assert_eq!(only["text"].as_str().unwrap(), changed_block("J"));
        assert!(add_block("not json", "J").is_none());
    }
}

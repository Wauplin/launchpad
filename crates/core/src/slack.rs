//! Slack profiles: named Slack tokens on Home that agents use through `launchpad slack --profile <handle> …`.
//!
//! A profile is a name, a short description of what it can do and in whose name, a handle (`user-read`), a token
//! (a user token `xoxp-…`, acting as the person who installed the app, or a bot token `xoxb-…`, acting as the app's
//! bot user) and two switches: whether a new session gets it without a choice (`default_on`, off by default) and
//! whether messages sent with it end with Launchpad's attribution line (`footer`, on by default). Launchpad has no
//! read or write mode of its own: what a profile may do is its token's scopes.
//!
//! Agents never hold a Slack token and runners never install slackline: the CLI sends the slackline arguments to
//! Home (`POST /api/slack/run`, the session's own call), Home checks that the session holds the profile
//! (`of_session`, from its own record) and runs its slackline with the token (`launchpad_api` slack). Which
//! sessions hold a profile follows the default tools (`tool_defaults`) with the key `slack:<handle>`: off unless the
//! profile is on by default or the owner switches it on (a launch's Tools, a bot's `[tools]`, a folder or workspace
//! default); an agent never switches one on (`tool_defaults::agent_switches_on`).

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::App;
use crate::util::now_ms;

/// The prefix of a profile's key in the default tools and a launch's `tools_on`/`tools_off`.
pub const KEY_PREFIX: &str = "slack:";

pub fn key(handle: &str) -> String {
    format!("{KEY_PREFIX}{handle}")
}

/// The vault entry of a profile's token (hidden from Settings → Credentials and tools, never lent to a tool).
pub const VAULT_PREFIX: &str = "slack-profile-";

fn vault_id(handle: &str) -> String {
    format!("{VAULT_PREFIX}{handle}")
}

/// Whose name a token acts in.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "SlackTokenKind")]
pub enum TokenKind {
    /// `xoxp-`: a user token, acting as the person who installed the app, with what they can see.
    User,
    /// `xoxb-`: a bot token, acting as the app's bot user, in the channels it was added to.
    Bot,
}

/// A profile as every API shows it: never the token.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "SlackProfile")]
pub struct Profile {
    /// Lowercase letters, digits and dashes: `--profile <handle>`.
    pub handle: String,
    pub name: String,
    /// What it can do and in whose name, for the owner and for the agents that hold it.
    pub description: String,
    pub token_kind: TokenKind,
    /// The last characters of the token.
    pub token_detail: String,
    /// Every session gets it unless a default or a launch switches it off.
    pub default_on: bool,
    /// Messages sent with it end with Launchpad's attribution line.
    pub footer: bool,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// What the owner saves. `token` may be left out to keep the profile's token.
#[derive(Clone, Debug, Deserialize, TS)]
#[ts(rename = "SlackProfileSave")]
pub struct Save {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub default_on: bool,
    #[serde(default = "yes")]
    pub footer: bool,
}

fn yes() -> bool {
    true
}

/// Pure: the handle a name suggests (`Read as me` → `read-as-me`).
pub fn handle_of(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(40).collect::<String>().trim_end_matches('-').to_string()
}

/// Pure: a handle of lowercase letters and digits, dashes between them, at most 40 characters.
pub fn valid_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 40 && !h.starts_with('-') && !h.ends_with('-') && !h.contains("--") && h.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Pure: whose name a token acts in, from its prefix. App-level tokens (`xapp-`) only open Socket Mode connections
/// and call a few app APIs: they cannot read or post messages.
pub fn token_kind(token: &str) -> Result<TokenKind> {
    let t = token.trim();
    ensure!(!t.is_empty() && !t.contains(char::is_whitespace), "a Slack token is one word, such as xoxp-…");
    if t.starts_with("xoxp-") {
        Ok(TokenKind::User)
    } else if t.starts_with("xoxb-") {
        Ok(TokenKind::Bot)
    } else if t.starts_with("xapp-") || t.starts_with("xoxe.xapp-") {
        bail!("an app-level token (xapp-…) only opens Socket Mode connections: it cannot read or send messages. Paste the app's User OAuth Token (xoxp-…, acting as you) or its Bot User OAuth Token (xoxb-…, acting as the app's bot), both under OAuth & Permissions")
    } else {
        bail!("paste a User OAuth Token (xoxp-…) or a Bot User OAuth Token (xoxb-…), from the app's OAuth & Permissions page")
    }
}

// ---------------------------------------------------------------- storage (Home)

fn ensure_home(app: &App) -> Result<()> {
    ensure!(crate::vault::here(app), "Slack profiles are kept on Home: change them there");
    Ok(())
}

/// Every profile, by handle.
pub async fn list(app: &App) -> Result<Vec<Profile>> {
    let mut all: Vec<Profile> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM slack_profile")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    all.sort_by(|a: &Profile, b| a.handle.cmp(&b.handle));
    Ok(all)
}

pub async fn get(app: &App, handle: &str) -> Result<Option<Profile>> {
    Ok(list(app).await?.into_iter().find(|p| p.handle == handle))
}

/// Creates or changes a profile. A new one needs a token; the token is kept in the vault, never returned.
pub async fn save(app: &App, handle: &str, s: Save) -> Result<Profile> {
    ensure_home(app)?;
    ensure!(valid_handle(handle), "a handle uses lowercase letters, digits and single dashes (at most 40 characters), like user-read");
    let name = s.name.trim().to_string();
    ensure!(!name.is_empty() && name.len() <= 80, "a profile needs a name (at most 80 characters)");
    let description = s.description.trim().to_string();
    ensure!(description.len() <= 600 && !description.contains(['\n', '\r']), "the description is one line of at most 600 characters");
    let old = get(app, handle).await?;
    let token = s.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let (token_kind, token_detail) = match (token, &old) {
        (Some(t), _) => {
            let kind = token_kind(t)?;
            let entry = crate::vault::put(app, &vault_id(handle), crate::vault::Kind::Token, &format!("Slack profile {name}"), t).await?;
            (kind, entry.detail)
        }
        (None, Some(o)) => (o.token_kind, o.token_detail.clone()),
        (None, None) => bail!("a new profile needs its token"),
    };
    let p = Profile { handle: handle.into(), name, description, token_kind, token_detail, default_on: s.default_on, footer: s.footer, updated_at: now_ms() };
    let (h, json) = (p.handle.clone(), serde_json::to_string(&p)?);
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO slack_profile (handle, json) VALUES (?1, ?2) ON CONFLICT (handle) DO UPDATE SET json = ?2", [h, json])?)).await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(p)
}

/// Deletes a profile and its token. Sessions lose it at once (Home checks every call).
pub async fn delete(app: &App, handle: &str) -> Result<()> {
    ensure_home(app)?;
    let h = handle.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM slack_profile WHERE handle = ?1", [h])?)).await?;
    crate::vault::delete(app, &vault_id(handle)).await
}

/// Home: a profile's token, read in Home's process only.
pub async fn token(app: &App, handle: &str) -> Result<String> {
    ensure_home(app)?;
    match crate::vault::secret(app, &vault_id(handle)).await? {
        Some((crate::vault::Kind::Token, s)) if !s.is_empty() => Ok(s),
        _ => bail!("the Slack profile {handle} has no token on Home (Settings → Slack)"),
    }
}

/// The result of Test: who the token acts as.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "SlackTest")]
pub struct Test {
    pub ok: bool,
    pub message: String,
}

/// Home: Test of a profile: Slack's `auth.test` with its token. The token never leaves Home.
pub async fn test(app: &App, handle: &str) -> Result<Test> {
    let token = token(app, handle).await?;
    let res = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?.get("https://slack.com/api/auth.test").bearer_auth(&token).header("user-agent", "launchpad").send().await;
    Ok(match res {
        Ok(r) => {
            let status = r.status();
            let body: Value = r.json().await.unwrap_or(Value::Null);
            match body["ok"].as_bool() {
                Some(true) => {
                    let who = body["user"].as_str().unwrap_or("?");
                    let team = body["team"].as_str().unwrap_or("?");
                    let bot = if body["bot_id"].is_string() { " (a bot user)" } else { "" };
                    Test { ok: true, message: format!("The token works: it acts as {who}{bot} in {team}.") }
                }
                _ => Test { ok: false, message: format!("Slack refused the token: {}.", body["error"].as_str().map(str::to_string).unwrap_or_else(|| status.to_string())) },
            }
        }
        Err(e) => Test { ok: false, message: format!("Could not reach slack.com: {e}") },
    })
}

// ---------------------------------------------------------------- creating a Slack app

/// Read scopes of slackline's commands. `search:read` exists only for user tokens.
const READ_USER: &[&str] = &["channels:history", "channels:read", "files:read", "groups:history", "groups:read", "im:history", "im:read", "mpim:history", "mpim:read", "pins:read", "reactions:read", "search:read", "users:read", "users:read.email"];
const WRITE_USER: &[&str] = &["chat:write", "files:write", "im:write", "pins:write", "reactions:write", "users.profile:write"];
const READ_BOT: &[&str] = &["channels:history", "channels:read", "files:read", "groups:history", "groups:read", "im:history", "im:read", "mpim:history", "mpim:read", "pins:read", "reactions:read", "users:read", "users:read.email"];
const WRITE_BOT: &[&str] = &["channels:join", "chat:write", "chat:write.public", "files:write", "im:write", "pins:write", "reactions:write"];

/// How to create a Slack app for a profile: Slack's "Create app" page with the app manifest filled in, and the steps.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "SlackAppLink")]
pub struct AppLink {
    pub url: String,
    pub steps: Vec<String>,
}

/// Pure: the app of a profile named `name`: user scopes for a user token (it acts as the person who installs it), or
/// bot scopes and a bot user for a bot token; read scopes, plus write scopes when `write` (like `slackline token
/// create [--write]`, whose user manifest it extends with a name and a bot variant).
pub fn app_link(name: &str, kind: TokenKind, write: bool) -> AppLink {
    let name: String = name.trim().chars().take(35).collect();
    let name = if name.is_empty() { "Launchpad".to_string() } else { name };
    let (read, writes) = match kind {
        TokenKind::User => (READ_USER, WRITE_USER),
        TokenKind::Bot => (READ_BOT, WRITE_BOT),
    };
    let scopes: Vec<&str> = read.iter().chain(if write { writes.iter() } else { [].iter() }).copied().collect();
    let what = if write { "read and write" } else { "read-only" };
    let mut manifest = serde_json::json!({
        "display_information": {"background_color": "#4a154b", "description": format!("Launchpad agents, {what}"), "name": name},
        "settings": {"org_deploy_enabled": false, "socket_mode_enabled": false, "token_rotation_enabled": false},
    });
    match kind {
        TokenKind::User => manifest["oauth_config"] = serde_json::json!({"scopes": {"user": scopes}}),
        TokenKind::Bot => {
            manifest["features"] = serde_json::json!({"bot_user": {"display_name": name, "always_online": false}});
            manifest["oauth_config"] = serde_json::json!({"scopes": {"bot": scopes}});
        }
    }
    // Percent-encode everything but unreserved characters, like `slackline token create`.
    let encoded: String = manifest.to_string().bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let mut steps = vec!["Open Slack: the app's name and permissions are already filled in.".to_string(), "Pick your Slack workspace, then click Next and Create.".into(), "In the sidebar, open Install App, click Install to Workspace, then Allow.".into()];
    match kind {
        TokenKind::User => steps.push("Copy the User OAuth Token (it starts with xoxp-) and paste it here. The app acts as you: what you can see, and messages in your name.".into()),
        TokenKind::Bot => {
            steps.push("Copy the Bot User OAuth Token (it starts with xoxb-) and paste it here. The app acts as its own bot user, under the app's name.".into());
            steps.push("In Slack, add the app to the channels it should read or post in (channel name → Integrations → Add apps).".into());
        }
    }
    AppLink { url: format!("https://api.slack.com/apps?new_app=1&manifest_json={encoded}"), steps }
}

// ---------------------------------------------------------------- access

/// The profiles a session of this request holds: on by its defaults (workspace, folder, bot) and its launch's own
/// choice, from Home's registry of profiles.
pub async fn for_request(app: &App, req: &crate::vault::LeaseRequest) -> Result<Vec<Profile>> {
    let layers = crate::tool_defaults::layers(app, req.workspace_id.as_deref(), req.project_id.as_deref(), req.bot_id.as_deref()).await?;
    let layers: Vec<&crate::tool_defaults::ToolChoice> = layers.iter().collect();
    Ok(list(app).await?.into_iter().filter(|p| crate::tool_defaults::is_on_with(&layers, &req.tools_off, &req.tools_on, &key(&p.handle), p.default_on)).collect())
}

/// Home: the profiles a session Home placed holds, from its own record of the session (never from the request).
pub async fn of_session(app: &App, session_id: &str) -> Result<Vec<Profile>> {
    let s = app.sessions.get(session_id).context("Home has no record of this session")?;
    let req = crate::contain::request(app, &s).await?;
    for_request(app, &req).await
}

/// The instructions of a session that holds these profiles (none: no instructions).
pub fn instructions(cli: &str, profiles: &[Profile]) -> Option<String> {
    if profiles.is_empty() {
        return None;
    }
    let mut s = format!(
        "# Slack\n\nYou use Slack with `{cli} slack --profile <handle> <command>`: the same commands and options as slackline (`{cli} slack --profile <handle> --help`, then `--help` of each command), for example `search messages \"from:@alice deploy\"`, `channels history <channel id> --after 2h`, `messages replies <channel id> <ts>`, `users search <name>`, `messages send <channel id> \"text\" [--thread-ts <ts>]`. Home runs it with the profile's token: you never see a token. Each profile can do only what its description says (its token's scopes, and in whose name it acts); pick the one that fits, and prefer reading with one profile and writing with another when the descriptions say so. Your profiles:\n"
    );
    for p in profiles {
        let whose = match p.token_kind {
            TokenKind::User => "user token: acts as the person who installed it",
            TokenKind::Bot => "bot token: acts as the app's bot, only in channels it was added to",
        };
        let desc = if p.description.is_empty() { String::new() } else { format!(": {}", p.description) };
        s.push_str(&format!("- `{}` ({}, {whose}){desc}\n", p.handle, p.name));
    }
    s.push_str("Messages you send (`messages send`, `dms send`, `files upload --comment`) may get Launchpad's attribution line, as the profile says: never add one yourself. `files upload <path>` sends a file of your folder (at most 8 MB); `files download <id> -o <path>` writes into your folder. `watch` (Socket Mode streaming) is not available.\n");
    Some(s)
}

/// The attribution line a message of this session sends with a profile whose `footer` is on (Slack's link syntax,
/// posted automatically).
pub fn footer_line(s: &crate::model::Session) -> String {
    crate::attribution::slack(&crate::attribution::with_origin(&crate::attribution::line(s), crate::attribution::Origin::Automatic))
}

// ---------------------------------------------------------------- slackline arguments

/// What `plan` found in slackline arguments: the places the CLI and Home treat themselves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The arguments, with `--option=value` of the options below split in two.
    pub args: Vec<String>,
    /// `files upload <PATH>`: the index of PATH (a file of the agent's folder, sent to Home).
    pub upload: Option<usize>,
    /// `files upload --channel`: shared in a channel.
    pub shared: bool,
    /// `files download -o <path>`: the indices of `-o` and its value (written by the CLI, never by Home).
    pub output: Option<usize>,
    /// The message text that gets the attribution line: `messages send`'s and `dms send`'s TEXT, `files upload`'s
    /// `--comment` value.
    pub text: Option<usize>,
}

/// Options that take a value, of the commands whose arguments Launchpad reads.
const VALUE_OPTS: &[(&str, &str, &[&str])] = &[
    ("messages", "send", &["--thread-ts"]),
    ("dms", "send", &[]),
    ("files", "upload", &["-c", "--channel", "--thread-ts", "--comment"]),
    ("files", "download", &["-o", "--output"]),
];
/// Flags every command takes.
const FLAGS: &[&str] = &["--json", "--pretty", "-q", "--quiet", "-h", "--help", "-V", "--version"];

/// Pure: reads slackline arguments. Refused: `--token` (Home gives the token), `watch` (a stream that does not end).
/// The commands that send text or files must use only the options Launchpad knows, so their text and paths are
/// found for sure.
pub fn plan(args: &[String]) -> std::result::Result<Plan, String> {
    fn value(out: &mut Plan, opt: &str, v: &str) {
        out.args.push(v.to_string());
        let at = out.args.len() - 1;
        match opt {
            "--comment" => out.text = Some(at),
            "-o" | "--output" => out.output = Some(at - 1),
            "-c" | "--channel" => out.shared = true,
            _ => {}
        }
    }
    let mut out = Plan::default();
    // Positional words (group, command, arguments) with their index in `out.args`.
    let mut words: Vec<(usize, String)> = vec![];
    let mut rest = false;
    let mut known: Option<&[&str]> = None;
    let mut pending: Option<String> = None;
    for a in args {
        if let Some(opt) = pending.take() {
            value(&mut out, &opt, a);
            continue;
        }
        if !rest && (a == "--token" || a.starts_with("--token=")) {
            return Err("Home gives the profile's token: leave out --token".into());
        }
        if !rest && a == "--" {
            rest = true;
            out.args.push(a.clone());
            continue;
        }
        if !rest && a.starts_with('-') && a.len() > 1 {
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n, Some(v)),
                _ => (a.as_str(), None),
            };
            match known {
                Some(opts) if !FLAGS.contains(&name) => {
                    if !opts.contains(&name) {
                        return Err(format!("{name} is not an option Launchpad knows for this command: see --help"));
                    }
                    out.args.push(name.to_string());
                    match inline {
                        Some(v) => value(&mut out, name, v),
                        None => pending = Some(name.to_string()),
                    }
                }
                _ => out.args.push(a.clone()),
            }
            continue;
        }
        out.args.push(a.clone());
        words.push((out.args.len() - 1, a.clone()));
        if words.len() == 2 {
            let (g, c) = (words[0].1.as_str(), words[1].1.as_str());
            known = VALUE_OPTS.iter().find(|(vg, vc, _)| *vg == g && *vc == c).map(|(_, _, o)| *o);
        }
    }
    if pending.is_some() {
        return Err("an option is missing its value".into());
    }
    let group = words.first().map(|w| w.1.as_str());
    if group == Some("watch") {
        return Err("watch streams Socket Mode events without end: it is not available through Launchpad".into());
    }
    let command = words.get(1).map(|w| w.1.as_str());
    let positional = |n: usize| words.get(2 + n).map(|w| w.0);
    match (group, command) {
        (Some("messages"), Some("send")) | (Some("dms"), Some("send")) => out.text = positional(1),
        (Some("files"), Some("upload")) => out.upload = positional(0),
        _ => {}
    }
    Ok(out)
}

/// Pure: whether the arguments only ask for help or the version (no token needed).
pub fn help_only(args: &[String]) -> bool {
    !args.is_empty() && args.iter().all(|a| !a.starts_with("--token")) && args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "-V" | "--version" | "help"))
}

// ---------------------------------------------------------------- running slackline (Home)

/// What a call answers: slackline's exit code and output, read by `launchpad slack` as its own.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Output {
    pub code: i32,
    /// stdout, base64 (a downloaded file is bytes).
    pub stdout: String,
    pub stderr: String,
}

/// stdout per call (a downloaded file, a long history): base64 in a JSON answer stays under the tunnel's limits.
pub const MAX_STDOUT: usize = 8 << 20;
/// A file `files upload` sends.
pub const MAX_UPLOAD: usize = 8 << 20;
const MAX_STDERR: usize = 64 << 10;
/// slackline runs Home starts at once: more calls wait their turn (`TIMEOUT` included).
const CONCURRENT: usize = 4;
/// A call ends after this long (Slack's rate limits make slackline wait and retry).
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A binary Home runs: `$<var>` when set, else `<name>` on PATH (Home's image installs slackline in /usr/local/bin).
pub fn binary(app: &App, var: &str, name: &str) -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os(var).filter(|p| !p.is_empty()) {
        return Some(std::path::PathBuf::from(p));
    }
    let paths = [app.env.get("PATH").map(str::to_string), std::env::var("PATH").ok(), Some("/usr/local/bin:/usr/bin:/bin".to_string())];
    for path in paths.into_iter().flatten() {
        for dir in path.split(':').filter(|d| !d.is_empty()) {
            let p = std::path::Path::new(dir).join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Pure: slackline's whole environment: nothing of Home's own. `token` None: help and version only.
pub fn slackline_env(token: Option<&str>, home: &std::path::Path) -> Vec<(String, String)> {
    let mut env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string()), ("HOME".into(), home.display().to_string()), ("LANG".into(), "C.UTF-8".into()), ("NO_COLOR".into(), "1".into())];
    if let Some(t) = token {
        env.push(("SLACK_TOKEN".into(), t.into()));
    }
    env
}

/// Pure: a text without these secrets.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut t = text.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 8) {
        t = t.replace(*s, "[redacted]");
    }
    t
}

static SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> = std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(CONCURRENT));

/// Home: runs slackline with `args` in `dir` (a fresh folder of this call, also its HOME), directly (never through a
/// shell), with exactly `slackline_env`, stdin from /dev/null, stdout capped at `MAX_STDOUT` and stderr at 64 KB,
/// its process group killed at the deadline.
pub async fn run(bin: &std::path::Path, args: &[String], token: Option<&str>, dir: &std::path::Path) -> Output {
    use base64::Engine;
    use tokio::io::AsyncReadExt;
    let fail = |code: i32, why: String| Output { code, stdout: String::new(), stderr: format!("{why}\n") };
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let Ok(Ok(_slot)) = tokio::time::timeout_at(deadline, SLOTS.acquire()).await else {
        return fail(124, "Home is busy with other Slack calls: try again in a minute".into());
    };
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args).env_clear().envs(slackline_env(token, dir)).current_dir(dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true).process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(127, format!("could not start slackline on Home: {e}")),
    };
    let pid = child.id();
    let kill_group = move || {
        if let Some(p) = pid {
            // SAFETY: kill(2) on the process group this call created; no memory is touched.
            unsafe {
                libc::kill(-(p as i32), libc::SIGKILL);
            }
        }
    };
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    let work = async {
        let read_out = async {
            let mut buf = Vec::new();
            if let Some(o) = out.as_mut() {
                let mut chunk = vec![0u8; 65536];
                loop {
                    match o.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.len() > MAX_STDOUT {
                                kill_group();
                                return None;
                            }
                        }
                    }
                }
            }
            Some(buf)
        };
        let read_err = async {
            let mut buf = Vec::new();
            if let Some(e) = err.as_mut() {
                let mut chunk = vec![0u8; 8192];
                loop {
                    match e.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) if buf.len() < MAX_STDERR => buf.extend_from_slice(&chunk[..n]),
                        Ok(_) => {}
                    }
                }
            }
            buf
        };
        let (out, err) = tokio::join!(read_out, read_err);
        let status = child.wait().await;
        kill_group();
        (out, err, status)
    };
    let secrets: Vec<&str> = token.into_iter().collect();
    match tokio::time::timeout_at(deadline, work).await {
        Ok((None, _, _)) => fail(1, format!("slackline's answer is larger than {} MiB: ask for less (--limit) or download the file in parts", MAX_STDOUT >> 20)),
        Ok((Some(out), err, status)) => Output {
            code: status.ok().and_then(|s| s.code()).unwrap_or(1),
            stdout: base64::engine::general_purpose::STANDARD.encode(out),
            stderr: redact(&String::from_utf8_lossy(&err), &secrets),
        },
        Err(_) => {
            kill_group();
            fail(124, format!("slackline did not finish within {} s on Home", TIMEOUT.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn handles_follow_names() {
        assert_eq!(handle_of("Read as me"), "read-as-me");
        assert_eq!(handle_of("  User / Read!  "), "user-read");
        assert_eq!(handle_of("Écrire"), "crire");
        assert!(valid_handle("user-read") && valid_handle("bot2"));
        for bad in ["", "-a", "a-", "a--b", "A", "a_b", &"x".repeat(41)] {
            assert!(!valid_handle(bad), "{bad}");
        }
    }

    #[test]
    fn token_kinds() {
        assert_eq!(token_kind("xoxp-1-2").unwrap(), TokenKind::User);
        assert_eq!(token_kind(" xoxb-1 ").unwrap(), TokenKind::Bot);
        assert!(token_kind("xapp-1-A").unwrap_err().to_string().contains("Socket Mode"));
        assert!(token_kind("ghp_x").is_err() && token_kind("xoxp-1 2").is_err());
    }

    #[test]
    fn plan_finds_text_files_and_output() {
        let p = plan(&v(&["messages", "send", "C1", "hello", "--thread-ts", "1.2"])).unwrap();
        assert_eq!((p.text, p.upload, p.output), (Some(3), None, None));
        let p = plan(&v(&["--json", "messages", "send", "--thread-ts=1.2", "C1", "hi"])).unwrap();
        assert_eq!(p.args, v(&["--json", "messages", "send", "--thread-ts", "1.2", "C1", "hi"]));
        assert_eq!(p.text, Some(6));
        let p = plan(&v(&["dms", "send", "--", "U1", "-x starts with a dash"])).unwrap();
        assert_eq!(p.text, Some(4));
        let p = plan(&v(&["files", "upload", "a.png", "-c", "C1", "--comment=look"])).unwrap();
        assert_eq!((p.upload, p.text, p.shared), (Some(2), Some(6), true));
        assert_eq!(p.args[6], "look");
        let p = plan(&v(&["files", "download", "F1", "-o", "out.bin"])).unwrap();
        assert_eq!((p.output, p.upload), (Some(3), None));
        let p = plan(&v(&["search", "messages", "--limit", "5", "deploy"])).unwrap();
        assert_eq!((p.text, p.upload, p.output), (None, None, None));
    }

    #[test]
    fn plan_refuses_tokens_streams_and_unknown_options() {
        assert!(plan(&v(&["--token", "x", "users", "list"])).is_err());
        assert!(plan(&v(&["users", "list", "--token=x"])).is_err());
        assert!(plan(&v(&["watch", "--events", "all"])).is_err());
        assert!(plan(&v(&["messages", "send", "--blocks", "x", "C1", "t"])).is_err());
        assert!(plan(&v(&["files", "download", "F1", "-o"])).is_err());
        assert!(plan(&v(&["files", "download", "F1", "-oout"])).is_err());
    }

    #[test]
    fn app_links_match_slackline_and_add_bots() {
        // `slackline token create` (v1.5.0) prints the same user scopes; Launchpad names the app.
        let read = app_link("Read as me", TokenKind::User, false);
        assert!(read.url.starts_with("https://api.slack.com/apps?new_app=1&manifest_json=%7B"));
        assert!(read.url.contains("%22name%22%3A%22Read%20as%20me%22"));
        assert!(read.url.contains("%22user%22%3A%5B%22channels%3Ahistory%22%2C%22channels%3Aread%22%2C%22files%3Aread%22%2C%22groups%3Ahistory%22%2C%22groups%3Aread%22%2C%22im%3Ahistory%22%2C%22im%3Aread%22%2C%22mpim%3Ahistory%22%2C%22mpim%3Aread%22%2C%22pins%3Aread%22%2C%22reactions%3Aread%22%2C%22search%3Aread%22%2C%22users%3Aread%22%2C%22users%3Aread.email%22%5D"));
        assert!(!read.url.contains("chat%3Awrite") && read.steps.iter().any(|s| s.contains("xoxp-")));
        let bot = app_link("Launchpad bot", TokenKind::Bot, true);
        assert!(bot.url.contains("%22bot%22%3A%5B") && bot.url.contains("chat%3Awrite") && bot.url.contains("bot_user") && !bot.url.contains("search%3Aread"));
        assert!(bot.steps.iter().any(|s| s.contains("xoxb-")));
        assert!(app_link("  ", TokenKind::User, true).url.contains("%22name%22%3A%22Launchpad%22"));
    }

    #[test]
    fn slackline_env_is_closed() {
        let keys = |t| slackline_env(t, std::path::Path::new("/tmp/h")).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(keys(Some("xoxp-1")), ["PATH", "HOME", "LANG", "NO_COLOR", "SLACK_TOKEN"]);
        assert_eq!(keys(None), ["PATH", "HOME", "LANG", "NO_COLOR"]);
        assert_eq!(redact("invalid_auth for xoxp-123456789", &["xoxp-123456789", "short"]), "invalid_auth for [redacted]");
    }

    #[test]
    fn instructions_list_each_profile() {
        assert!(instructions("launchpad", &[]).is_none());
        let p = Profile { handle: "user-read".into(), name: "Read as me".into(), description: "Searches and reads as Lucie.".into(), token_kind: TokenKind::User, token_detail: "…abcd".into(), default_on: true, footer: true, updated_at: 0 };
        let s = instructions("launchpad", &[p]).unwrap();
        assert!(s.contains("- `user-read` (Read as me, user token: acts as the person who installed it): Searches and reads as Lucie.\n"));
        assert!(s.contains("launchpad slack --profile <handle>"));
    }
}

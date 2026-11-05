//! `launchpad`: the daemon and the agent-facing commands.
//!
//! The environment comes from `LAUNCHPAD_ENV` (`prod` by default, `dev` for development).

mod browser;
mod extensions;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use launchpad_core::paths::Paths;

/// The daemon API: over its local port, or, for agents on an SSH host, through the Launchpad helper there.
#[derive(Clone)]
enum Client {
    Local { base: String, token: String, http: reqwest::Client },
    Helper(std::path::PathBuf),
}

impl Client {
    fn new(paths: &Paths) -> Result<Client> {
        let helper = std::env::var_os(launchpad_core::hosts::helper::HELPER_ENV).map(std::path::PathBuf::from).or_else(|| {
            let exe = std::env::current_exe().ok()?;
            let bin = dirs_home()?.join(launchpad_core::hosts::helper::BIN_DIR);
            exe.starts_with(&bin).then(|| launchpad_core::hosts::helper::dir(paths.app_env.name()))
        });
        Ok(match helper {
            Some(dir) => Client::Helper(dir),
            // LAUNCHPAD_TOKEN: a restricted credential (a session's CLI token).
            None => Client::Local { base: paths.base_url(), token: match std::env::var("LAUNCHPAD_TOKEN") { Ok(t) if !t.trim().is_empty() => t.trim().to_string(), _ => paths.token()? }, http: reqwest::Client::new() },
        })
    }

    /// `path` includes the query. Returns the status and body text.
    async fn send(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<(u16, String)> {
        match self {
            Client::Local { base, token, http } => {
                let mut req = http.request(reqwest::Method::from_bytes(method.as_bytes())?, format!("{base}{path}")).bearer_auth(token);
                if let Some(body) = body {
                    req = req.json(&body);
                }
                let res = req.send().await.context("is the daemon running?")?;
                let status = res.status().as_u16();
                Ok((status, res.text().await?))
            }
            Client::Helper(dir) => launchpad_core::hosts::helper::cli_request(dir, method, path, body.map(|b| b.to_string())).await,
        }
    }

    async fn json(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value> {
        let (status, text) = self.send(method, path, body).await?;
        let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        anyhow::ensure!((200..300).contains(&status), "{}", body["error"].as_str().unwrap_or("request failed"));
        Ok(body)
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

fn query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse(&format!("http://x{path}")).expect("valid path");
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or(""))
}

#[derive(Parser)]
#[command(name = "launchpad", about = "Launchpad daemon and command-line tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage local pre-draft pull requests.
    Pr {
        #[command(subcommand)]
        command: Pr,
    },
    /// Save an agent-written suggestion locally; never posts to GitHub.
    Suggest {
        #[command(subcommand)]
        command: Suggest,
    },
    /// Send a message through one of the owner's channels: `telegram` goes to the owner's own chat; other
    /// channels (Slack, Discord, webhooks) wait in the owner's Outbox until they click Send.
    Send {
        /// The channel name (Settings → Channels), or `telegram`.
        channel: String,
        #[arg(long)]
        body_file: std::path::PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Task worktrees managed by Launchpad (for agents).
    Work {
        #[command(subcommand)]
        command: Work,
    },
    /// Asset folders: large or generated files of a Git folder, stored by Launchpad instead of Git (for agents).
    Assets {
        #[command(subcommand)]
        command: Assets,
    },
    /// The workspace's knowledge base, the only memory agents have: read its files in $LAUNCHPAD_KB, write here.
    Kb {
        #[command(subcommand)]
        command: Kb,
    },
    /// Registered folders (JSON), optionally of one workspace.
    Folders {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Bots of a workspace with their job, profile, folder and current session (JSON).
    Bots {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Agent sessions (JSON): `list` or `show <id>`.
    Sessions {
        #[command(subcommand)]
        command: Sessions,
    },
    /// Profiles without their environment, with the quota of each subscription login (JSON).
    Profiles {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Prompt presets (JSON).
    Presets {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Active GitHub inbox threads (JSON), optionally of one workspace's repositories.
    Inbox {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Scheduled launches and cron agents (JSON).
    Schedules {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Ask Launchpad proposals with ids, titles, states and contents (JSON).
    Proposals {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// What waits for the owner today (JSON), optionally of one workspace.
    Today {
        #[arg(long)]
        workspace: Option<String>,
    },
    /// What sessions with admin access changed directly, with the values before (JSON).
    Activity {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Store an inert proposal for the owner to start with a click; never starts anything.
    Propose {
        #[arg(value_parser = ["launch", "message", "schedule", "cron", "status", "preset", "tool", "bot", "routine", "bot-instructions", "ready"])]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        note: Option<String>,
        /// Supersede a waiting, unstarted proposal in the same workspace.
        #[arg(long)]
        replaces: Option<String>,
        /// JSON file with the proposal fields (see the Launchpad CLI tool instructions).
        #[arg(long)]
        spec_file: std::path::PathBuf,
        /// The session writing it (defaults to LAUNCHPAD_SESSION_ID).
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Call Launchpad's API (sessions with admin access to the Launchpad CLI tool): `launchpad api PATCH /api/bots/<id> --body-file b.json`.
    Api {
        /// GET, POST, PUT, PATCH or DELETE.
        method: String,
        /// The API path, starting with /api/.
        path: String,
        /// JSON file with the request body.
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Run an action of Launchpad's action list (sessions with admin access): `launchpad do session.archive <id>`.
    /// Short verbs do the same: `launchpad session archive <id>`, `launchpad bot pause <id> --paused true`.
    Do {
        /// The action name, `<group>.<verb>`.
        action: String,
        /// Route ids in order, then `--<field> <value>` options and `--body-file <file.json>`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(external_subcommand)]
    Verb(Vec<String>),
    /// Print the stored trace of an agent session, for use as reference context.
    Trace {
        session_id: String,
        /// Last event included (defaults to the latest stored event).
        #[arg(long)]
        through: Option<i64>,
    },
    /// Read-only browser MCP bridge, started by Launchpad harness adapters.
    #[command(hide = true)]
    BrowserMcp { #[arg(long)] data_dir: Option<std::path::PathBuf> },
    /// Chrome native messaging host (registered for the pinned extension only).
    #[command(hide = true)]
    NativeHost { origin: String, #[arg(trailing_var_arg = true, allow_hyphen_values = true)] extra: Vec<String> },
    /// Run the daemon (HTTP + WebSocket; loopback by default).
    Daemon {
        /// home: records and services, no agents; runner: agents and terminals, in its own folders
        /// (`~/.local/share/launchpad-runner`); all: both in one process.
        #[arg(long, default_value = "all")]
        role: String,
    },
    /// Pair this machine as a runner of a Home: shows a code to approve in Home's Settings → Home and runners.
    Connect {
        /// Home's address, for example https://<you>-launchpad.hf.space
        url: String,
        /// Name shown on Home (default: the host name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Print the daemon status as JSON.
    Status,
    /// Print the address that signs a browser into the UI.
    Url,
    /// Print a five-minute pairing link and QR code for a browser or phone.
    Pair,
    /// The Launchpad helper on an SSH host (started by the daemon over ssh).
    #[command(hide = true)]
    HostHelper {
        #[command(subcommand)]
        command: HostHelper,
    },
    /// Write the TypeScript types shared with the UI.
    #[command(hide = true)]
    GenTypes {
        #[arg(default_value = "web/src/gen")]
        out: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum ListOnly {
    List {
        /// Workspace ID or name (default: every workspace).
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
enum Sessions {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Show { session_id: String },
}

/// Read-only context commands: GET `/api/context/<what>`.
async fn list_command(paths: &Paths, base: &str, workspace: Option<String>) -> Result<()> {
    let path = match workspace { Some(w) => query(base, &[("workspace", &w)]), None => base.into() };
    let body = Client::new(paths)?.json("GET", &path, None).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn context_command(paths: &Paths, what: &str, workspace: Option<String>) -> Result<()> {
    let path = match workspace { Some(w) => query(&format!("/api/context/{what}"), &[("workspace", &w)]), None => format!("/api/context/{what}") };
    let body = Client::new(paths)?.json("GET", &path, None).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[derive(Subcommand)]
enum HostHelper {
    /// Run the helper server (detached; owns harness and terminal processes).
    Serve {
        #[arg(long, default_value = "prod")]
        env: String,
    },
    /// Pipe stdio to the helper server, starting it when needed.
    Bridge {
        #[arg(long, default_value = "prod")]
        env: String,
    },
    /// Print the helper build.
    Version,
}

#[derive(Subcommand)]
enum Suggest {
    Comment(SuggestArgs),
    Review(SuggestArgs),
}
#[derive(clap::Args)]
struct SuggestArgs {
    #[arg(
        long,
        required_unless_present = "predraft",
        conflicts_with = "predraft",
        requires = "number"
    )]
    repo: Option<String>,
    #[arg(long, requires = "repo")]
    number: Option<u64>,
    #[arg(long, conflicts_with_all = ["repo", "number"])]
    predraft: Option<String>,
    #[arg(long)]
    body_file: std::path::PathBuf,
    #[arg(long, default_value = "comment", value_parser = ["comment", "approve", "request-changes"])]
    event: String,
    #[arg(long)]
    head: Option<String>,
    #[arg(long)]
    session_id: Option<String>,
}
async fn suggest_command(paths: &Paths, command: Suggest) -> Result<()> {
    let (kind, q) = match command {
        Suggest::Comment(q) => ("comment", q),
        Suggest::Review(q) => ("review", q),
    };
    let target = match q.predraft {
        Some(id) => serde_json::json!({"kind":"predraft","id":id}),
        None => serde_json::json!({"kind":"git_hub","repo":q.repo,"number":q.number}),
    };
    let body = Client::new(paths)?
        .json("POST", "/api/suggestions", Some(serde_json::json!({"target":target,"kind":kind,"body":std::fs::read_to_string(q.body_file)?,"event":q.event.replace('-', "_").to_uppercase(),"head":q.head,"session_id":q.session_id})))
        .await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[derive(Subcommand)]
enum Work {
    /// Ask Launchpad to merge this session's branches into their base branches after the turn ends.
    Finish {
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Create a dedicated worktree of another registered project for this session, and print its folder.
    AddProject {
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
        /// Project name or ID.
        #[arg(long)]
        project: String,
    },
}
#[derive(Subcommand)]
enum Kb {
    /// Print the folder of the knowledge base (read-only files).
    Path,
    /// Lines of the knowledge base that contain every word.
    Search { words: Vec<String> },
    /// Create or replace a note with stdin (or --file). Paths are relative to the knowledge base.
    Write {
        path: String,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Add stdin (or --file, or the text arguments) to the end of a note.
    Append {
        path: String,
        text: Vec<String>,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Copy an image, video or audio file into media/ (or --as <path>).
    AddMedia {
        file: std::path::PathBuf,
        #[arg(long = "as")]
        to: Option<String>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Remove a note (restorable for 30 days).
    Rm {
        path: String,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
}

fn kb_dir() -> Result<std::path::PathBuf> {
    std::env::var_os("LAUNCHPAD_KB").map(std::path::PathBuf::from).context("no knowledge base here: launchpad kb works in an agent session of a workspace ($LAUNCHPAD_KB is not set)")
}

/// A note path: relative to the knowledge base, or a path inside its folder.
fn kb_path(path: &str) -> Result<String> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        let dir = kb_dir()?;
        let rel = p.strip_prefix(&dir).with_context(|| format!("{path} is not in the knowledge base ({})", dir.display()))?;
        return Ok(rel.to_string_lossy().into_owned());
    }
    Ok(path.trim_start_matches("./").to_string())
}

fn read_input(file: Option<&std::path::Path>) -> Result<Vec<u8>> {
    use std::io::Read;
    match file {
        Some(f) => std::fs::read(f).with_context(|| format!("read {}", f.display())),
        None => {
            let mut buf = vec![];
            std::io::stdin().read_to_end(&mut buf)?;
            Ok(buf)
        }
    }
}

async fn kb_command(paths: &Paths, command: Kb) -> Result<()> {
    use base64::Engine;
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let (session_id, op) = match command {
        Kb::Path => {
            println!("{}", kb_dir()?.display());
            return Ok(());
        }
        Kb::Search { words } => {
            let dir = kb_dir()?;
            let mut bodies = vec![];
            for rel in launchpad_core::plain::walk(&dir) {
                if let Ok(text) = std::fs::read_to_string(dir.join(&rel)) {
                    bodies.push((rel, text));
                }
            }
            let hits = launchpad_core::kb::search_text(bodies.iter().map(|(p, t)| (p.clone(), t.as_str())), &words.join(" "), 200);
            if hits.is_empty() {
                println!("No match.");
            }
            for h in hits {
                println!("{}:{}: {}", h.path, h.line, h.text);
            }
            return Ok(());
        }
        Kb::Write { path, file, session_id } => (session_id, serde_json::json!({"op": "write", "path": kb_path(&path)?, "content_b64": b64(&read_input(file.as_deref())?)})),
        Kb::Append { path, text, file, session_id } => {
            let text = if !text.is_empty() && file.is_none() { text.join(" ") } else { String::from_utf8(read_input(file.as_deref())?).context("append takes text")? };
            (session_id, serde_json::json!({"op": "append", "path": kb_path(&path)?, "text": text}))
        }
        Kb::AddMedia { file, to, session_id } => {
            let name = file.file_name().context("name a file")?.to_string_lossy().into_owned();
            let path = match to {
                Some(t) => kb_path(&t)?,
                None => format!("media/{name}"),
            };
            (session_id, serde_json::json!({"op": "write", "path": path, "content_b64": b64(&read_input(Some(&file))?)}))
        }
        Kb::Rm { path, session_id } => (session_id, serde_json::json!({"op": "remove", "path": kb_path(&path)?})),
    };
    let body = Client::new(paths)?.json("POST", "/api/kb", Some(serde_json::json!({"session_id": session_id, "ops": [op]}))).await?;
    for line in body["lines"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    Ok(())
}

#[derive(Subcommand)]
enum Assets {
    /// Make `path` of this worktree an asset folder, mapped in `.launchpad/assets.toml` (commit that file).
    Create {
        path: String,
        #[arg(long)]
        session_id: String,
    },
}
async fn assets_command(paths: &Paths, command: Assets) -> Result<()> {
    let Assets::Create { path, session_id } = command;
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    let body = Client::new(paths)?.json("POST", "/api/assets", Some(serde_json::json!({"session_id": session_id, "cwd": cwd, "path": path}))).await?;
    println!("{}", body["message"].as_str().unwrap_or_default());
    Ok(())
}
async fn work_command(paths: &Paths, command: Work) -> Result<()> {
    let (path, body) = match command {
        Work::Finish { session_id } => ("finish", serde_json::json!({"session_id": session_id})),
        Work::AddProject { session_id, project } => ("add-project", serde_json::json!({"session_id": session_id, "project": project})),
    };
    let body = Client::new(paths)?.json("POST", &format!("/api/worktrees/{path}"), Some(body)).await?;
    println!("{}", body["message"].as_str().unwrap_or_default());
    Ok(())
}

#[derive(Subcommand)]
enum Pr {
    /// Create a dedicated local worktree from this checkout's committed HEAD.
    Create {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Update the local PR for the current worktree.
    Update {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Show the local PR for the current worktree.
    Show,
    /// List local PRs, including published and archived records.
    List,
}
async fn pr_command(paths: &Paths, command: Pr) -> Result<()> {
    let client = Client::new(paths)?;
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();
    let resolve = || async { client.json("GET", &query("/api/predrafts/resolve", &[("cwd", &cwd)]), None).await };
    let id = |p: &serde_json::Value| p["id"].as_str().map(str::to_string).context("pre-draft id");
    let body = match command {
        Pr::List => client.json("GET", "/api/predrafts", None).await?,
        Pr::Create { title, body_file } => client.json("POST", "/api/predrafts", Some(serde_json::json!({"cwd":cwd,"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?}))).await?,
        Pr::Show => {
            let p = resolve().await?;
            client.json("GET", &format!("/api/predrafts/{}", id(&p)?), None).await?
        }
        Pr::Update { title, body_file } => {
            let p = resolve().await?;
            client.json("PATCH", &format!("/api/predrafts/{}", id(&p)?), Some(serde_json::json!({"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?}))).await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

/// `SSH_ASKPASS` for Launchpad's `ssh`: the question appears in the UI; the answer goes to `ssh` on stdout.
async fn askpass(paths: &Paths, host: &str, prompt: &str) -> Result<()> {
    let token = paths.token()?;
    let res = reqwest::Client::new()
        .post(format!("{}/api/hosts/{host}/askpass", paths.base_url()))
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(240))
        .json(&serde_json::json!({"prompt": prompt}))
        .send()
        .await?;
    anyhow::ensure!(res.status().is_success(), "cancelled");
    let body: serde_json::Value = res.json().await?;
    println!("{}", body["answer"].as_str().unwrap_or(""));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    launchpad_core::set_build(env!("LAUNCHPAD_COMMIT"), env!("LAUNCHPAD_SOURCE"));
    launchpad_harness::install();
    launchpad_core::ext::install(Box::new(extensions::Extensions));
    let paths = Paths::from_env();
    if let Ok(host) = std::env::var("LAUNCHPAD_ASKPASS") {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        return askpass(&paths, &host, &prompt).await;
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Suggest { command } => suggest_command(&paths, command).await,
        Command::Send { channel, body_file, title, session_id } => {
            let body = std::fs::read_to_string(&body_file).with_context(|| format!("read {}", body_file.display()))?;
            let session_id = session_id.or_else(|| std::env::var("LAUNCHPAD_SESSION_ID").ok()).context("pass --session-id <your session id>")?;
            let result = Client::new(&paths)?.json("POST", "/api/channels/send", Some(serde_json::json!({"channel": channel, "body": body, "title": title, "session_id": session_id}))).await?;
            println!("{}", result["message"].as_str().unwrap_or("Done."));
            Ok(())
        }
        Command::Pr { command } => pr_command(&paths, command).await,
        Command::Work { command } => work_command(&paths, command).await,
        Command::Assets { command } => assets_command(&paths, command).await,
        Command::Kb { command } => kb_command(&paths, command).await,
        Command::Folders { command: ListOnly::List { workspace } } => context_command(&paths, "folders", workspace).await,
        Command::Bots { command: ListOnly::List { workspace } } => context_command(&paths, "bots", workspace).await,
        Command::Profiles { command: ListOnly::List { .. } } => context_command(&paths, "profiles", None).await,
        Command::Presets { command: ListOnly::List { .. } } => context_command(&paths, "presets", None).await,
        Command::Inbox { command: ListOnly::List { workspace } } => context_command(&paths, "inbox", workspace).await,
        Command::Schedules { command: ListOnly::List { workspace } } => context_command(&paths, "schedules", workspace).await,
        Command::Sessions { command: Sessions::List { workspace } } => context_command(&paths, "sessions", workspace).await,
        Command::Sessions { command: Sessions::Show { session_id } } => context_command(&paths, &format!("sessions/{}", url_segment(&session_id)), None).await,
        Command::Proposals { command: ListOnly::List { workspace } } => {
            let path = match workspace { Some(w) => query("/api/proposals", &[("workspace", &w)]), None => "/api/proposals".into() };
            let proposals = Client::new(&paths)?.json("GET", &path, None).await?;
            println!("{}", serde_json::to_string_pretty(&proposals)?);
            Ok(())
        }
        Command::Today { workspace } => list_command(&paths, "/api/today", workspace).await,
        Command::Activity { command: ListOnly::List { workspace } } => list_command(&paths, "/api/activity", workspace).await,
        Command::Propose { kind, title, note, replaces, spec_file, session_id } => {
            let text = std::fs::read_to_string(&spec_file).with_context(|| format!("read {}", spec_file.display()))?;
            let mut body: serde_json::Value = serde_json::from_str(&text).context("the spec file is not valid JSON")?;
            anyhow::ensure!(body.is_object(), "the spec file must hold a JSON object");
            body["kind"] = serde_json::Value::String(kind.replace('-', "_"));
            let session_id = session_id.or_else(|| std::env::var("LAUNCHPAD_SESSION_ID").ok());
            let created = Client::new(&paths)?.json("POST", "/api/proposals", Some(serde_json::json!({"title": title, "note": note, "replaces": replaces, "body": body, "session_id": session_id}))).await?;
            println!("Proposal {} saved (state: {}).", created["id"].as_str().unwrap_or("?"), created["state"].as_str().unwrap_or("unknown"));
            if let Some(error) = created["error"].as_str() { println!("Error: {error}"); }
            Ok(())
        }
        Command::Api { method, path, body_file } => {
            let method = method.to_ascii_uppercase();
            anyhow::ensure!(matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE"), "the method is GET, POST, PUT, PATCH or DELETE");
            anyhow::ensure!(path.starts_with("/api/"), "the path starts with /api/");
            let body = match body_file {
                Some(f) => Some(serde_json::from_str(&std::fs::read_to_string(&f).with_context(|| format!("read {}", f.display()))?).context("the body file is not valid JSON")?),
                None => None,
            };
            let result = Client::new(&paths)?.json(&method, &path, body).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Do { action, args } => do_action(&paths, &action, &args).await,
        Command::Verb(words) => {
            let [group, verb, args @ ..] = words.as_slice() else { anyhow::bail!("unknown command {:?}: see `launchpad --help`", words.join(" ")) };
            do_action(&paths, &format!("{group}.{verb}"), args).await
        }
        Command::Trace { session_id, through } => {
            let mut path = format!("/api/sessions/{}/trace", url_segment(&session_id));
            if let Some(through) = through {
                path = query(&path, &[("through", &through.to_string())]);
            }
            let (status, text) = Client::new(&paths)?.send("GET", &path, None).await?;
            if !(200..300).contains(&status) {
                let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                anyhow::bail!("{}", body["error"].as_str().unwrap_or("request failed"));
            }
            println!("{text}");
            Ok(())
        }
        Command::HostHelper { command } => {
            match command {
                HostHelper::Serve { env } => {
                    init_tracing();
                    launchpad_core::hosts::helper::serve(&env).await
                }
                HostHelper::Bridge { env } => launchpad_core::hosts::helper::bridge(&env).await,
                HostHelper::Version => {
                    println!("{}", launchpad_core::hosts::build_id());
                    Ok(())
                }
            }
        }
        Command::BrowserMcp { data_dir } => {
            let mut paths = paths;
            if let Some(dir) = data_dir { paths.data_dir = dir; }
            browser::mcp(&paths).await
        },
        Command::NativeHost { origin, .. } => browser::native_host(&paths, &origin).await,
        Command::Daemon { role } => {
            init_tracing();
            let role = launchpad_core::role::Role::parse(&role)?;
            // A runner gets its own folders and port (`LAUNCHPAD_PORT` overrides the port).
            let paths = if role == paths.role { paths } else { Paths::for_role(paths.app_env, role) };
            tracing::info!("env {}, role {}, data in {}", paths.app_env.name(), role.name(), paths.data_dir.display());
            let app = launchpad_core::App::start(paths).await?;
            extensions::start(&app).await?;
            launchpad_server::serve(app, launchpad_api::router()).await?;
            // A runner that just installed Home's build starts it in this process.
            launchpad_core::self_update::restart_if_updated();
            Ok(())
        }
        Command::Connect { url, name } => {
            let runner = Paths::for_role(paths.app_env, launchpad_core::role::Role::Runner);
            launchpad_server::connect(&runner, &url, name).await
        }
        Command::Status => {
            let body = Client::new(&paths)?.json("GET", "/api/status", None).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
            Ok(())
        }
        Command::Url | Command::Pair => {
            let res = reqwest::Client::new().post(format!("{}/api/devices/pair", paths.base_url()))
                .bearer_auth(paths.token()?).send().await.context("is the daemon running?")?;
            let body: serde_json::Value = res.error_for_status()?.json().await?;
            println!("{}", body["url"].as_str().context("missing pairing URL")?);
            if let Some(warning) = body["warning"].as_str() {
                eprintln!("Warning: {warning}");
            }
            if matches!(cli.command, Command::Pair) {
                println!("\n{}\nSingle-use link; expires in five minutes.", body["terminal"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        Command::GenTypes { out } => extensions::gen_types(&out),
    }
}

/// The request of an action from its arguments: route ids in order, `--<field> <value>`, `--body-file <file>`.
fn action_request(action: &str, args: &[String]) -> Result<(&'static launchpad_core::actions::Action, String, Option<serde_json::Value>)> {
    use launchpad_core::actions::{self, ActionAccess, FieldKind};
    let a = actions::get(action).with_context(|| format!("no action {action:?}: the Launchpad CLI reference lists them"))?;
    anyhow::ensure!(a.access == ActionAccess::Admin, "{} is the owner's click only: ask the owner to use the Launchpad window", a.name);
    let (mut ids, mut fields, mut body_file) = (vec![], serde_json::Map::new(), None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(name) = arg.strip_prefix("--") else { ids.push(arg.as_str()); continue };
        let value = it.next().with_context(|| format!("--{name} needs a value"))?;
        if name == "body-file" { body_file = Some(value); continue; }
        let field = a.input.iter().find(|f| f.name == name || f.name.replace('_', "-") == name).with_context(|| format!("{} has no --{name} option: {}", a.name, actions::usage("launchpad", a)))?;
        let v = match field.kind {
            FieldKind::Text => serde_json::Value::String(value.clone()),
            FieldKind::Bool => serde_json::Value::Bool(value.parse().with_context(|| format!("--{name} is true or false"))?),
            FieldKind::Json => serde_json::from_str(value).with_context(|| format!("--{name} takes JSON"))?,
        };
        fields.insert(field.name.into(), v);
    }
    let path = actions::path(a, &ids)?;
    let mut body = match body_file {
        Some(f) => serde_json::from_str(&std::fs::read_to_string(f).with_context(|| format!("read {f}"))?).context("the body file is not valid JSON")?,
        None => {
            anyhow::ensure!(a.body_file.is_none(), "{} takes --body-file <file.json> ({})", a.name, a.body_file.unwrap_or_default());
            for f in a.input.iter().filter(|f| !f.optional) {
                anyhow::ensure!(fields.contains_key(f.name), "{} needs --{}: {}", a.name, f.name, actions::usage("launchpad", a));
            }
            serde_json::Value::Object(Default::default())
        }
    };
    let obj = body.as_object_mut().context("the body file must hold a JSON object")?;
    obj.extend(fields);
    if let Some(fixed) = a.fixed {
        obj.extend(serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(fixed).expect("valid fixed body"));
    }
    let body = (a.method != "DELETE").then_some(body);
    Ok((a, path, body))
}

async fn do_action(paths: &Paths, action: &str, args: &[String]) -> Result<()> {
    let (a, path, body) = action_request(action, args)?;
    let result = Client::new(paths)?.json(a.method, &path, body).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn url_segment(s: &str) -> String {
    let mut url = reqwest::Url::parse("http://x/").expect("valid url");
    url.path_segments_mut().expect("base").push(s);
    url.path().trim_start_matches('/').to_string()
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let journald = std::env::var_os("JOURNAL_STREAM").is_some();
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false);
    if journald {
        // journald adds its own timestamps.
        builder.without_time().with_ansi(false).init();
    } else {
        builder.init();
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;

    #[test]
    fn proposal_cli_accepts_replacement_without_a_note_and_lists_by_workspace() {
        let cli = Cli::try_parse_from(["launchpad", "propose", "launch", "--title", "Refined", "--spec-file", "spec.json", "--replaces", "old-id"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { replaces: Some(id), note: None, .. } if id == "old-id"));
        let cli = Cli::try_parse_from(["launchpad", "propose", "status", "--title", "New", "--spec-file", "spec.json"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { replaces: None, .. }));
        let cli = Cli::try_parse_from(["launchpad", "propose", "preset", "--title", "Preset", "--spec-file", "spec.json"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { kind, .. } if kind == "preset"));
        assert!(Cli::try_parse_from(["launchpad", "propose", "delete", "--title", "X", "--spec-file", "spec.json"]).is_err());
        let cli = Cli::try_parse_from(["launchpad", "proposals", "list", "--workspace", "Work"]).unwrap();
        assert!(matches!(cli.command, Command::Proposals { command: ListOnly::List { workspace: Some(w) } } if w == "Work"));
    }
}

//! `launchpad`: the daemon and the agent-facing commands.
//!
//! The environment comes from `LAUNCHPAD_ENV` (`prod` by default, `dev` for development).

mod browser;

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
            None => Client::Local { base: paths.base_url(), token: paths.token()?, http: reqwest::Client::new() },
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
    /// Task worktrees managed by Launchpad (for agents).
    Work {
        #[command(subcommand)]
        command: Work,
    },
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
    Daemon,
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
        #[arg(long)]
        session_id: String,
    },
    /// Create a dedicated worktree of another registered project for this session, and print its folder.
    AddProject {
        #[arg(long)]
        session_id: String,
        /// Project name or ID.
        #[arg(long)]
        project: String,
    },
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
    let paths = Paths::from_env();
    if let Ok(host) = std::env::var("LAUNCHPAD_ASKPASS") {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        return askpass(&paths, &host, &prompt).await;
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Suggest { command } => suggest_command(&paths, command).await,
        Command::Pr { command } => pr_command(&paths, command).await,
        Command::Work { command } => work_command(&paths, command).await,
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
        Command::Daemon => {
            init_tracing();
            tracing::info!("env {}, data in {}", paths.app_env.name(), paths.data_dir.display());
            let app = launchpad_core::App::start(paths).await?;
            launchpad_server::serve(app).await
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
        Command::GenTypes { out } => launchpad_core::gen_types(&out),
    }
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

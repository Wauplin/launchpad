//! `launchpad`: the daemon and the agent-facing commands.
//!
//! The profile comes from `LAUNCHPAD_PROFILE` (`prod` by default, `dev` for development).

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use launchpad_core::paths::Paths;

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
    /// Run the daemon (HTTP + WebSocket on 127.0.0.1).
    Daemon,
    /// Print the daemon status as JSON.
    Status,
    /// Reserve an idle daemon for installation, or release it after failure.
    #[command(hide = true)]
    UpdateInstallLock { #[arg(long)] release: bool },
    /// Print the address that signs a browser into the UI.
    Url,
    /// Write the TypeScript types shared with the UI.
    #[command(hide = true)]
    GenTypes {
        #[arg(default_value = "web/src/gen")]
        out: std::path::PathBuf,
    },
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
    let res = reqwest::Client::new().post(format!("{}/api/suggestions", paths.base_url()))
        .bearer_auth(paths.token()?)
        .json(&serde_json::json!({"target":target,"kind":kind,"body":std::fs::read_to_string(q.body_file)?,"event":q.event.replace('-', "_").to_uppercase(),"head":q.head,"session_id":q.session_id}))
        .send().await.context("is the daemon running?")?;
    let status = res.status();
    let body: serde_json::Value = res.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "{}",
        body["error"].as_str().unwrap_or("request failed")
    );
    println!("{}", serde_json::to_string_pretty(&body)?);
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
    let client = reqwest::Client::new();
    let token = paths.token()?;
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();
    let endpoint = format!("{}/api/predrafts", paths.base_url());
    let mut resolve_url = reqwest::Url::parse(&format!("{endpoint}/resolve"))?;
    resolve_url.query_pairs_mut().append_pair("cwd", &cwd);
    let resolve = || client.get(resolve_url.clone()).bearer_auth(&token);
    let request = match command {
        Pr::List => client.get(&endpoint),
        Pr::Create { title, body_file } => client.post(&endpoint).json(&serde_json::json!({"cwd":cwd,"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?})),
        Pr::Show => {
            let p: serde_json::Value = resolve().send().await?.error_for_status()?.json().await?;
            client.get(format!("{endpoint}/{}", p["id"].as_str().context("pre-draft id")?))
        }
        Pr::Update { title, body_file } => {
            let p: serde_json::Value = resolve().send().await?.error_for_status()?.json().await?;
            client.patch(format!("{endpoint}/{}", p["id"].as_str().context("pre-draft id")?)).json(&serde_json::json!({"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?}))
        }
    };
    let res = request
        .bearer_auth(token)
        .send()
        .await
        .context("is the daemon running?")?;
    let status = res.status();
    let body: serde_json::Value = res.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "{}",
        body["error"].as_str().unwrap_or("request failed")
    );
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    match cli.command {
        Command::Suggest { command } => suggest_command(&paths, command).await,
        Command::Pr { command } => pr_command(&paths, command).await,
        Command::Daemon => {
            init_tracing();
            tracing::info!("profile {}, data in {}", paths.profile.name(), paths.data_dir.display());
            let app = launchpad_core::App::start(paths).await?;
            launchpad_server::serve(app).await
        }
        Command::Status => {
            let token = paths.token()?;
            let res = reqwest::Client::new()
                .get(format!("{}/api/status", paths.base_url()))
                .bearer_auth(token)
                .send()
                .await
                .context("is the daemon running?")?;
            let body: serde_json::Value = res.error_for_status()?.json().await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
            Ok(())
        }
        Command::UpdateInstallLock { release } => {
            let body: serde_json::Value = reqwest::Client::new()
                .post(format!("{}/api/settings/update/install-lock", paths.base_url()))
                .bearer_auth(paths.token()?)
                .json(&serde_json::json!({"release": release}))
                .send().await?.error_for_status()?.json().await?;
            match body["ready"].as_bool() {
                Some(true) => {}
                Some(false) => {
                    eprintln!("Launchpad is busy: {}", body["reason"].as_str().unwrap_or("agents are running"));
                    std::process::exit(3)
                }
                None => anyhow::bail!("invalid installation reservation response"),
            }
            Ok(())
        }
        Command::Url => {
            println!("{}/auth?t={}", paths.base_url(), paths.token()?);
            Ok(())
        }
        Command::GenTypes { out } => launchpad_core::gen_types(&out),
    }
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

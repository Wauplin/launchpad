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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    match cli.command {
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
                Some(true) => {},
                Some(false) => std::process::exit(3),
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

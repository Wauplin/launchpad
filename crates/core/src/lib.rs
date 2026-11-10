//! Domain types and services of the Launchpad daemon.

pub mod actions;
pub mod attribution;
pub mod ask_bot;
pub mod self_update;
pub mod attachments;
pub mod away;
pub mod bots;
pub mod bot_memory;
pub mod browser;
pub mod code;
pub mod compose;
pub mod config;
pub mod db;
pub mod drafts;
pub mod events;
pub mod exec;
pub mod git;
pub mod github;
pub mod harness;
pub mod files;
pub mod model;
pub mod notify;
pub mod operations;
pub mod paths;
pub mod predrafts;
pub mod procs;
pub mod projects;
pub mod session_pages;
pub mod sessions;
pub mod session_token;
pub mod shellenv;
pub mod signin;
pub mod spend;
pub mod status;
pub mod quota;
pub mod role;
pub mod runners;
pub mod replica;
pub mod home_link;
pub mod home_backup;
pub mod home_git;
pub mod plain;
pub mod kb;
pub mod repo_cache;
pub mod analytics;
pub mod review_state;
pub mod terminals;
pub mod hf;
pub(crate) mod trace_copies;
mod titles;
pub mod util;
pub mod profiles;
pub mod prompt_context;
pub mod session_git;
pub mod workspace;
pub mod worktrees;
pub mod turn_sync;
pub mod vault;
pub mod proxy;
pub mod contain;
pub mod jail;
pub mod netcap;
pub mod tool_install;
pub mod token_relay;
pub mod handoff;
pub mod burst;
pub mod mcp;
pub mod tool_defaults;
pub mod ext;
pub mod locks;
pub mod diff;
pub mod routines;
pub mod today;
pub mod launchpad_cli;

/// Build information stamped by the `launchpad` binary (its `build.rs`), so a new commit relinks only the binary
/// instead of rebuilding this crate.
static BUILD: std::sync::OnceLock<(&'static str, &'static str)> = std::sync::OnceLock::new();

/// Called once by the binary at startup.
pub fn set_build(commit: &'static str, source: &'static str) {
    let _ = BUILD.set((commit, source));
}

/// Source revision of the running binary.
pub fn commit() -> &'static str {
    BUILD.get().map_or("unknown", |b| b.0)
}

/// Main checkout used to find the repository skill in source builds.
pub fn source() -> &'static str {
    BUILD.get().map_or("", |b| b.1)
}

use std::sync::Arc;

use anyhow::{Context, Result};

/// Shared state of a running daemon. Cheap to clone.
#[derive(Clone)]
pub struct App {
    pub paths: Arc<paths::Paths>,
    pub config: config::ConfigHandle,
    pub state_db: db::Db,
    pub cache_db: db::Db,
    pub bus: events::Bus,
    pub env: shellenv::UserEnv,
    pub status: status::StatusHandle,
    pub harnesses: harness::Registry,
    pub sessions: sessions::Sessions,
    pub github: github::GitHub,
    pub models: harness::ModelCache,
    pub suggestion_locks: locks::Locks,
    pub predraft_lock: Arc<tokio::sync::Mutex<()>>,
    /// Serializes task worktree creation, merges and cleanup. Take the session action lock first.
    pub worktree_lock: Arc<tokio::sync::Mutex<()>>,
    pub terminals: terminals::Terminals,
    pub browser: browser::Broker,
    /// Live runner tunnels (Home).
    pub runners: runners::Hub,
    pub home_backups: home_backup::Backups,
    /// The runner proxy that adds Home's tokens to contained sessions' requests.
    pub proxy: proxy::Proxy,
    /// State of the crates above the core (`ext::Extras`).
    pub extras: ext::Extras,
}

impl App {
    /// Opens the databases and the config, and starts the background services.
    pub async fn start(paths: paths::Paths) -> Result<App> {
        paths.ensure_dirs()?;
        if paths.role == role::Role::Runner {
            replica::enable();
        }
        attachments::init(&paths.data_dir);
        let mut env = shellenv::UserEnv::resolve().await;
        if paths.role == role::Role::Runner {
            // Discovery runs before a session has its jail. Never open or refresh the laptop's existing
            // harness login (or use its Git configuration) while running a parallel runner.
            let home = paths.data_dir.join("discovery/home");
            // Codex refuses a `CODEX_HOME` that does not exist.
            for d in [".claude", ".codex", ".config", ".local/share", ".local/state", ".cache"] {
                std::fs::create_dir_all(home.join(d))?;
            }
            let value = |p: &std::path::Path| p.to_string_lossy().into_owned();
            env = env.with(&[
                ("HOME".into(), value(&home)),
                ("CLAUDE_CONFIG_DIR".into(), value(&home.join(".claude"))),
                ("CODEX_HOME".into(), value(&home.join(".codex"))),
                ("XDG_CONFIG_HOME".into(), value(&home.join(".config"))),
                ("XDG_DATA_HOME".into(), value(&home.join(".local/share"))),
                ("XDG_STATE_HOME".into(), value(&home.join(".local/state"))),
                ("XDG_CACHE_HOME".into(), value(&home.join(".cache"))),
            ]);
        }
        if paths.role == role::Role::Home {
            home_backup::restore_if_empty(&paths, &env).await.context("restore the Home backup")?;
        }
        let paths = Arc::new(paths);
        let bus = events::Bus::new();
        let config = config::ConfigHandle::load(&paths, bus.clone())?;
        // Burst and Local VM: agents run as the daemon's own user, with no jail between them. A non-dumpable
        // daemon cannot be attached to, and its memory and environment (a Burst runner's credential) cannot be
        // read through /proc by those agents.
        #[cfg(target_os = "linux")]
        if paths.role.runs_agents() && (home_link::burst().is_some() || config.get().runner.boundary == config::Boundary::Vm) {
            // SAFETY: prctl on this process only.
            unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) };
        }
        let state_db = db::open_state(&paths.state_db())?;
        let cache_db = db::open_cache(&paths.cache_db())?;
        let status = status::StatusHandle::new(bus.clone());
        let harnesses = harness::Registry::new(&env);
        let github = github::GitHub::new(env.clone(), cache_db.clone());
        let app = App {
            paths,
            config,
            state_db,
            cache_db,
            bus,
            env,
            status,
            harnesses,
            sessions: sessions::Sessions::new(),
            github,
            models: harness::ModelCache::default(),
            suggestion_locks: locks::Locks::default(),
            predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
            worktree_lock: Arc::new(tokio::sync::Mutex::new(())),
            terminals: terminals::Terminals::default(),
            browser: browser::Broker::default(),
            runners: runners::Hub::default(),
            home_backups: home_backup::Backups::default(),
            proxy: proxy::Proxy::default(),
            extras: ext::Extras::default(),
        };
        predrafts::link_skill()?;
        app.sessions.load(&app).await?;
        if let Err(e) = ask_bot::seed(&app).await {
            tracing::warn!("default bot and tools: {e:#}");
        }
        bot_memory::move_from_kb(&app).await;
        {
            let app = app.clone();
            tokio::spawn(async move { if let Err(e) = attachments::collect_garbage(&app).await { tracing::warn!("upload cleanup: {e:#}"); } });
        }
        home_backup::spawn(&app);
        burst::spawn(&app);
        status::spawn_monitors(&app);
        quota::spawn(&app);
        app.harnesses.spawn_services(&app);
        harness::spawn_model_refresh(&app);
        projects::spawn(&app);
        home_git::spawn(&app);
        plain::spawn(&app);
        kb::spawn(&app);
        if let Err(e) = projects::forget_disk_folders(&app).await {
            tracing::warn!("folders of one disk: {e:#}");
        }
        worktrees::spawn(&app);
        turn_sync::spawn(&app);
        today::spawn(&app);
        handoff::spawn(&app);
        runners::spawn(&app);
        Ok(app)
    }
}

/// The ts-rs settings for `gen_types` of every crate, writing to `out`.
pub fn types_config(out: &std::path::Path) -> Result<ts_rs::Config> {
    std::fs::create_dir_all(out)?;
    // Timestamps are i64 in Rust; they fit in a JS number.
    Ok(ts_rs::Config::new().with_out_dir(out).with_large_int("number"))
}

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    actions::Action::export_all(cfg)?;
    std::fs::write(cfg.out_dir().join("actions.ts"), actions::typescript())?;
    predrafts::Predraft::export_all(cfg)?;
    config::Config::export_all(cfg)?;
    burst::BurstOverview::export_all(cfg)?;
    mcp::McpServer::export_all(cfg)?;
    status::Status::export_all(cfg)?;
    analytics::AgentRun::export_all(cfg)?;
    signin::SignInCheck::export_all(cfg)?;
    model::Session::export_all(cfg)?;
    today::Today::export_all(cfg)?;
    model::MessageContext::export_all(cfg)?;
    model::SessionProcess::export_all(cfg)?;
    model::SessionEvent::export_all(cfg)?;
    session_pages::SessionEventPage::export_all(cfg)?;
    session_pages::MachineStint::export_all(cfg)?;
    model::Project::export_all(cfg)?;
    model::HarnessDefaults::export_all(cfg)?;
    harness::HarnessDescriptor::export_all(cfg)?;
    projects::ProjectPatch::export_all(cfg)?;
    sessions::Preview::export_all(cfg)?;
    github::GhItem::export_all(cfg)?;
    terminals::TerminalInfo::export_all(cfg)?;
    terminals::TerminalFolders::export_all(cfg)?;
    code::CodeSources::export_all(cfg)?;
    code::FileList::export_all(cfg)?;
    prompt_context::PathResults::export_all(cfg)?;
    prompt_context::SkillResults::export_all(cfg)?;
    code::CodeFile::export_all(cfg)?;
    code::SearchResult::export_all(cfg)?;
    code::CommitInfo::export_all(cfg)?;
    session_git::GitFile::export_all(cfg)?;
    session_git::GitStatus::export_all(cfg)?;
    session_git::GitDiff::export_all(cfg)?;
    session_git::GitHistory::export_all(cfg)?;
    workspace::Workspace::export_all(cfg)?;
    runners::RunnerStatus::export_all(cfg)?;
    vault::VaultOverview::export_all(cfg)?;
    vault::ToolTest::export_all(cfg)?;
    tool_defaults::ToolDefaults::export_all(cfg)?;
    vault::LeaseInfo::export_all(cfg)?;
    proxy::Access::export_all(cfg)?;
    home_backup::BackupInfo::export_all(cfg)?;
    worktrees::WorktreeInfo::export_all(cfg)?;
    home_git::HomeBranch::export_all(cfg)?;
    home_git::MergeOutcome::export_all(cfg)?;
    plain::Change::export_all(cfg)?;
    plain::Tree::export_all(cfg)?;
    plain::AssetInfo::export_all(cfg)?;
    kb::Notes::export_all(cfg)?;
    kb::Hit::export_all(cfg)?;
    kb::NativeMemory::export_all(cfg)?;
    bots::Bot::export_all(cfg)?;
    Ok(())
}


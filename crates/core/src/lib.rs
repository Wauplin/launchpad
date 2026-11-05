//! Domain types and services of the Launchpad daemon.

pub mod attachments;
pub mod bots;
pub mod browser;
pub mod code;
pub mod compose;
pub mod config;
pub mod crons;
pub mod db;
pub mod drafts;
pub mod events;
pub mod exec;
pub mod git;
pub mod github;
pub mod harness;
pub mod hostfs;
pub mod hosts;
pub mod inbox;
pub mod items;
pub mod model;
pub mod notify;
pub mod onboarding;
pub mod operations;
pub mod paths;
pub mod predrafts;
pub mod procs;
pub mod push;
pub mod project_clone;
pub mod projects;
pub mod proposals;
pub mod schedules;
pub mod session_pages;
pub mod sessions;
pub mod session_token;
pub mod shellenv;
pub mod setup_checklist;
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
pub mod suggestions;
pub mod terminals;
pub mod trace_backup;
pub(crate) mod trace_sync;
mod titles;
pub mod util;
pub mod profiles;
pub mod prompt_context;
pub mod session_git;
pub mod workspace;
pub mod worktrees;
pub mod vault;
pub mod ssh_agent;
pub mod proxy;
pub mod contain;
pub mod jail;
pub mod tool_install;
pub mod token_relay;
pub mod handoff;
pub mod burst;
pub mod mcp;

/// Source revision embedded in the running binary.
pub const COMMIT: &str = env!("LAUNCHPAD_COMMIT");
/// Main checkout used to find the repository skill in source builds.
pub const SOURCE: &str = env!("LAUNCHPAD_SOURCE");

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
    pub inbox: inbox::Inbox,
    pub models: harness::ModelCache,
    pub suggestion_locks: suggestions::Locks,
    pub predraft_lock: Arc<tokio::sync::Mutex<()>>,
    /// Serializes task worktree creation, merges and cleanup. Take the session action lock first.
    pub worktree_lock: Arc<tokio::sync::Mutex<()>>,
    pub terminals: terminals::Terminals,
    pub push: push::Push,
    pub hosts: hosts::Hosts,
    pub browser: browser::Broker,
    pub trace_backup: trace_backup::BackupStatus,
    /// Live runner tunnels (Home).
    pub runners: runners::Hub,
    pub home_backups: home_backup::Backups,
    /// The runner proxy that adds Home's tokens to contained sessions' requests.
    pub proxy: proxy::Proxy,
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
        let state_db = db::open_state(&paths.state_db())?;
        let cache_db = db::open_cache(&paths.cache_db())?;
        let status = status::StatusHandle::new(bus.clone());
        let harnesses = harness::Registry::new(&env);
        let github = github::GitHub::new(env.clone(), cache_db.clone());
        let web_push = push::Push::new(paths.clone());
        let hosts = hosts::Hosts::start(paths.clone(), env.clone(), state_db.clone(), status.clone()).await?;
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
            inbox: inbox::Inbox::new(),
            models: harness::ModelCache::default(),
            suggestion_locks: suggestions::Locks::default(),
            predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
            worktree_lock: Arc::new(tokio::sync::Mutex::new(())),
            terminals: terminals::Terminals::default(),
            push: web_push,
            hosts,
            browser: browser::Broker::default(),
            trace_backup: trace_backup::BackupStatus::default(),
            runners: runners::Hub::default(),
            home_backups: home_backup::Backups::default(),
            proxy: proxy::Proxy::default(),
        };
        predrafts::link_skill()?;
        app.sessions.load(&app).await?;
        schedules::recover(&app).await?;
        proposals::recover(&app).await?;
        schedules::spawn(&app);
        {
            let app = app.clone();
            tokio::spawn(async move { if let Err(e) = attachments::collect_garbage(&app).await { tracing::warn!("upload cleanup: {e:#}"); } });
        }
        {
            let dir = app.paths.data_dir.join("images");
            tokio::spawn(async move {
                loop {
                    if let Err(e) = items::collect_images(&dir).await { tracing::warn!("image cleanup: {e:#}"); }
                    tokio::time::sleep(std::time::Duration::from_secs(86400)).await;
                }
            });
        }
        trace_backup::spawn(&app);
        home_backup::spawn(&app);
        burst::spawn(&app);
        crons::spawn(&app);
        status::spawn_monitors(&app);
        quota::spawn(&app);
        app.harnesses.spawn_services(&app);
        harness::spawn_model_refresh(&app);
        inbox::spawn(&app);
        projects::spawn(&app);
        home_git::spawn(&app);
        plain::spawn(&app);
        kb::spawn(&app);
        if let Err(e) = projects::adopt_git_folders(&app).await {
            tracing::warn!("Home Git folders: {e:#}");
        }
        worktrees::spawn(&app);
        app.hosts.set_app(&app);
        hosts::preflight::spawn(&app);
        Ok(app)
    }
}

impl App {
    /// The `launchpad` invocation given to agents: this daemon's CLI, or the helper's on an SSH host.
    pub fn cli_command_on(&self, host: Option<&str>) -> String {
        match host {
            Some(_) => format!("LAUNCHPAD_ENV={} \"$HOME/{}/launchpad\"", self.paths.app_env.name(), hosts::helper::BIN_DIR),
            None => self.paths.cli_command(),
        }
    }

    /// The `launchpad` invocation for agents working in `path`.
    pub fn cli_command_in(&self, path: &str) -> String {
        self.cli_command_on(hosts::loc::host_of(path))
    }
}

/// Writes the TypeScript versions of the types shared with the UI (ts-rs).
pub fn gen_types(out: &std::path::Path) -> Result<()> {
    use ts_rs::TS;
    std::fs::create_dir_all(out)?;
    // Timestamps are i64 in Rust; they fit in a JS number.
    let cfg = ts_rs::Config::new().with_out_dir(out).with_large_int("number");
    predrafts::Predraft::export_all(&cfg)?;
    suggestions::Suggestion::export_all(&cfg)?;
    config::Config::export_all(&cfg)?;
    status::Status::export_all(&cfg)?;
    analytics::AgentRun::export_all(&cfg)?;
    signin::SignInCheck::export_all(&cfg)?;
    model::Session::export_all(&cfg)?;
    model::MessageContext::export_all(&cfg)?;
    model::SessionProcess::export_all(&cfg)?;
    model::SessionEvent::export_all(&cfg)?;
    session_pages::SessionEventPage::export_all(&cfg)?;
    model::Project::export_all(&cfg)?;
    model::HarnessDefaults::export_all(&cfg)?;
    harness::HarnessDescriptor::export_all(&cfg)?;
    projects::Discovered::export_all(&cfg)?;
    projects::ProjectPatch::export_all(&cfg)?;
    sessions::Preview::export_all(&cfg)?;
    schedules::ScheduledLaunch::export_all(&cfg)?;
    crons::CronAgent::export_all(&cfg)?;
    github::GhItem::export_all(&cfg)?;
    project_clone::CloneSuggestion::export_all(&cfg)?;
    inbox::InboxSnapshot::export_all(&cfg)?;
    inbox::ThreadPreview::export_all(&cfg)?;
    terminals::TerminalInfo::export_all(&cfg)?;
    terminals::TerminalFolders::export_all(&cfg)?;
    code::CodeSources::export_all(&cfg)?;
    code::FileList::export_all(&cfg)?;
    prompt_context::PathResults::export_all(&cfg)?;
    prompt_context::SkillResults::export_all(&cfg)?;
    code::CodeFile::export_all(&cfg)?;
    code::SearchResult::export_all(&cfg)?;
    code::CommitInfo::export_all(&cfg)?;
    session_git::GitFile::export_all(&cfg)?;
    session_git::GitStatus::export_all(&cfg)?;
    session_git::GitDiff::export_all(&cfg)?;
    session_git::GitHistory::export_all(&cfg)?;
    workspace::Workspace::export_all(&cfg)?;
    runners::RunnerStatus::export_all(&cfg)?;
    vault::VaultOverview::export_all(&cfg)?;
    vault::ToolTest::export_all(&cfg)?;
    vault::LeaseInfo::export_all(&cfg)?;
    proxy::Access::export_all(&cfg)?;
    home_link::RemoteLaunch::export_all(&cfg)?;
    home_backup::BackupInfo::export_all(&cfg)?;
    worktrees::WorktreeInfo::export_all(&cfg)?;
    home_git::HomeBranch::export_all(&cfg)?;
    home_git::MergeOutcome::export_all(&cfg)?;
    plain::Change::export_all(&cfg)?;
    plain::Tree::export_all(&cfg)?;
    plain::AssetInfo::export_all(&cfg)?;
    kb::Notes::export_all(&cfg)?;
    kb::Hit::export_all(&cfg)?;
    kb::NativeMemory::export_all(&cfg)?;
    hosts::HostStatus::export_all(&cfg)?;
    hosts::sshconfig::SshConfigHost::export_all(&cfg)?;
    proposals::Proposal::export_all(&cfg)?;
    bots::Bot::export_all(&cfg)?;
    onboarding::OnboardingState::export_all(&cfg)?;
    setup_checklist::Checklist::export_all(&cfg)?;
    setup_checklist::ChecklistSummary::export_all(&cfg)?;
    Ok(())
}


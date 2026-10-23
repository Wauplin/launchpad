//! Domain types and services of the Launchpad daemon.

pub mod attachments;
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
pub mod operations;
pub mod paths;
pub mod predrafts;
pub mod procs;
pub mod push;
pub mod project_clone;
pub mod projects;
pub mod schedules;
pub mod session_pages;
pub mod sessions;
pub mod shellenv;
pub mod signin;
pub mod spend;
pub mod status;
pub mod quota;
pub mod quota_history;
pub mod analytics;
pub mod review_state;
pub mod suggestions;
pub mod terminals;
mod titles;
pub mod util;
pub mod profiles;
pub mod session_git;
pub mod workspace;
pub mod worktrees;

/// Source revision embedded in the running binary.
pub const COMMIT: &str = env!("LAUNCHPAD_COMMIT");
/// Main checkout used to find the repository skill in source builds.
pub const SOURCE: &str = env!("LAUNCHPAD_SOURCE");

use std::sync::Arc;

use anyhow::Result;

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
}

impl App {
    /// Opens the databases and the config, and starts the background services.
    pub async fn start(paths: paths::Paths) -> Result<App> {
        paths.ensure_dirs()?;
        attachments::init(&paths.data_dir);
        let paths = Arc::new(paths);
        let bus = events::Bus::new();
        let config = config::ConfigHandle::load(&paths, bus.clone())?;
        let state_db = db::open_state(&paths.state_db())?;
        let cache_db = db::open_cache(&paths.cache_db())?;
        let env = shellenv::UserEnv::resolve().await;
        let status = status::StatusHandle::with_quota_history(bus.clone(), state_db.clone());
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
        };
        predrafts::link_skill()?;
        app.sessions.load(&app).await?;
        schedules::recover(&app).await?;
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
        crons::spawn(&app);
        status::spawn_monitors(&app);
        quota::spawn(&app);
        app.harnesses.spawn_services(&app);
        harness::spawn_model_refresh(&app);
        inbox::spawn(&app);
        projects::spawn(&app);
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
    quota_history::WeeklyReset::export_all(&cfg)?;
    analytics::AgentRun::export_all(&cfg)?;
    signin::SignInCheck::export_all(&cfg)?;
    model::Session::export_all(&cfg)?;
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
    inbox::InboxSnapshot::export_all(&cfg)?;
    inbox::ThreadPreview::export_all(&cfg)?;
    terminals::TerminalInfo::export_all(&cfg)?;
    code::CodeSources::export_all(&cfg)?;
    code::FileList::export_all(&cfg)?;
    code::CodeFile::export_all(&cfg)?;
    code::SearchResult::export_all(&cfg)?;
    code::CommitInfo::export_all(&cfg)?;
    session_git::GitFile::export_all(&cfg)?;
    session_git::GitStatus::export_all(&cfg)?;
    session_git::GitDiff::export_all(&cfg)?;
    session_git::GitHistory::export_all(&cfg)?;
    workspace::Workspace::export_all(&cfg)?;
    worktrees::WorktreeInfo::export_all(&cfg)?;
    hosts::HostStatus::export_all(&cfg)?;
    hosts::sshconfig::SshConfigHost::export_all(&cfg)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod worktree_merge_tests;

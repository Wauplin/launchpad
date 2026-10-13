//! Domain types and services of the Launchpad daemon.

pub mod attachments;
pub mod code;
pub mod compose;
pub mod config;
pub mod crons;
pub mod db;
pub mod drafts;
pub mod events;
pub mod git;
pub mod github;
pub mod harness;
pub mod inbox;
pub mod items;
pub mod model;
pub mod notify;
pub mod operations;
pub mod paths;
pub mod predrafts;
pub mod procs;
pub mod project_clone;
pub mod projects;
pub mod schedules;
pub mod session_pages;
pub mod sessions;
pub mod shellenv;
pub mod signin;
pub mod status;
pub mod quota;
pub mod review_state;
pub mod suggestions;
pub mod terminals;
mod titles;
pub mod util;
pub mod variants;
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
    pub predraft_lock: Arc<tokio::sync::Mutex<()>>,
    /// Serializes task worktree creation, merges and cleanup. Take the session action lock first.
    pub worktree_lock: Arc<tokio::sync::Mutex<()>>,
    pub terminals: terminals::Terminals,
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
            inbox: inbox::Inbox::new(),
            models: harness::ModelCache::default(),
            predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
            worktree_lock: Arc::new(tokio::sync::Mutex::new(())),
            terminals: terminals::Terminals::default(),
        };
        predrafts::link_skill()?;
        app.sessions.load(&app).await?;
        schedules::recover(&app).await?;
        schedules::spawn(&app);
        {
            let app = app.clone();
            tokio::spawn(async move { if let Err(e) = attachments::collect_garbage(&app).await { tracing::warn!("upload cleanup: {e:#}"); } });
        }
        crons::spawn(&app);
        status::spawn_monitors(&app);
        quota::spawn(&app);
        app.harnesses.spawn_services(&app);
        harness::spawn_model_refresh(&app);
        inbox::spawn(&app);
        projects::spawn(&app);
        worktrees::spawn(&app);
        Ok(app)
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
    signin::SignInCheck::export_all(&cfg)?;
    model::Session::export_all(&cfg)?;
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
    workspace::Workspace::export_all(&cfg)?;
    worktrees::WorktreeInfo::export_all(&cfg)?;
    Ok(())
}

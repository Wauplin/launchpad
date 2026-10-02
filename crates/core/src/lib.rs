//! Domain types and services of the Launchpad daemon.

pub mod code;
pub mod codex_rpc;
pub mod compose;
pub mod config;
pub mod db;
pub mod drafts;
pub mod events;
pub mod git;
pub mod github;
pub mod harness;
pub mod history;
pub mod inbox;
pub mod items;
pub mod model;
pub mod notify;
pub mod paths;
pub mod predrafts;
pub mod procs;
pub mod projects;
pub mod schedules;
pub mod sessions;
pub mod shellenv;
pub mod status;
pub mod terminal;
mod titles;
pub mod util;
pub mod workspace;

/// Source revision embedded in the running binary, and the main checkout used for updates.
pub const COMMIT: &str = env!("LAUNCHPAD_COMMIT");
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
    pub codex: codex_rpc::CodexServer,
    pub sessions: sessions::Sessions,
    pub github: github::GitHub,
    pub inbox: inbox::Inbox,
    pub models: harness::ModelCache,
    pub predraft_lock: Arc<tokio::sync::Mutex<()>>,
}

impl App {
    /// Opens the databases and the config, and starts the background services.
    pub async fn start(paths: paths::Paths) -> Result<App> {
        paths.ensure_dirs()?;
        let paths = Arc::new(paths);
        let bus = events::Bus::new();
        let config = config::ConfigHandle::load(&paths, bus.clone())?;
        let state_db = db::open_state(&paths.state_db())?;
        let cache_db = db::open_cache(&paths.cache_db())?;
        let env = shellenv::UserEnv::resolve().await;
        let status = status::StatusHandle::new(bus.clone());
        let codex = codex_rpc::CodexServer::new(env.clone());
        let github = github::GitHub::new(env.clone(), cache_db.clone());
        let app = App {
            paths,
            config,
            state_db,
            cache_db,
            bus,
            env,
            status,
            codex,
            sessions: sessions::Sessions::new(),
            github,
            inbox: inbox::Inbox::new(),
            models: harness::ModelCache::default(),
            predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        predrafts::link_skill()?;
        app.sessions.load(&app).await?;
        schedules::recover(&app).await?;
        schedules::spawn(&app);
        status::spawn_monitors(&app);
        harness::codex::spawn_router(&app);
        harness::spawn_model_refresh(&app);
        harness::spawn_codex_updates(&app);
        inbox::spawn(&app);
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
    config::Config::export_all(&cfg)?;
    status::Status::export_all(&cfg)?;
    model::Session::export_all(&cfg)?;
    model::SessionEvent::export_all(&cfg)?;
    model::Project::export_all(&cfg)?;
    model::HarnessDefaults::export_all(&cfg)?;
    projects::Discovered::export_all(&cfg)?;
    projects::ProjectPatch::export_all(&cfg)?;
    sessions::Preview::export_all(&cfg)?;
    schedules::ScheduledLaunch::export_all(&cfg)?;
    github::GhItem::export_all(&cfg)?;
    inbox::InboxSnapshot::export_all(&cfg)?;
    inbox::ThreadPreview::export_all(&cfg)?;
    terminal::TerminalOption::export_all(&cfg)?;
    code::CodeSources::export_all(&cfg)?;
    code::FileList::export_all(&cfg)?;
    code::CodeFile::export_all(&cfg)?;
    code::SearchResult::export_all(&cfg)?;
    code::CommitInfo::export_all(&cfg)?;
    history::Candidate::export_all(&cfg)?;
    history::Page::export_all(&cfg)?;
    workspace::GitFile::export_all(&cfg)?;
    workspace::GitStatus::export_all(&cfg)?;
    workspace::GitDiff::export_all(&cfg)?;
    Ok(())
}

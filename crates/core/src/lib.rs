//! Domain types and services of the Launchpad daemon.

pub mod codex_rpc;
pub mod config;
pub mod db;
pub mod events;
pub mod paths;
pub mod procs;
pub mod shellenv;
pub mod status;
pub mod util;

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
        let app = App { paths, config, state_db, cache_db, bus, env, status, codex };
        status::spawn_monitors(&app);
        Ok(app)
    }
}

/// Writes the TypeScript versions of the types shared with the UI (ts-rs).
pub fn gen_types(out: &std::path::Path) -> Result<()> {
    use ts_rs::TS;
    std::fs::create_dir_all(out)?;
    // Timestamps are i64 in Rust; they fit in a JS number.
    let cfg = ts_rs::Config::new().with_out_dir(out).with_large_int("number");
    config::Config::export_all(&cfg)?;
    status::Status::export_all(&cfg)?;
    Ok(())
}

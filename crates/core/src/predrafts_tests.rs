//! Local PR checks and the shared isolated fixture: no harnesses, remotes or background monitors.
use super::*;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
async fn fixture() -> (Fixture, App, Predraft) {
    let root = std::env::temp_dir().join(format!("launchpad-predraft-{}", uuid::Uuid::new_v4()));
    let paths = Arc::new(crate::paths::Paths {
        app_env: crate::paths::AppEnv::Dev,
        config_dir: root.join("config"), data_dir: root.join("data"), port: 0, role: Default::default(),
    });
    paths.ensure_dirs().unwrap();
    let bus = crate::events::Bus::new();
    let config = crate::config::ConfigHandle::load(&paths, bus.clone()).unwrap();
    let state_db = crate::db::open_state(&paths.state_db()).unwrap();
    let cache_db = crate::db::open_cache(&paths.cache_db()).unwrap();
    let env = crate::shellenv::UserEnv::resolve().await;
    let status = crate::status::StatusHandle::new(bus.clone());
    let hosts = crate::hosts::Hosts::start(paths.clone(), env.clone(), state_db.clone(), status.clone()).await.unwrap();
    let app = App {
        browser: Default::default(),
        trace_backup: Default::default(),
        runners: Default::default(),
        home_backups: Default::default(),
        proxy: Default::default(),
        push: crate::push::Push::new(paths.clone()), paths, config, state_db,
        github: crate::github::GitHub::new(env.clone(), cache_db.clone()), cache_db,
        harnesses: crate::harness::Registry::new(&env), env, status, bus, hosts,
        sessions: crate::sessions::Sessions::new(), inbox: crate::inbox::Inbox::new(),
        models: Default::default(), suggestion_locks: Default::default(), predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
        worktree_lock: Arc::new(tokio::sync::Mutex::new(())), terminals: Default::default(),
    };
    let p = Predraft {
        id: "draft".into(), project_id: "removed-project".into(), branch: "lp/test".into(),
        base_branch: "main".into(), initial_base: "head".into(), title: "Test".into(), body: String::new(),
        session_id: None, notes: vec![Note { path: None, text: "Fix it".into() }],
        archived: false, published_url: None, published_number: None, default_remote_branch: "test".into(),
        remote_branch: None, pushed_head: None, push_repo: None, created_at: 0, remote_sync_note: None, merged: None, change_set: None,
    };
    save(&app, &p).await.unwrap();
    let cwd = root.join("worktree").to_string_lossy().into_owned();
    app.state_db.call(move |c| {
        c.execute("INSERT INTO session (id,harness,cwd,title,status,launch_spec_json,prompt,started_at) VALUES ('agent','claude',?1,'Test','waiting','{}','',0)", [cwd])?;
        Ok(())
    }).await.unwrap();
    app.sessions.load(&app).await.unwrap();
    (Fixture(root), app, p)
}

#[path = "group10_tests.rs"]
mod group10;

#[test]
fn fork_url_keeps_the_origin_scheme() {
    use super::fork_url;
    assert_eq!(fork_url("git@github.com:huggingface/lerobot.git", "huggingface/lerobot", "Wauplin/lerobot").as_deref(), Some("git@github.com:Wauplin/lerobot.git"));
    assert_eq!(fork_url("https://github.com/huggingface/lerobot/", "huggingface/lerobot", "Wauplin/lerobot").as_deref(), Some("https://github.com/Wauplin/lerobot"));
    assert_eq!(fork_url("https://github.com/other/name.git", "huggingface/lerobot", "Wauplin/lerobot"), None);
}

#[test]
fn change_set_links_replace_the_previous_list() {
    let once = super::with_links("Body", &["a".into()]);
    assert_eq!(super::with_links(&once, &["a".into(), "b".into()]), "Body\n\n---\nPart of a change set: a, b");
    assert_eq!(super::with_links(&once, &[]), "Body");
}

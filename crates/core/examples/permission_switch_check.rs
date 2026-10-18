//! Manual native regression check: requires signed-in Claude and Codex CLIs.
//! Uses isolated state/folders and no monitors; native turns use the owner's accounts.
use anyhow::{Result, Context};
use launchpad_core as core;
use core::model::*;
use std::{sync::Arc, time::Duration};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn wait(app: &core::App, id: &str, question: bool) -> Result<Session> {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let s = app.sessions.get(id).unwrap();
            anyhow::ensure!(s.status != SessionStatus::Error, "native error: {:?}", app.sessions.events(app, id, 0).await?);
            if if question { s.open_question } else { s.status == SessionStatus::Waiting && !s.open_question } { return Ok(s); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.context("waiting for native session")?
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::env::temp_dir().join(format!("lp-permissions-{}", uuid::Uuid::new_v4()));
    let _cleanup = Scratch(root.clone());
    let paths = core::paths::Paths { profile: core::paths::Profile::Dev, config_dir: root.join("config"), data_dir: root.join("data"), port: 17349 };
    paths.ensure_dirs()?;
    let bus = core::events::Bus::new();
    let config = core::config::ConfigHandle::load(&paths, bus.clone())?;
    let mut settings = config.get();
    settings.agents.notify = false;
    settings.projects_root = root.join("projects").to_string_lossy().into();
    // A variant has its own on-demand Codex event router, so the fixture needs no
    // global services (quota monitors, CLI updates or GitHub synchronization).
    settings.variants.push(core::config::HarnessVariant {
        id: "permission-check".into(), name: "Permission check".into(),
        harness: HarnessKind::parse("codex").unwrap(),
        env: [("PERMISSION_CHECK_FIXTURE".into(), "1".into())].into(),
        model: None, effort: None, billing: Default::default(), budget_usd: None,
    });
    config.save(settings)?;
    let state_db = core::db::open_state(&paths.state_db())?;
    let cache_db = core::db::open_cache(&paths.cache_db())?;
    let env = core::shellenv::UserEnv::resolve().await;
    let status = core::status::StatusHandle::new(bus.clone());
    let hosts = core::hosts::Hosts::start(Arc::new(paths.clone()), env.clone(), state_db.clone(), status.clone()).await?;
    let app = core::App {
        paths: Arc::new(paths.clone()), config, state_db, cache_db: cache_db.clone(), status, hosts,
        harnesses: core::harness::Registry::new(&env), github: core::github::GitHub::new(env.clone(), cache_db), env, bus,
        sessions: core::sessions::Sessions::new(), inbox: core::inbox::Inbox::new(), models: Default::default(),
        predraft_lock: Arc::new(tokio::sync::Mutex::new(())), worktree_lock: Arc::new(tokio::sync::Mutex::new(())),
        terminals: Default::default(), push: core::push::Push::new(Arc::new(paths)),
    };
    let result = async {
        for name in ["codex", "claude"] {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder)?;
            let project = core::projects::register(&app, &folder, false).await?;
            let spec = LaunchSpec { harness: Some(HarnessKind::parse(name).unwrap()), variant: (name == "codex").then(|| "permission-check".into()), permission_mode: PermissionMode::Approvals,
                target: Target::Project { project_id: project.id },
                text: "Run a shell command to write the text success into a file called permission-check.txt in the current directory. Use the shell, not an edit tool. Then reply done. Do not do anything else.".into(), ..Default::default() };
            let session = app.sessions.launch(&app, spec).await?;
            let waiting = wait(&app, &session.id, true).await?;
            let native_id = waiting.harness_session_id.clone();
            anyhow::ensure!(native_id.is_some());
            let old_request = app.sessions.events(&app, &session.id, 0).await?.into_iter().rev().find_map(|e| match e.body {
                EventBody::AskUser { request_id, questions } if questions.iter().any(|q| q.permission) => Some(request_id), _ => None,
            }).context("missing approval request")?;
            anyhow::ensure!(!folder.join("permission-check.txt").exists());
            let changed = app.sessions.set_permissions(&app, &session.id, PermissionMode::Yolo).await?;
            anyhow::ensure!(changed.harness_session_id == native_id);
            anyhow::ensure!(changed.launch_spec.permission_mode == PermissionMode::Yolo);
            anyhow::ensure!(app.sessions.answer(&app, &session.id, &old_request, vec!["Allow once".into()]).await.is_err());
            let done = wait(&app, &session.id, false).await?;
            anyhow::ensure!(done.harness_session_id == native_id);
            anyhow::ensure!(folder.join("permission-check.txt").exists());
            println!("{name}: pending approval -> YOLO continued with same native ID and completed file write");
            app.sessions.set_permissions(&app, &session.id, PermissionMode::Approvals).await?;
            app.sessions.send(&app, &session.id, "Run a shell command to write success into permission-second.txt. Use the shell. Then reply done.", vec![]).await?;
            wait(&app, &session.id, true).await?;
            anyhow::ensure!(!folder.join("permission-second.txt").exists());
            println!("{name}: switched back to manual approvals; next command blocked");
            let auto = app.sessions.set_permissions(&app, &session.id, PermissionMode::Auto).await?;
            anyhow::ensure!(auto.harness_session_id == native_id);
            let done = wait(&app, &session.id, false).await?;
            anyhow::ensure!(done.launch_spec.permission_mode == PermissionMode::Auto);
            anyhow::ensure!(folder.join("permission-second.txt").exists());
            println!("{name}: pending approval -> Auto continued and completed file write");
            app.sessions.close(&app, &session.id).await?;
            app.sessions.set_permissions(&app, &session.id, PermissionMode::Approvals).await?;
            let restored = core::sessions::Sessions::new();
            restored.load(&app).await?;
            anyhow::ensure!(restored.get(&session.id).unwrap().launch_spec.permission_mode == PermissionMode::Approvals);
            println!("{name}: archived permission change persisted through reload");
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    if result.is_err() {
        for s in app.sessions.list() {
            eprintln!("{}: status={:?}, question={}", s.harness.as_str(), s.status, s.open_question);
            for event in app.sessions.events(&app, &s.id, 0).await? { eprintln!("{:?}", event.body); }
        }
    }
    for s in app.sessions.list() { let _ = app.sessions.close(&app, &s.id).await; }
    tokio::time::sleep(Duration::from_secs(6)).await;
    std::fs::remove_dir_all(&root)?;
    result
}

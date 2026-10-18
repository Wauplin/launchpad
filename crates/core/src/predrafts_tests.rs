//! Isolated pre-draft guard and lock regressions: no harnesses, remotes or background monitors.
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
        profile: crate::paths::Profile::Dev,
        config_dir: root.join("config"), data_dir: root.join("data"), port: 0,
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
        push: crate::push::Push::new(paths.clone()), paths, config, state_db,
        github: crate::github::GitHub::new(env.clone(), cache_db.clone()), cache_db,
        harnesses: crate::harness::Registry::new(&env), env, status, bus, hosts,
        sessions: crate::sessions::Sessions::new(), inbox: crate::inbox::Inbox::new(),
        models: Default::default(), predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
        worktree_lock: Arc::new(tokio::sync::Mutex::new(())), terminals: Default::default(),
    };
    let p = Predraft {
        id: "draft".into(), project_id: "removed-project".into(),
        root: root.join("worktree").to_string_lossy().into_owned(), branch: "lp/test".into(),
        base_branch: "main".into(), initial_base: "head".into(), title: "Test".into(), body: String::new(),
        session_id: None, notes: vec![Note { path: None, text: "Fix it".into() }],
        archived: false, published_url: None, published_number: None, default_remote_branch: "test".into(),
        remote_branch: None, pushed_head: None, created_at: 0,
    };
    save(&app, &p).await.unwrap();
    let cwd = Path::new(&p.root).join("subdir").to_string_lossy().into_owned();
    app.state_db.call(move |c| {
        c.execute("INSERT INTO session (id,harness,cwd,title,status,launch_spec_json,prompt,started_at) VALUES ('agent','claude',?1,'Test','waiting','{}','',0)", [cwd])?;
        Ok(())
    }).await.unwrap();
    app.sessions.load(&app).await.unwrap();
    (Fixture(root), app, p)
}

#[tokio::test]
async fn subfolder_guards_cover_activity_publication_feedback_and_recovery() {
    let (_fixture, app, mut p) = fixture().await;
    assert_eq!(get(&app, &p.id).await.unwrap().session_id.as_deref(), Some("agent"));
    app.sessions.update(&app, "agent", |s| s.status = SessionStatus::Working).await.unwrap();
    assert!(idle(&app, &p).unwrap_err().to_string().contains("active agent"));
    let err = publish(&app, &p.id, Publish { remote_branch: "test".into(), draft: true, head: "head".into() }).await.unwrap_err();
    assert!(err.to_string().contains("active agent"));
    assert!(discard(&app, &p.id).await.unwrap_err().to_string().contains("active agent"));
    app.sessions.update(&app, "agent", |s| s.status = SessionStatus::InTerminal).await.unwrap();
    assert!(idle(&app, &p).is_err());
    app.sessions.update(&app, "agent", |s| s.status = SessionStatus::Waiting).await.unwrap();
    idle(&app, &p).unwrap();
    let cwd = app.sessions.get("agent").unwrap().cwd;
    check_session_work(&app, &cwd).await.unwrap();
    p.pushed_head = Some("head".into());
    save(&app, &p).await.unwrap();
    assert!(check_session_work(&app, &cwd).await.is_err());
    let followup = crate::model::LaunchSpec {
        target: crate::model::Target::SessionFolder { session_id: "agent".into() },
        ..Default::default()
    };
    assert!(app.sessions.preview(&app, &followup).await.unwrap_err().to_string().contains("publication has started"));
    assert!(app.sessions.launch_with_id(&app, followup, "followup".into()).await.unwrap_err().to_string().contains("publication has started"));
    assert!(app.sessions.resume(&app, "agent", false, false).await.unwrap_err().to_string().contains("publication has started"));
    // Reaches send's publication guard, rather than rejecting the linked subfolder session.
    assert!(feedback(&app, &p.id).await.unwrap_err().to_string().contains("publication has started"));
    check_session_work(&app, &format!("{}-other/subdir", p.root)).await.unwrap();
    p.pushed_head = None;
    p.archived = true;
    save(&app, &p).await.unwrap();
    assert!(check_session_work(&app, &cwd).await.is_err());
}

#[tokio::test]
async fn discard_and_publish_wait_for_actions_before_taking_the_work_gate() {
    let (_fixture, app, p) = fixture().await;
    for publishing in [false, true] {
        let action = app.sessions.action_lock().await;
        let task_app = app.clone();
        let id = p.id.clone();
        let task = tokio::spawn(async move {
            if publishing {
                publish(&task_app, &id, Publish { remote_branch: "test".into(), draft: true, head: "head".into() }).await
            } else { discard(&task_app, &id).await }
        });
        // A concurrent send/launch already holding actions must still acquire work.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let work = timeout(Duration::from_secs(2), app.sessions.allow_work()).await.expect("lock inversion blocks session actions");
        assert!(!task.is_finished());
        drop(work);
        drop(action);
        let result = timeout(Duration::from_secs(2), task).await.expect("cleanup recursively acquired actions").unwrap();
        if publishing {
            assert!(result.is_err()); // Archived record, no GitHub calls.
        } else {
            assert!(result.unwrap().archived);
            assert_eq!(app.sessions.get("agent").unwrap().status, SessionStatus::Closed);
        }
    }
}

#[test]
fn folder_containment_respects_components_and_hosts() {
    use crate::hosts::loc::within;
    for root in ["/repo/worktree", "devbox:/repo/worktree"] {
        assert!(within(root, root));
        assert!(within(&format!("{root}/sub/nested"), root));
        assert!(within(&format!("{root}/sub"), &format!("{root}/")));
        assert!(!within(&format!("{root}-other/sub"), root));
        assert!(!within("other:/repo/worktree/sub", root));
    }
    assert!(!within("/repo/worktree/sub", "devbox:/repo/worktree"));
    #[cfg(windows)]
    assert!(within(r"C:\repo\worktree\sub", r"C:\repo\worktree"));
}

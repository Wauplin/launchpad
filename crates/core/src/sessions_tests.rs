#[tokio::test]
async fn session_snapshots_queue_before_polling_and_preserve_event_metadata() {
    let path = std::env::temp_dir().join(format!("launchpad-session-order-{}.db", uuid::Uuid::new_v4()));
    let db = crate::db::open_state(&path).unwrap();
    let mut session: crate::model::Session = serde_json::from_value(serde_json::json!({
        "id": "test", "harness": "codex", "cwd": "/tmp", "title": "task",
        "status": "working", "status_since": 1, "live": false, "open_question": false,
        "prompt": "task", "usage": {"input_tokens": 0, "cached_tokens": 0, "output_tokens": 0}, "started_at": 1, "last_seq": 0,
        "launch_spec": {}, "seen_at": 0
    })).unwrap();
    super::Sessions::persist(&db, &session).await.unwrap();
    let first = super::Sessions::persist(&db, &session);
    session.status = crate::model::SessionStatus::Waiting;
    let second = super::Sessions::persist(&db, &session);
    // Await in reverse order: queuing must already have happened at snapshot time.
    second.await.unwrap();
    first.await.unwrap();
    db.call(|c| {
        let status: String = c.query_row("SELECT status FROM session WHERE id = 'test'", [], |r| r.get(0))?;
        assert_eq!(status, "waiting");
        c.execute("INSERT INTO session_event VALUES ('test', 1, 1, 'assistant_message', '{}')", [])?;
        c.execute("UPDATE session SET last_seq = 1, last_message = 'final' WHERE id = 'test'", [])?;
        Ok(())
    }).await.unwrap();
    super::Sessions::persist(&db, &session).await.unwrap();
    db.call(|c| {
        let row: (String, i64, String) = c.query_row(
            "SELECT status, last_seq, last_message FROM session WHERE id = 'test'", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        assert_eq!(row, ("waiting".into(), 1, "final".into()));
        Ok(())
    }).await.unwrap();
    drop(db);
    let reopened = crate::db::open_state(&path).unwrap();
    reopened.call(|c| {
        let seq: i64 = c.query_row("SELECT last_seq FROM session WHERE id = 'test'", [], |r| r.get(0))?;
        c.execute("INSERT INTO session_event VALUES ('test', ?1, 2, 'status', '{}')", [seq + 1])?;
        Ok(())
    }).await.unwrap();
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn truncate_unicode_output_without_overlapping_ends() {
    let text = "🦀".repeat(30);
    let result = super::truncate_output(&text, 100);
    assert!(result.starts_with("🦀"));
    assert!(result.ends_with("🦀"));
    assert!(result.contains("24 bytes omitted"));
    assert_eq!(super::truncate_output("small", 100), "small");
    assert!(super::truncate_output("🦀", 1).contains("4 bytes omitted"));
}

// Build isolated state without starting daemon monitors, harnesses or network services.
struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn fixture() -> (Fixture, crate::App) {
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("launchpad-session-events-{}", uuid::Uuid::new_v4()));
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
    let app = crate::App {
        push: crate::push::Push::new(paths.clone()), paths, config, state_db,
        github: crate::github::GitHub::new(env.clone(), cache_db.clone()), cache_db,
        harnesses: crate::harness::Registry::new(&env), env, status, bus, hosts,
        sessions: super::Sessions::new(), inbox: crate::inbox::Inbox::new(),
        models: Default::default(), predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
        worktree_lock: Arc::new(tokio::sync::Mutex::new(())), terminals: Default::default(),
    };
    app.state_db.call(|c| {
        c.execute("INSERT INTO session (id,harness,cwd,title,status,launch_spec_json,prompt,started_at)
            VALUES ('test','codex','/tmp','task','waiting','{}','task',0)", [])?;
        Ok(())
    }).await.unwrap();
    app.sessions.load(&app).await.unwrap();
    (Fixture(root), app)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_events_and_snapshots_remain_ordered_after_restart() {
    use crate::model::EventBody;
    let (_fixture, app) = fixture().await;
    let mut viewer = app.bus.subscribe(None).rx;
    let mut tasks = Vec::new();
    for i in 0..100 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            app.sessions.sink(&app, "test").emit(EventBody::AssistantMessage {
                item_id: format!("m{i}"), text: format!("message {i}"),
            }).await;
            app.sessions.update(&app, "test", |s| s.title = format!("title {i}")).await.unwrap();
        }));
    }
    for task in tasks { task.await.unwrap(); }
    let mut delivered = Vec::new();
    while let Ok(event) = viewer.try_recv() {
        let event: serde_json::Value = serde_json::from_str(&event.text).unwrap();
        if event["topic"] == "session/test" { delivered.push(event["seq"].as_i64().unwrap()); }
    }
    assert_eq!(delivered, (1..=100).collect::<Vec<_>>());
    let expected = app.sessions.get("test").unwrap();
    app.state_db.call(move |c| {
        let row: (i64, String, String) = c.query_row(
            "SELECT last_seq, last_message, title FROM session WHERE id = 'test'", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        assert_eq!(row, (100, expected.last_message.unwrap(), expected.title));
        let count: i64 = c.query_row("SELECT count(*) FROM session_event WHERE session_id = 'test'", [], |r| r.get(0))?;
        assert_eq!(count, 100);
        Ok(())
    }).await.unwrap();
    // A new registry reading the same file simulates restart without launching any services.
    let restarted = crate::App {
        state_db: crate::db::open_state(&app.paths.state_db()).unwrap(),
        sessions: super::Sessions::new(),
        ..app.clone()
    };
    restarted.sessions.load(&restarted).await.unwrap();
    restarted.sessions.sink(&restarted, "test").emit(EventBody::AssistantMessage {
        item_id: "next".into(), text: "after restart".into(),
    }).await;
    restarted.state_db.call(|c| {
        let (seq, count): (i64, i64) = c.query_row(
            "SELECT last_seq, (SELECT count(*) FROM session_event WHERE session_id = 'test') FROM session WHERE id = 'test'",
            [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        assert_eq!((seq, count), (101, 101));
        Ok(())
    }).await.unwrap();
}

#[test]
fn session_lists_strip_held_images_without_changing_the_saved_input() {
    let session: crate::model::Session = serde_json::from_value(serde_json::json!({
        "id": "a", "harness": "codex", "cwd": "/tmp", "title": "task",
        "status": "quota_paused", "status_since": 0, "live": false, "open_question": false,
        "prompt": "task", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0}, "started_at": 0, "last_seq": 0,
        "launch_spec": {"attachments": [{"name": "shot.png", "data_url": "data:image/png;base64,AQ=="}]}, "seen_at": 0,
        "quota_pause": {"phase": "paused", "blockers": [], "message": "task", "images": [{"name": "shot.png", "data_url": "data:image/png;base64,AQ=="}]}
    })).unwrap();
    let light = session.light();
    assert!(light.launch_spec.attachments[0].data_url.is_empty());
    let image = &light.quota_pause.unwrap().images[0];
    assert_eq!(image.name, "shot.png");
    assert!(image.data_url.is_empty());
    assert!(!session.quota_pause.unwrap().images[0].data_url.is_empty());
}

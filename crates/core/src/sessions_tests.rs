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

#[tokio::test]
async fn timed_out_shutdowns_retain_ownership_until_all_waits_finish() {
    let sessions = super::Sessions::new();
    let mut finishes = Vec::new();
    for _ in 0..2 {
        let (finish, wait) = tokio::sync::oneshot::channel::<()>();
        finishes.push(finish);
        let action = sessions.action_lock().await;
        let result = sessions.wait_for_shutdown_timeout("task", async move { wait.await?; Ok(()) }, std::time::Duration::from_millis(1)).await;
        assert!(result.is_err());
        drop(action);
        assert!(sessions.inner.actions.try_lock().is_ok());
        assert!(sessions.ensure_shutdown_finished("task").is_err());
    }
    finishes.pop().unwrap().send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while sessions.inner.shutdowns.lock().unwrap().get("task") == Some(&2) { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(sessions.ensure_shutdown_finished("task").is_err());
    finishes.pop().unwrap().send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while sessions.ensure_shutdown_finished("task").is_err() { tokio::task::yield_now().await; }
    }).await.unwrap();
}

#[test]
fn capacity_errors_are_specific() {
    for message in ["Selected model is at capacity. Please try a different model.", "Model is overloaded", "Server is overloaded", "overloaded_error", "Overloaded", "The model is currently overloaded"] {
        assert!(super::is_capacity_error(message), "{message}");
    }
    for message in ["rate limit exceeded", "quota exceeded", "authentication failed", "context capacity exceeded", "tool failed", "network timeout"] {
        assert!(!super::is_capacity_error(message), "{message}");
    }
}

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
        c.execute("INSERT INTO session_event (session_id, seq, ts, kind, json) VALUES ('test', 1, 1, 'assistant_message', '{}')", [])?;
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
        c.execute("INSERT INTO session_event (session_id, seq, ts, kind, json) VALUES ('test', ?1, 2, 'status', '{}')", [seq + 1])?;
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

fn trace_source(id: &str, last_seq: i64) -> crate::model::Session {
    serde_json::from_value(serde_json::json!({
        "id": id, "harness": "codex", "cwd": "/tmp/source", "title": "source task",
        "status": "waiting", "status_since": 0, "live": false, "open_question": false,
        "prompt": "task", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0}, "started_at": 0, "last_seq": last_seq,
        "launch_spec": {}, "seen_at": 0
    })).unwrap()
}

fn trace_ref(id: &str, through_seq: i64) -> crate::model::AgentTraceRef {
    crate::model::AgentTraceRef { session_id: id.into(), through_seq }
}

/// A launch (or a follow-up message: both use `context_traces`) on a runner that never saw the source agent,
/// like a fresh Burst runner: the source comes from Home.
#[tokio::test]
async fn context_traces_of_agents_this_runner_never_ran_come_from_home() {
    let asked = std::sync::Mutex::new(vec![]);
    let traces = super::resolve_traces(&[trace_ref("e95db580", 4702)], |_| None, |id| {
        asked.lock().unwrap().push(id.clone());
        async move { Ok(trace_source(&id, 4702)) }
    }).await.unwrap();
    assert_eq!(*asked.lock().unwrap(), ["e95db580"]);
    assert_eq!((traces[0].0.id.as_str(), traces[0].1), ("e95db580", 4702));
    assert!(crate::compose::trace_references(&traces, "launchpad").contains("`launchpad trace e95db580 --through 4702`"));
}

#[tokio::test]
async fn context_traces_use_a_local_record_only_when_it_covers_the_events() {
    let asked = std::sync::Mutex::new(0);
    let home = |id: String| {
        *asked.lock().unwrap() += 1;
        async move { Ok(trace_source(&id, 50)) }
    };
    // Covered: no request to Home.
    super::resolve_traces(&[trace_ref("a", 20)], |id| Some(trace_source(id, 30)), home).await.unwrap();
    assert_eq!(*asked.lock().unwrap(), 0);
    // A session that moved away: the local record ends before the requested event, Home has more.
    let traces = super::resolve_traces(&[trace_ref("a", 40)], |id| Some(trace_source(id, 30)), home).await.unwrap();
    assert_eq!((*asked.lock().unwrap(), traces[0].0.last_seq), (1, 50));
}

#[tokio::test]
async fn context_trace_errors_name_the_agent_and_what_home_has() {
    let missing = super::resolve_traces(&[trace_ref("gone", 3)], |_| None, |_| async { anyhow::bail!("no such session") }).await.unwrap_err();
    assert_eq!(format!("{missing:#}"), "Could not read context agent gone from Home: no such session");
    let short = super::resolve_traces(&[trace_ref("a", 9)], |_| None, |id| async move { Ok(trace_source(&id, 5)) }).await.unwrap_err();
    assert_eq!(format!("{short:#}"), "The context trace of agent a goes through event 9, but Home has its events through 5 only");
    assert!(super::resolve_traces(&[trace_ref("../x", 1)], |_| None, |id| async move { Ok(trace_source(&id, 5)) }).await.is_err());
}

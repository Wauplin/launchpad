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
        app_env: crate::paths::AppEnv::Dev,
        config_dir: root.join("config"), data_dir: root.join("data"), port: 0,
    });
    paths.ensure_dirs().unwrap();
    // Tests exercise real session transitions, but must not send desktop or push notifications.
    std::fs::write(paths.config_file(), "[agents]\nnotify = false\n").unwrap();
    let bus = crate::events::Bus::new();
    let config = crate::config::ConfigHandle::load(&paths, bus.clone()).unwrap();
    assert!(!config.get().agents.notify, "session tests must not send notifications");
    let state_db = crate::db::open_state(&paths.state_db()).unwrap();
    let cache_db = crate::db::open_cache(&paths.cache_db()).unwrap();
    let env = crate::shellenv::UserEnv::resolve().await;
    let status = crate::status::StatusHandle::new(bus.clone());
    let hosts = crate::hosts::Hosts::start(paths.clone(), env.clone(), state_db.clone(), status.clone()).await.unwrap();
    let app = crate::App {
        browser: Default::default(),
        push: crate::push::Push::new(paths.clone()), paths, config, state_db,
        github: crate::github::GitHub::new(env.clone(), cache_db.clone()), cache_db,
        harnesses: crate::harness::Registry::new(&env), env, status, bus, hosts,
        sessions: super::Sessions::new(), inbox: crate::inbox::Inbox::new(),
        models: Default::default(), suggestion_locks: Default::default(), predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
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

struct QuotaRetryHarness {
    sends: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl super::LiveSession for QuotaRetryHarness {
    async fn send(&self, text: &str, _: &str, images: &[crate::model::ImageAttachment]) -> anyhow::Result<()> {
        assert_eq!(text, "held task");
        assert!(images.is_empty());
        self.sends.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    async fn interrupt(&self) -> anyhow::Result<()> { Ok(()) }
    fn ready_for_turn(&self) -> bool { true }
    fn turn_active(&self) -> bool { false }
    async fn answer(&self, _: &str, _: Vec<String>) -> anyhow::Result<()> { Ok(()) }
    async fn stop(&self) -> anyhow::Result<()> { Ok(()) }
}

#[tokio::test]
async fn quota_reset_fallback_sends_once_and_restores_protection_on_new_evidence() {
    use crate::{model::*, status::QuotaWindow};
    let (_fixture, app) = fixture().await;
    let reset = crate::util::now_ms() / 1000 - 61;
    app.status.update(|st| {
        let s = st.quotas.entry("codex".into()).or_default();
        s.five_hour = Some(QuotaWindow { used_percent: 100.0, resets_at: Some(reset) });
        s.weekly = Some(QuotaWindow { used_percent: 10.0, resets_at: None });
        s.quota_at = Some((reset - 10) * 1000);
        s.quota_error = Some("unknown".into());
    });
    app.sessions.update(&app, "test", |s| {
        s.status = SessionStatus::QuotaPaused;
        s.quota_pause = Some(QuotaPause {
            phase: QuotaPausePhase::Paused,
            blockers: crate::quota::blockers(&app, s.harness, &s.launch_spec),
            message: "held task".into(), images: vec![], client_id: Some("held".into()), error: None,
        });
    }).await.unwrap();
    // The reset survives persistence/reload, even though status is only in memory.
    app.sessions.load(&app).await.unwrap();
    let live = std::sync::Arc::new(QuotaRetryHarness { sends: Default::default() });
    app.sessions.inner.live.lock().unwrap().insert("test".into(), (0, live.clone()));
    app.sessions.set_watched(1, Some((true, vec!["test".into()], None)));
    app.sessions.update(&app, "test", |s| s.quota_pause.as_mut().unwrap().phase = QuotaPausePhase::Resuming).await.unwrap();
    app.sessions.load(&app).await.unwrap();
    app.sessions.check_quotas(&app).await.unwrap();
    assert_eq!(live.sends.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(app.sessions.get("test").unwrap().quota_pause.unwrap().phase, QuotaPausePhase::NeedsAttention);
    app.sessions.update(&app, "test", |s| s.quota_pause.as_mut().unwrap().phase = QuotaPausePhase::Paused).await.unwrap();
    app.sessions.check_quotas(&app).await.unwrap();
    let session = app.sessions.get("test").unwrap();
    assert!(session.quota_pause.is_none());
    assert!(!session.launch_spec.allow_paid_usage);
    assert!(app.sessions.quota_blockers(&app, &session).is_empty());
    app.sessions.check_quotas(&app).await.unwrap();
    assert_eq!(live.sends.load(std::sync::atomic::Ordering::SeqCst), 1);
    app.status.update(|st| st.quotas.entry("codex".into()).or_default().quota_at = Some(crate::util::now_ms()));
    assert_eq!(app.sessions.quota_blockers(&app, &session).len(), 1);
    app.sessions.check_quotas(&app).await.unwrap();
    app.sessions.check_quotas(&app).await.unwrap();
    assert!(app.sessions.get("test").unwrap().quota_pause.is_some());
    assert_eq!(live.sends.load(std::sync::atomic::Ordering::SeqCst), 1);
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

#[tokio::test]
async fn unpriced_costs_are_hidden_after_reload_and_cannot_drive_limits() {
    let (_fixture, app) = fixture().await;
    let mut config = app.config.get();
    for (id, model) in [("hf", "deepseek-ai/DeepSeek-V4.1-Flash-1017"), ("native", "opus")] {
        config.profiles.push(serde_json::from_value(serde_json::json!({
            "id":id,"name":id,"harness":"claude","model":model,"billing":"pay_as_you_go",
            "budget_usd":1.0,"spend_limits":[{"usd":1.0,"hours":5}]
        })).unwrap());
    }
    app.config.save(config).unwrap();
    app.sessions.update(&app, "test", |s| {
        s.harness = crate::harness::claude::ID;
        s.model = Some("deepseek-ai/DeepSeek-V4.1-Flash-1017".into());
        s.launch_spec.harness = Some(crate::harness::claude::ID);
        s.launch_spec.profile = Some("hf".into());
        s.launch_spec.budget_usd = Some(1.0);
        s.usage.cost_usd = Some(62.30);
    }).await.unwrap();
    // The shared fixture starts as Codex; harness identity is immutable in normal session updates.
    app.state_db.call(|c| {
        c.execute("UPDATE session SET harness = 'claude' WHERE id = 'test'", [])?;
        Ok(())
    }).await.unwrap();
    // Previously recorded unsupported costs must not block input or appear as a meter.
    crate::spend::record(&app, "hf", 62.30).await;
    app.sessions.load(&app).await.unwrap();
    let session = app.sessions.get("test").unwrap();
    assert!(session.usage.cost_unavailable);
    assert_eq!(session.usage.cost_usd, None);
    assert!(!session.budget_reached());
    assert!(crate::spend::reached(&app, &session.launch_spec).is_empty());
    assert!(crate::spend::current(&app).iter().all(|s| s.profile != "hf"));
    assert!(app.sessions.set_budget(&app, "test", Some(2.0)).await.unwrap_err().to_string().contains("Cost management is unavailable"));
    app.state_db.call(|c| {
        assert_eq!(c.query_row("SELECT cost_usd FROM session WHERE id = 'test'", [], |r| r.get::<_, Option<f64>>(0))?, None);
        Ok(())
    }).await.unwrap();
    let sink = app.sessions.sink(&app, "test");
    sink.update_usage(|u| { u.cost_usd = Some(100.0); u.output_tokens = 7; }).await;
    let session = app.sessions.get("test").unwrap();
    assert_eq!(session.usage.cost_usd, None);
    assert_eq!(session.usage.output_tokens, 7);
    // Reliable native costs still record and enforce limits normally.
    app.sessions.update(&app, "test", |s| {
        s.model = Some("opus".into());
        s.launch_spec.profile = Some("native".into());
        s.usage.cost_unavailable = false;
    }).await.unwrap();
    sink.update_usage(|u| u.cost_usd = Some(1.5)).await;
    assert!(app.sessions.get("test").unwrap().budget_reached());
    assert_eq!(crate::spend::current(&app).iter().find(|s| s.profile == "native").unwrap().windows[0].used_usd, 1.5);
    // Runtime loss of pricing disables both guards, even for a formerly supported profile.
    sink.update_usage(|u| u.cost_unavailable = true).await;
    assert!(!app.sessions.get("test").unwrap().budget_reached());
    assert!(crate::spend::current(&app).iter().all(|s| s.profile != "native"));
    assert!(super::ensure_budget(&app, &app.sessions.get("test").unwrap()).is_ok());
}

#[tokio::test]
async fn assistant_threads_stay_reachable_without_contributing_to_global_counts() {
    use crate::model::{SessionStatus, Target};
    let (_fixture, app) = fixture().await;
    for status in [SessionStatus::Working, SessionStatus::Waiting, SessionStatus::Error, SessionStatus::QuotaPaused, SessionStatus::Closed] {
        {
            let mut records = app.sessions.inner.records.lock().unwrap();
            let ordinary = records.get_mut("test").unwrap();
            ordinary.status = SessionStatus::Working;
            let mut assistant = ordinary.clone();
            assistant.id = "assistant".into();
            assistant.launch_spec.target = Target::Assistant { workspace_id: None };
            assistant.status = status;
            assistant.seen_at = 0;
            assistant.status_since = 1;
            records.insert(assistant.id.clone(), assistant);
        }
        app.sessions.publish_counts(&app);
        let counts = app.status.get().sessions;
        assert_eq!((counts.running, counts.done, counts.quota_paused), (1, 0, 0));
        assert_eq!(app.sessions.get("assistant").unwrap().status, status);
        assert_eq!(crate::proposals::thread(&app, None).is_some(), status != SessionStatus::Closed);
    }
}

#[tokio::test]
async fn accepted_proposals_close_only_their_author_and_keep_results_retry_safe() {
    use crate::model::{SessionStatus, Target};
    use crate::proposals::{self, Proposal, ProposalBody, ProposalState};
    let (_fixture, app) = fixture().await;
    let original = app.sessions.get("test").unwrap();
    for (id, assistant) in [("author", true), ("newer", true), ("other", false)] {
        let mut session = original.clone();
        session.id = id.into();
        session.started_at = if id == "newer" { 2 } else { 1 };
        if assistant { session.launch_spec.target = Target::Assistant { workspace_id: None }; }
        super::Sessions::persist(&app.state_db, &session).await.unwrap();
    }
    app.sessions.load(&app).await.unwrap();
    // The reserved launch already exists: no harness or network activity is needed.
    let mut proposal = Proposal {
        id: "accept".into(), workspace_id: None, session_id: Some("author".into()),
        title: "Launch".into(), note: None,
        body: ProposalBody::Launch { spec: original.launch_spec.clone() },
        state: ProposalState::Proposed, result_id: Some("test".into()), error: None,
        created_at: 0, updated_at: 0,
    };
    async fn store(app: &crate::App, p: &Proposal) {
        let id = p.id.clone();
        let json = serde_json::to_string(p).unwrap();
        app.state_db.call(move |c| {
            c.execute("INSERT OR REPLACE INTO launch_proposal (id,json) VALUES (?1,?2)", rusqlite::params![id, json])?;
            Ok(())
        }).await.unwrap();
    }
    store(&app, &proposal).await;
    let accepted = proposals::accept(&app, &proposal.id).await.unwrap();
    assert_eq!(accepted.state, ProposalState::Started);
    assert_eq!(accepted.result_id.as_deref(), Some("test"));
    assert_eq!(app.sessions.get("author").unwrap().status, SessionStatus::Closed);
    assert_eq!(proposals::thread(&app, None).unwrap().id, "newer");
    assert_eq!(app.sessions.get("test").unwrap().status, SessionStatus::Waiting);
    // Repeated delivery must not close a thread that the owner deliberately reopened.
    app.sessions.update(&app, "author", |s| s.status = SessionStatus::Waiting).await.unwrap();
    proposals::accept(&app, &proposal.id).await.unwrap();
    assert_eq!(app.sessions.get("author").unwrap().status, SessionStatus::Waiting);

    proposal.id = "failed".into();
    proposal.body = ProposalBody::Message { session_id: "missing".into(), text: "hello".into() };
    store(&app, &proposal).await;
    assert_eq!(proposals::accept(&app, &proposal.id).await.unwrap().state, ProposalState::Failed);
    assert_eq!(app.sessions.get("author").unwrap().status, SessionStatus::Waiting);
    proposal.id = "uncertain".into();
    proposal.state = ProposalState::Uncertain;
    store(&app, &proposal).await;
    assert!(proposals::accept(&app, &proposal.id).await.is_err());
    assert_eq!(app.sessions.get("author").unwrap().status, SessionStatus::Waiting);

    proposal.id = "recovered".into();
    proposal.body = ProposalBody::Launch { spec: original.launch_spec };
    store(&app, &proposal).await;
    assert_eq!(proposals::recheck(&app, &proposal.id).await.unwrap().state, ProposalState::Started);
    assert_eq!(app.sessions.get("author").unwrap().status, SessionStatus::Closed);

    proposal.id = "non-assistant".into();
    proposal.session_id = Some("other".into());
    proposal.state = ProposalState::Proposed;
    store(&app, &proposal).await;
    assert_eq!(proposals::accept(&app, &proposal.id).await.unwrap().state, ProposalState::Started);
    assert_eq!(app.sessions.get("other").unwrap().status, SessionStatus::Waiting);
}

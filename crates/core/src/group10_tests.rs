//! Report group 10 integration checks, using isolated state and a fake loopback GitHub.
use super::*;
use crate::{
    github::publication_tests::mock,
    suggestions::{self, Create, SuggestionKind, SuggestionTarget},
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn suggestion_failures_preserve_safe_retries_without_blocking_predrafts() {
    let (_fixture, mut app, _) = fixture().await;
    for (status, expected) in [(403, "draft"), (500, "uncertain"), (0, "uncertain")] {
        let server = mock(move |_| (status, "{}".into())).await;
        app.github = app.github.clone().with_test_origin(server.origin.clone());
        let s = suggestions::create(
            &app,
            Create {
                target: SuggestionTarget::GitHub {
                    repo: "test/fixture".into(),
                    number: 1,
                },
                kind: SuggestionKind::Comment,
                body: "test".into(),
                event: "COMMENT".into(),
                head: None,
                session_id: None,
            },
        )
        .await
        .unwrap();
        // A GitHub request must complete even while the pre-draft gate is occupied.
        let gate = app.predraft_lock.lock().await;
        assert!(
            timeout(Duration::from_secs(2), suggestions::post(&app, &s.id))
                .await
                .unwrap()
                .is_err()
        );
        drop(gate);
        let saved = suggestions::list(&app)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.id == s.id)
            .unwrap();
        assert_eq!(saved.status, expected);
        assert_eq!(
            suggestions::edit(
                &app,
                &s.id,
                suggestions::Edit {
                    body: "edited".into(),
                    event: "COMMENT".into()
                }
            )
            .await
            .is_ok(),
            expected == "draft"
        );
        if expected == "uncertain" {
            assert!(suggestions::post(&app, &s.id).await.is_err());
        }
    }
    let s = suggestions::create(
        &app,
        Create {
            target: SuggestionTarget::GitHub {
                repo: "test/fixture".into(),
                number: 1,
            },
            kind: SuggestionKind::Review,
            body: "review".into(),
            event: "COMMENT".into(),
            head: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".into()),
            session_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        s.head.as_deref(),
        Some("abcdef0123456789abcdef0123456789abcdef01")
    );
}

#[tokio::test]
async fn concurrent_suggestion_posts_send_once() {
    let (_fixture, mut app, _) = fixture().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = mock(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        (
            200,
            json!({"html_url":"https://example.test/comment"}).to_string(),
        )
    })
    .await;
    app.github = app.github.clone().with_test_origin(server.origin.clone());
    let s = suggestions::create(
        &app,
        Create {
            target: SuggestionTarget::GitHub {
                repo: "test/fixture".into(),
                number: 1,
            },
            kind: SuggestionKind::Comment,
            body: "test".into(),
            event: "COMMENT".into(),
            head: None,
            session_id: None,
        },
    )
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        suggestions::post(&app, &s.id),
        suggestions::post(&app, &s.id)
    );
    assert!(a.is_ok() ^ b.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn full_inbox_polls_reuse_subscriptions_and_skip_thread_errors() {
    let (_fixture, mut app, _) = fixture().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = mock(move |req| {
        if req.starts_with("GET /notifications?") {
            return (200, json!([{"id":"1","repository":{"full_name":"test/fixture"},"subject":{"type":"Other","title":"first"},"updated_at":"2026-10-19","unread":false},
                {"id":"2","repository":{"full_name":"test/fixture"},"subject":{"type":"Other","title":"second"},"updated_at":"2026-10-19","unread":false}]).to_string());
        }
        count.fetch_add(1, Ordering::SeqCst);
        if req.contains("/threads/1/subscription") { (403, "{}".into()) }
        else { (200, json!({"subscribed":true,"ignored":false}).to_string()) }
    }).await;
    app.github = app.github.clone().with_test_origin(server.origin.clone());
    app.inbox.sync(&app).await.unwrap();
    assert_eq!(app.inbox.list(&app).await.unwrap().threads.len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // New poll state forces another full reconciliation without advancing the clock.
    app.inbox = crate::inbox::Inbox::new();
    app.inbox.sync(&app).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "only the failed subscription is retried"
    );
    assert_eq!(app.inbox.list(&app).await.unwrap().threads.len(), 2);
}

#[tokio::test]
async fn unavailable_mapped_remote_falls_back_and_caches_file_patches() {
    let (_fixture, mut app, _) = fixture().await;
    app.state_db.call(|c| {
        c.execute("INSERT INTO project (id,name,path,gh_repo,created_at) VALUES ('test','test','missing-host:/tmp/fixture','test/fixture',0)", [])?;
        Ok(())
    }).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = mock(move |req| {
        count.fetch_add(1, Ordering::SeqCst);
        if req.starts_with("POST /graphql") {
            (200, json!({"data":{"repository":{"issueOrPullRequest":{"__typename":"PullRequest","baseRefOid":"base","headRefOid":"head","timelineItems":{"nodes":[]}}}}}).to_string())
        } else if req.contains("/files?") {
            (200, json!([{"filename":"real","patch":"@@ -1 +1 @@\n-old\n+new\n","status":"modified"}]).to_string())
        } else { (200, json!({"base":{"sha":"base"},"head":{"sha":"head"}}).to_string()) }
    }).await;
    app.github = app.github.clone().with_test_origin(server.origin.clone());
    let summary = crate::items::diff_summary(&app, "test/fixture", 1)
        .await
        .unwrap();
    assert_eq!(summary["fallback"], true);
    assert!(summary["local_error"].is_string());
    // Full detail, review threads, API files and a final base/head consistency check.
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let file = crate::items::diff_file(&app, "test/fixture", 1, "base", "head", None, "real")
        .await
        .unwrap();
    assert!(file["patch"].as_str().unwrap().contains("+new"));
    crate::items::diff_summary(&app, "test/fixture", 1)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn subscription_network_checks_do_not_block_local_inbox_actions() {
    let (_fixture, mut app, _) = fixture().await;
    let inspecting = Arc::new(tokio::sync::Notify::new());
    let signal = inspecting.clone();
    let server = mock(move |req| {
        if req.starts_with("GET /notifications?") {
            (200, json!([{"id":"1","repository":{"full_name":"test/fixture"},"subject":{"type":"Other"},"updated_at":"today","unread":false}]).to_string())
        } else {
            signal.notify_one();
            (200, "{}".into())
        }
    }).await;
    app.github = app.github.clone().with_test_origin(server.origin.clone());
    let poll_app = app.clone();
    let poll = tokio::spawn(async move { poll_app.inbox.sync(&poll_app).await });
    timeout(Duration::from_secs(2), inspecting.notified())
        .await
        .unwrap();
    timeout(Duration::from_millis(20), app.inbox.restore(&app, vec![]))
        .await
        .expect("local click must not wait for the subscription response")
        .unwrap();
    poll.await.unwrap().unwrap();
}

#[tokio::test]
async fn unchanged_unread_threads_do_not_prefetch_stale_item_details() {
    let (_fixture, mut app, _) = fixture().await;
    let t = crate::inbox::Thread {
        id: "1".into(),
        repo: "test/fixture".into(),
        number: Some(1),
        kind: "Issue".into(),
        title: "Test".into(),
        url: "https://example.test".into(),
        subject_path: None,
        reason: "mention".into(),
        unread: true,
        local_unread: false,
        updated_at: "today".into(),
        state: Some("open".into()),
        participants: vec![],
        participant_count: 0,
        review_decision: None,
        checks: None,
    };
    app.cache_db
        .call(move |c| {
            c.execute(
                "INSERT INTO inbox_thread VALUES ('1','today',?1)",
                [serde_json::to_string(&t)?],
            )?;
            c.execute(
                "INSERT INTO inbox_subscription VALUES ('1',1,0,'today',0)",
                [],
            )?;
            c.execute(
                "INSERT INTO inbox_preview VALUES ('1',0,?1)",
                [
                    json!({"id":"1","body":"test","author":"owner","comments":[],"fetched_at":0})
                        .to_string(),
                ],
            )?;
            c.execute(
                "INSERT INTO gh_response (url,json) VALUES ('item:test/fixture:1',?1)",
                [json!({"id":"item","fetched_at":0}).to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = mock(move |req| {
        count.fetch_add(1, Ordering::SeqCst);
        assert!(req.starts_with("GET /notifications?"), "unchanged items must not be fetched: {req}");
        (200, json!([{"id":"1","repository":{"full_name":"test/fixture"},"subject":{"type":"Issue","url":"https://api.github.com/repos/test/fixture/issues/1"},"updated_at":"today","unread":true}]).to_string())
    }).await;
    app.github = app.github.clone().with_test_origin(server.origin.clone());
    for _ in 0..2 {
        app.inbox = crate::inbox::Inbox::new();
        app.inbox.sync(&app).await.unwrap();
    }
    // A barrier on the current-thread runtime lets any mistakenly spawned prefetch run.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

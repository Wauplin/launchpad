//! Home's write barrier during a planned update (`launchpad_core::maintenance`): every mutating request is counted
//! while it runs (so the update waits for it), and refused with a retryable 503 while Home is in maintenance.
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use launchpad_core::maintenance;

pub async fn gate(req: Request, next: Next) -> Response {
    let (method, path) = (req.method().as_str().to_string(), req.uri().path().to_string());
    if !maintenance::gated(&method, &path) {
        return next.run(req).await;
    }
    // Counted before the check: `enter` then `drained` never misses a request that passed the check.
    let running = maintenance::Inflight::begin();
    if let Some(m) = maintenance::current() {
        drop(running);
        let mut res = launchpad_http::error::maintenance(&m).into_response();
        res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(m.retry_after));
        return res;
    }
    let res = next.run(req).await;
    drop(running);
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, routing::post};
    use launchpad_core::maintenance::{Maintenance, Phase};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// A delayed replacement: writes accepted before maintenance finish and count; writes during it are refused
    /// with a retryable 503 and never run; reads keep working.
    #[tokio::test]
    async fn writes_during_maintenance_are_refused_and_running_ones_drain() {
        let done = Arc::new(AtomicUsize::new(0));
        let d = done.clone();
        let app = Router::new()
            .route("/api/write", post(move || {
                let d = d.clone();
                async move {
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                    d.fetch_add(1, Ordering::SeqCst);
                    "ok"
                }
            }).get(|| async { "read" }))
            .layer(axum::middleware::from_fn(gate));
        let req = |m: &str| Request::builder().method(m).uri("/api/write").body(Body::empty()).unwrap();
        // Accepted before maintenance: it runs to the end, and the drain waits for it.
        let accepted = tokio::spawn(app.clone().oneshot(req("POST")));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        *launchpad_core::maintenance::state_for_tests() = Some(Maintenance { phase: Phase::Draining, reason: "Home updates to v2".into(), target: Some("v2".into()), since: 0, retry_after: 300 });
        assert!(maintenance::drained(std::time::Duration::from_millis(10)).await.is_err());
        let refused = app.clone().oneshot(req("POST")).await.unwrap();
        assert_eq!(refused.status(), 503);
        assert_eq!(refused.headers()[header::RETRY_AFTER], "300");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(refused.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(body["retryable"], true);
        assert_eq!(body["maintenance"]["phase"], "draining");
        assert!(body["error"].as_str().unwrap().contains("Nothing was changed"));
        assert_eq!(app.clone().oneshot(req("GET")).await.unwrap().status(), 200);
        maintenance::drained(std::time::Duration::from_secs(2)).await.unwrap();
        assert_eq!(accepted.await.unwrap().unwrap().status(), 200);
        // Exactly the accepted write ran; the refused one never reached the handler.
        assert_eq!(done.load(Ordering::SeqCst), 1);
        *launchpad_core::maintenance::state_for_tests() = None;
    }
}


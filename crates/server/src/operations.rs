//! Retry protection runs after authentication and before handlers.
use crate::error::ApiError;
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use launchpad_core::operations::{self, Claim};

#[derive(Clone)]
pub struct Operations {
    pub db: launchpad_core::db::Db,
    pub lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl Operations {
    pub fn new(db: launchpad_core::db::Db) -> Self {
        Self {
            db,
            lock: Default::default(),
        }
    }
}

const LIMIT: usize = 32 * 1024 * 1024;

pub async fn guard(State(s): State<Operations>, req: Request, next: Next) -> Response {
    match run(&s, req, next).await {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}

fn publication_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    matches!(parts.as_slice(), ["", "api", "items", _, _, _, "comment" | "review"] | ["", "api", "inbox", _, "reply"])
}

async fn run(s: &Operations, req: Request, next: Next) -> Result<Response, ApiError> {
    let path = req.uri().path().to_owned();
    let publication = publication_path(&path);
    let protected = publication || path == "/api/sessions"
        || path
            .strip_prefix("/api/sessions/")
            .is_some_and(|p| p.split('/').count() == 2 && p.ends_with("/input"));
    if req.method() != Method::POST || !protected {
        return Ok(next.run(req).await);
    }
    let Some(id) = req.headers().get("Idempotency-Key") else {
        if publication { return Err(ApiError::bad_request("GitHub posts require an Idempotency-Key")); }
        return Ok(next.run(req).await);
    };
    let id = id
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid operation id"))?
        .to_owned();
    let parsed = uuid::Uuid::parse_str(&id)
        .map_err(|_| ApiError::bad_request("operation id must be a UUID"))?;
    if parsed.to_string() != id {
        return Err(ApiError::bad_request(
            "operation id must be a lowercase, hyphenated UUID",
        ));
    }
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, LIMIT)
        .await
        .map_err(|_| ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "request too large"))?;
    // Serialize concurrent retries until the first response is durably recorded.
    let _lock = s.lock.lock().await;
    match operations::claim(&s.db, &id, &path, &bytes).await? {
        Claim::Completed { status, body } => Ok((
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    axum::http::HeaderName::from_static("idempotency-status"),
                    "completed",
                ),
            ],
            body,
        )
            .into_response()),
        Claim::Mismatch => Err(ApiError::conflict(
            "This operation id was already used for a different request.",
        )),
        Claim::Uncertain => Err(ApiError::conflict(
            "Delivery is uncertain after an interrupted request. Inspect the destination (GitHub for posts, Agents for launches and replies) before starting another attempt; this request will not be repeated.",
        )),
        Claim::New => {
            let response = next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
            if publication && response.headers().get("GitHub-Delivery").is_some_and(|v| v == "uncertain") {
                // Leave the durable claim unresolved; neither this process nor a restart may repost it.
                return Ok(response);
            }
            let (mut parts, body) = response.into_parts();
            let bytes = to_bytes(body, LIMIT).await?;
            let text = String::from_utf8(bytes.to_vec())?;
            operations::complete(&s.db, &id, parts.status.as_u16(), text).await?;
            parts.headers.insert(
                "Idempotency-Status",
                axum::http::HeaderValue::from_static("completed"),
            );
            Ok(Response::from_parts(parts, Body::from(bytes)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn manual_publications_replay_success_and_never_repeat_uncertain_posts() {
        let path = std::env::temp_dir().join(format!("lp-posts-{}.db", uuid::Uuid::new_v4()));
        let count = Arc::new(AtomicUsize::new(0));
        for route in ["/api/items/o/r/1/comment", "/api/items/o/r/1/review", "/api/inbox/thread/reply"] {
            let id = uuid::Uuid::new_v4().to_string();
            let uncertain = uuid::Uuid::new_v4().to_string();
            let baseline = count.load(Ordering::SeqCst);
            for restart in [false, true] {
                let db = launchpad_core::db::open_state(&path).unwrap();
                let calls = count.clone();
                let handler = move |body: String| {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        if body == "uncertain" {
                            let mut e = ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "check GitHub");
                            e.delivery_uncertain = true;
                            e.into_response()
                        } else { axum::Json(serde_json::json!({"posted":true})).into_response() }
                    }
                };
                let router = Router::new().route(route, post(handler))
                    .layer(axum::middleware::from_fn_with_state(Operations::new(db), guard));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let url = format!("http://{}{route}", listener.local_addr().unwrap());
                let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
                let client = reqwest::Client::new();
                assert_eq!(client.post(&url).body("ok").send().await.unwrap().status(), 400);
                for _ in 0..2 {
                    let response = client.post(&url).header("Idempotency-Key", &id).body("ok").send().await.unwrap();
                    assert_eq!(response.status(), 200);
                    assert_eq!(response.headers()["Idempotency-Status"], "completed");
                }
                let response = client.post(&url).header("Idempotency-Key", &uncertain).body("uncertain").send().await.unwrap();
                assert_eq!(response.status(), if restart { 409 } else { 500 });
                assert!(!response.headers().contains_key("Idempotency-Status"));
                assert_eq!(client.post(&url).header("Idempotency-Key", &uncertain).body("uncertain").send().await.unwrap().status(), 409);
                assert_eq!(count.load(Ordering::SeqCst), baseline + 2);
                server.abort();
            }
        }
        for suffix in ["", "-wal", "-shm"] { let _ = std::fs::remove_file(format!("{}{suffix}", path.display())); }
    }

    #[tokio::test]
    async fn lost_replies_concurrent_retries_and_restart_do_not_repeat_work() {
        let path =
            std::env::temp_dir().join(format!("launchpad-retries-{}.db", uuid::Uuid::new_v4()));
        let count = Arc::new(AtomicUsize::new(0));
        let id = uuid::Uuid::new_v4().to_string();
        let pending = uuid::Uuid::new_v4().to_string();
        let input_id = uuid::Uuid::new_v4().to_string();
        let failed_id = uuid::Uuid::new_v4().to_string();
        let inputs = Arc::new(AtomicUsize::new(0));
        for restart in [false, true] {
            let db = launchpad_core::db::open_state(&path).unwrap();
            let state = Operations::new(db.clone());
            let count2 = count.clone();
            let inputs2 = inputs.clone();
            let router = Router::new()
                .route(
                    "/api/sessions",
                    post(move || {
                        let count = count2.clone();
                        async move {
                            let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            axum::Json(serde_json::json!({"created": n}))
                        }
                    }),
                )
                .route(
                    "/api/sessions/test/input",
                    post(move |body: String| {
                        let inputs = inputs2.clone();
                        async move {
                            inputs.fetch_add(1, Ordering::SeqCst);
                            let status = if body == "fail" {
                                StatusCode::BAD_REQUEST
                            } else {
                                StatusCode::OK
                            };
                            (status, axum::Json(serde_json::json!({"result": body})))
                        }
                    }),
                )
                .layer(axum::middleware::from_fn_with_state(state, guard));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/api/sessions", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let client = reqwest::Client::new();
            let send = || {
                client
                    .post(&url)
                    .header("Idempotency-Key", &id)
                    .body("{}")
                    .send()
            };
            if !restart {
                // The caller discards the first response; two retries race with that request.
                let (first, a, b) = tokio::join!(send(), send(), send());
                drop(first.unwrap());
                assert_eq!(a.unwrap().text().await.unwrap(), "{\"created\":1}");
                assert_eq!(b.unwrap().text().await.unwrap(), "{\"created\":1}");
                assert!(matches!(
                    operations::claim(&db, &pending, "/api/sessions", b"{}")
                        .await
                        .unwrap(),
                    Claim::New
                ));
            } else {
                assert_eq!(
                    send().await.unwrap().text().await.unwrap(),
                    "{\"created\":1}"
                );
                assert_eq!(
                    client
                        .post(&url)
                        .header("Idempotency-Key", &pending)
                        .body("{}")
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    409
                );
            }
            assert_eq!(
                client
                    .post(&url)
                    .header("Idempotency-Key", &id)
                    .body("changed")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                409
            );
            assert_eq!(
                client
                    .post(&url)
                    .header("Idempotency-Key", "bad")
                    .body("{}")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                400
            );
            let input_url = format!("{url}/test/input");
            for (id, body, status) in [(&input_id, "message", 200), (&failed_id, "fail", 400)] {
                for _ in 0..2 {
                    let response = client
                        .post(&input_url)
                        .header("Idempotency-Key", id)
                        .body(body)
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.status().as_u16(), status);
                    assert_eq!(response.headers()["Idempotency-Status"], "completed");
                    assert_eq!(
                        response.text().await.unwrap(),
                        format!("{{\"result\":\"{body}\"}}")
                    );
                }
            }
            assert_eq!(inputs.load(Ordering::SeqCst), 2);
            assert_eq!(count.load(Ordering::SeqCst), 1);
            server.abort();
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}

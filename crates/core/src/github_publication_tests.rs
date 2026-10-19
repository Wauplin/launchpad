//! Loopback fixtures only: never resolve credentials or contact GitHub.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) struct Mock {
    pub origin: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(crate) async fn mock(f: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let f = Arc::new(f);
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let f = f.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 8192];
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let (status, body) = f(&String::from_utf8_lossy(&bytes));
                // Keep a network call in flight so the locking tests can race owner actions.
                tokio::time::sleep(Duration::from_millis(30)).await;
                if status == 0 {
                    return;
                } // request accepted, response lost
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Mock { origin, task }
}

#[tokio::test]
async fn publication_errors_distinguish_rejection_from_uncertain_delivery() {
    let root = std::env::temp_dir().join(format!("lp-gh-errors-{}", uuid::Uuid::new_v4()));
    let db = crate::db::open_cache(&root).unwrap();
    let client = GitHub::new(UserEnv::for_host("unused"), db);
    for (status, body, uncertain) in [
        (403, "{}", false),
        (422, "{}", false),
        (500, "{}", true),
        (0, "", true),
        (200, "invalid JSON", true),
    ] {
        let fixture = mock(move |_| (status, body.into())).await;
        let gh = client.clone().with_test_origin(fixture.origin.clone());
        let e = gh
            .request(
                reqwest::Method::POST,
                "/repos/test/fixture/issues/1/comments",
                Some(serde_json::json!({"body":"test"})),
                false,
            )
            .await
            .err()
            .unwrap();
        assert_eq!(delivery_uncertain(&e), uncertain, "{status}: {e:#}");
    }
    let fixture = mock(|_| (200, "{}".into())).await;
    let gh = client.clone().with_test_origin(fixture.origin.clone());
    drop(fixture);
    let e = gh
        .request(reqwest::Method::POST, "/test", None, false)
        .await
        .err()
        .unwrap();
    assert!(
        !delivery_uncertain(&e),
        "refused connection sent nothing: {e:#}"
    );
    gh.limits.write().await.insert(
        "core".into(),
        RateLimit {
            remaining: 0,
            reset_at: now_ms() / 1000 + 60,
            retry_at: None,
        },
    );
    let e = gh
        .request(reqwest::Method::POST, "/test", None, false)
        .await
        .err()
        .unwrap();
    assert!(!delivery_uncertain(&e));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", root.display()));
    }
}

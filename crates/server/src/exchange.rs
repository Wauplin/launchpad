//! Home: HTTP exchanges a runner opens through its tunnel (`FromRunner::HttpOpen`).
//!
//! - `Home`: Home Git and plain-folder requests of a runner that cannot reach Home over HTTP (Burst). They run
//!   through Home's own router with the runner's credential, which their handlers check.
//! - `Lease`: a request of a relayed lease (Burst): Home adds the token (`launchpad_core::token_relay`).
//! - `Mcp`: a call to an MCP server of Home, through the gateway (`launchpad_core::mcp`).

use axum::body::Body;
use axum::http::{Request, header};
use futures_util::StreamExt;
use launchpad_core::home_link::{ByteStream, HttpReply};
use launchpad_core::runners::proto::{HttpTarget, ToRunner};
use tower::ServiceExt;

use crate::Server;

/// Largest body part per frame, before base64.
const CHUNK: usize = 256 * 1024;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(s: Server, runner_id: String, id: u64, target: HttpTarget, method: String, path: String, headers: Vec<(String, String)>, body: tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>) {
    let body: ByteStream = Box::pin(futures_util::stream::unfold(body, |mut rx| async move { rx.recv().await.map(|b| (Ok(b), rx)) }));
    let result = match target {
        HttpTarget::Home => home(&s, &runner_id, &method, &path, headers, body).await,
        HttpTarget::Lease { lease_id, host } => launchpad_core::token_relay::upstream(&s.app, &runner_id, &lease_id, &host, &method, &path, headers, body).await,
        HttpTarget::Mcp { lease_id, server } => launchpad_core::mcp::gateway(&s.app, &runner_id, &lease_id, &server, &method, headers, body).await,
    };
    let out = s.app.runners.clone();
    let reply = result.unwrap_or_else(|e| {
        let text = serde_json::json!({"error": format!("{e:#}")}).to_string();
        HttpReply { status: 502, headers: vec![("content-type".into(), "application/json".into())], body: Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(text)) })) }
    });
    out.send_to(&runner_id, ToRunner::HttpHead { id, status: reply.status, headers: reply.headers });
    let mut stream = reply.body;
    let b64 = |d: &[u8]| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, d);
    while let Some(part) = stream.next().await {
        match part {
            Ok(bytes) => {
                for c in bytes.chunks(CHUNK) {
                    out.send_to(&runner_id, ToRunner::HttpBody { id, data: b64(c), end: false, error: None });
                }
            }
            Err(e) => {
                out.send_to(&runner_id, ToRunner::HttpBody { id, data: String::new(), end: true, error: Some(e.to_string()) });
                return;
            }
        }
        if !out.connected(&runner_id) {
            return;
        }
    }
    out.send_to(&runner_id, ToRunner::HttpBody { id, data: String::new(), end: true, error: None });
}

/// Home Git and plain folders, through Home's router, as the runner (never as the owner).
async fn home(s: &Server, runner_id: &str, method: &str, path: &str, headers: Vec<(String, String)>, body: ByteStream) -> anyhow::Result<HttpReply> {
    let bare = path.split('?').next().unwrap_or_default();
    anyhow::ensure!(bare.starts_with("/api/git/") || bare.starts_with("/api/plain/") || bare.starts_with("/api/handoff/"), "runners send only Home Git, plain-folder and handoff requests this way");
    let credential = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(crate::runners::CREDENTIAL_HEADER)).map(|(_, v)| v.as_str()).unwrap_or("");
    anyhow::ensure!(credential.split_once('.').is_some_and(|(id, _)| id == runner_id), "the request names another runner");
    let router = crate::ROUTER.get().ok_or_else(|| anyhow::anyhow!("Home is starting"))?;
    let mut req = Request::builder().method(method).uri(path).header(header::HOST, format!("127.0.0.1:{}", s.app.paths.port));
    for (k, v) in &headers {
        if !matches!(k.to_ascii_lowercase().as_str(), "host" | "authorization" | "cookie" | "connection" | "transfer-encoding" | "origin") {
            req = req.header(k, v);
        }
    }
    let mut req = req.body(Body::from_stream(body))?;
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let res = router.clone().oneshot(req).await?;
    let status = res.status().as_u16();
    let headers = res.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
    let stream = res.into_body().into_data_stream().map(|r| r.map_err(std::io::Error::other));
    Ok(HttpReply { status, headers, body: Box::pin(stream) })
}

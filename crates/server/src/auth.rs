//! The API can start agents that run any command, so it is treated like remote code execution:
//!
//! - every request needs the token, as a cookie (window) or a bearer header (CLI);
//! - `/auth?t=<token>` sets the cookie (`HttpOnly`, `SameSite=Strict`);
//! - a `Host` other than `127.0.0.1:<port>` or `localhost:<port>` is rejected (DNS rebinding);
//! - a foreign `Origin` is rejected on WebSocket upgrades and mutations (CSRF). No CORS headers.

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use launchpad_core::paths::{Paths, Profile};
use serde::Deserialize;

use crate::Server;

pub struct Auth {
    token: String,
    cookie: &'static str,
    hosts: Vec<String>,
    origins: Vec<String>,
}

impl Auth {
    pub fn new(paths: &Paths) -> anyhow::Result<Auth> {
        let port = paths.port;
        let hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let mut origins: Vec<String> = hosts.iter().map(|h| format!("http://{h}")).collect();
        if paths.profile == Profile::Dev {
            // Vite dev server (`pnpm dev`), which proxies /api and /ws to the dev daemon.
            origins.push("http://localhost:5173".into());
            origins.push("http://127.0.0.1:5173".into());
        }
        Ok(Auth { token: paths.token()?, cookie: paths.cookie_name(), hosts, origins })
    }

    fn check_token(&self, candidate: &str) -> bool {
        // Constant time comparison.
        let (a, b) = (candidate.as_bytes(), self.token.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    fn is_authenticated(&self, headers: &HeaderMap) -> bool {
        if let Some(v) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
            && let Some(t) = v.strip_prefix("Bearer ") {
            return self.check_token(t.trim());
        }
        for v in headers.get_all(header::COOKIE) {
            let Ok(v) = v.to_str() else { continue };
            for pair in v.split(';') {
                if let Some((k, val)) = pair.trim().split_once('=')
                    && k == self.cookie && self.check_token(val) {
                    return true;
                }
            }
        }
        false
    }
}

pub async fn guard(State(s): State<Server>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !s.auth.hosts.iter().any(|h| h == host) {
        return (StatusCode::FORBIDDEN, "unexpected Host header").into_response();
    }
    let is_upgrade = headers.get(header::UPGRADE).is_some();
    let is_mutation = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if (is_upgrade || is_mutation)
        && let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && !s.auth.origins.iter().any(|o| o == origin) {
        return (StatusCode::FORBIDDEN, "foreign Origin").into_response();
    }
    let path = req.uri().path();
    if path == "/api/health" || path == "/auth" || s.auth.is_authenticated(headers) {
        return next.run(req).await;
    }
    if path.starts_with("/api/") || path == "/ws" {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "not authenticated"}))).into_response();
    }
    (StatusCode::UNAUTHORIZED, Html(NOT_AUTHENTICATED)).into_response()
}

const NOT_AUTHENTICATED: &str = r#"<!doctype html><meta charset="utf-8"><title>Launchpad</title>
<body style="font:15px system-ui;padding:40px;color:#17202B;background:#F4F6F9">
<h1 style="font-size:20px">Launchpad: not signed in</h1>
<p>Open Launchpad from the dock, or run <code>launchpad url</code> and open the address it prints.</p>"#;

#[derive(Deserialize)]
pub struct HandshakeQuery {
    t: String,
    next: Option<String>,
}

/// `/auth?t=<token>[&next=/route]`: sets the session cookie, then redirects into the app.
pub async fn handshake(State(s): State<Server>, Query(q): Query<HandshakeQuery>) -> Response {
    if !s.auth.check_token(&q.t) {
        return (StatusCode::UNAUTHORIZED, Html(NOT_AUTHENTICATED)).into_response();
    }
    let next = q.next.filter(|n| n.starts_with('/') && !n.starts_with("//")).unwrap_or_else(|| "/".into());
    let cookie = format!("{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000", s.auth.cookie, s.auth.token);
    ([(header::SET_COOKIE, cookie)], Redirect::to(&next)).into_response()
}

//! `launchpad` CLI calls of agents on a host. The helper forwards them over the SSH connection and the daemon
//! sends them to its own API, so suggestions, pre-drafts and merge requests stay local behind owner buttons.
//! Only the agent-facing endpoints are allowed.

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::Hosts;

fn allowed(method: &str, path: &str) -> bool {
    let route = path.split('?').next().unwrap_or(path);
    let segments: Vec<&str> = route.trim_start_matches('/').split('/').collect();
    matches!(
        (method, segments.as_slice()),
        ("POST", ["api", "suggestions"])
            | ("POST", ["api", "worktrees", "finish" | "add-project"])
            | ("GET" | "POST", ["api", "predrafts"])
            | ("GET", ["api", "predrafts", "resolve"])
            | ("GET" | "PATCH", ["api", "predrafts", _])
            | ("GET", ["api", "sessions", _, "trace"])
            | ("GET", ["api", "status"])
    )
}

/// Folders the CLI reports are native paths on the host: the daemon knows them as host locations.
fn rehost_cwd(host: &str, path: &str, body: Option<String>) -> Result<(String, Option<String>)> {
    let mut url = reqwest::Url::parse(&format!("http://host{path}"))?;
    let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| {
        let v = if k == "cwd" && v.starts_with('/') { super::loc::join(Some(host), &v) } else { v.into_owned() };
        (k.into_owned(), v)
    }).collect();
    if !pairs.is_empty() {
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let body = match body {
        Some(b) if url.path().starts_with("/api/predrafts") => {
            let mut v: Value = serde_json::from_str(&b).context("invalid request body")?;
            if let Some(cwd) = v.get("cwd").and_then(Value::as_str).filter(|c| c.starts_with('/')) {
                v["cwd"] = Value::String(super::loc::join(Some(host), cwd));
            }
            Some(v.to_string())
        }
        other => other,
    };
    Ok((path, body))
}

/// Validate before URL parsing, then authorize the final URL that reqwest will send.
/// Agent routes have literal ASCII segments; percent escapes belong only in the query.
fn request_url(host: &str, method: &str, path: &str, body: Option<String>, base: &str) -> Result<(reqwest::Url, Option<String>)> {
    let route = path.split('?').next().unwrap_or(path);
    if !route.starts_with("/api/")
        || path.contains('#')
        || path.chars().any(char::is_control)
        || route.contains(['\\', '%'])
        || route.split('/').any(|s| matches!(s, "." | ".."))
    {
        bail!("{method} {path} is not available to agents on a host");
    }
    let (path, body) = rehost_cwd(host, path, body)?;
    let url = reqwest::Url::parse(&format!("{base}{path}"))?;
    // Reject any parser normalization, including whitespace or authority-like paths.
    if url.path() != route || !allowed(method, url.path()) {
        bail!("{method} {path} is not available to agents on a host");
    }
    Ok((url, body))
}

pub(super) async fn call(hosts: &Hosts, host: &str, method: &str, path: &str, body: Option<String>) -> Result<(u16, String)> {
    let (url, body) = request_url(host, method, path, body, &hosts.paths.base_url())?;
    let method = reqwest::Method::from_bytes(method.as_bytes())?;
    // A redirect must not turn an allowed agent request into another API call.
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
    let mut req = client.request(method, url).bearer_auth(hosts.paths.token()?);
    if let Some(body) = body {
        req = req.header(reqwest::header::CONTENT_TYPE, "application/json").body(body);
    }
    let res = req.send().await.context("the daemon API did not answer")?;
    let status = res.status().as_u16();
    Ok((status, res.text().await?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn relay_rejects_path_normalization_and_encoded_routes() {
        for path in [
            r"/api/predrafts/..\projects\victim",
            "/api/predrafts/../projects",
            "/api/predrafts/.",
            "/api/predrafts/%2e%2e",
            "/api/predrafts/.%2E",
            "/api/predrafts/%252e%252e",
            "/api/predrafts/x%2f..%2fprojects",
            "/api/predrafts/x%5c..%5cprojects",
            "/api/predrafts/x#fragment",
            "/api/predrafts/x\n",
            "/api/predrafts/x ",
            "//api/predrafts/x",
            "http://elsewhere/api/predrafts/x",
        ] {
            assert!(super::request_url("devbox", "PATCH", path, None, "http://127.0.0.1:7317").is_err(), "{path:?}");
        }
    }

    #[test]
    fn final_request_keeps_allowed_route_and_rehosts_only_cwd() {
        for (method, path) in [
            ("POST", "/api/suggestions"),
            ("POST", "/api/worktrees/finish"),
            ("POST", "/api/worktrees/add-project"),
            ("GET", "/api/predrafts"),
            ("POST", "/api/predrafts"),
            ("GET", "/api/predrafts/resolve"),
            ("GET", "/api/predrafts/abc"),
            ("PATCH", "/api/predrafts/abc"),
            ("GET", "/api/sessions/abc/trace"),
            ("GET", "/api/status"),
        ] {
            let input = format!("{path}?cwd=%2Fhome%2Fu%2Fa%20b&through=3");
            let (url, _) = super::request_url("devbox", method, &input, None, "http://127.0.0.1:7317").unwrap();
            assert_eq!(url.path(), path);
            assert_eq!(url.origin().ascii_serialization(), "http://127.0.0.1:7317");
            assert_eq!(url.query_pairs().collect::<Vec<_>>(), [("cwd".into(), "devbox:/home/u/a b".into()), ("through".into(), "3".into())]);
        }
        for (method, path) in [("PATCH", "/api/projects/abc"), ("POST", "/api/sessions/abc/send")] {
            assert!(super::request_url("devbox", method, path, None, "http://127.0.0.1:7317").is_err());
        }
    }

    #[test]
    fn only_agent_endpoints_are_relayed_with_host_folders() {
        assert!(super::allowed("POST", "/api/suggestions"));
        assert!(super::allowed("GET", "/api/predrafts/resolve?cwd=/x"));
        assert!(super::allowed("GET", "/api/sessions/abc/trace?through=3"));
        assert!(!super::allowed("POST", "/api/sessions/abc/send"));
        assert!(!super::allowed("DELETE", "/api/predrafts/x"));
        assert!(!super::allowed("GET", "/api/config"));
        let (path, body) = super::rehost_cwd("devbox", "/api/predrafts/resolve?cwd=%2Fhome%2Fu%2Fp", None).unwrap();
        assert_eq!(path, "/api/predrafts/resolve?cwd=devbox%3A%2Fhome%2Fu%2Fp");
        assert!(body.is_none());
        let (url, body) = super::request_url("devbox", "POST", "/api/predrafts", Some(r#"{"cwd":"/w","title":"t"}"#.into()), "http://127.0.0.1:7317").unwrap();
        assert_eq!(url.path(), "/api/predrafts");
        assert!(body.unwrap().contains(r#""cwd":"devbox:/w""#));
        let (_, body) = super::request_url("devbox", "POST", "/api/suggestions", Some(r#"{"body":"unchanged"}"#.into()), "http://127.0.0.1:7317").unwrap();
        assert_eq!(body.as_deref(), Some(r#"{"body":"unchanged"}"#));
    }
}

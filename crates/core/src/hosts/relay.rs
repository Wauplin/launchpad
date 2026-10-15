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

pub(super) async fn call(hosts: &Hosts, host: &str, method: &str, path: &str, body: Option<String>) -> Result<(u16, String)> {
    if !path.starts_with('/') || !allowed(method, path) {
        bail!("{method} {path} is not available to agents on a host");
    }
    let (path, body) = rehost_cwd(host, path, body)?;
    let method = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = reqwest::Client::new().request(method, format!("{}{path}", hosts.paths.base_url())).bearer_auth(hosts.paths.token()?);
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
        let (_, body) = super::rehost_cwd("devbox", "/api/predrafts", Some(r#"{"cwd":"/w","title":"t"}"#.into())).unwrap();
        assert!(body.unwrap().contains(r#""cwd":"devbox:/w""#));
    }
}

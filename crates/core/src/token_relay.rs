//! Home: requests of relayed leases. A Burst runner holds no token: its proxy sends each request that needs
//! one through the tunnel, and Home adds the token on the lease's hosts, sends it, records it in the access
//! log and streams the answer back. Revoke, Stop and Archive end the lease here at once.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result};
use futures_util::{StreamExt, TryStreamExt};

use crate::App;
use crate::home_link::{ByteStream, HttpReply};
use crate::vault::Lease;

/// Relayed leases with their secrets, by lease ID, with the runner that holds each.
static LEASES: std::sync::LazyLock<Mutex<HashMap<String, (String, Lease)>>> = std::sync::LazyLock::new(Default::default);

/// Keeps the full lease Home sent without secrets.
pub(crate) fn hold(runner_id: &str, lease: &Lease) {
    LEASES.lock().unwrap().insert(lease.id.clone(), (runner_id.to_string(), lease.clone()));
}

pub(crate) fn forget(lease_id: &str) {
    LEASES.lock().unwrap().remove(lease_id);
}

/// The lease of a relayed request: from memory, or rebuilt from its record after Home restarted.
async fn lease(app: &App, runner_id: &str, lease_id: &str) -> Result<Lease> {
    let cached = LEASES.lock().unwrap().get(lease_id).cloned();
    if let Some((r, l)) = cached {
        anyhow::ensure!(r == runner_id, "this lease belongs to another runner");
        crate::vault::check_owner(app, Some(runner_id), &l.session_id, l.generation)?;
        return Ok(l);
    }
    let info = crate::vault::lease_rows(app, None).await?.into_iter().find(|l| l.id == lease_id).context("unknown lease")?;
    anyhow::ensure!(info.revoked_at.is_none(), "this lease was revoked");
    anyhow::ensure!(info.runner_id.as_deref() == Some(runner_id), "this lease belongs to another runner");
    let req = info.request.context("this lease cannot be renewed here")?;
    crate::vault::check_owner(app, Some(runner_id), &req.session_id, req.generation)?;
    let l = crate::vault::build(app, lease_id, &req).await?;
    hold(runner_id, &l);
    Ok(l)
}

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).http1_only().connect_timeout(std::time::Duration::from_secs(30)).build().expect("relay HTTP client")
});

/// Sends one relayed request to `https://<host><target>` with the lease's token for that host.
#[allow(clippy::too_many_arguments)]
pub async fn upstream(app: &App, runner_id: &str, lease_id: &str, host: &str, method: &str, target: &str, headers: Vec<(String, String)>, body: ByteStream) -> Result<HttpReply> {
    anyhow::ensure!(crate::vault::here(app), "only Home adds tokens");
    let lease = lease(app, runner_id, lease_id).await?;
    let (secret, auth) = lease
        .credentials
        .iter()
        .find_map(|c| c.hosts.iter().find(|h| h.matches(host)).map(|h| (c.secret.clone(), h.auth.clone())))
        .with_context(|| format!("the lease has no token for {host}"))?;
    let mut url = reqwest::Url::parse(&format!("https://{host}{target}"))?;
    let mut out = reqwest::header::HeaderMap::new();
    for (k, v) in &headers {
        if crate::proxy::HOP.contains(&k.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (reqwest::header::HeaderName::from_bytes(k.as_bytes()), reqwest::header::HeaderValue::from_str(v)) {
            out.append(n, v);
        }
    }
    crate::proxy::add_auth(&mut out, &mut url, &auth, &secret);
    let path = url.path().to_string();
    let (read_req, read_res) = crate::proxy::wants_body(host, method, &path);
    let mut request_bytes = None;
    let body = if read_req {
        let parts: Vec<bytes::Bytes> = body.try_collect().await?;
        let all = bytes::Bytes::from(parts.concat());
        request_bytes = Some(all.clone());
        reqwest::Body::from(all)
    } else {
        reqwest::Body::wrap_stream(body)
    };
    let res = CLIENT.request(reqwest::Method::from_bytes(method.as_bytes())?, url).headers(out).body(body).send().await.with_context(|| host.to_string())?;
    let status = res.status().as_u16();
    let headers: Vec<(String, String)> = res.headers().iter().filter(|(k, _)| !crate::proxy::HOP.contains(&k.as_str())).filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
    let (body, response_bytes): (ByteStream, Option<bytes::Bytes>) = if read_res {
        let b = res.bytes().await?;
        let copy = b.clone();
        (Box::pin(futures_util::stream::once(async move { Ok(copy) })), Some(b))
    } else {
        (Box::pin(res.bytes_stream().map(|r| r.map_err(std::io::Error::other))), None)
    };
    let action = crate::vault::describe(host, method, &path, request_bytes.as_deref(), response_bytes.as_deref());
    let access = crate::proxy::Access { at: crate::util::now_ms(), session_id: lease.session_id.clone(), host: host.into(), method: method.into(), path, status, action };
    crate::contain::record_access(app, access);
    Ok(HttpReply { status, headers, body })
}

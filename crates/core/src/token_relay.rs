//! Home: requests of relayed leases. A Burst runner holds no token: its proxy sends each request that needs
//! one through the tunnel, and Home adds the token on the lease's hosts, sends it, records it in the access
//! log and streams the answer back. Revoke, Stop and Archive end the lease here at once.
//!
//! Every other connection of a Burst agent leaves from Home too (`tcp`): the job's traffic has one exit, Home's,
//! limited to public addresses.

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
    let sign = crate::attribution::signs_github(host, method, &path);
    let mut request_bytes = None;
    let body = if read_req || sign {
        let parts: Vec<bytes::Bytes> = body.try_collect().await?;
        let mut all = bytes::Bytes::from(parts.concat());
        // GitHub bodies the agent writes end with the attribution line, as on other runners (`proxy::forward`).
        if sign && let Some(signed) = crate::attribution::sign_request(app, &lease.session_id, host, method, &path, &all) {
            all = bytes::Bytes::from(signed);
            out.insert(reqwest::header::CONTENT_LENGTH, all.len().into());
        }
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

/// Home: opens a TCP connection for a Burst runner's agent and streams it both ways. Public addresses only
/// (`netcap::public`), checked on the address Home connects to. Errors carry `errno <n>` for the agent.
pub async fn tcp(app: &App, runner_id: &str, host: &str, port: u16, body: ByteStream) -> Result<HttpReply> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    anyhow::ensure!(crate::runners::is_burst(app, runner_id).await, "errno {}: only Burst runners send their traffic through Home", libc::EACCES);
    let addrs: Vec<std::net::SocketAddr> = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![std::net::SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port)).await.map_err(|e| anyhow::anyhow!("errno {}: {host}: {e}", libc::EHOSTUNREACH))?.collect(),
    };
    let allowed: Vec<_> = addrs.into_iter().filter(|a| crate::netcap::public(a.ip())).collect();
    anyhow::ensure!(!allowed.is_empty(), "errno {}: {host} is not a public address", libc::EACCES);
    let mut last = libc::ECONNREFUSED;
    let mut stream = None;
    for a in allowed {
        match tokio::time::timeout(std::time::Duration::from_secs(30), tokio::net::TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                stream = Some(s);
                break;
            }
            Ok(Err(e)) => last = crate::netcap::errno_of(&e),
            Err(_) => last = libc::ETIMEDOUT,
        }
    }
    let stream = stream.ok_or_else(|| anyhow::anyhow!("errno {last}: connect {host}:{port}"))?;
    let _ = stream.set_nodelay(true);
    let (rd, mut wr) = stream.into_split();
    tokio::spawn(async move {
        let mut body = body;
        while let Some(Ok(part)) = body.next().await {
            if wr.write_all(&part).await.is_err() {
                return;
            }
        }
        let _ = wr.shutdown().await;
    });
    let down: ByteStream = Box::pin(futures_util::stream::unfold(Some(rd), |rd| async move {
        let mut rd = rd?;
        let mut buf = vec![0u8; 64 * 1024];
        match rd.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(bytes::Bytes::from(buf)), Some(rd)))
            }
            Err(e) => Some((Err(e), None)),
        }
    }));
    Ok(HttpReply { status: 200, headers: vec![], body: down })
}

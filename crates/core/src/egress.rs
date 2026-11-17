//! Exit routes: the owner's routes (`[[egress]]` in config.toml, Settings → Home and runners) send matching URLs
//! out through one runner, the exit runner, for Home (its git, the GitHub client, Burst relays, oracle tools) and
//! machine runners (the runner proxy). A GitHub organization behind an IP allow list is the use case: only the exit
//! runner's address is allowed.
//!
//! - Callers decide per request (`way`, `Clients::pick`): direct, through the exit, or refused at once when a
//!   `require` route's exit is offline. `prefer` URLs (GraphQL, search) go direct while it is offline.
//! - A routed request goes to a loopback CONNECT bridge (`bridge`) with `<route>:<secret>` as proxy credentials. The
//!   bridge carries TCP only: TLS runs from the caller (which adds the token) to the destination, so no token ever
//!   reaches the exit or Home's splice.
//! - On Home the bridge dials the exit (`open`): `ToRunner::Dial`, then the exit connects and opens an exchange
//!   `HttpTarget::Dialed` whose bodies carry the bytes (`dial`, `dialed`). A runner's bridge opens an
//!   `HttpTarget::Egress` exchange to Home, which dials the same way (`for_runner`). Every stream rides the tunnel's
//!   existing windowed exchanges.
//! - Home and the exit both check that the target is one of the route's (`targets`): the exit never opens other
//!   hosts.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::home_link::{ByteStream, HttpReply};

/// One exit route of config.toml (`[[egress]]`): exactly one of `github_org` and `urls`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct ExitRoute {
    /// Lowercase letters, digits, `.`, `_` and `-`, unique (it goes into proxy credentials).
    pub name: String,
    /// A GitHub organization: its git, REST, raw and upload URLs, plus GraphQL and search while the exit is online.
    pub github_org: Option<String>,
    /// Explicit `http(s)://` URL prefixes; `https://host/` routes a whole host.
    pub urls: Vec<String>,
    /// The exit runner: its name or ID.
    pub via: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExitRouteKind")]
pub enum RouteKind {
    GithubOrg,
    Urls,
}

/// A route with its exit's state, as Home resolved it: for runners (`ToRunner::Settings`), the API, the CLI and
/// oracles. No secret.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "ExitRouteState")]
pub struct RouteState {
    pub name: String,
    pub kind: RouteKind,
    pub github_org: Option<String>,
    pub urls: Vec<String>,
    /// Every URL prefix the route sends through its exit (a GitHub organization expands to 8).
    pub prefixes: Vec<String>,
    /// The exit runner's name (as configured while no runner matches).
    pub exit: String,
    pub exit_id: Option<String>,
    /// The exit runner is connected to Home.
    pub online: bool,
    /// When `online` last changed (ms), when known.
    #[ts(type = "number | null")]
    pub since: Option<i64>,
    /// Why the route does not work: no such runner, or the exit's last failed dial.
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Always through the exit: refused at once while it is offline.
    Require,
    /// Through the exit while it is online, else direct (requests that cannot be matched by URL: GraphQL, search).
    Prefer,
}

/// One URL prefix of a route.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub host: String,
    pub port: u16,
    /// A path prefix without its trailing `/` (empty: the whole host).
    pub prefix: String,
    pub mode: Mode,
}

const GITHUB_HOSTS: [&str; 5] = ["github.com", "api.github.com", "codeload.github.com", "raw.githubusercontent.com", "uploads.github.com"];

/// The header of a `Dialed` exchange whose connection failed (its value says why).
pub const DIAL_ERROR: &str = "x-launchpad-dial-error";

fn valid_route_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A URL prefix of a `urls` route: http(s), a host, no user info, query or fragment.
fn parse_prefix(u: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(u.trim()).map_err(|e| format!("{u}: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{u}: only http:// and https:// addresses"));
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{u}: no user, query or fragment"));
    }
    let host = url.host_str().unwrap_or_default();
    let ip = host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>().is_ok();
    if !(ip || crate::vault::dns_name(host)) {
        return Err(format!("{u}: {host} is not a host name"));
    }
    Ok(url)
}

/// Checks the routes of config.toml.
pub fn validate(routes: &[ExitRoute]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for r in routes {
        if !valid_route_name(&r.name) {
            return Err(format!("route name {:?}: 1 to 40 lowercase letters, digits, '.', '_' or '-'", r.name));
        }
        if !names.insert(r.name.as_str()) {
            return Err(format!("two routes are named {}", r.name));
        }
        match (&r.github_org, r.urls.is_empty()) {
            (Some(org), true) => {
                if !crate::github::valid_name(org) {
                    return Err(format!("route {}: {org:?} is not a GitHub organization name", r.name));
                }
            }
            (None, false) => {
                if r.urls.len() > 20 {
                    return Err(format!("route {}: at most 20 URL prefixes", r.name));
                }
                for u in &r.urls {
                    parse_prefix(u).map_err(|e| format!("route {}: {e}", r.name))?;
                }
            }
            (Some(_), false) => return Err(format!("route {}: a GitHub organization or URL prefixes, not both", r.name)),
            (None, true) => return Err(format!("route {}: name a GitHub organization or URL prefixes", r.name)),
        }
        if r.via.trim().is_empty() {
            return Err(format!("route {}: choose its exit runner", r.name));
        }
    }
    Ok(())
}

fn trimmed(path: &str) -> String {
    path.trim_end_matches('/').to_ascii_lowercase()
}

/// The URL prefixes of a route (pure).
pub fn rules(github_org: Option<&str>, urls: &[String]) -> Vec<Rule> {
    let rule = |host: &str, prefix: String, mode| Rule { host: host.into(), port: 443, prefix: trimmed(&prefix), mode };
    if let Some(org) = github_org {
        let org = org.to_ascii_lowercase();
        return vec![
            rule("github.com", format!("/{org}/"), Mode::Require),
            rule("api.github.com", format!("/repos/{org}/"), Mode::Require),
            rule("api.github.com", format!("/orgs/{org}/"), Mode::Require),
            rule("codeload.github.com", format!("/{org}/"), Mode::Require),
            rule("raw.githubusercontent.com", format!("/{org}/"), Mode::Require),
            rule("uploads.github.com", format!("/repos/{org}/"), Mode::Require),
            rule("api.github.com", "/graphql".into(), Mode::Prefer),
            rule("api.github.com", "/search/".into(), Mode::Prefer),
        ];
    }
    urls.iter()
        .filter_map(|u| parse_prefix(u).ok())
        .filter_map(|u| Some(Rule { host: u.host_str()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase(), port: u.port_or_known_default()?, prefix: trimmed(u.path()), mode: Mode::Require }))
        .collect()
}

fn rules_of(s: &RouteState) -> Vec<Rule> {
    rules(s.github_org.as_deref(), &s.urls)
}

/// A route's prefixes as URLs, for display: an organization's expansion, or the URL prefixes as configured.
fn prefixes(github_org: Option<&str>, urls: &[String]) -> Vec<String> {
    if github_org.is_none() {
        return urls.iter().map(|u| u.trim().to_string()).collect();
    }
    rules(github_org, urls).into_iter().map(|r| {
        let slash = if r.prefix == "/graphql" { "" } else { "/" };
        format!("https://{}{}{slash}", r.host, r.prefix)
    }).collect()
}

/// The only destinations the exit opens for a route (pure): GitHub's hosts on 443 for an organization, each
/// prefix's host and port otherwise.
pub fn targets(github_org: Option<&str>, urls: &[String]) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = if github_org.is_some() {
        GITHUB_HOSTS.iter().map(|h| (h.to_string(), 443)).collect()
    } else {
        rules(None, urls).into_iter().map(|r| (r.host, r.port)).collect()
    };
    out.dedup();
    out.sort();
    out.dedup();
    out
}

fn allowed(s: &RouteState, host: &str, port: u16) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    targets(s.github_org.as_deref(), &s.urls).iter().any(|(h, p)| *h == host && *p == port)
}

/// Whether `path` starts with `prefix` on a segment boundary (both lowercase, `prefix` without a trailing `/`).
fn under(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path == prefix || path.strip_prefix(prefix).is_some_and(|r| r.starts_with('/'))
}

/// The route a URL takes (pure): the first `require` match in config order, else the first `prefer` match. The
/// query is ignored; host and path compare in lowercase, on segment boundaries.
pub fn matching<'a>(states: &'a [RouteState], url: &reqwest::Url) -> Option<(&'a RouteState, Mode, Rule)> {
    let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    let path = url.path().to_ascii_lowercase();
    let mut prefer = None;
    for s in states {
        for r in rules_of(s) {
            if r.host == host && r.port == port && under(&path, &r.prefix) {
                match r.mode {
                    Mode::Require => return Some((s, Mode::Require, r)),
                    Mode::Prefer if prefer.is_none() => prefer = Some((s, Mode::Prefer, r)),
                    Mode::Prefer => {}
                }
            }
        }
    }
    prefer
}

/// The route of a raw connection to `host:port` (pure): whole-host prefixes only (path `/`).
pub fn matching_host<'a>(states: &'a [RouteState], host: &str, port: u16) -> Option<&'a RouteState> {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    states.iter().find(|s| rules_of(s).iter().any(|r| r.host == host && r.port == port && r.prefix.is_empty() && r.mode == Mode::Require))
}

/// The one message of a refusal.
pub fn offline_message(s: &RouteState, host: &str) -> String {
    let what = s.github_org.clone().unwrap_or_else(|| host.to_string());
    format!("{what} is reached only through runner {}, which is offline (route {}).", s.exit, s.name)
}

// ---------------------------------------------------------------- state

#[derive(Default)]
struct State {
    routes: Vec<RouteState>,
    /// This runner's ID (None on Home, which is never an exit).
    me: Option<String>,
}

static STATE: std::sync::LazyLock<RwLock<State>> = std::sync::LazyLock::new(Default::default);
static APP: std::sync::OnceLock<App> = std::sync::OnceLock::new();

/// The routes with their exit's state, as this daemon knows them.
pub fn states() -> Vec<RouteState> {
    STATE.read().unwrap_or_else(|p| p.into_inner()).routes.clone()
}

fn state_of(route: &str) -> Option<RouteState> {
    STATE.read().unwrap_or_else(|p| p.into_inner()).routes.iter().find(|s| s.name == route).cloned()
}

fn me() -> Option<String> {
    STATE.read().unwrap_or_else(|p| p.into_inner()).me.clone()
}

/// Runner: the routes Home sent (`ToRunner::Settings`) and this runner's own ID.
pub fn set_states(routes: Vec<RouteState>, me: Option<String>) {
    let mut s = STATE.write().unwrap_or_else(|p| p.into_inner());
    s.routes = routes;
    s.me = me;
}

/// How a request leaves this daemon.
#[derive(Clone, Debug, PartialEq)]
pub enum Way {
    Direct,
    /// Through the bridge: the route's name and its exit runner's name.
    Exit { route: String, exit: String },
    /// Refused at once, with the one message.
    Offline(String),
}

fn decide(s: &RouteState, mode: Mode, host: &str, me: Option<&str>) -> Way {
    if s.exit_id.is_some() && s.exit_id.as_deref() == me {
        return Way::Direct;
    }
    if s.online {
        return Way::Exit { route: s.name.clone(), exit: s.exit.clone() };
    }
    match mode {
        Mode::Require => Way::Offline(offline_message(s, host)),
        Mode::Prefer => Way::Direct,
    }
}

/// The way of a URL.
pub fn way(url: &reqwest::Url) -> Way {
    let st = STATE.read().unwrap_or_else(|p| p.into_inner());
    match matching(&st.routes, url) {
        Some((s, mode, r)) => decide(s, mode, &r.host, st.me.as_deref()),
        None => Way::Direct,
    }
}

/// The way of a raw connection (whole-host routes only).
pub fn way_host(host: &str, port: u16) -> Way {
    let st = STATE.read().unwrap_or_else(|p| p.into_inner());
    match matching_host(&st.routes, host, port) {
        Some(s) => decide(s, Mode::Require, host, st.me.as_deref()),
        None => Way::Direct,
    }
}

/// Home: keeps the routes' states current, at start and on every change of config.toml or the runners, and
/// publishes `egress`/`changed` when they differ (runners get them in `ToRunner::Settings`).
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Home {
        return;
    }
    let _ = APP.set(app.clone());
    let app = app.clone();
    tokio::spawn(async move {
        let mut rx = app.bus.subscribe(None).rx;
        refresh(&app).await;
        loop {
            match rx.recv().await {
                Ok(d) => {
                    if d.text.contains(r#""topic":"config""#) || d.text.contains(r#""topic":"runners""#) {
                        refresh(&app).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => refresh(&app).await,
                Err(_) => break,
            }
        }
    });
}

/// Pure: the new states from the routes, the runners (id, name, connected, last connection) and the old states.
fn resolve(routes: &[ExitRoute], runners: &[(String, String, bool, Option<i64>)], old: &[RouteState], now: i64) -> Vec<RouteState> {
    routes
        .iter()
        .map(|r| {
            let via = r.via.trim();
            let runner = runners.iter().find(|(id, _, _, _)| id == via).or_else(|| runners.iter().find(|(_, name, _, _)| name.eq_ignore_ascii_case(via)));
            let online = runner.is_some_and(|x| x.2);
            let exit_id = runner.map(|x| x.0.clone());
            let prev = old.iter().find(|o| o.name == r.name && o.exit_id == exit_id);
            let since = match prev {
                Some(p) if p.online == online => p.since,
                Some(_) => Some(now),
                None if online => runner.and_then(|x| x.3).or(Some(now)),
                None => None,
            };
            let last_error = match runner {
                None => Some(format!("no runner named {via}")),
                Some(_) => prev.and_then(|p| p.last_error.clone()).filter(|e| !e.starts_with("no runner named")),
            };
            RouteState {
                name: r.name.clone(),
                kind: if r.github_org.is_some() { RouteKind::GithubOrg } else { RouteKind::Urls },
                github_org: r.github_org.clone(),
                urls: r.urls.clone(),
                prefixes: prefixes(r.github_org.as_deref(), &r.urls),
                exit: runner.map(|x| x.1.clone()).unwrap_or_else(|| via.to_string()),
                exit_id,
                online,
                since,
                last_error,
            }
        })
        .collect()
}

/// Home: computes the states again now (also after `egress.set`, so its answer holds them).
pub async fn refresh(app: &App) {
    let routes = app.config.get().egress;
    let runners: Vec<(String, String, bool, Option<i64>)> = match crate::runners::list(app).await {
        Ok(l) => l.into_iter().filter(|r| r.runner.state == crate::runners::RunnerState::Approved && !r.runner.burst).map(|r| (r.runner.id.clone(), r.runner.name.clone(), r.connected, r.runner.last_seen_at)).collect(),
        Err(e) => {
            tracing::warn!("exit routes: runners: {e:#}");
            return;
        }
    };
    let changed = {
        let mut st = STATE.write().unwrap_or_else(|p| p.into_inner());
        let new = resolve(&routes, &runners, &st.routes, crate::util::now_ms());
        let changed = new != st.routes;
        st.routes = new;
        changed
    };
    if changed {
        app.bus.publish("egress", "changed", serde_json::Value::Null);
    }
}

fn set_error(route: &str, error: Option<String>) {
    let changed = {
        let mut st = STATE.write().unwrap_or_else(|p| p.into_inner());
        match st.routes.iter_mut().find(|s| s.name == route) {
            Some(s) if s.last_error != error => {
                s.last_error = error;
                true
            }
            _ => false,
        }
    };
    if changed && let Some(app) = APP.get() {
        app.bus.publish("egress", "changed", serde_json::Value::Null);
    }
}

// ---------------------------------------------------------------- bridge

struct Bridge {
    port: u16,
    secret: String,
}

static BRIDGE: std::sync::OnceLock<std::result::Result<Bridge, String>> = std::sync::OnceLock::new();

/// The last refusal of the bridge: when, the route, the message (git shows only "503 from proxy after CONNECT").
static LAST_REFUSAL: Mutex<Option<(i64, String, String)>> = Mutex::new(None);

/// The bridge's last refusal within `max_age_ms`: its message.
pub fn last_refusal(max_age_ms: i64) -> Option<String> {
    let r = LAST_REFUSAL.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
    (crate::util::now_ms() - r.0 <= max_age_ms).then_some(r.2)
}

/// The loopback CONNECT bridge (started on first use): its port and the secret of its proxy credentials.
pub fn bridge() -> Result<(u16, String)> {
    let b = BRIDGE.get_or_init(|| {
        let start = || -> Result<Bridge> {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").context("start the exit bridge")?;
            listener.set_nonblocking(true)?;
            let port = listener.local_addr()?.port();
            let listener = tokio::net::TcpListener::from_std(listener)?;
            let secret = hex::encode(rand::random::<[u8; 24]>());
            let s = secret.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { continue };
                    let s = s.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_bridge(stream, &s).await {
                            tracing::debug!("exit bridge: {e:#}");
                        }
                    });
                }
            });
            tracing::info!("exit bridge on 127.0.0.1:{port}");
            Ok(Bridge { port, secret })
        };
        start().map_err(|e| format!("{e:#}"))
    });
    match b {
        Ok(b) => Ok((b.port, b.secret.clone())),
        Err(e) => anyhow::bail!("{e}"),
    }
}

/// The proxy URL of a route (`http://<route>:<secret>@127.0.0.1:<port>`).
pub fn proxy_url(route: &str) -> Result<String> {
    let (port, secret) = bridge()?;
    Ok(format!("http://{route}:{secret}@127.0.0.1:{port}"))
}

/// Home's git: `-c` arguments that send each route's git prefixes to the bridge (offline routes too, so the bridge
/// refuses them with the one message).
pub fn git_args() -> Vec<String> {
    let routes = states();
    if routes.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    for s in routes {
        let Ok(proxy) = proxy_url(&s.name) else { continue };
        let mut prefixes = vec![];
        match &s.github_org {
            Some(org) => {
                prefixes.push(format!("https://github.com/{org}/"));
                prefixes.push(format!("https://github.com/{}/", org.to_ascii_lowercase()));
            }
            None => prefixes.extend(s.urls.iter().cloned()),
        }
        prefixes.dedup();
        for p in prefixes {
            out.push("-c".into());
            out.push(format!("http.{p}.proxy={proxy}"));
            out.push("-c".into());
            out.push(format!("http.{p}.proxyAuthMethod=basic"));
        }
    }
    out
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A CONNECT target and the `route:secret` of its `Proxy-Authorization: Basic`.
type Connect = (String, u16, Option<(String, String)>);

/// Parses a CONNECT head.
fn parse_connect(head: &str) -> Option<Connect> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    if first.next()? != "CONNECT" {
        return None;
    }
    let authority = first.next()?;
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let mut auth = None;
    for l in lines {
        if let Some((k, v)) = l.split_once(':')
            && k.trim().eq_ignore_ascii_case("proxy-authorization")
            && let Some(b) = v.trim().strip_prefix("Basic ")
        {
            let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b.trim()).ok()?;
            let text = String::from_utf8(decoded).ok()?;
            auth = text.split_once(':').map(|(r, s)| (r.to_string(), s.to_string()));
        }
    }
    Some((host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase(), port, auth))
}

async fn serve_bridge(mut stream: tokio::net::TcpStream, secret: &str) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _ = stream.set_nodelay(true);
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    let end = loop {
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut chunk)).await.context("no CONNECT")??;
        anyhow::ensure!(n > 0, "closed before CONNECT");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        anyhow::ensure!(buf.len() < 16 * 1024, "CONNECT head too long");
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let rest = buf[end..].to_vec();
    let answer = |status: &str, text: &str| format!("HTTP/1.1 {status}\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
    let Some((host, port, auth)) = parse_connect(&head) else {
        stream.write_all(answer("400 Bad Request", "Launchpad exit bridge: CONNECT host:port only").as_bytes()).await?;
        return Ok(());
    };
    let route = match auth {
        Some((route, s)) if constant_eq(s.as_bytes(), secret.as_bytes()) => route,
        _ => {
            stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nproxy-authenticate: Basic realm=\"Launchpad exit\"\r\ncontent-length: 0\r\n\r\n").await?;
            return Ok(());
        }
    };
    let refuse = |text: String| {
        *LAST_REFUSAL.lock().unwrap_or_else(|p| p.into_inner()) = Some((crate::util::now_ms(), route.clone(), text.clone()));
        text
    };
    let (rd, mut wr) = stream.into_split();
    let mut upload = read_stream(rd);
    if !rest.is_empty() {
        let first: ByteStream = Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(rest)) }));
        upload = Box::pin(first.chain(upload));
    }
    match carry(&route, &host, port, upload).await {
        Ok(down) => {
            wr.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
            write_stream(down, wr).await;
        }
        Err(e) => {
            let text = refuse(e);
            wr.write_all(answer("503 Service Unavailable", &text).as_bytes()).await?;
        }
    }
    Ok(())
}

/// The bytes of a read half as a stream.
fn read_stream<R: tokio::io::AsyncRead + Unpin + Send + 'static>(rd: R) -> ByteStream {
    use tokio::io::AsyncReadExt;
    Box::pin(futures_util::stream::unfold(Some(rd), |rd| async move {
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
    }))
}

/// Writes a stream to a write half, then shuts it down.
async fn write_stream<W: tokio::io::AsyncWrite + Unpin>(mut body: ByteStream, mut wr: W) {
    use tokio::io::AsyncWriteExt;
    while let Some(Ok(part)) = body.next().await {
        if wr.write_all(&part).await.is_err() {
            return;
        }
    }
    let _ = wr.shutdown().await;
}

/// Connects a route's target through its exit, from whichever daemon this is: on Home a dial (`open`), on a runner
/// an `Egress` exchange to Home. The exit itself connects directly. Errors are the message to show.
pub async fn carry(route: &str, host: &str, port: u16, upload: ByteStream) -> std::result::Result<ByteStream, String> {
    let s = state_of(route).ok_or_else(|| format!("no exit route named {route}"))?;
    if !allowed(&s, host, port) {
        return Err(format!("route {route} does not lead to {host}:{port}"));
    }
    if s.exit_id.is_some() && s.exit_id == me() {
        let sock = connect_tcp(host, port).await?;
        let (rd, wr) = sock.into_split();
        tokio::spawn(write_stream(upload, wr));
        return Ok(read_stream(rd));
    }
    if !s.online {
        return Err(offline_message(&s, host));
    }
    if let Some(app) = APP.get() {
        return open(app, route, host, port, upload).await.map_err(|e| format!("{e:#}"));
    }
    let target = crate::runners::proto::HttpTarget::Egress { route: route.into(), host: host.into(), port };
    let reply = crate::home_link::http(target, "CONNECT", "", vec![], upload).await.map_err(|e| format!("Home: {e:#}"))?;
    if reply.status != 200 {
        let parts: Vec<bytes::Bytes> = futures_util::TryStreamExt::try_collect(reply.body).await.unwrap_or_default();
        let text = String::from_utf8_lossy(&parts.concat()).to_string();
        let message = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or(text);
        return Err(message);
    }
    Ok(reply.body)
}

/// A connection to a route's target as a socket-like stream (the runner proxy's `connect_out`).
pub async fn connect(route: &str, host: &str, port: u16) -> std::result::Result<tokio::io::DuplexStream, String> {
    let (ours, theirs) = tokio::io::duplex(256 * 1024);
    let (rd, wr) = tokio::io::split(theirs);
    let down = carry(route, host, port, read_stream(rd)).await?;
    tokio::spawn(write_stream(down, wr));
    Ok(ours)
}

/// Connects from this machine, for the exit: link-local and metadata addresses stay refused (`netcap::refused`);
/// private and tailnet addresses are allowed, since reaching them is the point of a route.
async fn connect_tcp(host: &str, port: u16) -> std::result::Result<tokio::net::TcpStream, String> {
    let connect = async {
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
        let allowed: Vec<std::net::SocketAddr> = addrs.iter().copied().filter(|a| !crate::netcap::refused(a.ip(), a.port())).collect();
        if allowed.is_empty() {
            return Err(std::io::Error::from_raw_os_error(libc::EACCES));
        }
        tokio::net::TcpStream::connect(&allowed[..]).await
    };
    match tokio::time::timeout(std::time::Duration::from_secs(30), connect).await {
        Ok(Ok(s)) => {
            let _ = s.set_nodelay(true);
            Ok(s)
        }
        Ok(Err(e)) => Err(format!("errno {}: connect {host}:{port}: {e}", crate::netcap::errno_of(&e))),
        Err(_) => Err(format!("errno {}: connect {host}:{port}: timed out", libc::ETIMEDOUT)),
    }
}

// ---------------------------------------------------------------- clients

/// The routed HTTP clients of one call site: the caller's direct client, or one whose proxy is the bridge with the
/// route's credentials, picked per request. reqwest's `Proxy::custom` sees only the origin, so it cannot route by
/// path: never use it for routes.
pub struct Clients {
    build: fn(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    cache: Mutex<Vec<(String, reqwest::Client)>>,
}

impl Clients {
    /// `build`: the call site's client settings (timeouts, redirects…), without a resolver of its own.
    pub const fn new(build: fn(reqwest::ClientBuilder) -> reqwest::ClientBuilder) -> Clients {
        Clients { build, cache: Mutex::new(Vec::new()) }
    }

    /// The client for `url`: `direct`, or a routed one with the exit's name; Err with the message when the route's
    /// exit is offline.
    pub fn pick(&self, direct: &reqwest::Client, url: &reqwest::Url) -> std::result::Result<(reqwest::Client, Option<String>), String> {
        match way(url) {
            Way::Direct => Ok((direct.clone(), None)),
            Way::Offline(m) => Err(m),
            Way::Exit { route, exit } => Ok((self.routed(&route)?, Some(exit))),
        }
    }

    fn routed(&self, route: &str) -> std::result::Result<reqwest::Client, String> {
        if let Some((_, c)) = self.cache.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(r, _)| r == route) {
            return Ok(c.clone());
        }
        let proxy = reqwest::Proxy::all(proxy_url(route).map_err(|e| format!("{e:#}"))?).map_err(|e| e.to_string())?;
        let build = self.build;
        // Loading the system's root certificates recurses deeply: build on a thread with a large stack.
        let client = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || build(reqwest::Client::builder()).proxy(proxy).build().map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "the routed HTTP client could not be built".to_string())??;
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).push((route.to_string(), client.clone()));
        Ok(client)
    }
}

// ---------------------------------------------------------------- Home: dials

struct Pending {
    exit: String,
    upload: Option<ByteStream>,
    tx: tokio::sync::oneshot::Sender<std::result::Result<ByteStream, String>>,
}

static PENDING: std::sync::LazyLock<Mutex<HashMap<u64, Pending>>> = std::sync::LazyLock::new(Default::default);
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Home: asks a route's exit to connect to `host:port` and returns its bytes; `upload` goes to the connection.
/// Waits 30 seconds at most, less when the exit disconnects.
pub async fn open(app: &App, route: &str, host: &str, port: u16, upload: ByteStream) -> Result<ByteStream> {
    anyhow::ensure!(crate::vault::here(app), "only Home dials exit runners");
    let s = state_of(route).with_context(|| format!("no exit route named {route}"))?;
    anyhow::ensure!(allowed(&s, host, port), "route {route} does not lead to {host}:{port}");
    let exit = match &s.exit_id {
        Some(id) if app.runners.connected(id) => id.clone(),
        _ => anyhow::bail!("{}", offline_message(&s, host)),
    };
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    PENDING.lock().unwrap_or_else(|p| p.into_inner()).insert(id, Pending { exit: exit.clone(), upload: Some(upload), tx });
    app.runners.send_to(&exit, crate::runners::proto::ToRunner::Dial { id, route: route.into(), host: host.into(), port });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    let result = loop {
        tokio::select! {
            r = &mut rx => break r.unwrap_or_else(|_| Err("the dial was dropped".into())),
            _ = tick.tick() => {
                if !app.runners.connected(&exit) {
                    break Err(offline_message(&s, host));
                }
                if tokio::time::Instant::now() >= deadline {
                    break Err(format!("runner {} did not connect to {host}:{port} within 30 seconds (route {route})", s.exit));
                }
            }
        }
    };
    PENDING.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
    match result {
        Ok(down) => {
            tracing::info!("exit route {route}: {host}:{port} through {}", s.exit);
            set_error(route, None);
            Ok(down)
        }
        Err(e) => {
            tracing::info!("exit route {route}: {host}:{port}: {e}");
            if !e.contains("which is offline") {
                set_error(route, Some(e.clone()));
            }
            anyhow::bail!("{e}")
        }
    }
}

/// Home: the far end of a dial, an exchange the exit opened (`HttpTarget::Dialed`): its request body is the
/// connection's bytes, the answer's body what goes to it. Only the runner that was asked.
pub fn dialed(runner_id: &str, id: u64, headers: &[(String, String)], body: ByteStream) -> Result<HttpReply> {
    let p = {
        let mut pending = PENDING.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(pending.get(&id).is_some_and(|p| p.exit == runner_id), "no dial {id} waits for this runner");
        pending.remove(&id).context("no such dial")?
    };
    let empty = || -> ByteStream { Box::pin(futures_util::stream::empty()) };
    if let Some((_, e)) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(DIAL_ERROR)) {
        let _ = p.tx.send(Err(e.clone()));
        return Ok(HttpReply { status: 200, headers: vec![], body: empty() });
    }
    let upload = p.upload.unwrap_or_else(empty);
    p.tx.send(Ok(body)).map_err(|_| anyhow::anyhow!("the dial was given up"))?;
    Ok(HttpReply { status: 200, headers: vec![], body: upload })
}

/// Home: a runner's routed connection (`HttpTarget::Egress`): dialed like Home's own. Refused for the exit itself,
/// which connects directly. A refusal answers 503 with the message.
pub async fn for_runner(app: &App, runner_id: &str, route: &str, host: &str, port: u16, body: ByteStream) -> Result<HttpReply> {
    let s = state_of(route).with_context(|| format!("no exit route named {route}"))?;
    anyhow::ensure!(s.exit_id.as_deref() != Some(runner_id), "this runner is the exit of route {route}: it connects directly");
    match open(app, route, host, port, body).await {
        Ok(down) => Ok(HttpReply { status: 200, headers: vec![], body: down }),
        Err(e) => {
            let text = format!("{e:#}");
            Ok(HttpReply { status: 503, headers: vec![("content-type".into(), "text/plain".into())], body: Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(text)) })) })
        }
    }
}

/// The exit runner: answers Home's `ToRunner::Dial`. It checks its own routes (it is the route's exit, the target is
/// the route's), connects from this machine and carries the bytes in an `HttpTarget::Dialed` exchange.
pub async fn dial(id: u64, route: String, host: String, port: u16) {
    let target = crate::runners::proto::HttpTarget::Dialed { id };
    let checked = match state_of(&route) {
        None => Err(format!("this runner has no exit route named {route}")),
        Some(s) if s.exit_id.is_none() || s.exit_id != me() => Err(format!("this runner is not the exit of route {route}")),
        Some(s) if !allowed(&s, &host, port) => Err(format!("route {route} does not lead to {host}:{port}")),
        Some(_) => Ok(()),
    };
    let sock = match checked {
        Ok(()) => connect_tcp(&host, port).await,
        Err(e) => Err(e),
    };
    tracing::info!("exit route {route}: dial {host}:{port}{}", sock.as_ref().err().map(|e| format!(": {e}")).unwrap_or_default());
    match sock {
        Err(e) => {
            let empty: ByteStream = Box::pin(futures_util::stream::empty());
            if let Err(e) = crate::home_link::http(target, "CONNECT", "", vec![(DIAL_ERROR.into(), e)], empty).await {
                tracing::debug!("exit route {route}: {e:#}");
            }
        }
        Ok(sock) => {
            let (rd, wr) = sock.into_split();
            match crate::home_link::http(target, "CONNECT", "", vec![], read_stream(rd)).await {
                Ok(reply) if reply.status == 200 => write_stream(reply.body, wr).await,
                Ok(reply) => tracing::debug!("exit route {route}: Home answered {}", reply.status),
                Err(e) => tracing::debug!("exit route {route}: {e:#}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(name: &str, org: Option<&str>, urls: &[&str], online: bool) -> RouteState {
        let urls: Vec<String> = urls.iter().map(|s| s.to_string()).collect();
        RouteState { name: name.into(), kind: if org.is_some() { RouteKind::GithubOrg } else { RouteKind::Urls }, github_org: org.map(str::to_string), prefixes: prefixes(org, &urls), urls, exit: "XPS-13".into(), exit_id: Some("r1".into()), online, since: None, last_error: None }
    }

    fn url(u: &str) -> reqwest::Url {
        reqwest::Url::parse(u).unwrap()
    }

    #[test]
    fn org_expands_to_eight_prefixes() {
        let r = rules(Some("HuggingFace-Internal"), &[]);
        assert_eq!(r.len(), 8);
        assert_eq!(r[0], Rule { host: "github.com".into(), port: 443, prefix: "/huggingface-internal".into(), mode: Mode::Require });
        assert_eq!(r.iter().filter(|r| r.mode == Mode::Prefer).count(), 2);
        let p = prefixes(Some("hf-int"), &[]);
        assert!(p.contains(&"https://github.com/hf-int/".to_string()), "{p:?}");
        assert!(p.contains(&"https://api.github.com/graphql".to_string()), "{p:?}");
        assert_eq!(prefixes(None, &["https://grafana.example.ts.net/".into()]), ["https://grafana.example.ts.net/"]);
    }

    #[test]
    fn matcher_uses_segments_case_and_order() {
        let routes = vec![st("org", Some("huggingface-internal"), &[], true)];
        let m = |u: &str| matching(&routes, &url(u)).map(|(s, mode, _)| (s.name.clone(), mode));
        assert_eq!(m("https://github.com/huggingface-internal/moon-landing.git/info/refs?service=git-upload-pack"), Some(("org".into(), Mode::Require)));
        assert_eq!(m("https://GitHub.com/HuggingFace-Internal/x"), Some(("org".into(), Mode::Require)));
        assert_eq!(m("https://api.github.com/repos/huggingface-internal"), Some(("org".into(), Mode::Require)));
        assert_eq!(m("https://api.github.com/repos/huggingface-internalx/a"), None);
        assert_eq!(m("https://github.com/huggingface/huggingface_hub"), None);
        assert_eq!(m("https://api.github.com/graphql"), Some(("org".into(), Mode::Prefer)));
        assert_eq!(m("https://api.github.com/graphqlx"), None);
        assert_eq!(m("https://api.github.com/search/issues?q=org:huggingface-internal"), Some(("org".into(), Mode::Prefer)));
        assert_eq!(m("http://github.com/huggingface-internal/x"), None);
        // Require beats prefer, then config order.
        let two = vec![st("a", Some("one"), &[], true), st("b", None, &["https://api.github.com/graphql"], true)];
        assert_eq!(matching(&two, &url("https://api.github.com/graphql")).map(|(s, m, _)| (s.name.clone(), m)), Some(("b".into(), Mode::Require)));
        let first = vec![st("a", None, &["https://x.example/"], true), st("b", None, &["https://x.example/api/"], true)];
        assert_eq!(matching(&first, &url("https://x.example/api/v1")).unwrap().0.name, "a");
    }

    #[test]
    fn offline_require_refuses_and_prefer_goes_direct() {
        let s = st("org", Some("acme"), &[], false);
        assert_eq!(decide(&s, Mode::Require, "github.com", None), Way::Offline("acme is reached only through runner XPS-13, which is offline (route org).".into()));
        assert_eq!(decide(&s, Mode::Prefer, "api.github.com", None), Way::Direct);
        assert_eq!(decide(&s, Mode::Require, "github.com", Some("r1")), Way::Direct);
        let on = st("org", Some("acme"), &[], true);
        assert_eq!(decide(&on, Mode::Prefer, "api.github.com", None), Way::Exit { route: "org".into(), exit: "XPS-13".into() });
        let host = st("g", None, &["https://grafana.tail.ts.net/"], false);
        assert_eq!(decide(&host, Mode::Require, "grafana.tail.ts.net", None), Way::Offline("grafana.tail.ts.net is reached only through runner XPS-13, which is offline (route g).".into()));
    }

    #[test]
    fn whole_host_and_targets() {
        let routes = vec![st("g", None, &["https://grafana.tail.ts.net/", "https://api.example.com/v1/"], true), st("org", Some("acme"), &[], true)];
        assert_eq!(matching_host(&routes, "Grafana.tail.ts.net", 443).map(|s| s.name.as_str()), Some("g"));
        assert!(matching_host(&routes, "grafana.tail.ts.net", 80).is_none());
        assert!(matching_host(&routes, "api.example.com", 443).is_none());
        assert!(matching_host(&routes, "github.com", 443).is_none());
        assert_eq!(targets(None, &["https://a.example/x/".into(), "https://a.example/y/".into(), "http://b.example:8080/".into()]), [("a.example".to_string(), 443), ("b.example".to_string(), 8080)]);
        assert_eq!(targets(Some("acme"), &[]).len(), 5);
        assert!(allowed(&routes[1], "api.github.com", 443));
        assert!(!allowed(&routes[1], "api.github.com", 22));
        assert!(!allowed(&routes[1], "example.com", 443));
    }

    #[test]
    fn validation() {
        let r = |name: &str, org: Option<&str>, urls: &[&str], via: &str| ExitRoute { name: name.into(), github_org: org.map(str::to_string), urls: urls.iter().map(|s| s.to_string()).collect(), via: via.into() };
        assert!(validate(&[r("hf-int", Some("huggingface-internal"), &[], "xps-13")]).is_ok());
        assert!(validate(&[r("g", None, &["https://grafana.tail.ts.net/", "http://10.0.0.2:3000/x"], "xps")]).is_ok());
        assert!(validate(&[r("Bad Name", Some("a"), &[], "x")]).is_err());
        assert!(validate(&[r("a", Some("a"), &[], "x"), r("a", Some("b"), &[], "x")]).is_err());
        assert!(validate(&[r("a", Some("a/b"), &[], "x")]).is_err());
        assert!(validate(&[r("a", Some("a"), &["https://x.example/"], "x")]).is_err());
        assert!(validate(&[r("a", None, &[], "x")]).is_err());
        assert!(validate(&[r("a", Some("a"), &[], " ")]).is_err());
        for bad in ["ftp://x.example/", "https://u:p@x.example/", "https://x.example/?a=1", "https://x.example/#f", "https://bad_host/"] {
            assert!(validate(&[r("a", None, &[bad], "x")]).is_err(), "{bad}");
        }
    }

    #[test]
    fn resolve_names_exits_and_keeps_since() {
        let routes = [ExitRoute { name: "org".into(), github_org: Some("acme".into()), urls: vec![], via: "xps-13".into() }];
        let runners = vec![("r1".to_string(), "XPS-13".to_string(), true, Some(5))];
        let a = resolve(&routes, &runners, &[], 100);
        assert_eq!((a[0].exit.as_str(), a[0].exit_id.as_deref(), a[0].online, a[0].since), ("XPS-13", Some("r1"), true, Some(5)));
        assert_eq!(resolve(&routes, &runners, &a, 200)[0].since, Some(5));
        let off = resolve(&routes, &[("r1".to_string(), "XPS-13".to_string(), false, Some(5))], &a, 300);
        assert_eq!((off[0].online, off[0].since), (false, Some(300)));
        let none = resolve(&routes, &[], &[], 100);
        assert_eq!((none[0].exit.as_str(), none[0].exit_id.as_deref(), none[0].last_error.as_deref()), ("xps-13", None, Some("no runner named xps-13")));
    }

    #[test]
    fn connect_head() {
        let auth = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "org:s3cret");
        let head = format!("CONNECT GitHub.com:443 HTTP/1.1\r\nHost: github.com:443\r\nProxy-Authorization: Basic {auth}\r\n\r\n");
        assert_eq!(parse_connect(&head), Some(("github.com".into(), 443, Some(("org".into(), "s3cret".into())))));
        assert_eq!(parse_connect("GET / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_connect("CONNECT [::1]:22 HTTP/1.1\r\n\r\n"), Some(("::1".into(), 22, None)));
    }
}

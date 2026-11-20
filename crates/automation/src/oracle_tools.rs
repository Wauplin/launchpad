//! The host side of `launchpad_host.call` (Home): what an oracle's `tools` do. Every call is checked, run and
//! logged here, never in the guest: the tool must be in the oracle's header (and, for github and huggingface,
//! granted to the workspace with a token), its arguments are checked, and it counts against the call and byte
//! budgets. `.get` of http, github and huggingface share one GET client: public addresses only (registry hosts by
//! name), the connection pinned to the checked address, a credential only for the tool's own host, redirects
//! checked hop by hop, size caps. Launchpad commands are GETs of Home's own router as a read-level principal of the workspace
//! (`ext::Extensions::oracle_read`). Tokens never reach the guest: answers are bodies only, and errors are Home's.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use launchpad_core::App;
use monty_types::{ExcType, MontyObject};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::oracle_eval::ErrorClass;
use crate::oracles::Tool;

/// Tool calls per evaluation.
pub const MAX_CALLS: usize = 200;
/// Response body per call, and bytes received per evaluation.
pub const MAX_BODY: usize = 10 << 20;
pub const MAX_TOTAL: u64 = 50 << 20;
/// Redirects followed per call.
pub const MAX_REDIRECTS: usize = 5;

/// One tool call of an evaluation (its call log), without request headers.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleCall")]
pub struct CallLog {
    pub tool: String,
    /// The call as people read it: `GET /repos/acme/app/actions/runs?branch=main`, `search.messages "to:me" limit=20`.
    pub call: String,
    /// `ok`, `HTTP 404`, or the error it raised (`PermissionError: …`).
    pub outcome: String,
    pub status: Option<u16>,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub duration_ms: i64,
}

/// What Home answers a call.
pub enum Reply {
    /// Resume the guest with this value.
    Value(MontyObject),
    /// Raise this exception in the guest; uncaught, it fails the evaluation with `class`.
    Raise { exc: ExcType, message: String, class: ErrorClass },
    /// End the evaluation at once (a budget ran out, the deadline passed).
    Stop { class: ErrorClass, message: String },
}

fn raise(exc: ExcType, class: ErrorClass, message: impl Into<String>) -> Reply {
    Reply::Raise { exc, message: message.into(), class }
}

fn permission(m: impl Into<String>) -> Reply {
    raise(ExcType::PermissionError, ErrorClass::Access, m)
}

fn value_error(m: impl Into<String>) -> Reply {
    raise(ExcType::ValueError, ErrorClass::Script, m)
}

/// The tools of one evaluation.
pub struct ToolBox {
    app: App,
    ws: String,
    tools: Vec<Tool>,
    deadline: Instant,
    pub calls: Vec<CallLog>,
    received: u64,
    tokens: HashMap<&'static str, Result<String, String>>,
    /// The tokens this evaluation used: never recorded (`oracle_eval::redact`).
    pub secrets: Vec<String>,
    trusted: Option<Vec<String>>,
}

impl ToolBox {
    pub fn new(app: &App, ws: &str, tools: &[Tool], deadline: Instant) -> ToolBox {
        ToolBox { app: app.clone(), ws: ws.to_string(), tools: tools.to_vec(), deadline, calls: vec![], received: 0, tokens: HashMap::new(), secrets: vec![], trusted: None }
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// One call of the guest: `tool` (http, github, huggingface, launchpad), `op` (`get` or a command such as
    /// `sessions.list`) and its arguments as JSON. Logs it, unless a budget stops the evaluation.
    pub async fn call(&mut self, tool: &str, op: &str, args: &str) -> Reply {
        if self.calls.len() >= MAX_CALLS {
            return Reply::Stop { class: ErrorClass::Limit, message: format!("more than {MAX_CALLS} tool calls") };
        }
        let started = Instant::now();
        let mut log = CallLog { tool: tool.to_string(), call: op.to_string(), outcome: String::new(), status: None, bytes: 0, duration_ms: 0 };
        let reply = self.dispatch(tool, op, args, &mut log).await;
        log.duration_ms = started.elapsed().as_millis() as i64;
        log.outcome = match &reply {
            Reply::Value(_) if log.status.is_some_and(|s| s >= 400) => format!("HTTP {}", log.status.unwrap_or_default()),
            Reply::Value(_) => "ok".into(),
            Reply::Raise { exc, message, .. } => format!("{exc}: {message}"),
            Reply::Stop { message, .. } => message.clone(),
        };
        self.calls.push(log);
        reply
    }

    async fn dispatch(&mut self, tool: &str, op: &str, args: &str, log: &mut CallLog) -> Reply {
        let Some(t) = Tool::parse(tool) else { return value_error(format!("there is no tool {tool}")) };
        if !self.tools.contains(&t) {
            return permission(format!("tools.{tool} is not in this oracle's header: add \"{tool}\" to `# tools = [...]`"));
        }
        let Ok(Value::Object(args)) = serde_json::from_str::<Value>(args) else { return value_error("the call's arguments are not a JSON object") };
        if self.remaining().is_zero() {
            return Reply::Stop { class: ErrorClass::Timeout, message: "the oracle's timeout passed".into() };
        }
        match (t, op) {
            (Tool::Launchpad, "get") => self.launchpad_get(&args, log).await,
            (Tool::Launchpad, _) => self.launchpad_command(op, &args, log).await,
            (_, "get") => self.get(t, &args, log).await,
            _ => value_error(format!("tools.{tool} has no command {op}")),
        }
    }

    /// The token of a credentialed tool, read once per evaluation.
    async fn token(&mut self, t: Tool) -> Result<String, Reply> {
        let Some(id) = t.registry() else { return Err(permission(format!("tools.{} needs no token", t.name()))) };
        if !self.tokens.contains_key(id) {
            let r = launchpad_core::vault::oracle_token(&self.app, &self.ws, id).await;
            if let Ok(s) = &r {
                self.secrets.push(s.clone());
            }
            self.tokens.insert(id, r);
        }
        match &self.tokens[id] {
            Ok(s) => Ok(s.clone()),
            Err(why) => Err(permission(format!("tools.{}: {why}", t.name()))),
        }
    }

    async fn trusted_hosts(&mut self) -> Vec<String> {
        if self.trusted.is_none() {
            self.trusted = Some(launchpad_core::vault::registry_hosts(&self.app).await);
        }
        self.trusted.clone().unwrap_or_default()
    }

    /// Home's own host names: refused like a private address.
    fn own_hosts(&self) -> Vec<String> {
        let mut out = vec![];
        if let Ok(h) = std::env::var("SPACE_HOST") {
            out.push(h.to_ascii_lowercase());
        }
        if let Some(o) = self.app.config.get().server.public_origin
            && let Ok(u) = reqwest::Url::parse(&o)
            && let Some(h) = u.host_str()
        {
            out.push(h.to_ascii_lowercase());
        }
        out
    }

    /// `.get` of http, github and huggingface.
    async fn get(&mut self, t: Tool, args: &Map<String, Value>, log: &mut CallLog) -> Reply {
        if let Some(k) = args.keys().find(|k| !["target", "params", "headers"].contains(&k.as_str())) {
            return value_error(format!("get takes no argument {k}"));
        }
        let Some(target) = args.get("target").and_then(Value::as_str) else { return value_error("the first argument of get is a string") };
        log.call = format!("GET {}", target.chars().take(300).collect::<String>());
        let pairs = match query_pairs(args.get("params").unwrap_or(&Value::Null)) {
            Ok(p) => p,
            Err(e) => return value_error(e),
        };
        let mut headers = match check_headers(args.get("headers").unwrap_or(&Value::Null)) {
            Ok(h) => h,
            Err(e) => return value_error(e),
        };
        let (url, credential) = match t {
            Tool::Http => match http_url(target, &pairs) {
                Ok(u) => (u, None),
                Err(e) => return value_error(e),
            },
            Tool::Github | Tool::Huggingface => {
                let base = if t == Tool::Github { GITHUB } else { HUGGINGFACE };
                let url = match api_url(base, target, &pairs) {
                    Ok(u) => u,
                    Err(e) => return value_error(e),
                };
                if t == Tool::Github {
                    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
                        headers.push(("Accept".into(), "application/vnd.github+json".into()));
                    }
                    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-github-api-version")) {
                        headers.push(("X-GitHub-Api-Version".into(), "2022-11-28".into()));
                    }
                }
                let token = match self.token(t).await {
                    Ok(s) => s,
                    Err(r) => return r,
                };
                (url, Some((host_of(base), format!("Bearer {token}"))))
            }
            Tool::Launchpad => unreachable!("dispatched before"),
        };
        log.call = format!("GET {}", shown(&url, t));
        match self.fetch(url, headers, credential).await {
            Ok((status, body, via)) => {
                if let Some(exit) = via {
                    log.call = format!("{} via {exit}", log.call);
                }
                log.status = Some(status);
                log.bytes = body.len() as u64;
                Reply::Value(response(status, &body, self.calls.len()))
            }
            Err(r) => r,
        }
    }

    /// GETs `url`, following redirects (each hop checked again), with the credential only on its own host. The
    /// shared client resolves host names through `Checked`, so it connects only to the addresses it checked. A URL of
    /// an exit route (`egress`) goes through its exit runner, which resolves and checks the host instead (Home's
    /// public-address rule does not apply to it); while that exit is offline the call raises OSError. The answer's
    /// last element names the exit of the last hop.
    async fn fetch(&mut self, mut url: reqwest::Url, headers: Vec<(String, String)>, credential: Option<(String, String)>) -> Result<(u16, Vec<u8>, Option<String>), Reply> {
        let trusted = self.trusted_hosts().await;
        set_trusted(&trusted);
        let own = self.own_hosts();
        let client = client().map_err(|e| raise(ExcType::OSError, ErrorClass::Http, e))?;
        for _hop in 0..=MAX_REDIRECTS {
            let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
            // `home.example.` is the same host as `home.example`.
            if own.contains(&host) || own.iter().any(|o| host.strip_suffix('.') == Some(o.as_str())) {
                return Err(permission(format!("tools.http does not call Home itself ({host})")));
            }
            let (hop_client, via) = match launchpad_core::egress::way(&url) {
                launchpad_core::egress::Way::Direct => (client.clone(), None),
                launchpad_core::egress::Way::Offline(m) => return Err(raise(ExcType::OSError, ErrorClass::Http, m)),
                launchpad_core::egress::Way::Exit { route, exit } => match ROUTED.pick(&client, &url) {
                    Ok((c, _)) => (c, Some(exit)),
                    Err(m) => return Err(raise(ExcType::OSError, ErrorClass::Http, format!("{m} (route {route})"))),
                },
            };
            if via.is_none()
                && let Ok(ip) = host.parse::<IpAddr>()
                && !public_ip(ip)
            {
                return Err(permission(format!("tools.http refuses {host}: not a public address")));
            }
            let mut req = hop_client.get(url.clone()).timeout(self.remaining());
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            if let Some((h, value)) = &credential
                && launchpad_core::vault::url_is_host(&url, h)
            {
                req = req.header(reqwest::header::AUTHORIZATION, value.as_str());
            }
            let mut res = match req.send().await {
                Ok(r) => r,
                Err(e) if refused(&e) => return Err(permission(format!("tools.http refuses {host}: it resolves to an address that is not public"))),
                Err(e) if e.is_timeout() || self.remaining().is_zero() => return Err(Reply::Stop { class: ErrorClass::Timeout, message: "the oracle's timeout passed during a call".into() }),
                Err(e) if via.is_some() && let Some(m) = launchpad_core::egress::refusal(&e) => return Err(raise(ExcType::OSError, ErrorClass::Http, m)),
                Err(e) => return Err(raise(ExcType::OSError, ErrorClass::Http, format!("{host}: {}", error_chain(&e)))),
            };
            let status = res.status().as_u16();
            if res.status().is_redirection()
                && let Some(loc) = res.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok())
            {
                url = match url.join(loc) {
                    Ok(u) if matches!(u.scheme(), "http" | "https") && u.username().is_empty() && u.password().is_none() => u,
                    _ => return Err(raise(ExcType::OSError, ErrorClass::Http, format!("{host} redirected to an address tools do not follow"))),
                };
                continue;
            }
            let mut body = Vec::new();
            loop {
                match res.chunk().await {
                    Ok(Some(c)) => {
                        body.extend_from_slice(&c);
                        self.received += c.len() as u64;
                        if body.len() > MAX_BODY {
                            return Err(Reply::Stop { class: ErrorClass::Limit, message: format!("a response is larger than {} MiB", MAX_BODY >> 20) });
                        }
                        if self.received > MAX_TOTAL {
                            return Err(Reply::Stop { class: ErrorClass::Limit, message: format!("more than {} MiB received", MAX_TOTAL >> 20) });
                        }
                    }
                    Ok(None) => break,
                    Err(e) if e.is_timeout() || self.remaining().is_zero() => return Err(Reply::Stop { class: ErrorClass::Timeout, message: "the oracle's timeout passed during a call".into() }),
                    Err(e) => return Err(raise(ExcType::OSError, ErrorClass::Http, format!("{host}: {}", error_chain(&e)))),
                }
            }
            return Ok((status, body, via));
        }
        Err(raise(ExcType::OSError, ErrorClass::Http, format!("more than {MAX_REDIRECTS} redirects")))
    }

    async fn launchpad_command(&mut self, op: &str, args: &Map<String, Value>, log: &mut CallLog) -> Reply {
        let path = match launchpad_path(op, args, &self.ws) {
            Ok(p) => p,
            Err(e) => return value_error(e),
        };
        log.call = format!("{op} → GET {path}");
        match self.read(&path, log).await {
            Ok((status, body)) if (200..300).contains(&status) => Reply::Value(MontyObject::string(body)),
            Ok((403, body)) => permission(api_error(&body)),
            Ok((_, body)) => raise(ExcType::RuntimeError, ErrorClass::Http, format!("launchpad: {}", api_error(&body))),
            Err(r) => r,
        }
    }

    async fn launchpad_get(&mut self, args: &Map<String, Value>, log: &mut CallLog) -> Reply {
        if let Some(k) = args.keys().find(|k| !["target", "params", "headers"].contains(&k.as_str())) {
            return value_error(format!("get takes no argument {k}"));
        }
        let Some(path) = args.get("target").and_then(Value::as_str) else { return value_error("the first argument of get is a path such as /api/context/sessions") };
        let pairs = match query_pairs(args.get("params").unwrap_or(&Value::Null)) {
            Ok(p) => p,
            Err(e) => return value_error(e),
        };
        let path = match launchpad_raw_path(path, &pairs) {
            Ok(p) => p,
            Err(e) => return value_error(e),
        };
        log.call = format!("GET {path}");
        match self.read(&path, log).await {
            Ok((403, body)) => permission(api_error(&body)),
            Ok((status, body)) => Reply::Value(response(status, body.as_bytes(), self.calls.len())),
            Err(r) => r,
        }
    }

    /// A GET of Home's router as a read-level principal of the oracle's workspace, in process.
    async fn read(&mut self, path: &str, log: &mut CallLog) -> Result<(u16, String), Reply> {
        let call = launchpad_core::ext::extensions().oracle_read(&self.app, &self.ws, path);
        let (status, body) = match tokio::time::timeout(self.remaining(), call).await {
            Err(_) => return Err(Reply::Stop { class: ErrorClass::Timeout, message: "the oracle's timeout passed during a call".into() }),
            Ok(Err(e)) => return Err(raise(ExcType::RuntimeError, ErrorClass::Http, format!("launchpad: {e:#}"))),
            Ok(Ok(r)) => r,
        };
        log.status = Some(status);
        log.bytes = body.len() as u64;
        self.received += body.len() as u64;
        if body.len() > MAX_BODY {
            return Err(Reply::Stop { class: ErrorClass::Limit, message: format!("a response is larger than {} MiB", MAX_BODY >> 20) });
        }
        if self.received > MAX_TOTAL {
            return Err(Reply::Stop { class: ErrorClass::Limit, message: format!("more than {} MiB received", MAX_TOTAL >> 20) });
        }
        Ok((status, body))
    }
}

const GITHUB: &str = "https://api.github.com";
const HUGGINGFACE: &str = "https://huggingface.co";

fn host_of(base: &str) -> String {
    base.trim_start_matches("https://").to_string()
}

/// The error text of a JSON error body (`{"error": "…"}`), else the body's start.
fn api_error(body: &str) -> String {
    serde_json::from_str::<Value>(body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_else(|| body.chars().take(300).collect())
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(x) = src {
        s.push_str(": ");
        s.push_str(&x.to_string());
        src = x.source();
    }
    s
}

/// The registry's host names (`vault::registry_hosts`): `Checked` lets them resolve to any address.
static TRUSTED: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

fn set_trusted(hosts: &[String]) {
    let mut t = TRUSTED.write().unwrap_or_else(|p| p.into_inner());
    if t.as_slice() != hosts {
        *t = hosts.to_vec();
    }
}

/// A host that resolves to an address that is not public.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} resolves to an address that is not public", self.0)
    }
}

impl std::error::Error for Refused {}

fn refused(e: &reqwest::Error) -> bool {
    let mut src: Option<&dyn std::error::Error> = Some(e);
    while let Some(x) = src {
        if x.is::<Refused>() || x.to_string().contains("resolves to an address that is not public") {
            return true;
        }
        src = x.source();
    }
    false
}

/// The resolver of the oracles' client: resolves once and gives the connector only addresses it checked (every one
/// public, unless the host is a registry host), so a second DNS answer can never change where it connects.
struct Checked;

impl reqwest::dns::Resolve for Checked {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_ascii_lowercase();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let trusted = TRUSTED.read().unwrap_or_else(|p| p.into_inner()).contains(&host);
            if addrs.is_empty() {
                return Err(format!("{host}: no address").into());
            }
            if !trusted && addrs.iter().any(|a| !public_ip(a.ip())) {
                return Err(Box::new(Refused(host)) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The oracle client's settings without its resolver, for URLs of exit routes: Home never resolves a routed host,
/// the exit does (and checks it against the route).
fn routed_client(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    b.redirect(reqwest::redirect::Policy::none()).no_proxy().user_agent(concat!("Launchpad-Oracle/", env!("CARGO_PKG_VERSION")))
}

static ROUTED: launchpad_core::egress::Clients = launchpad_core::egress::Clients::new(routed_client);

/// The one HTTP client of oracle tools: no proxy, no cookies, no redirects of its own (`fetch` follows them), the
/// checking resolver. Built once, on a thread with a large stack (loading the system's root certificates recurses).
fn client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(|| {
                    reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .no_proxy()
                        .dns_resolver(std::sync::Arc::new(Checked))
                        .user_agent(concat!("Launchpad-Oracle/", env!("CARGO_PKG_VERSION")))
                        .build()
                        .map_err(|e| format!("the HTTP client of oracles: {e}"))
                })
                .map_err(|e| e.to_string())?
                .join()
                .map_err(|_| "the HTTP client of oracles could not be built".to_string())?
        })
        .clone()
}

/// The guest's view of a response: status, the body as strict UTF-8 without a leading BOM (None when it is not
/// UTF-8), and the call's index in the call log.
fn response(status: u16, body: &[u8], call: usize) -> MontyObject {
    let text = decode(body).map_or_else(MontyObject::none, MontyObject::string);
    MontyObject::dict([(MontyObject::string("status_code"), MontyObject::int(i64::from(status))), (MontyObject::string("text"), text), (MontyObject::string("call"), MontyObject::int(call as i64))])
}

/// Pure: a body as strict UTF-8 without its byte-order mark; None when it is not valid UTF-8.
pub fn decode(body: &[u8]) -> Option<String> {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    std::str::from_utf8(body).ok().map(str::to_string)
}

/// Pure: what the call log shows of a URL (the path and query for an API tool, the whole URL for http).
fn shown(url: &reqwest::Url, t: Tool) -> String {
    match t {
        Tool::Http => url.to_string(),
        _ => match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        },
    }
}

/// Pure: whether an address is public. Loopback, private (RFC 1918, ULA), link-local (cloud metadata included),
/// carrier-grade NAT, multicast, unspecified, broadcast, documentation, benchmarking and reserved ranges are not,
/// nor IPv4-mapped, IPv4-compatible or NAT64 forms of them.
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => public_v4(a),
        IpAddr::V6(a) => public_v6(a),
    }
}

fn public_v4(a: Ipv4Addr) -> bool {
    let [o0, o1, o2, _] = a.octets();
    !(a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_multicast()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_documentation()
        || o0 == 0
        || (o0 == 100 && (64..128).contains(&o1))
        || (o0 == 192 && o1 == 0 && o2 == 0)
        || (o0 == 198 && (18..20).contains(&o1))
        || o0 >= 240)
}

fn public_v6(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = a.segments();
    // IPv4-compatible (::a.b.c.d) and NAT64 (64:ff9b::/96) carry an IPv4 address.
    if s[..6] == [0, 0, 0, 0, 0, 0] || s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return !a.is_unspecified() && !a.is_loopback() && public_v4(v4);
    }
    !(a.is_loopback()
        || a.is_unspecified()
        || a.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        || (s[0] & 0xffc0) == 0xfec0
        || (s[0] == 0x2001 && s[1] == 0x0db8)
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0]))
}

/// Pure: `params` as query pairs: lists repeat the key, booleans become true/false, None values are left out.
pub fn query_pairs(params: &Value) -> Result<Vec<(String, String)>, String> {
    let scalar = |k: &str, v: &Value| -> Result<Option<String>, String> {
        Ok(match v {
            Value::Null => None,
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(s.clone()),
            _ => return Err(format!("params[{k:?}] is a string, a number, a bool, None or a list of them")),
        })
    };
    let mut out = vec![];
    match params {
        Value::Null => {}
        Value::Object(m) => {
            for (k, v) in m {
                match v {
                    Value::Array(items) => {
                        for i in items {
                            if let Some(s) = scalar(k, i)? {
                                out.push((k.clone(), s));
                            }
                        }
                    }
                    v => {
                        if let Some(s) = scalar(k, v)? {
                            out.push((k.clone(), s));
                        }
                    }
                }
            }
        }
        _ => return Err("params is a dict".into()),
    }
    Ok(out)
}

/// Request headers an oracle may not set: credentials, cookies, the host and connection headers.
const REFUSED_HEADERS: [&str; 12] = ["authorization", "proxy-authorization", "cookie", "host", "connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "te", "trailer", "content-length"];

/// Pure: an oracle's request headers, checked (T7).
pub fn check_headers(headers: &Value) -> Result<Vec<(String, String)>, String> {
    let mut out = vec![];
    match headers {
        Value::Null => {}
        Value::Object(m) => {
            for (k, v) in m {
                let Some(v) = v.as_str() else { return Err(format!("headers[{k:?}] is a string")) };
                if REFUSED_HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
                    return Err(format!("the header {k} is refused: credentials come only from tools, and Home sets the connection headers"));
                }
                if reqwest::header::HeaderName::from_bytes(k.as_bytes()).is_err() || reqwest::header::HeaderValue::from_str(v).is_err() {
                    return Err(format!("the header {k} is not a valid header"));
                }
                out.push((k.clone(), v.to_string()));
            }
        }
        _ => return Err("headers is a dict of strings".into()),
    }
    Ok(out)
}

/// Pure: an absolute http(s) URL of `tools.http`, with `params` added to its query.
pub fn http_url(target: &str, pairs: &[(String, String)]) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(target).map_err(|e| format!("tools.http.get takes an absolute http(s) URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("tools.http.get takes an absolute http:// or https:// URL".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("a URL with a user name or password is refused: credentials come only from tools".into());
    }
    url.set_fragment(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(url)
}

/// Pure: `base` + an endpoint of an API tool (T5): it starts with `/`, and holds no scheme, host, `//`, `.` or `..`
/// segment (also encoded), fragment or control character, so the token only ever goes to the tool's host.
pub fn api_url(base: &str, endpoint: &str, pairs: &[(String, String)]) -> Result<reqwest::Url, String> {
    if !endpoint.starts_with('/') {
        return Err(format!("an endpoint starts with / (such as /repos/owner/name), not {endpoint:?}"));
    }
    if endpoint.contains("//") || endpoint.contains('#') || endpoint.contains('\\') || endpoint.chars().any(char::is_control) {
        return Err(format!("the endpoint {endpoint:?} holds //, #, \\ or a control character"));
    }
    let path = endpoint.split('?').next().unwrap_or_default();
    for seg in path.split('/') {
        let decoded = seg.replace("%2e", ".").replace("%2E", ".");
        if decoded == "." || decoded == ".." {
            return Err(format!("the endpoint {endpoint:?} holds a . or .. segment"));
        }
    }
    let mut url = reqwest::Url::parse(&format!("{base}{endpoint}")).map_err(|e| format!("the endpoint {endpoint:?}: {e}"))?;
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    if !launchpad_core::vault::url_is_host(&url, &host_of(base)) {
        return Err(format!("the endpoint {endpoint:?} leaves {}", host_of(base)));
    }
    Ok(url)
}

/// Pure: the path of a Launchpad command (`sessions.list` → `/api/context/sessions`), lists narrowed to `ws` by Home.
pub fn launchpad_path(op: &str, args: &Map<String, Value>, ws: &str) -> Result<String, String> {
    let str_arg = |k: &str| -> Result<String, String> {
        let v = args.get(k).and_then(Value::as_str).ok_or_else(|| format!("{op} takes {k} as a string"))?;
        if v.is_empty() || v.len() > 200 || v.contains(['/', '?', '#', '%', '\\']) || v == "." || v == ".." || v.chars().any(char::is_control) {
            return Err(format!("{k} {v:?} is not a valid name"));
        }
        Ok(v.to_string())
    };
    let allowed: &[&str] = match op {
        "sessions.show" => &["session_id"],
        "topics.show" => &["topic"],
        "oracle.show" => &["slug"],
        "oracle.history" => &["slug", "failed", "limit"],
        "notes" => &["repo", "number"],
        _ => &[],
    };
    if let Some(k) = args.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("{op} takes no argument {k}"));
    }
    let q = |pairs: &[(&str, String)]| {
        let mut u = reqwest::Url::parse("http://local/").expect("static");
        u.query_pairs_mut().extend_pairs(pairs);
        u.query().unwrap_or_default().to_string()
    };
    Ok(match op {
        "folders.list" => "/api/context/folders".into(),
        "bots.list" => "/api/context/bots".into(),
        "sessions.list" => "/api/context/sessions".into(),
        "sessions.show" => format!("/api/context/sessions/{}", str_arg("session_id")?),
        "schedules.list" => "/api/context/schedules".into(),
        "proposals.list" => "/api/proposals".into(),
        "today" => "/api/today".into(),
        "activity.list" => "/api/activity".into(),
        "profiles.list" => "/api/context/profiles".into(),
        "presets.list" => "/api/context/presets".into(),
        "egress.list" => "/api/context/egress".into(),
        "topics.list" => "/api/context/topics".into(),
        "topics.show" => format!("/api/context/topics?{}", q(&[("id", str_arg("topic")?)])),
        "oracle.list" => format!("/api/oracles/{ws}"),
        "oracle.show" => format!("/api/oracles/{ws}/{}", str_arg("slug")?),
        "oracle.history" => {
            let failed = args.get("failed").and_then(Value::as_bool).unwrap_or(false);
            let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(20).clamp(1, 500);
            format!("/api/oracles/{ws}/{}/evaluations?{}", str_arg("slug")?, q(&[("failed", failed.to_string()), ("limit", limit.to_string())]))
        }
        "notes" => {
            let repo = args.get("repo").and_then(Value::as_str).filter(|r| r.split_once('/').is_some_and(|(o, n)| launchpad_core::github::valid_name(o) && launchpad_core::github::valid_name(n))).ok_or("notes takes repo as owner/name")?;
            let number = args.get("number").and_then(Value::as_i64).filter(|n| *n > 0).ok_or("notes takes number as a positive int")?;
            format!("/api/context/notes?{}", q(&[("repo", repo.to_string()), ("number", number.to_string())]))
        }
        _ => return Err(format!("tools.launchpad has no command {op}")),
    })
}

/// Pure: the path of `tools.launchpad.get` (a path of Home's API), with `params` added to its query.
pub fn launchpad_raw_path(path: &str, pairs: &[(String, String)]) -> Result<String, String> {
    if !path.starts_with("/api/") || path.contains("//") || path.contains('#') || path.contains('\\') || path.chars().any(char::is_control) {
        return Err(format!("tools.launchpad.get takes a path of Home's API (/api/…), not {path:?}"));
    }
    let bare = path.split('?').next().unwrap_or_default();
    if bare.split('/').any(|s| matches!(s.replace("%2e", ".").replace("%2E", ".").as_str(), "." | "..")) {
        return Err(format!("the path {path:?} holds a . or .. segment"));
    }
    let mut url = reqwest::Url::parse(&format!("http://local{path}")).map_err(|e| format!("the path {path:?}: {e}"))?;
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T3: loopback, private, link-local and metadata, CGNAT, documentation, unspecified and mapped forms are refused.
    #[test]
    fn classifies_addresses() {
        for a in ["127.0.0.1", "10.0.0.1", "172.16.5.4", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255", "192.0.2.1", "198.18.0.1", "240.0.0.1", "::1", "::", "fe80::1", "fd00::1", "fc00::1", "ff02::1", "2001:db8::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "::127.0.0.1", "64:ff9b::a00:1", "100::1"] {
            assert!(!public_ip(a.parse().unwrap()), "{a}");
        }
        for a in ["1.1.1.1", "140.82.112.3", "2606:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(public_ip(a.parse().unwrap()), "{a}");
        }
    }

    /// T5: an endpoint never leaves the tool's host.
    #[test]
    fn api_endpoints_stay_on_their_host() {
        for bad in ["https://evil.example/x", "//evil.example/x", "/../x", "/x#y", "repos/a", "/a/%2e%2e/b", "/a/./b", "/a\\b", "/a\nb", "/a//b", "@evil.example/x"] {
            assert!(api_url(GITHUB, bad, &[]).is_err(), "{bad}");
        }
        let u = api_url(GITHUB, "/repos/a/b/pulls?state=open", &[("per_page".into(), "100".into())]).unwrap();
        assert_eq!(u.as_str(), "https://api.github.com/repos/a/b/pulls?state=open&per_page=100");
        assert_eq!(api_url(HUGGINGFACE, "/api/models/gpt2", &[]).unwrap().host_str(), Some("huggingface.co"));
    }

    /// T7: credentials, cookies, the host and connection headers are refused.
    #[test]
    fn refuses_credential_headers() {
        for h in ["Authorization", "authorization", "Cookie", "Proxy-Authorization", "Host", "Connection", "Transfer-Encoding"] {
            assert!(check_headers(&serde_json::json!({h: "x"})).is_err(), "{h}");
        }
        assert_eq!(check_headers(&serde_json::json!({"Accept": "application/vnd.github.diff"})).unwrap().len(), 1);
        assert!(check_headers(&serde_json::json!({"X": "a\nb"})).is_err());
        assert!(check_headers(&serde_json::json!({"X": 1})).is_err());
    }

    #[test]
    fn encodes_params() {
        let p = query_pairs(&serde_json::json!({"state": "open", "per_page": 100, "draft": false, "label": ["a", "b"], "skip": null})).unwrap();
        assert_eq!(p, vec![("draft".into(), "false".into()), ("label".into(), "a".into()), ("label".into(), "b".into()), ("per_page".into(), "100".into()), ("state".into(), "open".into())]);
        assert!(query_pairs(&serde_json::json!({"x": {"y": 1}})).is_err());
        let u = http_url("https://pypi.org/pypi/x/json#frag", &p[..1]).unwrap();
        assert_eq!(u.as_str(), "https://pypi.org/pypi/x/json?draft=false");
        assert!(http_url("ftp://x/y", &[]).is_err() && http_url("https://u:p@x/y", &[]).is_err() && http_url("/x", &[]).is_err());
    }

    /// T15: bodies decode as strict UTF-8 without a BOM; anything else is None.
    #[test]
    fn decodes_bodies() {
        assert_eq!(decode(b"\xEF\xBB\xBF{\"a\": 1}").as_deref(), Some("{\"a\": 1}"));
        assert_eq!(decode(b"").as_deref(), Some(""));
        assert_eq!(decode(b"caf\xe9"), None);
        assert_eq!(decode(b"\x89PNG\r\n\x1a\n\x00"), None);
        assert_eq!(decode(b"null").as_deref(), Some("null"));
    }

    /// T10 (the pure part): Launchpad commands map to fixed read routes, with checked values.
    #[test]
    fn launchpad_commands_map_to_read_routes() {
        let args = |v: Value| -> Map<String, Value> { serde_json::from_value(v).unwrap() };
        assert_eq!(launchpad_path("sessions.list", &Map::new(), "w1").unwrap(), "/api/context/sessions");
        assert_eq!(launchpad_path("egress.list", &Map::new(), "w1").unwrap(), "/api/context/egress");
        assert!(launchpad_path("egress.list", &args(serde_json::json!({"route": "x"})), "w1").is_err());
        assert_eq!(launchpad_path("oracle.history", &args(serde_json::json!({"slug": "ci-red", "failed": true, "limit": 5})), "w1").unwrap(), "/api/oracles/w1/ci-red/evaluations?failed=true&limit=5");
        assert_eq!(launchpad_path("notes", &args(serde_json::json!({"repo": "a/b", "number": 3})), "w1").unwrap(), "/api/context/notes?repo=a%2Fb&number=3");
        assert!(launchpad_path("sessions.show", &args(serde_json::json!({"session_id": "../config"})), "w1").is_err());
        assert!(launchpad_path("sessions.delete", &Map::new(), "w1").is_err());
        assert!(launchpad_path("sessions.list", &args(serde_json::json!({"workspace": "w2"})), "w1").is_err());
        assert_eq!(launchpad_raw_path("/api/sessions/s1/trace", &[]).unwrap(), "/api/sessions/s1/trace");
        for bad in ["/config", "api/x", "/api/../config", "/api/x#y", "//api/x"] {
            assert!(launchpad_raw_path(bad, &[]).is_err(), "{bad}");
        }
    }
}

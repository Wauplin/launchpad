//! The runner proxy: agents use Home's accounts without holding a token.
//!
//! It listens on `127.0.0.1` outside the jail. Agents reach it through `HTTPS_PROXY` (a per-session key in the
//! proxy credentials) and, for harness logins, through a base URL (`/lp/<key>/<host>/…`). On a host where the
//! session's lease has a credential, the proxy terminates TLS with a certificate of the runner's own CA (trusted
//! only inside the jail), adds the real token and records host, method and path in the access log. Every other
//! host is a plain tunnel: the proxy never reads or judges that traffic. What a request may do is decided by the
//! token's scope on Home, not here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::vault::{Auth, Lease};

type Body = BoxBody<Bytes, std::io::Error>;

/// One request that received a token.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Access {
    #[ts(type = "number")]
    pub at: i64,
    pub session_id: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// What it did, when known: "opened PR #3421", "GraphQL mutation …".
    pub action: Option<String>,
}

/// A lease held in memory for one session.
pub struct Active {
    pub key: String,
    pub session_id: String,
    pub lease: RwLock<Lease>,
    /// Last successful renewal with Home (ms).
    pub renewed_at: std::sync::atomic::AtomicI64,
}

#[derive(Clone, Default)]
pub struct Proxy {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    port: tokio::sync::OnceCell<u16>,
    app: std::sync::OnceLock<App>,
    ca: std::sync::OnceLock<Ca>,
    leases: Mutex<HashMap<String, Arc<Active>>>,
    tls: Mutex<HashMap<String, Arc<rustls::ServerConfig>>>,
}

struct Ca {
    cert_pem: String,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
}

const CA_NAME: &str = "Launchpad runner proxy";

fn ca_params() -> rcgen::CertificateParams {
    let mut p = rcgen::CertificateParams::default();
    p.distinguished_name = rcgen::DistinguishedName::new();
    p.distinguished_name.push(rcgen::DnType::CommonName, CA_NAME);
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    p.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign, rcgen::KeyUsagePurpose::DigitalSignature];
    p
}

/// The runner's CA, created once in `<data_dir>/proxy/` (key mode 0600, never mounted in a jail).
fn load_ca(dir: &std::path::Path) -> Result<Ca> {
    std::fs::create_dir_all(dir)?;
    let (cert_file, key_file) = (dir.join("ca.pem"), dir.join("ca-key.pem"));
    if let (Ok(cert_pem), Ok(key_pem)) = (std::fs::read_to_string(&cert_file), std::fs::read_to_string(&key_file)) {
        let key = rcgen::KeyPair::from_pem(&key_pem)?;
        return Ok(Ca { cert_pem, issuer: rcgen::Issuer::new(ca_params(), key) });
    }
    let key = rcgen::KeyPair::generate()?;
    let mut params = ca_params();
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2044, 1, 1);
    let cert = params.self_signed(&key)?;
    crate::paths::write_private(&key_file, &key.serialize_pem())?;
    std::fs::write(&cert_file, cert.pem())?;
    Ok(Ca { cert_pem: cert.pem(), issuer: rcgen::Issuer::new(ca_params(), key) })
}

/// System CA bundles, in the usual Linux locations.
const SYSTEM_BUNDLES: &[&str] = &["/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt", "/etc/ssl/ca-bundle.pem", "/etc/ssl/cert.pem"];

impl Proxy {
    /// Starts the proxy once and returns its port.
    pub async fn ensure(&self, app: &App) -> Result<u16> {
        let inner = self.inner.clone();
        let port = self
            .inner
            .port
            .get_or_try_init(|| async move {
                let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
                let ca = load_ca(&app.paths.data_dir.join("proxy"))?;
                let _ = inner.ca.set(ca);
                let _ = inner.app.set(app.clone());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.context("start the runner proxy")?;
                let port = listener.local_addr()?.port();
                let accept_inner = inner.clone();
                tokio::spawn(async move {
                    loop {
                        let Ok((stream, _)) = listener.accept().await else { continue };
                        let inner = accept_inner.clone();
                        tokio::spawn(async move {
                            let service = hyper::service::service_fn(move |req| handle(inner.clone(), req));
                            let _ = hyper::server::conn::http1::Builder::new().preserve_header_case(true).serve_connection(TokioIo::new(stream), service).with_upgrades().await;
                        });
                    }
                });
                tracing::info!("runner proxy on 127.0.0.1:{port}");
                Ok::<u16, anyhow::Error>(port)
            })
            .await?;
        Ok(*port)
    }

    /// The CA certificate, and a bundle of the system CAs plus it, for the jail.
    pub fn ca_files(&self, app: &App) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
        let ca = self.inner.ca.get().context("the proxy is not started")?;
        let dir = app.paths.data_dir.join("proxy/public");
        std::fs::create_dir_all(&dir)?;
        let (cert, bundle) = (dir.join("launchpad-ca.pem"), dir.join("ca-bundle.pem"));
        std::fs::write(&cert, &ca.cert_pem)?;
        let system = SYSTEM_BUNDLES.iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        std::fs::write(&bundle, format!("{system}\n{}", ca.cert_pem))?;
        Ok((cert, bundle))
    }

    /// Holds a lease and returns its proxy key. A newer lease of the same session replaces the old one.
    pub fn register(&self, lease: Lease) -> Arc<Active> {
        let mut leases = self.inner.leases.lock().unwrap();
        leases.retain(|_, a| a.session_id != lease.session_id);
        let active = Arc::new(Active {
            key: hex::encode(rand::random::<[u8; 24]>()),
            session_id: lease.session_id.clone(),
            lease: RwLock::new(lease),
            renewed_at: std::sync::atomic::AtomicI64::new(crate::util::now_ms()),
        });
        leases.insert(active.key.clone(), active.clone());
        active
    }

    pub fn for_session(&self, session_id: &str) -> Option<Arc<Active>> {
        self.inner.leases.lock().unwrap().values().find(|a| a.session_id == session_id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<Active>> {
        self.inner.leases.lock().unwrap().values().cloned().collect()
    }

    /// Forgets a session's lease: requests stop receiving its tokens at once.
    pub fn drop_session(&self, session_id: &str) -> Option<Arc<Active>> {
        let mut leases = self.inner.leases.lock().unwrap();
        let key = leases.iter().find(|(_, a)| a.session_id == session_id).map(|(k, _)| k.clone())?;
        leases.remove(&key)
    }

    pub fn drop_lease(&self, lease_id: &str) {
        self.inner.leases.lock().unwrap().retain(|_, a| a.lease.read().unwrap().id != lease_id);
    }

    fn by_key(&self, key: &str) -> Option<Arc<Active>> {
        self.inner.leases.lock().unwrap().get(key).cloned()
    }
}

fn full(status: StatusCode, text: &str) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from(text.to_string())).map_err(|never| match never {}).boxed());
    *r.status_mut() = status;
    r
}

/// `Proxy-Authorization: Basic base64(lp:<key>)`.
fn proxy_key(req: &Request<Incoming>) -> Option<String> {
    use base64::Engine;
    let v = req.headers().get(hyper::header::PROXY_AUTHORIZATION)?.to_str().ok()?;
    let b = base64::engine::general_purpose::STANDARD.decode(v.strip_prefix("Basic ")?.trim()).ok()?;
    let text = String::from_utf8(b).ok()?;
    Some(text.split_once(':')?.1.to_string())
}

fn credential_for(active: &Active, host: &str) -> Option<(String, Auth)> {
    let lease = active.lease.read().unwrap();
    lease.credentials.iter().find_map(|c| c.hosts.iter().find(|h| h.matches(host)).map(|h| (c.secret.clone(), h.auth.clone())))
}

async fn handle(inner: Arc<Inner>, req: Request<Incoming>) -> Result<Response<Body>, std::io::Error> {
    let proxy = Proxy { inner: inner.clone() };
    if req.method() == hyper::Method::CONNECT {
        let authority = req.uri().authority().map(|a| a.to_string()).unwrap_or_default();
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
            None => (authority.clone(), 443),
        };
        let active = proxy_key(&req).and_then(|k| proxy.by_key(&k));
        let intercept = port == 443 && active.as_ref().is_some_and(|a| credential_for(a, &host).is_some());
        tokio::spawn(async move {
            let upgraded = match hyper::upgrade::on(req).await {
                Ok(u) => u,
                Err(e) => return tracing::debug!("proxy upgrade: {e}"),
            };
            let result = match (intercept, active) {
                (true, Some(active)) => intercept_tls(inner, active, host.clone(), upgraded).await,
                _ => tunnel(upgraded, &host, port).await,
            };
            if let Err(e) = result {
                tracing::debug!("proxy {host}:{port}: {e:#}");
            }
        });
        return Ok(Response::new(http_body_util::Empty::new().map_err(|never| match never {}).boxed()));
    }
    // Base URLs of harness logins: /lp/<key>/<host>/<path>.
    let path = req.uri().path().to_string();
    if let Some(rest) = path.strip_prefix("/lp/") {
        let mut parts = rest.splitn(3, '/');
        let (key, host, tail) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").to_string(), parts.next().unwrap_or(""));
        let Some(active) = proxy.by_key(key) else { return Ok(full(StatusCode::FORBIDDEN, "unknown or revoked Launchpad lease")) };
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        let target = format!("/{tail}{query}");
        return Ok(forward(&inner, Some(active), &host, &target, req).await);
    }
    // Plain HTTP through the proxy: passed through, never with a token.
    if req.uri().scheme_str() == Some("http") {
        return Ok(plain_http(req).await);
    }
    Ok(full(StatusCode::BAD_REQUEST, "Launchpad runner proxy"))
}

async fn tunnel(upgraded: hyper::upgrade::Upgraded, host: &str, port: u16) -> Result<()> {
    let mut upstream = tokio::net::TcpStream::connect((host, port)).await?;
    let mut client = TokioIo::new(upgraded);
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

fn tls_for(inner: &Inner, host: &str) -> Result<Arc<rustls::ServerConfig>> {
    if let Some(c) = inner.tls.lock().unwrap().get(host) {
        return Ok(c.clone());
    }
    let ca = inner.ca.get().context("no CA")?;
    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(vec![host.to_string()])?;
    params.distinguished_name.push(rcgen::DnType::CommonName, host);
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2044, 1, 1);
    let cert = params.signed_by(&key, &ca.issuer)?;
    let der = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()));
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], der)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let config = Arc::new(config);
    inner.tls.lock().unwrap().insert(host.to_string(), config.clone());
    Ok(config)
}

async fn intercept_tls(inner: Arc<Inner>, active: Arc<Active>, host: String, upgraded: hyper::upgrade::Upgraded) -> Result<()> {
    let config = tls_for(&inner, &host)?;
    let tls = tokio_rustls::TlsAcceptor::from(config).accept(TokioIo::new(upgraded)).await.context("TLS with the agent")?;
    let service = hyper::service::service_fn(move |req: Request<Incoming>| {
        let (inner, active, host) = (inner.clone(), active.clone(), host.clone());
        async move {
            let target = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_else(|| "/".into());
            Ok::<_, std::io::Error>(forward(&inner, Some(active), &host, &target, req).await)
        }
    });
    hyper::server::conn::http1::Builder::new().preserve_header_case(true).serve_connection(TokioIo::new(tls), service).await?;
    Ok(())
}

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy().http1_only().build().expect("proxy HTTP client")
});

pub(crate) const HOP: &[&str] = &["connection", "proxy-connection", "keep-alive", "proxy-authorization", "proxy-authenticate", "te", "trailer", "transfer-encoding", "upgrade", "host"];

/// Requests whose body the access log reads to name the action (small JSON bodies only).
pub(crate) fn wants_body(host: &str, method: &str, path: &str) -> (bool, bool) {
    let p = path.split('?').next().unwrap_or("");
    let req = method == "POST" && ((host == "api.github.com" && p == "/graphql") || (host == "huggingface.co" && p == "/api/repos/create"));
    let res = method == "POST" && host == "api.github.com" && (p.ends_with("/pulls") || p.ends_with("/issues"));
    (req, res)
}

/// Sets a token on a request the way a host rule says.
pub fn add_auth(headers: &mut reqwest::header::HeaderMap, url: &mut reqwest::Url, auth: &Auth, secret: &str) {
    use base64::Engine;
    let mut set = |name: &str, value: String| {
        if let (Ok(n), Ok(v)) = (reqwest::header::HeaderName::from_bytes(name.as_bytes()), reqwest::header::HeaderValue::from_str(&value)) {
            headers.insert(n, v);
        }
    };
    match auth {
        Auth::Bearer => set("authorization", format!("Bearer {secret}")),
        Auth::Basic { user } => set("authorization", format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}")))),
        Auth::Header { name, prefix } => set(name, format!("{prefix}{secret}")),
        Auth::Query { param } => {
            let pairs: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != param).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            url.query_pairs_mut().clear().extend_pairs(pairs).append_pair(param, secret);
        }
    }
}

/// Adds the credential (if the lease has one for this host), sends the request to `https://<host><target>`
/// and streams the answer back.
async fn forward(inner: &Inner, active: Option<Arc<Active>>, host: &str, target: &str, req: Request<Incoming>) -> Response<Body> {
    let credential = active.as_ref().and_then(|a| credential_for(a, host));
    let method = req.method().as_str().to_string();
    let mut url = match reqwest::Url::parse(&format!("https://{host}{target}")) {
        Ok(u) => u,
        Err(e) => return full(StatusCode::BAD_REQUEST, &format!("bad target: {e}")),
    };
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in req.headers() {
        if !HOP.contains(&k.as_str()) {
            headers.append(k.clone(), v.clone());
        }
    }
    // A relayed lease (Burst): Home adds the token, sends the request and records it.
    let relayed = active.as_ref().filter(|_| credential.is_some()).map(|a| a.lease.read().unwrap().clone()).filter(|l| l.relayed);
    if let Some(lease) = relayed {
        return relay(lease.id, host, &method, target, &headers, req).await;
    }
    if let Some((secret, auth)) = &credential {
        add_auth(&mut headers, &mut url, auth, secret);
    }
    let path = url.path().to_string();
    let (read_req, read_res) = if credential.is_some() { wants_body(host, &method, &path) } else { (false, false) };
    let mut request_bytes = None;
    let body = if read_req {
        match http_body_util::Limited::new(req.into_body(), 2 << 20).collect().await {
            Ok(b) => {
                let b = b.to_bytes();
                request_bytes = Some(b.clone());
                reqwest::Body::from(b)
            }
            Err(e) => return full(StatusCode::BAD_REQUEST, &format!("request body: {e}")),
        }
    } else {
        reqwest::Body::wrap_stream(http_body_util::BodyDataStream::new(req.into_body()))
    };
    let Ok(m) = reqwest::Method::from_bytes(method.as_bytes()) else { return full(StatusCode::BAD_REQUEST, "bad method") };
    let res = match CLIENT.request(m, url).headers(headers).body(body).send().await {
        Ok(r) => r,
        Err(e) => return full(StatusCode::BAD_GATEWAY, &format!("Launchpad proxy: {host}: {e}")),
    };
    let status = res.status().as_u16();
    let mut out = Response::builder().status(status);
    for (k, v) in res.headers() {
        if !HOP.contains(&k.as_str()) {
            out = out.header(k, v);
        }
    }
    let (body, response_bytes): (Body, Option<Bytes>) = if read_res {
        match res.bytes().await {
            Ok(b) => (Full::new(b.clone()).map_err(|never| match never {}).boxed(), Some(b)),
            Err(e) => return full(StatusCode::BAD_GATEWAY, &format!("{e}")),
        }
    } else {
        let stream = res.bytes_stream().map_ok(Frame::data).map_err(std::io::Error::other);
        (BodyExt::boxed(StreamBody::new(stream)), None)
    };
    if credential.is_some()
        && let (Some(active), Some(app)) = (active, inner.app.get())
    {
        let action = crate::vault::describe(host, &method, &path, request_bytes.as_deref(), response_bytes.as_deref());
        crate::contain::record_access(app, Access { at: crate::util::now_ms(), session_id: active.session_id.clone(), host: host.into(), method, path, status, action });
    }
    out.body(body).unwrap_or_else(|e| full(StatusCode::BAD_GATEWAY, &format!("{e}")))
}

/// Sends a request of a relayed lease through the tunnel; Home adds the token.
async fn relay(lease_id: String, host: &str, method: &str, target: &str, headers: &reqwest::header::HeaderMap, req: Request<Incoming>) -> Response<Body> {
    let headers: Vec<(String, String)> = headers.iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
    let body: crate::home_link::ByteStream = Box::pin(http_body_util::BodyDataStream::new(req.into_body()).map_err(std::io::Error::other));
    match crate::home_link::http(crate::runners::proto::HttpTarget::Lease { lease_id, host: host.into() }, method, target, headers, body).await {
        Ok(reply) => {
            let mut out = Response::builder().status(reply.status);
            for (k, v) in &reply.headers {
                out = out.header(k, v);
            }
            let stream = crate::home_link::SyncStream::new(reply.body).map_ok(Frame::data);
            out.body(BodyExt::boxed(StreamBody::new(stream))).unwrap_or_else(|e| full(StatusCode::BAD_GATEWAY, &format!("{e}")))
        }
        Err(e) => full(StatusCode::BAD_GATEWAY, &format!("Launchpad proxy: Home: {e:#}")),
    }
}

async fn plain_http(req: Request<Incoming>) -> Response<Body> {
    let url = req.uri().to_string();
    let Ok(m) = reqwest::Method::from_bytes(req.method().as_str().as_bytes()) else { return full(StatusCode::BAD_REQUEST, "bad method") };
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in req.headers() {
        if !HOP.contains(&k.as_str()) {
            headers.append(k.clone(), v.clone());
        }
    }
    let body = reqwest::Body::wrap_stream(http_body_util::BodyDataStream::new(req.into_body()));
    match CLIENT.request(m, &url).headers(headers).body(body).send().await {
        Ok(res) => {
            let mut out = Response::builder().status(res.status().as_u16());
            for (k, v) in res.headers() {
                if !HOP.contains(&k.as_str()) {
                    out = out.header(k, v);
                }
            }
            let stream = res.bytes_stream().map_ok(Frame::data).map_err(std::io::Error::other);
            out.body(BodyExt::boxed(StreamBody::new(stream))).unwrap_or_else(|e| full(StatusCode::BAD_GATEWAY, &format!("{e}")))
        }
        Err(e) => full(StatusCode::BAD_GATEWAY, &format!("{e}")),
    }
}

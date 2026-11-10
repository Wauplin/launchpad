//! The runner proxy: agents use Home's accounts without holding a token.
//!
//! It listens on `127.0.0.1` outside the jail. Agents reach it through `HTTPS_PROXY` (a per-session key in the
//! proxy credentials) and, for harness logins, through a base URL (`/lp/<key>/<host>/…`). On a host where the
//! session's lease has a credential, the proxy terminates TLS with a certificate of the runner's own CA (trusted
//! only inside the jail), adds the real token and records host, method and path in the access log. Every other
//! host is a plain tunnel: the proxy never reads or judges that traffic. What a request may do is decided by the
//! token's scope on Home, not here.
//!
//! Captured connections (`netcap.rs`) arrive on a second port, the capture port, with a header that names the
//! session's key and the destination. The proxy connects to the destination first and answers the supervisor;
//! on port 443 of a session with tokens it then reads the TLS server name, and on a lease host it terminates
//! TLS and adds the token as above. On Burst every outgoing connection (tunnels, plain HTTP, captured
//! connections) leaves from Home through the runner tunnel (`token_relay::tcp`).

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
    /// Connections received on the capture port.
    pub captured: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Default)]
pub struct Proxy {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    port: tokio::sync::OnceCell<u16>,
    capture_port: std::sync::OnceLock<u16>,
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

use crate::netcap::SYSTEM_BUNDLES;

/// The runner CA certificate in `<data_dir>/proxy/public/` (the CA is created if needed).
pub fn ca_certificate(paths: &crate::paths::Paths) -> Result<std::path::PathBuf> {
    let ca = load_ca(&paths.data_dir.join("proxy"))?;
    let dir = paths.data_dir.join("proxy/public");
    std::fs::create_dir_all(&dir)?;
    let cert = dir.join("launchpad-ca.pem");
    std::fs::write(&cert, &ca.cert_pem)?;
    Ok(cert)
}

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
                // The capture port: connections of the network capture (`netcap`).
                let capture = tokio::net::TcpListener::bind("127.0.0.1:0").await.context("start the runner proxy's capture port")?;
                let _ = inner.capture_port.set(capture.local_addr()?.port());
                let capture_inner = inner.clone();
                tokio::spawn(async move {
                    loop {
                        let Ok((stream, _)) = capture.accept().await else { continue };
                        let inner = capture_inner.clone();
                        tokio::spawn(async move {
                            if let Err(e) = captured(inner, stream).await {
                                tracing::debug!("captured connection: {e:#}");
                            }
                        });
                    }
                });
                tracing::info!("runner proxy on 127.0.0.1:{port}");
                Ok::<u16, anyhow::Error>(port)
            })
            .await?;
        Ok(*port)
    }

    /// The capture port, once the proxy started.
    pub fn capture_port(&self) -> Option<u16> {
        self.inner.capture_port.get().copied()
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
            captured: std::sync::atomic::AtomicU64::new(0),
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
                (true, Some(active)) => intercept_tls(inner, active, host.clone(), TokioIo::new(upgraded)).await,
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
        // MCP servers of Home: /lp/<key>/mcp/<server>, through Home's gateway.
        if host == "mcp" {
            let server = tail.split('/').next().unwrap_or_default().to_string();
            return Ok(mcp(&active, &server, req).await);
        }
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
    let mut upstream = connect_out(host, port).await.map_err(|e| anyhow::anyhow!("connect {host}:{port}: errno {e}"))?;
    let mut client = TokioIo::new(upgraded);
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// A connection out of the runner proxy.
pub trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}

/// Opens a TCP connection to `host:port`: from this machine on a runner, from Home on Burst. Errors are errnos
/// for the agent.
async fn connect_out(host: &str, port: u16) -> std::result::Result<Box<dyn Stream>, i32> {
    if crate::home_link::burst().is_some() {
        return through_home(host, port).await.map(|s| Box::new(s) as Box<dyn Stream>);
    }
    // Link-local and metadata addresses are refused on the resolved address (`netcap::refused`).
    let connect = async {
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
        let allowed: Vec<std::net::SocketAddr> = addrs.iter().copied().filter(|a| !crate::netcap::refused(a.ip(), a.port())).collect();
        if allowed.is_empty() && !addrs.is_empty() {
            return Err(std::io::Error::from_raw_os_error(libc::EACCES));
        }
        tokio::net::TcpStream::connect(&allowed[..]).await
    };
    match tokio::time::timeout(std::time::Duration::from_secs(30), connect).await {
        Ok(Ok(s)) => {
            let _ = s.set_nodelay(true);
            Ok(Box::new(s))
        }
        Ok(Err(e)) => Err(crate::netcap::errno_of(&e)),
        Err(_) => Err(libc::ETIMEDOUT),
    }
}

/// Burst: a TCP connection opened by Home (`HttpTarget::Tcp`), both directions streamed through the tunnel.
async fn through_home(host: &str, port: u16) -> std::result::Result<tokio::io::DuplexStream, i32> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (ours, theirs) = tokio::io::duplex(256 * 1024);
    let (rd, mut wr) = tokio::io::split(theirs);
    let upload: crate::home_link::ByteStream = Box::pin(futures_util::stream::unfold(Some(rd), |rd| async move {
        let mut rd = rd?;
        let mut buf = vec![0u8; 64 * 1024];
        match rd.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), Some(rd)))
            }
            Err(e) => Some((Err(e), None)),
        }
    }));
    let target = crate::runners::proto::HttpTarget::Tcp { host: host.into(), port };
    let reply = crate::home_link::http(target, "CONNECT", "", vec![], upload).await.map_err(|e| {
        tracing::debug!("Burst connection to {host}:{port}: {e:#}");
        libc::ENETUNREACH
    })?;
    if reply.status != 200 {
        let parts: Vec<Bytes> = reply.body.try_collect().await.unwrap_or_default();
        return Err(crate::netcap::errno_in(&String::from_utf8_lossy(&parts.concat())));
    }
    let mut body = reply.body;
    tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(Ok(part)) = body.next().await {
            if wr.write_all(&part).await.is_err() {
                return;
            }
        }
        let _ = wr.shutdown().await;
    });
    Ok(ours)
}

/// A connection of the network capture: `LPCAP1 <key> <ip> <port>`, then the agent's bytes.
async fn captured(inner: Arc<Inner>, mut stream: tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _ = stream.set_nodelay(true);
    let mut line = vec![];
    let read_line = async {
        let mut b = [0u8; 1];
        while line.len() < 200 {
            if stream.read(&mut b).await? == 0 {
                anyhow::bail!("closed before the header");
            }
            if b[0] == b'\n' {
                return Ok(());
            }
            line.push(b[0]);
        }
        anyhow::bail!("header too long")
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), read_line).await.context("no header")??;
    let (key, dest) = crate::netcap::parse_capture_header(&String::from_utf8_lossy(&line)).context("bad header")?;
    let proxy = Proxy { inner: inner.clone() };
    let Some(active) = proxy.by_key(&key) else {
        stream.write_all(crate::netcap::capture_answer(Err(libc::EACCES)).as_bytes()).await?;
        return Ok(());
    };
    active.captured.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut upstream = match connect_out(&dest.ip().to_string(), dest.port()).await {
        Ok(u) => u,
        Err(e) => {
            stream.write_all(crate::netcap::capture_answer(Err(e)).as_bytes()).await?;
            return Ok(());
        }
    };
    stream.write_all(crate::netcap::capture_answer(Ok(())).as_bytes()).await?;
    let tokens = dest.port() == 443 && !active.lease.read().unwrap().credentials.is_empty();
    if !tokens {
        tokio::io::copy_bidirectional(&mut stream, &mut upstream).await?;
        return Ok(());
    }
    // Port 443 of a session with tokens: the TLS server name decides, unless the server speaks first.
    let mut hello = vec![];
    let mut first = vec![0u8; 16 * 1024];
    let name = tokio::select! {
        r = read_hello(&mut stream, &mut hello) => r?,
        r = upstream.read(&mut first) => {
            let n = r?;
            stream.write_all(&first[..n]).await?;
            upstream.write_all(&hello).await?;
            tokio::io::copy_bidirectional(&mut stream, &mut upstream).await?;
            return Ok(());
        }
    };
    if let Some(host) = name.filter(|h| credential_for(&active, h).is_some()) {
        drop(upstream);
        return intercept_tls(inner, active, host, Prefixed { prefix: hello, pos: 0, inner: stream }).await;
    }
    upstream.write_all(&hello).await?;
    tokio::io::copy_bidirectional(&mut stream, &mut upstream).await?;
    Ok(())
}

/// Reads a TLS ClientHello into `buf` and returns its server name; None for other protocols.
async fn read_hello(stream: &mut tokio::net::TcpStream, buf: &mut Vec<u8>) -> Result<Option<String>> {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0u8; 4096];
    loop {
        if buf.len() >= 5 {
            if buf[0] != 0x16 {
                return Ok(None);
            }
            let need = 5 + u16::from_be_bytes([buf[3], buf[4]]) as usize;
            if buf.len() >= need {
                return Ok(server_name(&buf[5..need]));
            }
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// The server name of a ClientHello handshake message (the record's payload).
fn server_name(h: &[u8]) -> Option<String> {
    let mut r = Reader(h);
    (r.u8()? == 1).then_some(())?;
    r.skip(3 + 2 + 32)?;
    let sid = r.u8()? as usize;
    r.skip(sid)?;
    let suites = r.u16()? as usize;
    r.skip(suites)?;
    let comp = r.u8()? as usize;
    r.skip(comp)?;
    let len = r.u16()? as usize;
    let mut ext = Reader(r.take(len)?);
    while let (Some(kind), Some(len)) = (ext.u16(), ext.u16()) {
        let data = ext.take(len as usize)?;
        if kind == 0 {
            let mut list = Reader(data);
            let len = list.u16()? as usize;
            let mut names = Reader(list.take(len)?);
            while let Some(t) = names.u8() {
                let len = names.u16()? as usize;
                let name = names.take(len)?;
                if t == 0 {
                    return std::str::from_utf8(name).ok().map(|n| n.trim_end_matches('.').to_ascii_lowercase());
                }
            }
            return None;
        }
    }
    None
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        (self.0.len() >= n).then(|| {
            let (a, b) = self.0.split_at(n);
            self.0 = b;
            a
        })
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
}

/// A stream that first replays bytes already read from it.
struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Prefixed<S> {
    fn poll_read(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.remaining());
            let start = self.pos;
            buf.put_slice(&self.prefix[start..start + n]);
            self.pos += n;
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Prefixed<S> {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
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

async fn intercept_tls<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static>(inner: Arc<Inner>, active: Arc<Active>, host: String, io: S) -> Result<()> {
    let config = tls_for(&inner, &host)?;
    let tls = tokio_rustls::TlsAcceptor::from(config).accept(io).await.context("TLS with the agent")?;
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
    // GitHub bodies the agent writes end with the attribution line (`attribution`).
    let sign = credential.is_some() && crate::attribution::signs_github(host, &method, &path);
    let mut request_bytes = None;
    let body = if read_req || sign {
        match http_body_util::Limited::new(req.into_body(), 2 << 20).collect().await {
            Ok(b) => {
                let mut b = b.to_bytes();
                if let (true, Some(app), Some(a)) = (sign, inner.app.get(), active.as_ref())
                    && let Some(signed) = crate::attribution::sign_request(app, &a.session_id, host, &method, &path, &b)
                {
                    b = Bytes::from(signed);
                    headers.insert(reqwest::header::CONTENT_LENGTH, b.len().into());
                }
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

/// A request of a session to an MCP server of Home, through the tunnel.
async fn mcp(active: &Active, server: &str, req: Request<Incoming>) -> Response<Body> {
    let lease_id = active.lease.read().unwrap().id.clone();
    let granted = active.lease.read().unwrap().mcp.iter().any(|m| m.id == server);
    if !granted {
        return full(StatusCode::FORBIDDEN, "this MCP server is not granted to the session");
    }
    let method = req.method().as_str().to_string();
    let headers: Vec<(String, String)> = req.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
    let body: crate::home_link::ByteStream = Box::pin(http_body_util::BodyDataStream::new(req.into_body()).map_err(std::io::Error::other));
    let reply = crate::home_link::http(crate::runners::proto::HttpTarget::Mcp { lease_id, server: server.into() }, &method, "", headers, body).await;
    match reply {
        Ok(reply) => {
            let mut out = Response::builder().status(reply.status);
            for (k, v) in &reply.headers {
                out = out.header(k, v);
            }
            let stream = crate::home_link::SyncStream::new(reply.body).map_ok(Frame::data);
            out.body(BodyExt::boxed(StreamBody::new(stream))).unwrap_or_else(|e| full(StatusCode::BAD_GATEWAY, &format!("{e}")))
        }
        Err(e) => full(StatusCode::BAD_GATEWAY, &format!("Launchpad MCP gateway: {e:#}")),
    }
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
    if crate::home_link::burst().is_some() {
        return plain_http_out(req).await.unwrap_or_else(|e| full(StatusCode::BAD_GATEWAY, &format!("Launchpad proxy: {e:#}")));
    }
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

/// Burst: plain HTTP over a connection Home opens (`connect_out`).
async fn plain_http_out(req: Request<Incoming>) -> Result<Response<Body>> {
    let host = req.uri().host().context("no host")?.to_string();
    let port = req.uri().port_u16().unwrap_or(80);
    let stream = connect_out(&host, port).await.map_err(|e| anyhow::anyhow!("{host}:{port}: errno {e}"))?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut parts, body) = req.into_parts();
    parts.uri = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/").parse()?;
    for h in ["proxy-authorization", "proxy-connection"] {
        parts.headers.remove(h);
    }
    if !parts.headers.contains_key(hyper::header::HOST) {
        parts.headers.insert(hyper::header::HOST, hyper::header::HeaderValue::from_str(&if port == 80 { host.clone() } else { format!("{host}:{port}") })?);
    }
    let res = sender.send_request(Request::from_parts(parts, body)).await?;
    Ok(res.map(|b| b.map_err(std::io::Error::other).boxed()))
}


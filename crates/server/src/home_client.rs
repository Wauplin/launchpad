//! A runner's tunnel to Home: one outbound WebSocket, kept open with retries. No inbound port is needed.
//!
//! Home sends API requests (`Req`), which the runner answers by calling its own router in process, as its
//! local owner. The runner reports its guard (local settings) on connect and when it changes. Home owns the
//! workspaces: it sends them on connect and after each change, and the runner forwards their changes to Home.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, header};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use launchpad_core::App;
use launchpad_core::home_link::{self, Link, TunnelState};
use launchpad_core::{replica, role};
use launchpad_core::runners::proto::{FromRunner, ToRunner};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const PING: Duration = Duration::from_secs(25);

/// Marks requests that arrived from Home through the tunnel.
pub const FROM_HOME: &str = "x-launchpad-from-home";

/// The open tunnel, for requests from this runner to Home.
#[derive(Default)]
struct HomeHandle {
    tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<FromRunner>>>,
    pending: std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<(u16, String)>>>,
    next: std::sync::atomic::AtomicU64,
}

static HOME: std::sync::LazyLock<HomeHandle> = std::sync::LazyLock::new(HomeHandle::default);

/// Sends an API request to Home through the tunnel.
async fn to_home(method: &str, path: &str, body: Option<String>) -> anyhow::Result<(u16, String)> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (tx, rx) = tokio::sync::oneshot::channel();
    HOME.pending.lock().unwrap().insert(id, tx);
    let sent = HOME.tx.lock().unwrap().as_ref().is_some_and(|t| t.send(FromRunner::Req { id, method: method.into(), path: path.into(), body }).is_ok());
    if !sent {
        HOME.pending.lock().unwrap().remove(&id);
        anyhow::bail!("Home is unreachable; this change needs Home");
    }
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => {
            HOME.pending.lock().unwrap().remove(&id);
            anyhow::bail!("Home did not answer within a minute")
        }
    }
}

/// Sends a lease call to Home through the tunnel.
async fn call_home(op: String, body: serde_json::Value) -> anyhow::Result<(u16, String)> {
    let id = HOME.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let (tx, rx) = tokio::sync::oneshot::channel();
    HOME.pending.lock().unwrap().insert(id, tx);
    let sent = HOME.tx.lock().unwrap().as_ref().is_some_and(|t| t.send(FromRunner::Call { id, op, body }).is_ok());
    if !sent {
        HOME.pending.lock().unwrap().remove(&id);
        anyhow::bail!("Home is unreachable");
    }
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(_)) => anyhow::bail!("Home disconnected before answering"),
        Err(_) => {
            HOME.pending.lock().unwrap().remove(&id);
            anyhow::bail!("Home did not answer within a minute")
        }
    }
}

/// Requests about records Home owns, which a runner sends to Home: workspace changes (the runner keeps a copy
/// for reading) and Home Git.
pub(crate) fn home_owned(path: &str) -> bool {
    path == "/api/workspaces" || path.starts_with("/api/workspaces/") || path.starts_with("/api/home-git/") || path.starts_with("/api/plain/") || path.starts_with("/api/knowledge/") || path.starts_with("/api/vault")
        || path == "/api/proposals" || path.starts_with("/api/proposals/")
        || (path.starts_with("/api/sessions/") && path.ends_with("/access"))
}

/// Home: requests a runner may send through its tunnel (as Home's owner).
pub(crate) fn home_accepts(path: &str) -> bool {
    home_owned(path) || path.starts_with("/api/projects") || path.starts_with("/api/predrafts")
}

/// Runner: requests about records Home owns go to Home; the runner's copy follows Home's snapshots.
pub async fn forward_writes(axum::extract::State(s): axum::extract::State<crate::Server>, req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    let forwarded = home_owned(path) && (req.method() != axum::http::Method::GET || !(path == "/api/workspaces" || path.starts_with("/api/workspaces/")));
    if s.app.paths.role != role::Role::Runner || !forwarded {
        return next.run(req).await;
    }
    let (method, path) = (req.method().to_string(), req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default());
    let body = match axum::body::to_bytes(req.into_body(), 1 << 20).await {
        Ok(b) => (!b.is_empty()).then(|| String::from_utf8_lossy(&b).into_owned()),
        Err(e) => return crate::error::ApiError::bad_request(format!("{e}")).into_response(),
    };
    match to_home(&method, &path, body).await {
        Ok((status, body)) => (axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::BAD_GATEWAY), [(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => crate::error::ApiError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")).into_response(),
    }
}

pub fn spawn(app: App, router: Router) {
    launchpad_core::home_link::set_requester(Box::new(|method, path, body| Box::pin(async move { to_home(&method, &path, body).await })));
    launchpad_core::home_link::set_caller(Box::new(|op, body| Box::pin(call_home(op, body))));
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(2);
        loop {
            let Some(link) = home_link::load(&app.paths) else {
                home_link::set_state(&app, None, TunnelState::NotConnected, Some("run launchpad connect <home address>".into()), None);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            home_link::set_state(&app, Some(link.url.clone()), TunnelState::Connecting, None, None);
            match run(&app, &router, &link).await {
                Ok(Some(reason)) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Refused, Some(reason), None);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                Ok(None) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Offline, Some("Home closed the connection".into()), None);
                    backoff = Duration::from_secs(2);
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    home_link::set_state(&app, Some(link.url.clone()), TunnelState::Offline, Some(format!("{e:#}")), None);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

fn tunnel_url(url: &str) -> String {
    let base = url.trim_end_matches('/');
    let base = base.strip_prefix("https://").map(|r| format!("wss://{r}")).or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}"))).unwrap_or_else(|| base.to_string());
    format!("{base}/api/runners/tunnel")
}

/// One connection. `Ok(Some(reason))`: Home refused the runner.
async fn run(app: &App, router: &Router, link: &Link) -> anyhow::Result<Option<String>> {
    let mut request = tunnel_url(&link.url).into_client_request()?;
    request.headers_mut().insert(crate::runners::CREDENTIAL_HEADER, link.credential().parse()?);
    if let Some(token) = home_link::space_token(&link.url) {
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {token}").parse()?);
    }
    let socket = match tokio::time::timeout(Duration::from_secs(30), tokio_tungstenite::connect_async(request)).await? {
        Ok((socket, _)) => socket,
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) if res.status() == 401 => {
            return Ok(Some("this runner is unknown to Home (removed or never approved): run launchpad connect again".into()));
        }
        Err(e) => return Err(e.into()),
    };
    let (mut tx, mut rx) = socket.split();
    let text = |m: &FromRunner| Message::Text(serde_json::to_string(m).unwrap_or_default().into());
    let guard = home_link::guard(&app.paths);
    let status = app.status.get();
    let capabilities = launchpad_core::runners::capabilities(app).await;
    tx.send(text(&FromRunner::Hello { protocol: role::PROTOCOL, commit: launchpad_core::COMMIT.into(), name: status.host.clone(), os: std::env::consts::OS.into(), guard: guard.clone(), capabilities })).await?;
    let (compat, acked) = loop {
        match tokio::time::timeout(Duration::from_secs(30), rx.next()).await? {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ToRunner>(&t)? {
                ToRunner::Welcome { compat, acked, .. } => break (compat, acked),
                ToRunner::Refuse { reason } => return Ok(Some(reason)),
                _ => {}
            },
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.into()),
            None => return Ok(None),
        }
    };
    home_link::set_state(app, Some(link.url.clone()), TunnelState::Connected, None, Some(compat));
    tracing::info!("connected to Home {} ({compat:?})", link.url);
    // Everything Home saved can go; the rest is sent again, in order.
    replica::acked(app, acked).await?;
    let cursor = acked;
    let (res_tx, mut res_rx) = tokio::sync::mpsc::unbounded_channel::<FromRunner>();
    *HOME.tx.lock().unwrap() = Some(res_tx.clone());
    let result = serve(app, router, &mut tx, &mut rx, res_tx, &mut res_rx, cursor).await;
    // Requests to Home fail now instead of timing out.
    *HOME.tx.lock().unwrap() = None;
    HOME.pending.lock().unwrap().clear();
    result
}

type Tx = futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>;
type Rx = futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>;

async fn serve(app: &App, router: &Router, tx: &mut Tx, rx: &mut Rx, res_tx: tokio::sync::mpsc::UnboundedSender<FromRunner>, res_rx: &mut tokio::sync::mpsc::UnboundedReceiver<FromRunner>, mut cursor: u64) -> anyhow::Result<Option<String>> {
    let text = |m: &FromRunner| Message::Text(serde_json::to_string(m).unwrap_or_default().into());
    let mut guard = home_link::guard(&app.paths);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            res = res_rx.recv() => {
                let Some(res) = res else { break };
                tx.send(text(&res)).await?;
            }
            _ = replica::wait() => {
                let items = replica::pending(app, cursor, 200).await?;
                if let Some((n, _)) = items.last() {
                    cursor = *n;
                    tx.send(text(&FromRunner::Items { items })).await?;
                }
            }
            _ = home_link::guard_changed() => {
                let now = home_link::guard(&app.paths);
                if now != guard {
                    guard = now;
                    tx.send(text(&FromRunner::Guard { guard: guard.clone() })).await?;
                }
            }
            _ = ping.tick() => {
                anyhow::ensure!(last_heard.elapsed() < PING * 3, "Home stopped answering");
                tx.send(text(&FromRunner::Ping)).await?;
            }
            frame = rx.next() => {
                let Some(frame) = frame else { return Ok(None) };
                let frame = frame?;
                last_heard = tokio::time::Instant::now();
                let Message::Text(t) = frame else { continue };
                match serde_json::from_str::<ToRunner>(&t) {
                    Ok(ToRunner::Req { id, method, path, body, headers }) => {
                        let (app, router, res_tx) = (app.clone(), router.clone(), res_tx.clone());
                        tokio::spawn(async move {
                            let (status, body) = local(&app, &router, &method, &path, body, &headers).await.unwrap_or_else(|e| (500, serde_json::json!({"error": format!("{e:#}")}).to_string()));
                            let _ = res_tx.send(FromRunner::Res { id, status, body });
                        });
                    }
                    Ok(ToRunner::Res { id, status, body }) => {
                        if let Some(tx) = HOME.pending.lock().unwrap().remove(&id) { let _ = tx.send((status, body)); }
                    }
                    Ok(ToRunner::Workspaces { workspaces }) => launchpad_core::workspace::replace_all(app, workspaces).await?,
                    Ok(ToRunner::Folders { projects }) => launchpad_core::projects::store_home(app, projects, true).await?,
                    Ok(ToRunner::Profiles { profiles }) => {
                        let mut config = app.config.get();
                        if config.profiles != profiles {
                            config.profiles = profiles;
                            app.config.save(config)?;
                        }
                    }
                    Ok(ToRunner::Ack { n }) => replica::acked(app, n).await?,
                    Ok(ToRunner::Revoke { lease_id }) => launchpad_core::contain::revoked(app, &lease_id).await,
                    Ok(ToRunner::Quotas { quotas }) => launchpad_core::quota::apply_home_readings(app, quotas),
                    Ok(ToRunner::Refuse { reason }) => return Ok(Some(reason)),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Home sent a bad message: {e}"),
                }
            }
        }
    }
    Ok(None)
}

/// Runs an API request against this daemon's own router, as its local owner.
pub(crate) async fn local(app: &App, router: &Router, method: &str, path: &str, body: Option<String>, headers: &[(String, String)]) -> anyhow::Result<(u16, String)> {
    anyhow::ensure!(path.starts_with("/api/"), "only API requests go through the tunnel");
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.paths.token()?))
        .header(header::CONTENT_TYPE, "application/json")
        .header(FROM_HOME, "1")
        .body(Body::from(body.unwrap_or_default()))?;
    for (k, v) in headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("idempotency-key")) {
        req.headers_mut().insert(header::HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
    }
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let res = router.clone().oneshot(req).await?;
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 64 << 20).await?;
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// `launchpad connect <home>`: asks Home for a pairing request, shows its code, and waits for the owner to
/// approve it in Settings → Home and runners. Saves the credential in the runner's folder.
pub async fn connect(paths: &launchpad_core::paths::Paths, url: &str, name: Option<String>) -> anyhow::Result<()> {
    let url = url.trim().trim_end_matches('/').to_string();
    anyhow::ensure!(url.starts_with("https://") || url.starts_with("http://"), "the Home address starts with https://");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let auth = |r: reqwest::RequestBuilder| match home_link::space_token(&url) { Some(t) => r.bearer_auth(t), None => r };
    let name = name.unwrap_or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "Runner".into()));
    let res = auth(client.post(format!("{url}/api/runners/request"))).json(&serde_json::json!({"name": name, "os": std::env::consts::OS})).send().await?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    anyhow::ensure!(status.is_success(), "Home refused the request: {}", body["error"].as_str().unwrap_or(status.as_str()));
    let link = Link { url: url.clone(), id: body["id"].as_str().unwrap_or_default().into(), secret: body["secret"].as_str().unwrap_or_default().into() };
    println!("Approve this runner on Home: the setup page or Settings → Home and runners.\nCode: {}\nWaiting…", body["code"].as_str().unwrap_or("?"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10 * 60);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let res = auth(client.get(format!("{url}/api/runners/request/{}", link.id))).header(crate::runners::CREDENTIAL_HEADER, link.credential()).send().await;
        let Ok(res) = res else { continue };
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("the request was removed on Home");
        }
        let state: serde_json::Value = res.json().await.unwrap_or_default();
        if state["state"] == "approved" {
            home_link::save(paths, &link)?;
            println!("Approved. The runner daemon connects to {url}.");
            return Ok(());
        }
    }
    anyhow::bail!("not approved within ten minutes; run launchpad connect again")
}

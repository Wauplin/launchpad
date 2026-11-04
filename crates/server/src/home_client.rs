//! A runner's tunnel to Home: one outbound WebSocket, kept open with retries. No inbound port is needed.
//!
//! Home sends API requests (`Req`), which the runner answers by calling its own router in process, as its
//! local owner. The runner reports its guard (local settings) on connect and when it changes.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, header};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use launchpad_core::App;
use launchpad_core::home_link::{self, Link, TunnelState};
use launchpad_core::role;
use launchpad_core::runners::proto::{FromRunner, ToRunner};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const PING: Duration = Duration::from_secs(25);

pub fn spawn(app: App, router: Router) {
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
    let mut guard = home_link::guard(&app.paths);
    let status = app.status.get();
    tx.send(text(&FromRunner::Hello { protocol: role::PROTOCOL, commit: launchpad_core::COMMIT.into(), name: status.host.clone(), os: std::env::consts::OS.into(), guard: guard.clone() })).await?;
    let compat = loop {
        match tokio::time::timeout(Duration::from_secs(30), rx.next()).await? {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ToRunner>(&t)? {
                ToRunner::Welcome { compat, .. } => break compat,
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
    let (res_tx, mut res_rx) = tokio::sync::mpsc::unbounded_channel::<FromRunner>();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            res = res_rx.recv() => {
                let Some(res) = res else { break };
                tx.send(text(&res)).await?;
            }
            _ = ping.tick() => {
                anyhow::ensure!(last_heard.elapsed() < PING * 3, "Home stopped answering");
                tx.send(text(&FromRunner::Ping)).await?;
                let now = home_link::guard(&app.paths);
                if now != guard {
                    guard = now;
                    tx.send(text(&FromRunner::Guard { guard: guard.clone() })).await?;
                }
            }
            frame = rx.next() => {
                let Some(frame) = frame else { return Ok(None) };
                let frame = frame?;
                last_heard = tokio::time::Instant::now();
                let Message::Text(t) = frame else { continue };
                match serde_json::from_str::<ToRunner>(&t) {
                    Ok(ToRunner::Req { id, method, path, body }) => {
                        let (app, router, res_tx) = (app.clone(), router.clone(), res_tx.clone());
                        tokio::spawn(async move {
                            let (status, body) = local(&app, &router, &method, &path, body).await.unwrap_or_else(|e| (500, serde_json::json!({"error": format!("{e:#}")}).to_string()));
                            let _ = res_tx.send(FromRunner::Res { id, status, body });
                        });
                    }
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
async fn local(app: &App, router: &Router, method: &str, path: &str, body: Option<String>) -> anyhow::Result<(u16, String)> {
    anyhow::ensure!(path.starts_with("/api/"), "only API requests go through the tunnel");
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.paths.token()?))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.unwrap_or_default()))?;
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
    println!("Approve this runner on Home: Settings → Home and runners.\nCode: {}\nWaiting…", body["code"].as_str().unwrap_or("?"));
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

//! Trusted-network front door: bearer credentials for local tools, revocable browser devices,
//! one-time pairing in URL fragments, and explicit Host/Origin checks (including WebSockets).

use crate::{
    Server,
    error::{ApiError, ApiResult},
};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::{
    Extension, Json, Router,
    routing::{get, post},
};
use launchpad_core::{
    config::ServerConfig,
    db::Db,
    paths::{Paths, Profile},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
use tokio::sync::watch;

const DEVICE_LIFETIME: i64 = 365 * 24 * 60 * 60;
const PAIR_LIFETIME: i64 = 5 * 60;
/// `browser_device.last_seen_at` is written at most this often per device (seconds).
const SEEN_INTERVAL: i64 = 5 * 60;
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn secret() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}
fn hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn local_pair_warning(origin: &str) -> Option<&'static str> {
    let authority = origin.split_once("://")?.1;
    let host = if let Some(ipv6) = authority.strip_prefix('[') {
        ipv6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    if host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        Some("This link only works on this machine. Use a reachable network address to pair a browser on another device.")
    } else {
        None
    }
}

struct Session {
    id: String,
    expires_at: i64,
    valid: Arc<AtomicBool>,
    seen: AtomicI64,
}

/// Shared validity flag and wakeup for upgraded sockets. Revocation closes existing viewers too.
#[derive(Clone)]
pub struct Access {
    session: Option<(Arc<AtomicBool>, i64)>,
    /// The browser device; None for bearer clients (CLI, desktop shell bootstrap).
    pub device: Option<String>,
    pub changes: watch::Receiver<u64>,
}
impl Access {
    pub fn valid(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|(valid, expiry)| valid.load(Ordering::Acquire) && *expiry > now())
    }
}

struct FrontDoor {
    origin: String,
    remote: bool,
    hosts: Vec<String>,
    origins: Vec<String>,
}

pub struct Auth {
    token: String,
    cookie: &'static str,
    bind: std::net::IpAddr,
    front_door: RwLock<FrontDoor>,
    serve_lock: tokio::sync::Mutex<()>,
    sessions: RwLock<HashMap<String, Session>>,
    pairs: RwLock<HashMap<String, i64>>,
    changes: watch::Sender<u64>,
    db: Db,
}

impl Auth {
    pub async fn new(paths: &Paths, config: &ServerConfig, db: Db) -> anyhow::Result<Auth> {
        let port = paths.port;
        let mut hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let mut origins: Vec<String> = hosts.iter().map(|h| format!("http://{h}")).collect();
        let origin = config.origin()?.unwrap_or_else(|| paths.base_url());
        let authority = origin.split_once("://").unwrap().1.to_string();
        hosts.push(authority);
        origins.push(origin.clone());
        if paths.profile == Profile::Dev {
            origins.extend([
                "http://localhost:5173".into(),
                "http://127.0.0.1:5173".into(),
            ]);
        }
        let sessions = db
            .call(|conn| {
                conn.execute("DELETE FROM browser_device WHERE expires_at <= ?1", [now()])?;
                conn.execute(
                    "DELETE FROM push_subscription WHERE device_id NOT IN (SELECT id FROM browser_device)",
                    [],
                )?;
                let mut stmt = conn.prepare(
                    "SELECT secret_hash, id, expires_at, last_seen_at FROM browser_device",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Session {
                            id: r.get(1)?,
                            expires_at: r.get(2)?,
                            valid: Arc::new(AtomicBool::new(true)),
                            seen: AtomicI64::new(r.get::<_, Option<i64>>(3)?.unwrap_or(0)),
                        },
                    ))
                })?;
                Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
            })
            .await?;
        let (changes, _) = watch::channel(0);
        Ok(Auth {
            token: paths.token()?,
            cookie: paths.cookie_name(),
            bind: config.bind.parse()?,
            front_door: RwLock::new(FrontDoor { origin, hosts, origins, remote: config.public_origin.is_some() }),
            serve_lock: tokio::sync::Mutex::new(()),
            sessions: RwLock::new(sessions),
            pairs: RwLock::new(HashMap::new()),
            changes,
            db,
        })
    }

    pub fn remote(&self) -> bool {
        self.front_door.read().unwrap().remote
    }

    pub fn origin(&self) -> String {
        self.front_door.read().unwrap().origin.clone()
    }

    fn activate_https(&self, origin: &str) -> anyhow::Result<()> {
        let config = ServerConfig { public_origin: Some(origin.into()), ..ServerConfig::default() };
        let origin = config.origin()?.unwrap();
        anyhow::ensure!(origin.starts_with("https://"), "HTTPS origin required");
        let mut network = self.front_door.write().unwrap();
        let host = origin.split_once("://").unwrap().1.to_string();
        if !network.hosts.contains(&host) { network.hosts.push(host); }
        if !network.origins.contains(&origin) { network.origins.push(origin.clone()); }
        network.origin = origin;
        network.remote = true;
        Ok(())
    }

    fn secure_cookie(&self, headers: &HeaderMap) -> bool {
        let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        self.front_door.read().unwrap().origins.iter().any(|allowed| {
            allowed.starts_with("https://") && (host == allowed.split_once("://").map(|(_, host)| host) || origin == Some(allowed.as_str()))
        })
    }

    fn check_token(&self, candidate: &str) -> bool {
        let (a, b) = (candidate.as_bytes(), self.token.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    fn authenticate(&self, headers: &HeaderMap) -> Option<Access> {
        let changes = self.changes.subscribe();
        if let Some(v) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            && let Some(t) = v.strip_prefix("Bearer ")
            && self.check_token(t.trim())
        {
            return Some(Access {
                session: None,
                device: None,
                changes,
            });
        }
        for v in headers.get_all(header::COOKIE) {
            let Ok(v) = v.to_str() else { continue };
            for pair in v.split(';') {
                if let Some((k, val)) = pair.trim().split_once('=')
                    && k == self.cookie
                    && let Some(session) = self.sessions.read().unwrap().get(&hash(val))
                    && session.expires_at > now()
                    && session.valid.load(Ordering::Acquire)
                {
                    self.touch(session);
                    return Some(Access {
                        session: Some((session.valid.clone(), session.expires_at)),
                        device: Some(session.id.clone()),
                        changes,
                    });
                }
            }
        }
        None
    }

    /// Records the device's last activity, at most once per `SEEN_INTERVAL`, off the request path.
    fn touch(&self, session: &Session) {
        let (seen, now) = (session.seen.load(Ordering::Relaxed), now());
        if now - seen < SEEN_INTERVAL
            || session
                .seen
                .compare_exchange(seen, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (db, id) = (self.db.clone(), session.id.clone());
        runtime.spawn(async move {
            let _ = db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE browser_device SET last_seen_at = ?1 WHERE id = ?2",
                        (now, id),
                    )?;
                    Ok(())
                })
                .await;
        });
    }

    fn check_front_door(&self, headers: &HeaderMap, method: &Method) -> Result<(), &'static str> {
        let network = self.front_door.read().unwrap();
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if headers.get_all(header::HOST).iter().count() != 1
            || !network.hosts.iter().any(|h| h == host)
        {
            return Err("unexpected Host header");
        }
        if headers.contains_key(header::UPGRADE)
            || !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        {
            if headers.get_all(header::ORIGIN).iter().count() > 1 {
                return Err("foreign Origin");
            }
            if let Some(origin) = headers.get(header::ORIGIN)
                && !origin
                    .to_str()
                    .is_ok_and(|o| network.origins.iter().any(|allowed| allowed == o))
            {
                return Err("foreign Origin");
            }
        }
        Ok(())
    }

    fn pair(&self, origin: &str) -> anyhow::Result<Value> {
        let code = secret();
        let expires_at = now() + PAIR_LIFETIME;
        let url = format!("{origin}/auth#code={code}");
        let qr = qrcode::QrCode::new(url.as_bytes())?;
        let svg = qr
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(256, 256)
            .build();
        let terminal = qr
            .render::<char>()
            .light_color(' ')
            .dark_color('█')
            .module_dimensions(2, 1)
            .build();
        let mut pairs = self.pairs.write().unwrap();
        pairs.retain(|_, expiry| *expiry > now());
        anyhow::ensure!(
            pairs.len() < 100,
            "too many pending pairing links; wait five minutes"
        );
        pairs.insert(hash(&code), expires_at);
        let warning = local_pair_warning(origin);
        Ok(json!({"url":url,"expires_at":expires_at,"svg":svg,"terminal":terminal,"warning":warning}))
    }

    async fn exchange(&self, code: &str, name: String) -> anyhow::Result<Option<String>> {
        let expiry = self.pairs.write().unwrap().remove(&hash(code));
        if expiry.is_none_or(|expiry| expiry <= now()) {
            return Ok(None);
        }
        let token = secret();
        let id = uuid::Uuid::new_v4().to_string();
        let secret_hash = hash(&token);
        let expires_at = now() + DEVICE_LIFETIME;
        let (h, i) = (secret_hash.clone(), id.clone());
        self.db.call(move |conn| {
            conn.execute("INSERT INTO browser_device (id, secret_hash, name, created_at, expires_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?4)",
                (i, h, name, now(), expires_at))?;
            Ok(())
        }).await?;
        self.sessions.write().unwrap().insert(
            secret_hash,
            Session {
                id,
                expires_at,
                valid: Arc::new(AtomicBool::new(true)),
                seen: AtomicI64::new(now()),
            },
        );
        Ok(Some(token))
    }

    async fn revoke(&self, id: String) -> anyhow::Result<()> {
        let db_id = id.clone();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM browser_device WHERE id = ?1", [&db_id])?;
                conn.execute("DELETE FROM push_subscription WHERE device_id = ?1", [&db_id])?;
                Ok(())
            })
            .await?;
        self.sessions.write().unwrap().retain(|_, s| {
            if s.id == id {
                s.valid.store(false, Ordering::Release);
                false
            } else {
                true
            }
        });
        self.changes.send_modify(|generation| *generation += 1);
        Ok(())
    }
}

pub fn router() -> Router<Server> {
    Router::new()
        .route("/auth", get(sign_in).post(exchange))
        .route("/api/devices", get(devices))
        .route("/api/devices/pair", post(pair))
        .route("/api/devices/pairing-info", get(pairing_info))
        .route("/api/devices/https/run", post(run_https))
        .route("/api/devices/https/check", post(check_https))
        .route("/api/devices/{id}", axum::routing::delete(revoke))
}

pub async fn guard(State(s): State<Server>, mut req: Request, next: Next) -> Response {
    if let Err(message) = s.auth.check_front_door(req.headers(), req.method()) {
        return (StatusCode::FORBIDDEN, message).into_response();
    }
    let path = req.uri().path();
    let private =
        path == "/auth" || path.starts_with("/api/devices") || path.starts_with("/api/push");
    // Installable-app files hold no data, and browsers fetch the manifest, icons and service
    // worker updates without the session cookie (or before pairing on a Home Screen).
    let public = matches!(path, "/sw.js" | "/manifest.webmanifest") || path.starts_with("/icons/");
    if path == "/api/health" || path == "/auth" || public {
        // Health answers everyone, but only signed-in clients learn the profile and process id.
        if let Some(access) = s.auth.authenticate(req.headers()) {
            req.extensions_mut().insert(access);
        }
        return private_response(next.run(req).await, private);
    }
    if let Some(access) = s.auth.authenticate(req.headers()) {
        req.extensions_mut().insert(access);
        return private_response(next.run(req).await, private);
    }
    if path.starts_with("/api/") || path == "/ws" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"not authenticated; pair this browser again"})),
        )
            .into_response();
    }
    sign_in().await.into_response()
}

fn private_response(mut response: Response, private: bool) -> Response {
    if private {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
}

async fn pairing_info(State(s): State<Server>) -> ApiResult<Value> {
    let setup = crate::tailnet::setup(&s.app.env, s.app.paths.port, s.auth.bind).await;
    Ok(Json(json!({
        "origin": s.auth.origin(), "daemon_port": s.app.paths.port,
        "installed": setup.installed, "connected": setup.connected, "https_port": setup.port,
    })))
}

#[derive(Deserialize)]
struct HttpsPort { port: u16 }

fn adopt_https(s: &Server, origin: &str) -> anyhow::Result<Value> {
    // Save only the selected origin, preserving all host-edited commands and environments.
    let mut config = s.app.config.get();
    config.server.public_origin = Some(origin.into());
    s.app.config.save(config)?;
    s.auth.activate_https(origin)?;
    s.auth.pair(&s.auth.origin())
}

async fn run_https(State(s): State<Server>, Json(request): Json<HttpsPort>) -> ApiResult<Value> {
    let _lock = s.auth.serve_lock.try_lock().map_err(|_| ApiError::conflict("HTTPS setup is already running"))?;
    if request.port == 0 { return Err(ApiError::bad_request("Choose an HTTPS port from 1 to 65535")); }
    let result = crate::tailnet::run(&s.app.env, s.app.paths.port, s.auth.bind, request.port).await?;
    let pair = result.origin.as_deref().map(|origin| adopt_https(&s, origin)).transpose()?;
    Ok(Json(json!({"pair":pair, "output":result.output, "approval_url":result.approval_url})))
}

async fn check_https(State(s): State<Server>, Json(request): Json<HttpsPort>) -> ApiResult<Value> {
    let _lock = s.auth.serve_lock.try_lock().map_err(|_| ApiError::conflict("HTTPS setup is already running"))?;
    if request.port == 0 { return Err(ApiError::bad_request("Choose an HTTPS port from 1 to 65535")); }
    let origin = crate::tailnet::check(&s.app.env, s.app.paths.port, s.auth.bind, request.port).await?;
    let pair = origin.as_deref().map(|origin| adopt_https(&s, origin)).transpose()?;
    Ok(Json(json!({"pair":pair, "output":if pair.is_some() { "HTTPS is ready. Pairing URL updated." } else { "HTTPS is not ready yet. Complete any Tailscale approval, then check again." }, "approval_url":null})))
}

async fn pair(State(s): State<Server>, headers: HeaderMap) -> ApiResult<Value> {
    // Only desktop bootstrap requests a local link; device pairing uses the remote origin.
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let origin = if host == format!("127.0.0.1:{}", s.app.paths.port)
        || host == format!("localhost:{}", s.app.paths.port)
    {
        // Desktop bootstrap must stay on loopback even when a remote origin is available.
        if headers.get("x-launchpad-local-pair").is_some() {
            format!("http://{host}")
        } else {
            s.auth.origin()
        }
    } else {
        s.auth.origin()
    };
    Ok(Json(s.auth.pair(&origin)?))
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
    name: String,
}
async fn exchange(
    State(s): State<Server>,
    headers: HeaderMap,
    Json(body): Json<Exchange>,
) -> Result<Response, ApiError> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "Use a device name of 1 to 80 characters",
        ));
    }
    let Some(token) = s.auth.exchange(&body.code, name.to_string()).await? else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Pairing link expired or already used. Create a new link.",
        ));
    };
    // Never trust forwarded protocol headers. Secure is decided by the configured external origin
    // and its Host/Origin; the local desktop's HTTP cookie must keep working behind the same daemon.
    let secure = if s.auth.secure_cookie(&headers) { "; Secure" } else { "" };
    let cookie = format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={DEVICE_LIFETIME}{secure}",
        s.auth.cookie
    );
    Ok((
        [
            (header::SET_COOKIE, cookie),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        Json(json!({"ok":true})),
    )
        .into_response())
}

#[derive(Serialize)]
struct Device {
    id: String,
    name: String,
    created_at: i64,
    expires_at: i64,
    last_seen_at: Option<i64>,
    /// At least one Web Push subscription.
    push: bool,
    current: bool,
}
async fn devices(
    State(s): State<Server>,
    Extension(access): Extension<Access>,
) -> ApiResult<Value> {
    let rows = s.auth.db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT id, name, created_at, expires_at, last_seen_at,
            EXISTS (SELECT 1 FROM push_subscription p WHERE p.device_id = browser_device.id)
            FROM browser_device WHERE expires_at > ?1 ORDER BY created_at DESC")?;
        let rows = stmt.query_map([now()], |r| Ok(Device { id:r.get(0)?, name:r.get(1)?, created_at:r.get(2)?, expires_at:r.get(3)?, last_seen_at:r.get(4)?, push:r.get(5)?, current:false }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }).await?;
    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut d| {
            d.current = access.device.as_deref() == Some(d.id.as_str());
            d
        })
        .collect();
    Ok(Json(json!(rows)))
}
async fn revoke(State(s): State<Server>, Path(id): Path<String>) -> ApiResult<Value> {
    s.auth.revoke(id).await?;
    Ok(Json(json!({"ok":true})))
}

async fn sign_in() -> impl IntoResponse {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost; form-action 'self'; frame-ancestors 'none'",
            ),
        ],
        Html(SIGN_IN),
    )
}
const SIGN_IN: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Pair Launchpad</title></head>
<body style="font:16px system-ui;padding:24px;color:#17202B;background:#F4F6F9;max-width:480px;margin:auto">
<h1 id="heading">Pair this browser</h1><p id="instructions">On the host, run <code>launchpad pair</code>, or open Settings → Devices in a signed-in browser. Scan the QR code or open its link on this device.</p>
<button id="reconnect" hidden style="font:inherit;padding:8px">Reconnect to this machine</button>
<form id="form"><p><label>Device name<br><input id="name" autocomplete="off" maxlength="80" required style="font:inherit;padding:8px;max-width:90%" placeholder="My phone"></label></p>
<button style="font:inherit;padding:8px" id="button">Pair browser</button></form><p id="message" role="status"></p>
<script>
const params=new URLSearchParams(location.hash.slice(1)); const code=params.get('code'); const next=params.get('next');
history.replaceState(null,'', '/auth');
const form=document.getElementById('form'), message=document.getElementById('message');
function destination(){try{const url=new URL(next || '/',location.origin);return next && next.startsWith('/') && url.origin===location.origin ? url.href : '/';}catch{return '/';}}
const reconnect=document.getElementById('reconnect');
if(window.__launchpadDesktop && window.__launchpadReconnect){
 document.getElementById('heading').textContent='Connect Launchpad';
 document.getElementById('instructions').textContent='Reconnect the desktop app to Launchpad on this machine. Your agents and terminals keep running.';
 reconnect.hidden=false;
 reconnect.onclick=async()=>{reconnect.disabled=true;message.textContent='Connecting…';try{await window.__launchpadReconnect();}catch(e){message.textContent=String(e);reconnect.disabled=false;}};
}
if(!code){form.hidden=true;}else{document.getElementById('name').focus();}
form.onsubmit=async(e)=>{e.preventDefault(); const button=document.getElementById('button'); button.disabled=true;
try{const r=await fetch('/auth',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({code,name:document.getElementById('name').value})});
const data=await r.json();if(!r.ok)throw new Error(data.error || 'Could not pair browser');
location.replace(destination());
}catch(e){message.textContent=e.message;button.disabled=false;}};
// The local shell has already requested a one-time link through its bearer credential.
if(code && params.get('desktop')==='1'){
fetch('/api/devices').then(r=>{if(r.ok){location.replace(destination());}
else{document.getElementById('name').value='Desktop';form.requestSubmit();}}).catch(()=>{message.textContent='Could not reach daemon. Try again.';});
}
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) struct Fixture(pub(super) Paths);
    impl Fixture {
        pub(super) fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("launchpad-auth-{}", uuid::Uuid::new_v4()));
            let paths = Paths {
                profile: Profile::Dev,
                config_dir: root.join("config"),
                data_dir: root.join("data"),
                port: 7318,
            };
            paths.ensure_dirs().unwrap();
            Self(paths)
        }
        async fn auth(&self) -> Auth {
            Auth::new(
                &self.0,
                &ServerConfig {
                    public_origin: Some("https://launchpad.example".into()),
                    ..ServerConfig::default()
                },
                launchpad_core::db::open_state(&self.0.state_db()).unwrap(),
            )
            .await
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.data_dir.parent().unwrap());
        }
    }
    fn browser(auth: &Auth, secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "launchpad.example".parse().unwrap());
        headers.insert(
            header::COOKIE,
            format!("{}={secret}", auth.cookie).parse().unwrap(),
        );
        headers
    }
    pub(super) fn code(pair: Value) -> String {
        pair["url"]
            .as_str()
            .unwrap()
            .split_once("#code=")
            .unwrap()
            .1
            .to_string()
    }

    #[tokio::test]
    async fn pairing_is_single_use_expiring_and_independent_of_bearer_token() {
        let fixture = Fixture::new();
        let auth = fixture.auth().await;
        let pair = auth.pair(&auth.origin()).unwrap();
        assert!(pair["svg"].as_str().unwrap().contains("<svg"));
        assert!(!pair["url"].as_str().unwrap().contains(&auth.token));
        let code = code(pair);
        let (one, two) = tokio::join!(
            auth.exchange(&code, "Phone".into()),
            auth.exchange(&code, "Other".into())
        );
        assert_ne!(
            one.as_ref().unwrap().is_some(),
            two.as_ref().unwrap().is_some()
        );
        let token = one.unwrap().or(two.unwrap()).unwrap();
        assert!(auth.authenticate(&browser(&auth, &token)).unwrap().valid());
        assert!(auth.authenticate(&browser(&auth, &auth.token)).is_none());
        let expired = secret();
        auth.pairs
            .write()
            .unwrap()
            .insert(hash(&expired), now() - 1);
        assert!(
            auth.exchange(&expired, "Expired".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            auth.exchange("unknown", "Unknown".into())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn devices_survive_restart_and_revocation_invalidates_live_sockets() {
        let fixture = Fixture::new();
        let auth = fixture.auth().await;
        let token = auth
            .exchange(&code(auth.pair(&auth.origin()).unwrap()), "Laptop".into())
            .await
            .unwrap()
            .unwrap();
        let token2 = auth
            .exchange(&code(auth.pair(&auth.origin()).unwrap()), "Phone".into())
            .await
            .unwrap()
            .unwrap();
        drop(auth);
        let auth = fixture.auth().await;
        let headers = browser(&auth, &token);
        let mut access = auth.authenticate(&headers).unwrap();
        let id = auth
            .sessions
            .read()
            .unwrap()
            .get(&hash(&token))
            .unwrap()
            .id
            .clone();
        auth.revoke(id).await.unwrap();
        access.changes.changed().await.unwrap();
        assert!(!access.valid());
        assert!(auth.authenticate(&headers).is_none());
        assert!(auth.authenticate(&browser(&auth, &token2)).is_some());
        let mut bearer = HeaderMap::new();
        bearer.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", auth.token).parse().unwrap(),
        );
        assert!(auth.authenticate(&bearer).unwrap().valid());
        drop(auth);
        let auth = fixture.auth().await;
        assert!(auth.authenticate(&headers).is_none());
        // Expiration blocks an already upgraded socket as well as a new request.
        let mut access = auth.authenticate(&browser(&auth, &token2)).unwrap();
        access.session.as_mut().unwrap().1 = now() - 1;
        assert!(!access.valid());
    }

    #[tokio::test]
    async fn https_activation_updates_guard_and_cookies_without_losing_loopback() {
        let fixture = Fixture::new();
        let auth = Auth::new(&fixture.0, &ServerConfig::default(),
            launchpad_core::db::open_state(&fixture.0.state_db()).unwrap()).await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "host.example.ts.net".parse().unwrap());
        headers.insert(header::ORIGIN, "https://host.example.ts.net".parse().unwrap());
        assert!(!auth.remote());
        assert!(auth.check_front_door(&headers, &Method::POST).is_err());
        auth.activate_https("https://host.example.ts.net:443").unwrap();
        assert_eq!(auth.origin(), "https://host.example.ts.net");
        assert!(auth.remote());
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        assert!(auth.secure_cookie(&headers));
        auth.activate_https("https://host.example.ts.net:8443").unwrap();
        assert!(auth.secure_cookie(&headers)); // old HTTPS viewers remain trusted too
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        headers.insert(header::HOST, "127.0.0.1:7318".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:7318".parse().unwrap());
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        assert!(!auth.secure_cookie(&headers));
        assert!(auth.activate_https("http://foreign.example").is_err());
        assert!(auth.activate_https("https://host.example/path").is_err());
        let pair = auth.pair(&auth.origin()).unwrap();
        assert!(pair["url"].as_str().unwrap().starts_with("https://host.example.ts.net:8443/auth#code="));
    }

    #[tokio::test]
    async fn tailnet_origin_is_exact_and_local_fallback_warns() {
        let fixture = Fixture::new();
        let config = ServerConfig {
            public_origin: Some("http://100.64.1.2:7318".into()),
            ..ServerConfig::default()
        };
        let auth = Auth::new(&fixture.0, &config,
            launchpad_core::db::open_state(&fixture.0.state_db()).unwrap()).await.unwrap();
        assert!(auth.remote());
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "100.64.1.2:7318".parse().unwrap());
        headers.insert(header::ORIGIN, "http://100.64.1.2:7318".parse().unwrap());
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        headers.insert(header::ORIGIN, "http://100.64.1.3:7318".parse().unwrap());
        assert_eq!(auth.check_front_door(&headers, &Method::POST), Err("foreign Origin"));
        let pair = auth.pair(&auth.origin()).unwrap();
        assert!(pair["url"].as_str().unwrap().starts_with("http://100.64.1.2:7318/auth#code="));
        assert!(pair["warning"].is_null());
        let local = auth.pair(&fixture.0.base_url()).unwrap();
        assert!(local["warning"].as_str().unwrap().contains("only works on this machine"));
        assert!(local_pair_warning("http://[::1]:7318").is_some());
        assert!(local_pair_warning("http://[::1]").is_some());
        assert!(local_pair_warning("http://127.0.0.2").is_some());
        assert!(local_pair_warning("http://[fd7a:115c:a1e0::1]:7318").is_none());
    }

    #[tokio::test]
    async fn public_and_local_origins_work_but_foreign_mutations_and_upgrades_fail() {
        let fixture = Fixture::new();
        let auth = fixture.auth().await;
        let mut headers = browser(&auth, "unused");
        headers.insert(header::ORIGIN, "https://launchpad.example".parse().unwrap());
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        assert!(auth.check_front_door(&headers, &Method::GET).is_ok());
        headers.insert(header::ORIGIN, "https://foreign.example".parse().unwrap());
        assert_eq!(
            auth.check_front_door(&headers, &Method::GET),
            Err("foreign Origin")
        );
        assert_eq!(
            auth.check_front_door(&headers, &Method::POST),
            Err("foreign Origin")
        );
        headers.remove(header::UPGRADE);
        assert!(auth.check_front_door(&headers, &Method::GET).is_ok());
        headers.insert(header::HOST, "foreign.example".parse().unwrap());
        assert_eq!(
            auth.check_front_door(&headers, &Method::GET),
            Err("unexpected Host header")
        );
        headers.insert(header::HOST, "127.0.0.1:7318".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:7318".parse().unwrap());
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
        headers.remove(header::ORIGIN); // CLI and native shell do not send Origin.
        assert!(auth.check_front_door(&headers, &Method::POST).is_ok());
    }
}

#[cfg(test)]
#[path = "auth_protocol_tests.rs"]
mod protocol_tests;

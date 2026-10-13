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
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;

const DEVICE_LIFETIME: i64 = 365 * 24 * 60 * 60;
const PAIR_LIFETIME: i64 = 5 * 60;
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

struct Session {
    id: String,
    expires_at: i64,
    valid: Arc<AtomicBool>,
}

/// Shared validity flag and wakeup for upgraded sockets. Revocation closes existing viewers too.
#[derive(Clone)]
pub struct Access {
    session: Option<(Arc<AtomicBool>, i64)>,
    pub changes: watch::Receiver<u64>,
}
impl Access {
    pub fn valid(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|(valid, expiry)| valid.load(Ordering::Acquire) && *expiry > now())
    }
}

pub struct Auth {
    token: String,
    cookie: &'static str,
    pub origin: String,
    remote: bool,
    hosts: Vec<String>,
    origins: Vec<String>,
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
                let mut stmt =
                    conn.prepare("SELECT secret_hash, id, expires_at FROM browser_device")?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Session {
                            id: r.get(1)?,
                            expires_at: r.get(2)?,
                            valid: Arc::new(AtomicBool::new(true)),
                        },
                    ))
                })?;
                Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
            })
            .await?;
        let (changes, _) = watch::channel(0);
        Ok(Auth {
            remote: config.public_origin.is_some(),
            token: paths.token()?,
            cookie: paths.cookie_name(),
            origin,
            hosts,
            origins,
            sessions: RwLock::new(sessions),
            pairs: RwLock::new(HashMap::new()),
            changes,
            db,
        })
    }

    pub fn remote(&self) -> bool {
        self.remote
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
                    return Some(Access {
                        session: Some((session.valid.clone(), session.expires_at)),
                        changes,
                    });
                }
            }
        }
        None
    }

    fn check_front_door(&self, headers: &HeaderMap, method: &Method) -> Result<(), &'static str> {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if headers.get_all(header::HOST).iter().count() != 1
            || !self.hosts.iter().any(|h| h == host)
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
                    .is_ok_and(|o| self.origins.iter().any(|allowed| allowed == o))
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
        Ok(json!({"url":url,"expires_at":expires_at,"svg":svg,"terminal":terminal}))
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
            conn.execute("INSERT INTO browser_device (id, secret_hash, name, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                (i, h, name, now(), expires_at))?;
            Ok(())
        }).await?;
        self.sessions.write().unwrap().insert(
            secret_hash,
            Session {
                id,
                expires_at,
                valid: Arc::new(AtomicBool::new(true)),
            },
        );
        Ok(Some(token))
    }

    async fn revoke(&self, id: String) -> anyhow::Result<()> {
        let db_id = id.clone();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM browser_device WHERE id = ?1", [db_id])?;
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
        .route("/api/devices/{id}", axum::routing::delete(revoke))
}

pub async fn guard(State(s): State<Server>, mut req: Request, next: Next) -> Response {
    if let Err(message) = s.auth.check_front_door(req.headers(), req.method()) {
        return (StatusCode::FORBIDDEN, message).into_response();
    }
    let path = req.uri().path();
    let private = path == "/auth" || path.starts_with("/api/devices");
    if path == "/api/health" || path == "/auth" {
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

async fn pair(State(s): State<Server>, headers: HeaderMap) -> ApiResult<Value> {
    // Local browser links must work even when a public origin is configured.
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let origin = if host == format!("127.0.0.1:{}", s.app.paths.port)
        || host == format!("localhost:{}", s.app.paths.port)
    {
        // CLI explicitly requests the public link; local browser buttons use their own origin.
        if headers.get("x-launchpad-local-pair").is_some() {
            format!("http://{host}")
        } else {
            s.auth.origin.clone()
        }
    } else {
        s.auth.origin.clone()
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
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let secure = if s.auth.origin.starts_with("https://")
        && (host == s.auth.origin.split_once("://").unwrap().1
            || headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
                == Some(s.auth.origin.as_str()))
    {
        "; Secure"
    } else {
        ""
    };
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
    current: bool,
}
async fn devices(
    State(s): State<Server>,
    Extension(access): Extension<Access>,
) -> ApiResult<Value> {
    let rows = s.auth.db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT id, name, created_at, expires_at FROM browser_device WHERE expires_at > ?1 ORDER BY created_at DESC")?;
        let rows = stmt.query_map([now()], |r| Ok(Device { id:r.get(0)?, name:r.get(1)?, created_at:r.get(2)?, expires_at:r.get(3)?, current:false }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }).await?;
    let sessions = s.auth.sessions.read().unwrap();
    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut d| {
            d.current = access.session.as_ref().is_some_and(|(flag, _)| {
                sessions
                    .values()
                    .any(|s| s.id == d.id && Arc::ptr_eq(flag, &s.valid))
            });
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
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'",
            ),
        ],
        Html(SIGN_IN),
    )
}
const SIGN_IN: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Pair Launchpad</title></head>
<body style="font:16px system-ui;padding:24px;color:#17202B;background:#F4F6F9;max-width:480px;margin:auto">
<h1>Pair this browser</h1><p>On the host, run <code>launchpad pair</code>, or open Settings → Devices in a signed-in browser. Scan the QR code or open its link on this device.</p>
<form id="form"><p><label>Device name<br><input id="name" autocomplete="off" maxlength="80" required style="font:inherit;padding:8px;max-width:90%" placeholder="My phone"></label></p>
<button style="font:inherit;padding:8px" id="button">Pair browser</button></form><p id="message" role="status"></p>
<script>
const params=new URLSearchParams(location.hash.slice(1)); const code=params.get('code'); const next=params.get('next');
history.replaceState(null,'', '/auth');
const form=document.getElementById('form'), message=document.getElementById('message');
function destination(){try{const url=new URL(next || '/',location.origin);return next && next.startsWith('/') && url.origin===location.origin ? url.href : '/';}catch{return '/';}}
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
        let pair = auth.pair(&auth.origin).unwrap();
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
            .exchange(&code(auth.pair(&auth.origin).unwrap()), "Laptop".into())
            .await
            .unwrap()
            .unwrap();
        let token2 = auth
            .exchange(&code(auth.pair(&auth.origin).unwrap()), "Phone".into())
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

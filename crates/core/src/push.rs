//! Web Push to paired browsers: VAPID (RFC 8292) and `aes128gcm` payload encryption (RFC 8291).
//!
//! The VAPID key pair is created on first use in `<data_dir>/vapid.json` (mode 0600). Subscriptions
//! live in `state.db` (`push_subscription`), one row per endpoint and owned by a browser device:
//! revoking or expiring the device deletes them. Payloads never contain code or messages; the
//! session title is included only when the subscription opted in (`titles`).

use std::sync::{Arc, Mutex};

use aes_gcm::{Aes128Gcm, KeyInit, aead::Aead};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hkdf::Hkdf;
use p256::{PublicKey, SecretKey, ecdsa::SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;

use crate::App;
use crate::model::Session;

/// Push services keep undelivered messages this long (seconds). A stale "done" is not worth more.
const TTL: u32 = 12 * 60 * 60;
/// Record size announced in the header; one record is enough for our small payloads.
const RECORD_SIZE: u32 = 4096;

#[derive(Clone)]
pub struct Push {
    paths: Arc<crate::paths::Paths>,
    vapid: Arc<Mutex<Option<Arc<Vapid>>>>,
    client: reqwest::Client,
}

struct Vapid {
    key: SecretKey,
    /// Uncompressed SEC1 public key (65 bytes).
    public: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct VapidFile {
    private_key: String,
}

/// A browser's subscription, as sent by `PushManager.subscribe()`.
#[derive(Clone, Debug, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// What the service worker shows (`web/public/sw.js`).
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
    /// Needs-you sessions, for the app badge.
    pub needs: usize,
    #[serde(skip)]
    pub urgent: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

impl Push {
    pub fn new(paths: Arc<crate::paths::Paths>) -> Push {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("launchpad")
            .build()
            .expect("push HTTP client");
        Push { paths, vapid: Arc::new(Mutex::new(None)), client }
    }

    fn vapid(&self) -> Result<Arc<Vapid>> {
        let mut slot = self.vapid.lock().unwrap();
        if let Some(v) = slot.as_ref() {
            return Ok(v.clone());
        }
        let path = self.paths.data_dir.join("vapid.json");
        let key = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: VapidFile = serde_json::from_str(&text).with_context(|| format!("read {}", path.display()))?;
                SecretKey::from_slice(&B64.decode(file.private_key.trim())?).map_err(|_| anyhow!("invalid VAPID key in {}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = random_key();
                let file = VapidFile { private_key: B64.encode(key.to_bytes()) };
                crate::paths::write_private(&path, &serde_json::to_string_pretty(&file)?)?;
                tracing::info!("push: created VAPID key {}", path.display());
                key
            }
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let public = key.public_key().to_sec1_bytes().to_vec();
        let v = Arc::new(Vapid { key, public });
        *slot = Some(v.clone());
        Ok(v)
    }

    /// The application server key for `PushManager.subscribe()` (base64url, uncompressed P-256).
    pub fn public_key(&self) -> Result<String> {
        Ok(B64.encode(&self.vapid()?.public))
    }

    /// Encrypts and sends one message. `Ok(false)`: the subscription is gone (404/410) and was deleted.
    async fn deliver(&self, app: &App, sub: &Subscription, message: &Message) -> Result<bool> {
        let vapid = self.vapid()?;
        let payload = serde_json::to_vec(message)?;
        let ua_public = B64.decode(sub.p256dh.trim_end_matches('='))?;
        let auth = B64.decode(sub.auth.trim_end_matches('='))?;
        let body = encrypt(&ua_public, &auth, &payload, &random_key(), rand::random())?;
        let subject = app.config.get().server.origin().ok().flatten().filter(|o| o.starts_with("https://")).unwrap_or_else(|| "mailto:launchpad@example.com".into());
        let authorization = vapid_header(&vapid, &sub.endpoint, &subject, now())?;
        let response = self
            .client
            .post(&sub.endpoint)
            .header("Authorization", authorization)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", TTL.to_string())
            .header("Urgency", if message.urgent { "high" } else { "normal" })
            .body(body)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(true);
        }
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
            let endpoint = sub.endpoint.clone();
            app.state_db.call(move |c| Ok(c.execute("DELETE FROM push_subscription WHERE endpoint = ?1", [endpoint])?)).await?;
            tracing::info!("push: subscription expired ({status}); deleted it");
            return Ok(false);
        }
        let text = response.text().await.unwrap_or_default();
        Err(anyhow!("push service answered {status}: {}", text.chars().take(200).collect::<String>()))
    }

    /// Sends to every subscription (of `device` only, when given) of a valid browser device. Returns
    /// the number of messages accepted by push services.
    async fn send(&self, app: &App, device: Option<String>, build: impl Fn(bool) -> Message) -> Result<usize> {
        let rows = app
            .state_db
            .call(move |c| {
                // Expired or revoked devices lose their subscriptions here too.
                c.execute("DELETE FROM push_subscription WHERE device_id NOT IN (SELECT id FROM browser_device WHERE expires_at > ?1)", [now()])?;
                let mut st = c.prepare("SELECT endpoint, p256dh, auth, titles FROM push_subscription WHERE ?1 IS NULL OR device_id = ?1")?;
                let rows = st
                    .query_map([device], |r| Ok((Subscription { endpoint: r.get(0)?, p256dh: r.get(1)?, auth: r.get(2)? }, r.get::<_, i64>(3)? != 0)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        let mut sent = 0;
        let mut errors = Vec::new();
        for (sub, titles) in rows {
            match self.deliver(app, &sub, &build(titles)).await {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => errors.push(e.to_string()),
            }
        }
        if sent == 0 && let Some(e) = errors.into_iter().next() {
            return Err(anyhow!(e));
        }
        Ok(sent)
    }

    /// A test notification to the subscriptions of one device.
    pub async fn send_test(&self, app: &App, device: &str) -> Result<usize> {
        let needs = app.sessions.list().iter().filter(|s| s.needs_you()).count();
        let n = self
            .send(app, Some(device.to_string()), |_| Message {
                title: "Launchpad test notification".into(),
                body: "Notifications work on this browser.".into(),
                url: "/settings/devices".into(),
                tag: "launchpad-test".into(),
                needs,
                urgent: true,
            })
            .await?;
        anyhow::ensure!(n > 0, "This browser has no push subscription. Turn notifications on first.");
        Ok(n)
    }

    /// An eligible session notification. Sent in the background; never blocks callers.
    pub(crate) fn session(&self, app: &App, s: &Session, event: crate::notify::SessionNotification) {
        let (push, app, s) = (self.clone(), app.clone(), s.clone());
        tokio::spawn(async move {
            let project = project_name(&app, &s).await;
            let needs = app.sessions.list().iter().filter(|s| s.needs_you()).count();
            let title = event.summary(&project);
            let result = push
                .send(&app, None, |titles| Message {
                    title: title.clone(),
                    body: if titles && !s.title.is_empty() { s.title.chars().take(160).collect() } else { s.harness.display().to_string() },
                    url: format!("/agents?session={}", s.id),
                    tag: s.id.clone(),
                    needs,
                    urgent: false,
                })
                .await;
            if let Err(e) = result {
                tracing::warn!("push: {e:#}");
            }
        });
    }
}

async fn project_name(app: &App, s: &Session) -> String {
    let id = s.project_id.clone();
    let name = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT name FROM project WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional()?)
        })
        .await
        .ok()
        .flatten();
    name.unwrap_or_else(|| std::path::Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

fn random_key() -> SecretKey {
    loop {
        if let Ok(key) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return key;
        }
    }
}

/// Validates the keys of a browser subscription before it is stored.
pub fn check_subscription(sub: &Subscription) -> Result<()> {
    let url = reqwest::Url::parse(&sub.endpoint).context("invalid push endpoint")?;
    anyhow::ensure!(url.scheme() == "https" && url.host_str().is_some(), "push endpoints must use HTTPS");
    let key = B64.decode(sub.p256dh.trim_end_matches('=')).context("invalid p256dh key")?;
    PublicKey::from_sec1_bytes(&key).map_err(|_| anyhow!("invalid p256dh key"))?;
    anyhow::ensure!(B64.decode(sub.auth.trim_end_matches('=')).is_ok_and(|a| a.len() == 16), "invalid auth secret");
    Ok(())
}

/// RFC 8291 `aes128gcm` content coding with a single record: salt, record size, the sender's
/// public key, then the encrypted payload followed by the 0x02 last-record delimiter.
fn encrypt(ua_public: &[u8], auth: &[u8], plaintext: &[u8], as_key: &SecretKey, salt: [u8; 16]) -> Result<Vec<u8>> {
    let ua = PublicKey::from_sec1_bytes(ua_public).map_err(|_| anyhow!("invalid p256dh key"))?;
    let as_public = as_key.public_key().to_sec1_bytes();
    let shared = as_key.diffie_hellman(&ua);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes()).expand(&key_info, &mut ikm).map_err(|_| anyhow!("hkdf"))?;
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek).map_err(|_| anyhow!("hkdf"))?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).map_err(|_| anyhow!("hkdf"))?;
    let mut record = plaintext.to_vec();
    record.push(2);
    anyhow::ensure!(record.len() + 16 + 86 <= RECORD_SIZE as usize, "push payload too large");
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| anyhow!("aes key"))?;
    let encrypted = cipher.encrypt(&nonce.into(), record.as_slice()).map_err(|_| anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(86 + encrypted.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(&as_public);
    out.extend_from_slice(&encrypted);
    Ok(out)
}

/// `Authorization: vapid t=<ES256 JWT>, k=<public key>`; `aud` is the push service origin.
fn vapid_header(vapid: &Vapid, endpoint: &str, subject: &str, now: i64) -> Result<String> {
    use p256::ecdsa::{Signature, signature::Signer};
    let aud = reqwest::Url::parse(endpoint)?.origin().ascii_serialization();
    let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(serde_json::to_vec(&json!({"aud": aud, "exp": now + 12 * 60 * 60, "sub": subject}))?);
    let input = format!("{header}.{claims}");
    let signature: Signature = SigningKey::from(&vapid.key).sign(input.as_bytes());
    Ok(format!("vapid t={input}.{}, k={}", B64.encode(signature.to_bytes()), B64.encode(&vapid.public)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc8291_test_vector() {
        let d = |s: &str| B64.decode(s).unwrap();
        let as_key = SecretKey::from_slice(&d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        assert_eq!(as_key.public_key().to_sec1_bytes().to_vec(), d("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"));
        let salt: [u8; 16] = d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let out = encrypt(
            &d("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
            &d("BTBZMqHH6r4Tts7J_aSIgg"),
            b"When I grow up, I want to be a watermelon",
            &as_key,
            salt,
        )
        .unwrap();
        assert_eq!(
            B64.encode(out),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn vapid_header_is_a_verifiable_es256_jwt() {
        use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
        let key = random_key();
        let vapid = Vapid { public: key.public_key().to_sec1_bytes().to_vec(), key };
        let header = vapid_header(&vapid, "https://push.example.net:8443/send/abc?x=1", "mailto:a@example.com", 1_000).unwrap();
        let (t, k) = header.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(B64.decode(k).unwrap(), vapid.public);
        let (input, signature) = t.rsplit_once('.').unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&B64.decode(input.split_once('.').unwrap().1).unwrap()).unwrap();
        assert_eq!(claims, json!({"aud": "https://push.example.net:8443", "exp": 1_000 + 12 * 3600, "sub": "mailto:a@example.com"}));
        let signature = Signature::from_slice(&B64.decode(signature).unwrap()).unwrap();
        VerifyingKey::from(&SigningKey::from(&vapid.key)).verify(input.as_bytes(), &signature).unwrap();
    }
}

//! The Hugging Face side: the OAuth loopback sign-in, token refresh, and discovery of the owner's Launchpad Space
//! (the extension's rules in extension/src/background/hf.ts). The access token stays in memory; the refresh token
//! (or a pasted token) goes to the keyring.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::oauth::{self, Callback, HUB};
use crate::secrets;

/// Renew this long before the access token expires.
const MARGIN: Duration = Duration::from_secs(5 * 60);
/// The browser approval waits this long.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const SPACE_TAG: &str = "launchpad-home";
const SPACE_TITLE: &str = "Launchpad Home";

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

enum Login {
    /// OAuth: an access token in memory and the refresh token (also in the keyring).
    OAuth { client_id: String, refresh: String, access: Option<(String, Instant)> },
    /// A token the owner pasted (read scope): used as it is.
    Token(String),
}

/// The owner's Hugging Face login, shared by every request to the Space.
#[derive(Clone)]
pub struct HfAuth {
    login: Arc<Mutex<Login>>,
    /// The Home the refresh token is stored for (empty before a Space is chosen).
    origin: Arc<Mutex<String>>,
    /// OAuth (refreshable), not a pasted token.
    pub oauth: bool,
}

impl HfAuth {
    pub fn token(token: String) -> HfAuth {
        HfAuth { login: Arc::new(Mutex::new(Login::Token(token))), origin: Arc::new(Mutex::new(String::new())), oauth: false }
    }

    fn oauth(client_id: String, answer: TokenAnswer) -> HfAuth {
        let refresh = answer.refresh_token.clone().unwrap_or_default();
        let access = Some((answer.access_token, Instant::now() + Duration::from_secs(answer.expires_in.unwrap_or(3600))));
        HfAuth { login: Arc::new(Mutex::new(Login::OAuth { client_id, refresh, access })), origin: Arc::new(Mutex::new(String::new())), oauth: true }
    }

    /// From the keyring of a linked Space: the refresh token (OAuth) or the pasted token.
    pub async fn from_keyring(origin: &str, client_id: Option<String>) -> anyhow::Result<Option<HfAuth>> {
        if let Some(token) = secrets::load(secrets::HF_TOKEN, origin).await? {
            let auth = HfAuth::token(token);
            *auth.origin.lock().await = origin.to_string();
            return Ok(Some(auth));
        }
        let (Some(refresh), Some(client_id)) = (secrets::load(secrets::HF_REFRESH, origin).await?, client_id) else { return Ok(None) };
        let auth = HfAuth { login: Arc::new(Mutex::new(Login::OAuth { client_id, refresh, access: None })), origin: Arc::new(Mutex::new(origin.to_string())), oauth: true };
        Ok(Some(auth))
    }

    /// Stores the long-lived part in the keyring for this Home (after the Space is chosen).
    pub async fn keep_for(&self, origin: &str) -> anyhow::Result<()> {
        *self.origin.lock().await = origin.to_string();
        match &*self.login.lock().await {
            Login::OAuth { refresh, .. } if !refresh.is_empty() => secrets::store(secrets::HF_REFRESH, origin, refresh).await,
            Login::Token(token) => secrets::store(secrets::HF_TOKEN, origin, token).await,
            Login::OAuth { .. } => Ok(()),
        }
    }

    /// A valid bearer for the Hub and the Space, refreshed when it expires soon.
    pub async fn bearer(&self, http: &reqwest::Client) -> anyhow::Result<String> {
        let mut login = self.login.lock().await;
        match &mut *login {
            Login::Token(t) => Ok(t.clone()),
            Login::OAuth { access: Some((token, until)), .. } if *until > Instant::now() + MARGIN => Ok(token.clone()),
            Login::OAuth { client_id, refresh, access } => {
                let answer = token(http, &[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", client_id.as_str())]).await?;
                if let Some(next) = answer.refresh_token.clone().filter(|r| r != refresh) {
                    *refresh = next;
                    let origin = self.origin.lock().await.clone();
                    if !origin.is_empty() {
                        secrets::store(secrets::HF_REFRESH, &origin, refresh).await?;
                    }
                }
                let value = answer.access_token.clone();
                *access = Some((answer.access_token, Instant::now() + Duration::from_secs(answer.expires_in.unwrap_or(3600))));
                Ok(value)
            }
        }
    }

    /// Drops the cached access token so the next `bearer` refreshes (after a 401 of the Space).
    pub async fn expire(&self) {
        if let Login::OAuth { access, .. } = &mut *self.login.lock().await {
            *access = None;
        }
    }
}

async fn token(http: &reqwest::Client, form: &[(&str, &str)]) -> anyhow::Result<TokenAnswer> {
    let response = http.post(format!("{HUB}/oauth/token")).form(form).send().await?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if !status.is_success() || body.get("access_token").is_none() {
        let why = body.get("error_description").or_else(|| body.get("error")).and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| status.to_string());
        anyhow::bail!("Hugging Face refused the sign-in: {why}");
    }
    Ok(serde_json::from_value(body)?)
}

/// The OAuth loopback flow: listens once on `127.0.0.1:<random port>`, opens the authorize page in the default browser
/// (`open`), waits up to 5 minutes for the redirect with our `state`, and exchanges the code.
pub async fn sign_in(http: &reqwest::Client, client_id: &str, open: impl FnOnce(String)) -> anyhow::Result<HfAuth> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let redirect = oauth::redirect_uri(listener.local_addr()?.port());
    let (verifier, state) = (oauth::random(48), oauth::random(16));
    open(oauth::authorize_url(client_id, &redirect, &state, &verifier));
    let code = tokio::time::timeout(SIGN_IN_TIMEOUT, async {
        loop {
            let (mut socket, _) = listener.accept().await?;
            let mut buf = vec![0u8; 8192];
            let n = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut buf)).await.unwrap_or(Ok(0)).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let line = request.lines().next().unwrap_or("");
            let (status, text, result) = match oauth::parse_callback(line, &state) {
                Callback::Code(code) => ("200 OK", "Launchpad is signed in to Hugging Face. You can close this tab.", Some(Ok(code))),
                Callback::Refused(why) => ("200 OK", "Hugging Face did not sign Launchpad in. You can close this tab.", Some(Err(anyhow::anyhow!("Hugging Face refused: {why}")))),
                Callback::Other => ("404 Not Found", "Not found", None),
            };
            let page = format!("<!doctype html><meta charset=utf-8><title>Launchpad</title><p style=\"font:15px sans-serif;margin:3em\">{text}</p>");
            let answer = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{page}", page.len());
            let _ = socket.write_all(answer.as_bytes()).await;
            if let Some(result) = result {
                return result;
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("The sign-in was not approved within 5 minutes"))??;
    drop(listener);
    let answer = token(http, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", client_id), ("code_verifier", &verifier)]).await?;
    Ok(HfAuth::oauth(client_id.to_string(), answer))
}

async fn hub<T: serde::de::DeserializeOwned>(http: &reqwest::Client, auth: &HfAuth, path: &str) -> anyhow::Result<T> {
    let bearer = auth.bearer(http).await?;
    let response = http.get(format!("{HUB}{path}")).bearer_auth(bearer).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("Hugging Face answered {} for {}", response.status(), path.split('?').next().unwrap_or(path));
    }
    Ok(response.json().await?)
}

#[derive(Deserialize)]
struct Org {
    name: String,
}
#[derive(Deserialize)]
pub struct Whoami {
    pub name: String,
    #[serde(default)]
    orgs: Vec<Org>,
}

pub async fn whoami(http: &reqwest::Client, auth: &HfAuth) -> anyhow::Result<Whoami> {
    hub(http, auth, "/api/whoami-v2").await
}

#[derive(Deserialize, Default)]
struct CardData {
    #[serde(default)]
    title: Option<String>,
}
#[derive(Deserialize, Default)]
struct Runtime {
    #[serde(default)]
    stage: Option<String>,
}
#[derive(Deserialize)]
struct SpaceInfo {
    id: String,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    subdomain: Option<String>,
    #[serde(default)]
    runtime: Option<Runtime>,
    #[serde(default, rename = "cardData")]
    card_data: Option<CardData>,
    #[serde(default)]
    tags: Vec<String>,
}

/// A Launchpad Space found on the account or one of its organizations.
#[derive(Clone, Debug)]
pub struct FoundHome {
    pub space: String,
    pub origin: String,
    pub stage: Option<String>,
    /// It runs and answered `/_home/status` as Launchpad (asked only of the account's own running Spaces).
    pub answers: bool,
    pub own: bool,
}

/// A Space's own address: the only place besides the Hub that ever receives the Hugging Face token.
pub fn space_origin(origin: &str) -> bool {
    origin.strip_prefix("https://").and_then(|h| h.strip_suffix(".hf.space")).is_some_and(|sub| !sub.is_empty() && sub.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// The Launchpad Spaces of the account, then of its organizations (tag `launchpad-home`, or the title of Spaces
/// deployed before the tag).
pub async fn discover(http: &reqwest::Client, auth: &HfAuth) -> anyhow::Result<Vec<FoundHome>> {
    let me = whoami(http, auth).await?;
    let authors: Vec<String> = std::iter::once(me.name.clone()).chain(me.orgs.into_iter().map(|o| o.name)).collect();
    let mut found = Vec::new();
    for author in authors {
        let q = |pairs: &[(&str, &str)]| url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
        let tagged: Vec<SpaceInfo> = hub(http, auth, &format!("/api/spaces?{}", q(&[("author", &author), ("filter", SPACE_TAG)]))).await.unwrap_or_default();
        let named: Vec<SpaceInfo> = hub(http, auth, &format!("/api/spaces?{}", q(&[("author", &author), ("search", "launchpad"), ("full", "true")]))).await.unwrap_or_default();
        let mut ids: Vec<String> = tagged.into_iter().map(|s| s.id).collect();
        for s in named {
            if (s.card_data.as_ref().and_then(|c| c.title.as_deref()) == Some(SPACE_TITLE) || s.tags.iter().any(|t| t == SPACE_TAG)) && !ids.contains(&s.id) {
                ids.push(s.id);
            }
        }
        for id in ids {
            let Ok(info) = hub::<SpaceInfo>(http, auth, &format!("/api/spaces/{id}")).await else { continue };
            let origin = info.host.clone().or_else(|| info.subdomain.as_ref().map(|s| format!("https://{s}.hf.space")));
            let Some(origin) = origin.filter(|o| space_origin(o)) else { continue };
            let stage = info.runtime.and_then(|r| r.stage);
            // Only the account's own running Spaces are asked: a request wakes a sleeping one, and an organization's
            // Space runs code other members may change.
            let own = author == me.name;
            let answers = own && stage.as_deref() == Some("RUNNING") && matches!(home_status(http, auth, &origin).await, HomeStatus::Home { .. });
            found.push(FoundHome { space: id, origin, stage, answers, own });
        }
    }
    Ok(found)
}

pub enum HomeStatus {
    Home { complete: bool },
    Starting,
    Unreachable(String),
}

/// `GET /_home/status` of a Space, as the desktop client.
pub async fn home_status(http: &reqwest::Client, auth: &HfAuth, origin: &str) -> HomeStatus {
    if !space_origin(origin) {
        return HomeStatus::Unreachable("Not a Hugging Face Space address".into());
    }
    let Ok(bearer) = auth.bearer(http).await else { return HomeStatus::Unreachable("Sign in with Hugging Face again".into()) };
    let response = http.get(format!("{origin}/_home/status")).bearer_auth(bearer).header(crate::home::CLIENT_HEADER, crate::home::client()).send().await;
    match response {
        Ok(r) => {
            let status = r.status().as_u16();
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            if status == 200 && body.get("launchpad") == Some(&serde_json::Value::Bool(true)) {
                HomeStatus::Home { complete: body.get("complete").and_then(|v| v.as_bool()).unwrap_or(false) }
            } else if matches!(status, 401 | 403 | 404) {
                HomeStatus::Unreachable("Hugging Face refused access to this Space".into())
            } else {
                HomeStatus::Starting
            }
        }
        Err(e) => HomeStatus::Unreachable(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_space_addresses_get_the_token() {
        assert!(super::space_origin("https://me-launchpad.hf.space"));
        for bad in ["http://me.hf.space", "https://evil.com/.hf.space", "https://a.b.hf.space", "https://.hf.space", "https://ME.hf.space"] {
            assert!(!super::space_origin(bad), "{bad}");
        }
    }
}

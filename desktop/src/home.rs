//! The Home client. Every call names the companion (`X-Launchpad-Client: desktop/<version>`), sends its device
//! secret (`X-Launchpad-Device`) and never an Origin; on a Space it also sends the owner's Hugging Face token, which
//! passes the Hub's private-Space check (the gateway strips it before Home).

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::conn::{Failure, classify};
use crate::hf::HfAuth;
use crate::settings::{HomeKind, HomeLink};

pub const CLIENT_HEADER: &str = "X-Launchpad-Client";
pub const DEVICE_HEADER: &str = "X-Launchpad-Device";

pub fn client() -> String {
    format!("desktop/{}", env!("CARGO_PKG_VERSION"))
}

/// The HTTP client of every request: rustls, no cookies, no proxy secrets, a user agent naming the companion.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("launchpad-desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP client")
}

/// A failed request: what kind of failure (for the connection state) and a message for the owner.
#[derive(Debug)]
pub struct HomeError {
    pub failure: Failure,
    pub message: String,
}

impl std::fmt::Display for HomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for HomeError {}

fn network(e: impl std::fmt::Display) -> HomeError {
    HomeError { failure: Failure::Network, message: format!("Home does not answer: {e}") }
}

#[derive(Clone)]
pub struct Home {
    pub http: reqwest::Client,
    pub link: HomeLink,
    device: String,
    pub hf: Option<HfAuth>,
}

impl Home {
    pub fn new(http: reqwest::Client, link: HomeLink, device: String, hf: Option<HfAuth>) -> Home {
        Home { http, link, device, hf }
    }

    fn space(&self) -> bool {
        self.link.kind == HomeKind::Space
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: Option<&serde_json::Value>, extra: &[(&str, String)]) -> Result<reqwest::Response, HomeError> {
        let mut retried = false;
        loop {
            let mut req = self.http.request(method.clone(), format!("{}{path}", self.link.origin)).header(CLIENT_HEADER, client()).header(DEVICE_HEADER, &self.device);
            for (k, v) in extra {
                req = req.header(*k, v);
            }
            if let (true, Some(hf)) = (self.space(), &self.hf) {
                let bearer = hf.bearer(&self.http).await.map_err(|e| HomeError { failure: Failure::AuthExpired, message: e.to_string() })?;
                req = req.bearer_auth(bearer);
            }
            if let Some(body) = body {
                req = req.json(body);
            }
            let response = req.send().await.map_err(network)?;
            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }
            let text = response.text().await.unwrap_or_default();
            let failure = classify(status, &text, self.space());
            // A Space's 401 may be an expired access token: refresh once.
            if failure == Failure::AuthExpired && !retried && let Some(hf) = &self.hf {
                hf.expire().await;
                retried = true;
                continue;
            }
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| format!("Home answered {status}"));
            return Err(HomeError { failure, message });
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, HomeError> {
        let response = self.send(reqwest::Method::GET, path, None, &[]).await?;
        response.json().await.map_err(|e| HomeError { failure: Failure::Other, message: format!("Unexpected answer from Home: {e}") })
    }

    pub async fn delete(&self, path: &str) -> Result<(), HomeError> {
        self.send(reqwest::Method::DELETE, path, None, &[]).await.map(|_| ())
    }

    /// `GET /api/events` from a position (`Last-Event-ID: <boot>:<bus>`).
    pub async fn events(&self, position: Option<&(String, u64)>) -> Result<reqwest::Response, HomeError> {
        let mut extra = vec![("Accept", "text/event-stream".to_string())];
        if let Some((boot, bus)) = position {
            extra.push(("Last-Event-ID", format!("{boot}:{bus}")));
        }
        self.send(reqwest::Method::GET, "/api/events", None, &extra).await
    }
}

/// `POST` without a device (linking): reverse pairing, and the Space's desktop connect route with the Hugging Face
/// bearer. Answers Home's JSON, or its error text.
pub async fn post_unlinked(http: &reqwest::Client, url: &str, body: &serde_json::Value, bearer: Option<&str>) -> Result<serde_json::Value, (u16, String)> {
    let mut req = http.post(url).header(CLIENT_HEADER, client()).json(body);
    if let Some(b) = bearer {
        req = req.bearer_auth(b);
    }
    let response = req.send().await.map_err(|e| (0, format!("Home does not answer: {e}")))?;
    let status = response.status().as_u16();
    let data: serde_json::Value = response.json().await.unwrap_or_default();
    if status >= 400 {
        return Err((status, data.get("error").and_then(|e| e.as_str()).map(str::to_string).unwrap_or_else(|| format!("Home answered {status}"))));
    }
    Ok(data)
}

/// The manual address: HTTPS, or HTTP on this computer (like the extension's `addressOrigin`).
pub fn address_origin(input: &str) -> Result<String, String> {
    let url = url::Url::parse(input.trim()).map_err(|_| "Type an address such as http://127.0.0.1:7317 or https://home.example.ts.net".to_string())?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let ok = url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none() && url.path() == "/" && (url.scheme() == "https" || (url.scheme() == "http" && local));
    if !ok {
        return Err("Use an HTTPS address, or HTTP on this computer".into());
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    #[test]
    fn manual_addresses() {
        assert_eq!(super::address_origin("http://127.0.0.1:7317/").unwrap(), "http://127.0.0.1:7317");
        assert_eq!(super::address_origin(" https://home.tail.ts.net ").unwrap(), "https://home.tail.ts.net");
        for bad in ["http://192.168.1.2:7317", "https://h/path", "https://u:p@h", "ftp://h", "nonsense"] {
            assert!(super::address_origin(bad).is_err(), "{bad}");
        }
    }
}

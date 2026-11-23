//! Sign in with Hugging Face as a native app (folio page Linking, L-1 to L-3): authorization code with PKCE (S256), a
//! public client, and a loopback redirect `http://127.0.0.1:<port>/callback` (RFC 8252 §7.3). Hugging Face accepts
//! any port for an `http` loopback redirect URI registered without one (https://huggingface.co/docs/hub/oauth,
//! "Redirect URIs"); `127.0.0.1` and `localhost` are matched separately, so the app registers
//! `http://127.0.0.1/callback`. The pure parts are here; the listener is in `hf.rs`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub const HUB: &str = "https://huggingface.co";
pub const SCOPES: &str = "openid profile read-repos";
pub const CALLBACK: &str = "/callback";

/// A random URL-safe string of `bytes` random bytes.
pub fn random(bytes: usize) -> String {
    let data: Vec<u8> = (0..bytes).map(|_| rand::random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(data)
}

/// The S256 challenge of a PKCE verifier (RFC 7636 §4.2).
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK}")
}

pub fn authorize_url(client_id: &str, redirect: &str, state: &str, verifier: &str) -> String {
    let mut url = url::Url::parse(&format!("{HUB}/oauth/authorize")).expect("valid hub URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("state", state)
        .append_pair("code_challenge", &challenge(verifier))
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

/// What the browser's request to the loopback listener said.
#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    /// Hugging Face's error (the owner refused, or a bad request), with its description.
    Refused(String),
    /// Not our callback (another path, a favicon, a wrong `state`): answer 404 and keep listening.
    Other,
}

/// Reads the request line of the browser's redirect (`GET /callback?code=…&state=… HTTP/1.1`).
pub fn parse_callback(request_line: &str, state: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else { return Callback::Other };
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else { return Callback::Other };
    if url.path() != CALLBACK {
        return Callback::Other;
    }
    let get = |k: &str| url.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
    if get("state").as_deref() != Some(state) {
        return Callback::Other;
    }
    if let Some(error) = get("error") {
        return Callback::Refused(get("error_description").unwrap_or(error));
    }
    match get("code") {
        Some(code) if !code.is_empty() => Callback::Code(code),
        _ => Callback::Refused("Hugging Face returned no authorization code".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(random(32).len(), 43);
        assert_ne!(random(16), random(16));
    }

    #[test]
    fn callback_needs_our_path_and_state() {
        assert_eq!(parse_callback("GET /callback?code=abc&state=s1 HTTP/1.1", "s1"), Callback::Code("abc".into()));
        assert_eq!(parse_callback("GET /callback?code=abc&state=other HTTP/1.1", "s1"), Callback::Other);
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1", "s1"), Callback::Other);
        assert_eq!(parse_callback("POST /callback?code=abc&state=s1 HTTP/1.1", "s1"), Callback::Other);
        assert_eq!(parse_callback("GET /callback?error=access_denied&error_description=No+thanks&state=s1 HTTP/1.1", "s1"), Callback::Refused("No thanks".into()));
        let url = authorize_url("cid", &redirect_uri(4242), "st", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert!(url.starts_with("https://huggingface.co/oauth/authorize?client_id=cid&redirect_uri=http%3A%2F%2F127.0.0.1%3A4242%2Fcallback&response_type=code&scope=openid+profile+read-repos&state=st&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
    }
}

//! Bot avatars: DiceBear's `bottts` style, fetched once per seed by Home and kept on disk, so windows never call
//! DiceBear themselves and a seed is fetched at most once (`GET /api/avatars/bottts?seed=…`). The URL names the
//! seed, so a new seed is a new file and a new URL: nothing is ever invalidated. A failed fetch is remembered for a
//! few minutes (the window shows the bot's letter mark meanwhile). The SVG is served as an image only: `nosniff`
//! and a sandboxing CSP, so opening it alone runs nothing.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::Ctx;
use crate::error::ApiError;

/// DiceBear's API, version pinned: a new major version may draw other robots for the same seed.
const API: &str = "https://api.dicebear.com/10.x/bottts/svg";
/// Longest seed accepted (bot names are short; a custom seed too).
pub const MAX_SEED: usize = 128;
/// Largest SVG kept (DiceBear's bottts are about 25 KB).
const MAX_BYTES: usize = 256 * 1024;
/// Files kept before the oldest go.
const MAX_FILES: usize = 1000;
/// How long a failed seed is not asked again.
const RETRY_AFTER: Duration = Duration::from_secs(300);

pub fn router() -> Router<Ctx> {
    Router::new().route("/api/avatars/bottts", get(avatar))
}

#[derive(Deserialize)]
struct Seed {
    seed: String,
}

/// Pure: the seed as fetched (outer spaces trimmed), or why it is refused.
pub fn clean_seed(seed: &str) -> Result<&str, &'static str> {
    let s = seed.trim();
    if s.is_empty() {
        Err("name a seed")
    } else if s.chars().count() > MAX_SEED {
        Err("the seed is too long (128 characters at most)")
    } else if s.chars().any(char::is_control) {
        Err("the seed has control characters")
    } else {
        Ok(s)
    }
}

/// Pure: the cache file of a seed (the style and API version are part of the name).
pub fn file_name(seed: &str) -> String {
    format!("bottts-10-{}.svg", hex::encode(Sha256::digest(seed.as_bytes())))
}

/// Pure: whether a body looks like the SVG DiceBear sends (size, an `<svg` root).
pub fn looks_like_svg(body: &[u8]) -> bool {
    if body.is_empty() || body.len() > MAX_BYTES {
        return false;
    }
    let Ok(text) = std::str::from_utf8(body) else { return false };
    let t = text.trim_start_matches('\u{feff}').trim_start();
    let t = if t.starts_with("<?xml") { t.find("?>").map_or("", |i| t[i + 2..].trim_start()) } else { t };
    t.starts_with("<svg")
}

static FAILED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);
/// One fetch per seed at a time (a chat list asks for the same avatars at once).
static FETCHING: LazyLock<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);
static ROUTED: launchpad_core::egress::Clients = launchpad_core::egress::Clients::new(client_settings);

fn client_settings(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    b.timeout(Duration::from_secs(15)).user_agent("launchpad-home")
}

fn direct() -> Result<reqwest::Client, String> {
    static CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| client_settings(reqwest::Client::builder()).build().map_err(|e| e.to_string()));
    CLIENT.clone()
}

async fn avatar(State(s): State<Ctx>, Query(q): Query<Seed>) -> Result<Response, ApiError> {
    let seed = clean_seed(&q.seed).map_err(ApiError::bad_request)?.to_string();
    let dir = s.app.paths.data_dir.join("cache").join("avatars");
    let path = dir.join(file_name(&seed));
    if let Ok(body) = tokio::fs::read(&path).await {
        return Ok(svg(body));
    }
    let lock = FETCHING.lock().unwrap_or_else(|p| p.into_inner()).entry(seed.clone()).or_default().clone();
    let _held = lock.lock().await;
    let done = |seed: &str| {
        FETCHING.lock().unwrap_or_else(|p| p.into_inner()).remove(seed);
    };
    // Another request fetched it while this one waited.
    if let Ok(body) = tokio::fs::read(&path).await {
        done(&seed);
        return Ok(svg(body));
    }
    if FAILED.lock().unwrap_or_else(|p| p.into_inner()).get(&seed).is_some_and(|t| t.elapsed() < RETRY_AFTER) {
        done(&seed);
        return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "the avatar could not be fetched recently: try again later"));
    }
    let result = fetch(&seed).await;
    done(&seed);
    match result {
        Ok(body) => {
            FAILED.lock().unwrap_or_else(|p| p.into_inner()).remove(&seed);
            if let Err(e) = store(&dir, &path, &body).await {
                tracing::warn!("avatar cache: {e:#}");
            }
            Ok(svg(body))
        }
        Err(e) => {
            tracing::warn!("avatar of seed {seed:?}: {e}");
            let mut failed = FAILED.lock().unwrap_or_else(|p| p.into_inner());
            failed.retain(|_, t| t.elapsed() < RETRY_AFTER);
            failed.insert(seed, Instant::now());
            Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY, format!("the avatar could not be fetched: {e}")))
        }
    }
}

async fn fetch(seed: &str) -> Result<Vec<u8>, String> {
    let mut url = reqwest::Url::parse(API).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("seed", seed);
    let (client, _) = ROUTED.pick(&direct()?, &url)?;
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("DiceBear answered {}", res.status()));
    }
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    if !looks_like_svg(&body) {
        return Err("DiceBear's answer is not an SVG".into());
    }
    Ok(body.to_vec())
}

/// Writes the file (renamed into place) and keeps the folder under `MAX_FILES`, oldest first.
async fn store(dir: &std::path::Path, path: &std::path::Path, body: &[u8]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, path).await?;
    let mut files = Vec::new();
    let mut rd = tokio::fs::read_dir(dir).await?;
    while let Some(e) = rd.next_entry().await? {
        if let Ok(m) = e.metadata().await {
            files.push((m.modified().ok(), e.path()));
        }
    }
    if files.len() > MAX_FILES {
        files.sort();
        for (_, p) in files.iter().take(files.len() - MAX_FILES) {
            let _ = tokio::fs::remove_file(p).await;
        }
    }
    Ok(())
}

fn svg(body: Vec<u8>) -> Response {
    let mut r = body.into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_files_and_bodies() {
        assert_eq!(clean_seed("  Scout "), Ok("Scout"));
        assert!(clean_seed("   ").is_err());
        assert!(clean_seed(&"x".repeat(129)).is_err());
        assert!(clean_seed("a\nb").is_err());
        assert_ne!(file_name("Scout"), file_name("scout"));
        assert!(file_name("Scout").starts_with("bottts-10-") && file_name("Scout").ends_with(".svg"));
        assert!(looks_like_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"));
        assert!(looks_like_svg(b"<?xml version=\"1.0\"?>\n<svg></svg>"));
        assert!(!looks_like_svg(b"<html><svg></svg></html>"));
        assert!(!looks_like_svg(b""));
    }
}

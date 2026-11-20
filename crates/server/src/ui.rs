//! The React app, built into `web/dist` and embedded in the release binary.
//! Debug builds read the files from disk, so `pnpm build` is enough to update a running dev daemon.
//!
//! The `rust-embed` derive lives in the binary (`crates/cli`), which hands its files over with [`set_assets`]:
//! the embedded file list is read anew whenever the binary compiles, and this crate stays cacheable in CI's
//! compiler cache (`.github/scripts/rustc-cache.sh`).

use std::sync::OnceLock;

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::EmbeddedFile;

/// The embedded UI: one file by its path under `web/dist`, and every path.
pub struct Assets {
    pub get: fn(&str) -> Option<EmbeddedFile>,
    pub paths: fn() -> Vec<String>,
}

static ASSETS: OnceLock<Assets> = OnceLock::new();

/// Set once at start, before serving; without it the UI reads as not built.
pub fn set_assets(assets: Assets) {
    let _ = ASSETS.set(assets);
}

impl Assets {
    fn file(path: &str) -> Option<EmbeddedFile> {
        ASSETS.get().and_then(|a| (a.get)(path))
    }
    fn all() -> Vec<String> {
        ASSETS.get().map(|a| (a.paths)()).unwrap_or_default()
    }
}

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "no such route"}))).into_response();
    }
    if let Some(file) = Assets::file(path).filter(|_| !path.is_empty()) {
        let mime = match path {
            // The manifest has its own type; browsers register the service worker only as JavaScript.
            "manifest.webmanifest" => "application/manifest+json".parse().unwrap(),
            "sw.js" => "text/javascript".parse().unwrap(),
            _ => mime_guess::from_path(path).first_or_octet_stream(),
        };
        // Vite puts content hashes in asset names, so they never change. Everything else is revalidated.
        let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
        return ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], file.data).into_response();
    }
    // Client-side routes all get the app shell.
    match Assets::file("index.html") {
        Some(file) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], file.data)
            .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built: run `(cd web && pnpm build)`").into_response(),
    }
}

/// Identifies the UI build: changes whenever `index.html` (which names the hashed bundles) changes.
pub fn build_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    Assets::file("index.html").map(|f| f.data.into_owned()).unwrap_or_default().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The folio-viewer build (`web/dist/folio-viewer/`), paths relative to it: what a folio's shared snapshot carries.
pub fn folio_viewer_files() -> Vec<(String, Vec<u8>)> {
    Assets::all().into_iter().filter_map(|p| {
        let rel = p.strip_prefix("folio-viewer/")?.to_string();
        Some((rel, Assets::file(&p)?.data.into_owned()))
    }).collect()
}

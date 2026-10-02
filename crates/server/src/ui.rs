//! The React app, built into `web/dist` and embedded in the release binary.
//! Debug builds read the files from disk, so `pnpm build` is enough to update a running dev daemon.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "no such route"}))).into_response();
    }
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        // Vite puts content hashes in asset names, so they never change.
        let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
        return ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], file.data).into_response();
    }
    // Client-side routes all get the app shell.
    match Assets::get("index.html") {
        Some(file) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], file.data)
            .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built: run `pnpm -C web build`").into_response(),
    }
}

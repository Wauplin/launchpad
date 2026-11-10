//! Local uploads: bounded data URLs, native image input, and private reference files.
//!
//! Browsers upload each file once (`POST /api/uploads`) and keep a short reference instead of the
//! bytes: `upload:<mime>;<sha256>`. Drafts, schedules, crons and launch specs store the reference;
//! the bytes live once in `<data_dir>/uploads/<sha256>` (in Home backups). Data URLs remain accepted
//! everywhere. Session records, events and tunnel messages carry only references: Home turns data URLs into
//! references before it sends a launch or a message to a runner (`to_refs`), a runner fetches the bytes it lacks
//! from Home (`fetch_missing`), and harnesses receive data URLs only at delivery (`for_harness`). A session's
//! attached files (`dir`) travel in its handoffs (`handoff`), so they follow a move.
use crate::model::{FileAttachment, ImageAttachment};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::OnceLock;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];
static UPLOADS: OnceLock<PathBuf> = OnceLock::new();

/// Sets the upload folder (`<data_dir>/uploads`) once at startup.
pub fn init(data_dir: &std::path::Path) {
    let _ = UPLOADS.set(data_dir.join("uploads"));
}

fn uploads_dir() -> Result<&'static PathBuf> {
    UPLOADS.get().context("the upload store is not initialized")
}

/// `(mime, sha256)` of an `upload:` reference.
fn parse_ref(url: &str) -> Option<(&str, &str)> {
    let (mime, hash) = url.strip_prefix("upload:")?.split_once(';')?;
    let valid_mime = !mime.is_empty() && mime.len() <= 100 && mime.contains('/') && mime.chars().all(|c| c.is_ascii_alphanumeric() || "/.+-".contains(c));
    let valid_hash = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    (valid_mime && valid_hash).then_some((mime, hash))
}

pub fn is_image(data_url: &str) -> bool {
    data_url.starts_with("data:image/") || data_url.starts_with("upload:image/")
}

/// Stores uploaded bytes once (content-addressed) and returns their reference.
pub async fn store(bytes: &[u8], mime: &str) -> Result<String> {
    let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let mime = if mime.is_empty() { "application/octet-stream".to_string() } else { mime };
    ensure!(bytes.len() <= MAX_BYTES, "each attachment must be 5 MB or smaller");
    ensure!(!mime.starts_with("image/") || IMAGE_TYPES.contains(&mime.as_str()), "use PNG, JPEG, WebP or GIF images");
    ensure!(!mime.starts_with("image/") || !bytes.is_empty(), "the image is empty");
    let hash = hex::encode(Sha256::digest(bytes));
    let reference = format!("upload:{mime};{hash}");
    ensure!(parse_ref(&reference).is_some(), "unsupported file type {mime}");
    let dir = uploads_dir()?;
    tokio::fs::create_dir_all(dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let path = dir.join(&hash);
    if tokio::fs::metadata(&path).await.is_err() {
        let tmp = dir.join(format!("{hash}.tmp-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await?;
        }
        tokio::fs::rename(&tmp, &path).await?;
        // Home backups carry uploads (`home_backup::data_files`).
        crate::home_backup::changed();
    } else {
        // Touch: garbage collection keeps recently used uploads.
        let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    }
    Ok(reference)
}

/// Bytes of a stored image for display (`GET /api/uploads/{hash}`). Only images are served, never other types.
pub fn read_image(hash: &str, mime: &str) -> Result<Vec<u8>> {
    ensure!(IMAGE_TYPES.contains(&mime), "only images can be shown");
    decode(&format!("upload:{mime};{hash}"), true)
}

/// Deletes uploads older than two weeks that no draft, scheduled launch, cron or message image still references.
/// Never-started sessions still need their launch images when resumed; files were copied at launch.
pub async fn collect_garbage(app: &crate::App) -> Result<()> {
    let Ok(dir) = uploads_dir() else { return Ok(()) };
    // Only the hashes are kept (rows are read one at a time), so the cost stays one pass over each table.
    let kept: std::collections::HashSet<String> = app.state_db.call(|c| {
        let mut out = std::collections::HashSet::new();
        let mut queries: Vec<String> = Vec::new();
        for table in ["launch_draft", "scheduled_launch", "cron_agent"] {
            let column = if table == "launch_draft" { "spec_json" } else { "json" };
            queries.push(format!("SELECT {column} FROM {table}"));
        }
        queries.push("SELECT launch_spec_json FROM session WHERE harness_session_id IS NULL".into());
        // Message images are references: the chat shows them from here.
        queries.push("SELECT json FROM session_event WHERE kind = 'user_message' AND json LIKE '%upload:%'".into());
        for query in queries {
            let Ok(mut stmt) = c.prepare(&query) else { continue };
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                if let Ok(text) = r.get::<_, String>(0) {
                    referenced_hashes(&text, &mut out);
                }
            }
        }
        Ok(out)
    }).await?;
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return Ok(()) };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(14 * 24 * 3600);
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let old = entry.metadata().await.ok().and_then(|m| m.modified().ok()).is_some_and(|t| t < cutoff);
        if old && !kept.contains(&name) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

/// Adds the hashes of the `upload:` references in `text` (JSON or plain) to `out`.
fn referenced_hashes(text: &str, out: &mut std::collections::HashSet<String>) {
    for (i, _) in text.match_indices("upload:") {
        let rest = &text[i..];
        let end = rest.find(|c: char| c == '"' || c.is_whitespace()).unwrap_or(rest.len());
        if let Some((_, hash)) = parse_ref(&rest[..end]) {
            out.insert(hash.to_string());
        }
    }
}

/// Encoded length (for the total limit), counting a reference as its stored bytes in base64.
fn encoded_len(url: &str) -> usize {
    match parse_ref(url) {
        Some((_, hash)) => uploads_dir().ok().and_then(|d| std::fs::metadata(d.join(hash)).ok()).map(|m| m.len() as usize * 4 / 3).unwrap_or(0),
        None => url.len(),
    }
}

/// A `data:` URL for an attachment that may be a stored reference (harnesses take data URLs).
pub fn to_data_url(url: &str) -> Result<String> {
    let Some((mime, _)) = parse_ref(url) else { return Ok(url.to_string()) };
    let bytes = decode(url, false)?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Home: replaces data URLs by stored references before a launch or a message goes to a runner or to Burst, so
/// records, events and tunnel messages never carry the bytes (the runner fetches them, `fetch_missing`). The
/// session may fetch these references (`read_for_runner`).
pub async fn to_refs(session_id: &str, files: &mut [FileAttachment]) -> Result<()> {
    for file in files.iter_mut() {
        if parse_ref(&file.data_url).is_some() {
            continue;
        }
        let bytes = decode(&file.data_url, false)?;
        let mime = file.data_url.strip_prefix("data:").and_then(|h| h.split([';', ',']).next()).unwrap_or_default().to_string();
        file.data_url = store(&bytes, &mime).await?;
    }
    grant(session_id, files.iter().filter_map(|f| parse_ref(&f.data_url)).map(|(_, hash)| hash.to_string()));
    Ok(())
}

/// How long Home lets a session's runner fetch the references of a message it relayed (`to_refs`). Launch images
/// and held messages stay fetchable through Home's record of the session.
const GRANT_MS: i64 = 24 * 3600 * 1000;

/// Home: `(session, hash)` → when `to_refs` named it, for `read_for_runner`.
static GRANTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, String), i64>>> = std::sync::LazyLock::new(Default::default);

fn grant(session_id: &str, hashes: impl Iterator<Item = String>) {
    let now = crate::util::now_ms();
    let mut grants = GRANTS.lock().unwrap();
    grants.retain(|_, at| now - *at < GRANT_MS);
    for hash in hashes {
        grants.insert((session_id.to_string(), hash), now);
    }
}

/// Runner: fetches from Home the stored bytes of the files' references it does not hold (runner call `upload`).
pub async fn fetch_missing(app: &crate::App, session_id: &str, files: &[FileAttachment]) -> Result<()> {
    fetch_urls(app, session_id, files.iter().map(|f| f.data_url.clone()).filter(|u| u.starts_with("upload:")).collect()).await
}

/// `fetch_missing` for message images.
pub async fn fetch_missing_images(app: &crate::App, session_id: &str, images: &[ImageAttachment]) -> Result<()> {
    fetch_urls(app, session_id, images.iter().map(|f| f.data_url.clone()).filter(|u| u.starts_with("upload:")).collect()).await
}

async fn fetch_urls(app: &crate::App, session_id: &str, urls: Vec<String>) -> Result<()> {
    if app.paths.role != crate::role::Role::Runner {
        return Ok(());
    }
    let dir = uploads_dir()?;
    for url in &urls {
        let Some((mime, hash)) = parse_ref(url) else { continue };
        if dir.join(hash).exists() {
            continue;
        }
        let v = crate::home_link::home_call("upload", serde_json::json!({"reference": url, "session": session_id})).await.context("could not fetch an attachment from Home")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().context("Home sent no attachment")?)?;
        ensure!(hex::encode(Sha256::digest(&bytes)) == hash, "Home sent a different attachment");
        store(&bytes, mime).await?;
    }
    Ok(())
}

/// Home: the bytes of a stored reference, in base64, for a runner (`fetch_missing`): only for a session Home placed
/// on that runner, and only a reference Home sent it (`to_refs`) or that Home's record of it names (launch
/// attachments, held messages).
pub fn read_for_runner(app: &crate::App, runner_id: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    let url = body["reference"].as_str().context("reference")?;
    let session_id = body["session"].as_str().context("session")?;
    let (_, hash) = parse_ref(url).context("invalid attachment reference")?;
    let s = app.sessions.get(session_id).filter(|s| s.runner_id.as_deref() == Some(runner_id)).context("Home did not place this session on this runner")?;
    let named = |u: &str| parse_ref(u).is_some_and(|(_, h)| h == hash);
    let in_record = s.launch_spec.attachments.iter().any(|a| named(&a.data_url))
        || s.quota_pause.as_ref().is_some_and(|p| p.images.iter().chain(p.messages.iter().flat_map(|m| m.images.iter())).any(|i| named(&i.data_url)));
    let granted = GRANTS.lock().unwrap().get(&(session_id.to_string(), hash.to_string())).is_some_and(|at| crate::util::now_ms() - *at < GRANT_MS);
    ensure!(in_record || granted, "this attachment was not sent to this session");
    Ok(serde_json::json!({"data": base64::engine::general_purpose::STANDARD.encode(decode(url, false)?)}))
}

/// Message images as the harness takes them (data URLs), fetched from Home when missing here.
pub async fn for_harness(app: &crate::App, session_id: &str, images: &[ImageAttachment]) -> Result<Vec<ImageAttachment>> {
    fetch_missing_images(app, session_id, images).await?;
    images.iter().map(|i| Ok(ImageAttachment { name: i.name.clone(), data_url: to_data_url(&i.data_url)? })).collect()
}

fn decode(data_url: &str, image_only: bool) -> Result<Vec<u8>> {
    if data_url.starts_with("upload:") {
        let (mime, hash) = parse_ref(data_url).context("invalid attachment reference")?;
        ensure!(!(image_only || mime.starts_with("image/")) || IMAGE_TYPES.contains(&mime), "use PNG, JPEG, WebP or GIF images");
        let bytes = std::fs::read(uploads_dir()?.join(hash)).context("the uploaded file is no longer available; attach it again")?;
        ensure!(bytes.len() <= MAX_BYTES, "each attachment must be 5 MB or smaller");
        return Ok(bytes);
    }
    let (header, data) = data_url
        .split_once(',')
        .context("invalid attachment data URL")?;
    ensure!(
        header.starts_with("data:") && header.ends_with(";base64"),
        "attachments must be base64 data URLs"
    );
    if image_only || is_image(data_url) {
        ensure!(
            matches!(
                header,
                "data:image/png;base64"
                    | "data:image/jpeg;base64"
                    | "data:image/webp;base64"
                    | "data:image/gif;base64"
            ),
            "use PNG, JPEG, WebP or GIF images"
        );
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .context("invalid attachment base64")?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "each attachment must be 5 MB or smaller"
    );
    ensure!(!(image_only || is_image(data_url)) || !bytes.is_empty(), "the image is empty");
    Ok(bytes)
}

/// The bytes and MIME type of a validated image data URL.
pub fn decode_image(data_url: &str) -> Result<(Vec<u8>, String)> {
    let bytes = decode(data_url, true)?;
    let mime = match parse_ref(data_url) {
        Some((mime, _)) => mime.to_string(),
        None => data_url[5..].split(';').next().unwrap_or("image/png").to_string(),
    };
    Ok((bytes, mime))
}

pub fn validate(files: &[FileAttachment]) -> Result<()> {
    ensure!(
        files.iter().map(|f| encoded_len(&f.data_url)).sum::<usize>() <= 28_000_000,
        "attachments must total 20 MB or less"
    );
    for file in files {
        decode(&file.data_url, false)?;
    }
    Ok(())
}

pub fn validate_images(images: &[ImageAttachment]) -> Result<()> {
    ensure!(
        images.iter().map(|f| encoded_len(&f.data_url)).sum::<usize>() <= 28_000_000,
        "images must total 20 MB or less"
    );
    for image in images {
        decode(&image.data_url, true)?;
    }
    Ok(())
}

/// The private attachment folder of a session, in the data folder.
pub fn dir(app: &crate::App, id: &str) -> PathBuf {
    app.paths.data_dir.join("attachments").join(id)
}

/// Keep files outside the checkout; numbered names avoid collisions and path traversal.
pub async fn prepare(
    app: &crate::App,
    id: &str,
    files: &[FileAttachment],
) -> Result<(Vec<ImageAttachment>, Option<std::path::PathBuf>, String)> {
    fetch_missing(app, id, files).await?;
    let mut images = Vec::new();
    let dir = dir(app, id);
    let mut paths = Vec::new();
    for file in files {
        if is_image(&file.data_url) {
            // Kept as a reference in records and events; `for_harness` turns it into a data URL.
            images.push(ImageAttachment {
                name: file.name.clone(),
                data_url: file.data_url.clone(),
            });
        } else {
            crate::files::create_dir_all_mode(&dir, Some(0o700)).await?;
            let name: String = file
                .name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .take(120)
                .collect();
            // A content prefix keeps names stable across retries and distinct across later messages.
            let bytes = decode(&file.data_url, false)?;
            let path = dir.join(format!(
                "{}-{}",
                &hex::encode(Sha256::digest(&bytes))[..8],
                if name.is_empty() { "attachment" } else { &name }
            ));
            crate::files::write_mode(&path, &bytes, Some(0o600)).await?;
            paths.push(format!("- {}", path.display()));
        }
    }
    let has_files = !paths.is_empty();
    let instruction = if has_files {
        format!(
            "\n\nAttached files (read these local copies as reference material):\n{}",
            paths.join("\n")
        )
    } else {
        String::new()
    };
    Ok((images, has_files.then_some(dir), instruction))
}

/// Attachments of a message to a running session: images become native image input, other files are
/// copied to the session's private attachment folder and listed after the text.
pub async fn for_message(app: &crate::App, id: &str, text: &str, files: &[FileAttachment]) -> Result<(String, Vec<ImageAttachment>)> {
    if files.is_empty() {
        return Ok((text.to_string(), Vec::new()));
    }
    fetch_missing(app, id, files).await?;
    validate(files)?;
    ensure!(app.sessions.get(id).is_some(), "no such session");
    let (images, _, instruction) = prepare(app, id, files).await?;
    Ok((format!("{text}{instruction}").trim_start().to_string(), images))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleanup_finds_the_hashes_of_references_in_json() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let json = format!(r#"{{"images":[{{"data_url":"upload:image/png;{a}"}}],"text":"see upload:text/plain;{b} and upload:bad"}}"#);
        let mut out = std::collections::HashSet::new();
        super::referenced_hashes(&json, &mut out);
        assert_eq!(out, [a, b].into_iter().collect());
    }
}

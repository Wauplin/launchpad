//! Local uploads: bounded data URLs, native image input, and private reference files.
//!
//! Browsers upload each file once (`POST /api/uploads`) and keep a short reference instead of the
//! bytes: `upload:<mime>;<sha256>`. Drafts, schedules, crons and launch specs store the reference;
//! the bytes live once in `<data_dir>/uploads/<sha256>`. Data URLs remain accepted everywhere.
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

/// Deletes uploads older than two weeks that no draft, scheduled launch or cron still references.
/// Never-started sessions still need their launch images when resumed. Other sessions sent images to the harness; files were copied at launch.
pub async fn collect_garbage(app: &crate::App) -> Result<()> {
    let Ok(dir) = uploads_dir() else { return Ok(()) };
    let specs: Vec<String> = app.state_db.call(|c| {
        let mut out = Vec::new();
        for table in ["launch_draft", "scheduled_launch", "cron_agent"] {
            let column = if table == "launch_draft" { "spec_json" } else { "json" };
            let Ok(mut stmt) = c.prepare(&format!("SELECT {column} FROM {table}")) else { continue };
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            out.extend(rows.filter_map(|r| r.ok()));
        }
        let mut stmt = c.prepare("SELECT launch_spec_json FROM session WHERE harness_session_id IS NULL")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        out.extend(rows.filter_map(|r| r.ok()));
        Ok(out)
    }).await?;
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return Ok(()) };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(14 * 24 * 3600);
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let old = entry.metadata().await.ok().and_then(|m| m.modified().ok()).is_some_and(|t| t < cutoff);
        if old && !specs.iter().any(|s| s.contains(&name)) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
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

/// Replaces stored references in message images by data URLs (event logs keep the bytes).
pub fn resolve_images(images: &mut [ImageAttachment]) -> Result<()> {
    for image in images {
        image.data_url = to_data_url(&image.data_url)?;
    }
    Ok(())
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
    let mime = data_url[5..].split(';').next().unwrap_or("image/png").to_string();
    Ok((bytes, mime))
}

pub fn validate(files: &[FileAttachment]) -> Result<()> {
    ensure!(
        files.len() <= 8 && files.iter().map(|f| encoded_len(&f.data_url)).sum::<usize>() <= 28_000_000,
        "attach at most 8 files, totaling 20 MB"
    );
    for file in files {
        decode(&file.data_url, false)?;
    }
    Ok(())
}

pub fn validate_images(images: &[ImageAttachment]) -> Result<()> {
    ensure!(
        images.len() <= 8 && images.iter().map(|f| encoded_len(&f.data_url)).sum::<usize>() <= 28_000_000,
        "attach at most 8 images, totaling 20 MB"
    );
    for image in images {
        decode(&image.data_url, true)?;
    }
    Ok(())
}

/// The private attachment folder of a session: in the data folder here, or in the helper folder of its host.
pub fn dir(app: &crate::App, host: Option<&str>, id: &str) -> PathBuf {
    match host {
        Some(h) => {
            let home = app.hosts.home(h).unwrap_or_else(|| "/nonexistent".into());
            PathBuf::from(crate::hosts::loc::join(Some(h), &format!("{}/.local/share/launchpad-host/{}/attachments/{id}", home.trim_end_matches('/'), app.paths.app_env.name())))
        }
        None => app.paths.data_dir.join("attachments").join(id),
    }
}

/// Keep files outside the checkout; numbered names avoid collisions and path traversal. Files of a session
/// on an SSH host go to a private folder on that host.
pub async fn prepare(
    app: &crate::App,
    host: Option<&str>,
    id: &str,
    files: &[FileAttachment],
) -> Result<(Vec<ImageAttachment>, Option<std::path::PathBuf>, String)> {
    let mut images = Vec::new();
    if let Some(h) = host {
        ensure!(files.iter().all(|f| is_image(&f.data_url)) || app.hosts.home(h).is_some(), "{h} is not connected: attached files cannot be copied there now");
    }
    let dir = dir(app, host, id);
    let mut paths = Vec::new();
    for file in files {
        if is_image(&file.data_url) {
            images.push(ImageAttachment {
                name: file.name.clone(),
                data_url: to_data_url(&file.data_url)?,
            });
        } else {
            crate::hostfs::create_dir_all_mode(&dir, Some(0o700)).await?;
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
            crate::hostfs::write_mode(&path, &bytes, Some(0o600)).await?;
            paths.push(format!("- {}", crate::hosts::loc::native(&path.to_string_lossy())));
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
    validate(files)?;
    let s = app.sessions.get(id).context("no such session")?;
    let (images, _, instruction) = prepare(app, crate::hosts::loc::host_of(&s.cwd), id, files).await?;
    Ok((format!("{text}{instruction}").trim_start().to_string(), images))
}

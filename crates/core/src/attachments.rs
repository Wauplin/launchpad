//! Local uploads: bounded data URLs, native image input, and private reference files.
use crate::model::{FileAttachment, ImageAttachment};
use anyhow::{Context, Result, ensure};
use base64::Engine;

pub fn is_image(data_url: &str) -> bool {
    data_url.starts_with("data:image/")
}

fn decode(data_url: &str, image_only: bool) -> Result<Vec<u8>> {
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
        bytes.len() <= 5 * 1024 * 1024,
        "each attachment must be 5 MB or smaller"
    );
    ensure!(!(image_only || is_image(data_url)) || !bytes.is_empty(), "the image is empty");
    Ok(bytes)
}

pub fn validate(files: &[FileAttachment]) -> Result<()> {
    ensure!(
        files.len() <= 8 && files.iter().map(|f| f.data_url.len()).sum::<usize>() <= 28_000_000,
        "attach at most 8 files, totaling 20 MB"
    );
    for file in files {
        decode(&file.data_url, false)?;
    }
    Ok(())
}

pub fn validate_images(images: &[ImageAttachment]) -> Result<()> {
    ensure!(
        images.len() <= 8 && images.iter().map(|f| f.data_url.len()).sum::<usize>() <= 28_000_000,
        "attach at most 8 images, totaling 20 MB"
    );
    for image in images {
        decode(&image.data_url, true)?;
    }
    Ok(())
}

/// Keep files outside the checkout; numbered names avoid collisions and path traversal.
pub async fn prepare(
    app: &crate::App,
    id: &str,
    files: &[FileAttachment],
) -> Result<(Vec<ImageAttachment>, Option<std::path::PathBuf>, String)> {
    let mut images = Vec::new();
    let dir = app.paths.data_dir.join("attachments").join(id);
    let mut paths = Vec::new();
    for (i, file) in files.iter().enumerate() {
        if is_image(&file.data_url) {
            images.push(ImageAttachment {
                name: file.name.clone(),
                data_url: file.data_url.clone(),
            });
        } else {
            tokio::fs::create_dir_all(&dir).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).await?;
            }
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
            let path = dir.join(format!(
                "{i}-{}",
                if name.is_empty() { "attachment" } else { &name }
            ));
            tokio::fs::write(&path, decode(&file.data_url, false)?).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
            }
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

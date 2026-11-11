//! Async file operations on project, session and worktree paths.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    /// `file`, `dir`, `symlink` or `other`.
    pub kind: String,
    pub len: u64,
    pub modified_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub kind: String,
}

fn kind(t: std::fs::FileType) -> &'static str {
    if t.is_symlink() { "symlink" } else if t.is_dir() { "dir" } else if t.is_file() { "file" } else { "other" }
}

impl Meta {
    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }
    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }
}

pub async fn canonicalize(p: impl AsRef<Path>) -> Result<PathBuf> {
    tokio::fs::canonicalize(p).await
}

async fn meta(p: &Path, follow: bool) -> Result<Meta> {
    let m = if follow { tokio::fs::metadata(p).await? } else { tokio::fs::symlink_metadata(p).await? };
    let modified_ms = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
    Ok(Meta { kind: kind(m.file_type()).into(), len: m.len(), modified_ms })
}

pub async fn metadata(p: impl AsRef<Path>) -> Result<Meta> {
    meta(p.as_ref(), true).await
}

pub async fn symlink_metadata(p: impl AsRef<Path>) -> Result<Meta> {
    meta(p.as_ref(), false).await
}

pub async fn is_dir(p: impl AsRef<Path>) -> bool {
    metadata(p).await.is_ok_and(|m| m.is_dir())
}

pub async fn is_file(p: impl AsRef<Path>) -> bool {
    metadata(p).await.is_ok_and(|m| m.is_file())
}

pub async fn exists(p: impl AsRef<Path>) -> bool {
    metadata(p).await.is_ok()
}

/// The whole file, at most `max` bytes (an error above).
pub async fn read_limited(p: impl AsRef<Path>, max: u64) -> Result<Vec<u8>> {
    let p = p.as_ref();
    let len = tokio::fs::metadata(p).await?.len();
    if len > max {
        return Err(Error::other(format!("{} is larger than {max} bytes", p.display())));
    }
    tokio::fs::read(p).await
}

pub async fn read(p: impl AsRef<Path>) -> Result<Vec<u8>> {
    read_limited(p, 64 << 20).await
}

pub async fn read_to_string(p: impl AsRef<Path>) -> Result<String> {
    String::from_utf8(read(p).await?).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

/// Writes a file, with Unix permissions when given.
pub async fn write_mode(p: impl AsRef<Path>, data: &[u8], mode: Option<u32>) -> Result<()> {
    let p = p.as_ref();
    tokio::fs::write(p, data).await?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).await?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

pub async fn write(p: impl AsRef<Path>, data: &[u8]) -> Result<()> {
    write_mode(p, data, None).await
}

pub async fn create_dir_all_mode(p: impl AsRef<Path>, mode: Option<u32>) -> Result<()> {
    let p = p.as_ref();
    tokio::fs::create_dir_all(p).await?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).await?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

pub async fn create_dir_all(p: impl AsRef<Path>) -> Result<()> {
    create_dir_all_mode(p, None).await
}

pub async fn remove_file(p: impl AsRef<Path>) -> Result<()> {
    tokio::fs::remove_file(p).await
}

pub async fn remove_dir_all(p: impl AsRef<Path>) -> Result<()> {
    tokio::fs::remove_dir_all(p).await
}

pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    tokio::fs::rename(from, to).await
}

pub async fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    tokio::fs::copy(from, to).await.map(|_| ())
}

/// Names and kinds of the entries of a folder.
pub async fn read_dir(p: impl AsRef<Path>) -> Result<Vec<DirEntry>> {
    let mut out = vec![];
    let mut d = tokio::fs::read_dir(p).await?;
    while let Some(e) = d.next_entry().await? {
        let k = e.file_type().await.map(kind).unwrap_or("other");
        out.push(DirEntry { name: e.file_name().to_string_lossy().into_owned(), kind: k.into() });
    }
    Ok(out)
}

/// Files of a folder that is not a Git repository (code viewer).
pub async fn walk(root: impl AsRef<Path>) -> Result<Vec<String>> { walk_paths(root, false).await }

/// Include folders (with trailing slash), including empty folders, respecting ignore files.
pub async fn walk_paths(root: impl AsRef<Path>, directories: bool) -> Result<Vec<String>> {
    let root = root.as_ref().to_path_buf();
    Ok(tokio::task::spawn_blocking(move || crate::code::walk_paths(&root, directories)).await?)
}

/// Disk usage of folders, in bytes.
pub async fn disk_usage(paths: &[String]) -> Vec<u64> {
    fn walk(p: &Path) -> u64 {
        let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
        if meta.is_dir() { std::fs::read_dir(p).map(|d| d.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0) } else { meta.len() }
    }
    let paths = paths.to_vec();
    tokio::task::spawn_blocking(move || paths.iter().map(|p| walk(Path::new(p))).collect()).await.unwrap_or_default()
}

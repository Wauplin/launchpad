//! File operations on this computer or on an SSH host, chosen by the path (`devbox:/home/…` goes to the
//! host's helper). Paths returned for a host are host locations again.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use crate::hosts::loc;
use crate::hosts::proto::{B64, DirEntry, ErrorInfo, FsOp, Meta, Op};

fn to_io(e: ErrorInfo) -> Error {
    let kind = match e.kind.as_str() {
        "not_found" => ErrorKind::NotFound,
        "exists" => ErrorKind::AlreadyExists,
        _ => ErrorKind::Other,
    };
    Error::new(kind, e.message)
}

async fn remote(host: &str, op: FsOp) -> Result<serde_json::Value> {
    let conn = crate::hosts::conn(host).map_err(|e| Error::other(format!("{e:#}")))?;
    conn.request_info(Op::Fs(op)).await.map_err(to_io)
}

fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::other(e.to_string()))
}

fn split(p: &Path) -> Option<(String, String)> {
    loc::split_path(p).map(|(h, n)| (h.to_string(), n.to_string()))
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
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => {
            let native: String = parse(remote(&host, FsOp::Canonicalize { path }).await?)?;
            Ok(PathBuf::from(loc::join(Some(&host), &native)))
        }
        None => tokio::fs::canonicalize(p).await,
    }
}

async fn meta(p: &Path, follow: bool) -> Result<Meta> {
    match split(p) {
        Some((host, path)) => parse(remote(&host, FsOp::Metadata { path, follow }).await?),
        None => {
            let m = if follow { tokio::fs::metadata(p).await? } else { tokio::fs::symlink_metadata(p).await? };
            let modified_ms = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
            Ok(Meta { kind: kind(m.file_type()).into(), len: m.len(), modified_ms })
        }
    }
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
    match split(p) {
        Some((host, path)) => Ok(parse::<B64>(remote(&host, FsOp::Read { path, max }).await?)?.0),
        None => {
            let len = tokio::fs::metadata(p).await?.len();
            if len > max {
                return Err(Error::other(format!("{} is larger than {max} bytes", p.display())));
            }
            tokio::fs::read(p).await
        }
    }
}

pub async fn read(p: impl AsRef<Path>) -> Result<Vec<u8>> {
    read_limited(p, 64 << 20).await
}

pub async fn read_to_string(p: impl AsRef<Path>) -> Result<String> {
    String::from_utf8(read(p).await?).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

/// The last `len` bytes.
pub async fn tail(p: impl AsRef<Path>, len: u64) -> Result<Vec<u8>> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => Ok(parse::<B64>(remote(&host, FsOp::Tail { path, len }).await?)?.0),
        None => {
            let p = p.to_path_buf();
            tokio::task::spawn_blocking(move || {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::File::open(&p)?;
                let size = f.metadata()?.len();
                f.seek(SeekFrom::Start(size.saturating_sub(len)))?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                Ok(buf)
            })
            .await?
        }
    }
}

/// Writes a file (through a temporary file on a host), with Unix permissions when given.
pub async fn write_mode(p: impl AsRef<Path>, data: &[u8], mode: Option<u32>) -> Result<()> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => remote(&host, FsOp::Write { path, data: B64(data.to_vec()), mode }).await.map(|_| ()),
        None => {
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
    }
}

pub async fn write(p: impl AsRef<Path>, data: &[u8]) -> Result<()> {
    write_mode(p, data, None).await
}

pub async fn create_dir_all_mode(p: impl AsRef<Path>, mode: Option<u32>) -> Result<()> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => remote(&host, FsOp::CreateDirAll { path, mode }).await.map(|_| ()),
        None => {
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
    }
}

pub async fn create_dir_all(p: impl AsRef<Path>) -> Result<()> {
    create_dir_all_mode(p, None).await
}

pub async fn remove_file(p: impl AsRef<Path>) -> Result<()> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => remote(&host, FsOp::Remove { path, recursive: false }).await.map(|_| ()),
        None => tokio::fs::remove_file(p).await,
    }
}

pub async fn remove_dir_all(p: impl AsRef<Path>) -> Result<()> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => remote(&host, FsOp::Remove { path, recursive: true }).await.map(|_| ()),
        None => tokio::fs::remove_dir_all(p).await,
    }
}

fn same_host(a: &Path, b: &Path) -> Result<()> {
    if loc::host_of_path(a) != loc::host_of_path(b) {
        return Err(Error::other("source and destination are on different machines"));
    }
    Ok(())
}

pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    same_host(from, to)?;
    match split(from) {
        Some((host, path)) => remote(&host, FsOp::Rename { from: path, to: loc::native(&to.to_string_lossy()).into() }).await.map(|_| ()),
        None => tokio::fs::rename(from, to).await,
    }
}

pub async fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    same_host(from, to)?;
    match split(from) {
        Some((host, path)) => remote(&host, FsOp::Copy { from: path, to: loc::native(&to.to_string_lossy()).into() }).await.map(|_| ()),
        None => tokio::fs::copy(from, to).await.map(|_| ()),
    }
}

/// Names and kinds of the entries of a folder.
pub async fn read_dir(p: impl AsRef<Path>) -> Result<Vec<DirEntry>> {
    let p = p.as_ref();
    match split(p) {
        Some((host, path)) => parse(remote(&host, FsOp::ReadDir { path }).await?),
        None => {
            let mut out = vec![];
            let mut d = tokio::fs::read_dir(p).await?;
            while let Some(e) = d.next_entry().await? {
                let k = e.file_type().await.map(kind).unwrap_or("other");
                out.push(DirEntry { name: e.file_name().to_string_lossy().into_owned(), kind: k.into() });
            }
            Ok(out)
        }
    }
}

/// Files of a folder that is not a Git repository (code viewer).
pub async fn walk(root: impl AsRef<Path>) -> Result<Vec<String>> {
    let root = root.as_ref();
    match split(root) {
        Some((host, path)) => parse(remote(&host, FsOp::Walk { root: path }).await?),
        None => {
            let root = root.to_path_buf();
            Ok(tokio::task::spawn_blocking(move || crate::code::walk(&root)).await?)
        }
    }
}

/// Disk usage of folders, in bytes, grouped by machine.
pub async fn disk_usage(paths: &[String]) -> Vec<u64> {
    let mut out = vec![0; paths.len()];
    let mut groups: std::collections::BTreeMap<Option<String>, Vec<usize>> = Default::default();
    for (i, p) in paths.iter().enumerate() {
        groups.entry(loc::host_of(p).map(str::to_string)).or_default().push(i);
    }
    for (host, idx) in groups {
        let natives: Vec<String> = idx.iter().map(|&i| loc::native(&paths[i]).to_string()).collect();
        let sizes: Vec<u64> = match host {
            Some(h) => match remote(&h, FsOp::DiskUsage { paths: natives }).await.and_then(parse) {
                Ok(s) => s,
                Err(_) => continue,
            },
            None => tokio::task::spawn_blocking(move || match crate::hosts::helper::fs_op(FsOp::DiskUsage { paths: natives }) {
                Ok(v) => serde_json::from_value(v).unwrap_or_default(),
                Err(_) => vec![],
            })
            .await
            .unwrap_or_default(),
        };
        for (&i, s) in idx.iter().zip(sizes) {
            out[i] = s;
        }
    }
    out
}

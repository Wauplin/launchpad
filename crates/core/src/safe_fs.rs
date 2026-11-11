//! Files in folders an agent can write, read and written by the daemon outside the jail: plain, temporary and
//! asset folders, native traces and harness setup files in a session's harness home, its attachment folder and
//! its `/tmp`.
//!
//! An agent can plant a symbolic link anywhere in those folders (a file, or a folder on the way to it), so the
//! daemon would read or write a file of the machine through it. Every call here takes a trusted `base` (opened
//! normally: one of the data folder's own folders or a session's harness home, never agent-writable themselves)
//! and a relative path below it whose every component is
//! opened with `O_NOFOLLOW` from the folder before it (`openat`), so a link is refused wherever it is, with no
//! window between a check and the use. Files must be regular files (no FIFO, device or link); writes go to a new
//! file renamed over the old one, so a link or hard link in its place is replaced, never written through.
//! [`anchor`] picks the base for a path below the data folder.

use std::ffi::OsStr;
use std::io::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};

/// The components of a relative path: no root, no `..`, no empty name (`.` is skipped).
pub fn components(rel: &Path) -> Result<Vec<&OsStr>> {
    let mut out = vec![];
    for c in rel.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return Err(Error::new(ErrorKind::InvalidInput, format!("{} is not a relative path inside its folder", rel.display()))),
        }
    }
    Ok(out)
}

/// The trusted base and the part below it of `path`. When `path` lies below `trusted` (the data folder, as
/// given or resolved: it may itself be reached through a link, such as a linked home folder), the base is the
/// data folder's own folder on the way (`worktrees`, `sessions`, `plain`…: the daemon's, never agent-writable
/// themselves, so the owner may move one to another disk with a link) and nothing below it is followed. Else
/// its parent folder (followed) and its name (never followed).
pub fn anchor(trusted: &Path, path: &Path) -> (PathBuf, PathBuf) {
    let below = |root: &Path| -> Option<(PathBuf, PathBuf)> {
        let parts = components(path.strip_prefix(root).ok()?).ok()?;
        let (first, rest) = parts.split_first()?;
        Some(if rest.is_empty() { (root.to_path_buf(), PathBuf::from(first)) } else { (root.join(first), rest.iter().collect()) })
    };
    match below(trusted).or_else(|| below(&std::fs::canonicalize(trusted).ok()?)) {
        Some(found) => found,
        None => match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), PathBuf::from(name)),
            _ => (path.to_path_buf(), PathBuf::new()),
        },
    }
}

/// Opens a regular file at `base`/`rel` for reading.
pub fn open(base: &Path, rel: &Path) -> Result<std::fs::File> {
    imp::open(base, rel)
}

/// Reads a regular file of at most `max` bytes.
pub fn read(base: &Path, rel: &Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut f = open(base, rel)?;
    let len = f.metadata()?.len();
    if len > max {
        return Err(Error::other(format!("{} is larger than {max} bytes", rel.display())));
    }
    let mut data = Vec::with_capacity(len as usize);
    f.by_ref().take(max).read_to_end(&mut data)?;
    Ok(data)
}

/// A text file (at most 256 MiB); None when it is missing, not a regular file, reached through a link or not
/// UTF-8.
pub fn read_text(base: &Path, rel: &Path) -> Option<String> {
    String::from_utf8(read(base, rel, 256 << 20).ok()?).ok()
}

/// The last `len` bytes of a regular file.
pub fn tail(base: &Path, rel: &Path, len: u64) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = open(base, rel)?;
    let size = f.metadata()?.len();
    f.seek(SeekFrom::Start(size.saturating_sub(len)))?;
    let mut data = vec![];
    f.take(len).read_to_end(&mut data)?;
    Ok(data)
}

/// Writes a file (its folders created as needed), with Unix permissions when given (else the umask's).
pub fn write(base: &Path, rel: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
    imp::write(base, rel, data, mode)
}

/// Removes a file (a link planted in its place is removed itself).
pub fn remove_file(base: &Path, rel: &Path) -> Result<()> {
    imp::remove_file(base, rel)
}

/// Whether `base`/`rel` is a folder reached without a link (the root of a folder the daemon walks).
pub fn is_dir(base: &Path, rel: &Path) -> bool {
    imp::open_dir(base, rel, false).is_ok()
}

/// Creates the folder `base`/`rel` and its parents, never through a link.
pub fn create_dir_all(base: &Path, rel: &Path) -> Result<()> {
    imp::open_dir(base, rel, true).map(drop)
}

/// Async versions, on the blocking pool.
pub async fn read_async(base: PathBuf, rel: PathBuf, max: u64) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || read(&base, &rel, max)).await.map_err(Error::other)?
}

pub async fn write_async(base: PathBuf, rel: PathBuf, data: Vec<u8>, mode: Option<u32>) -> Result<()> {
    tokio::task::spawn_blocking(move || write(&base, &rel, &data, mode)).await.map_err(Error::other)?
}

fn refused(rel: &Path, e: Error) -> Error {
    if e.raw_os_error().is_some_and(|c| c == libc_eloop() || c == libc_enotdir()) {
        Error::new(ErrorKind::InvalidInput, format!("{} goes through a symbolic link or a file; refused", rel.display()))
    } else {
        e
    }
}

#[cfg(unix)]
fn libc_eloop() -> i32 {
    libc::ELOOP
}
#[cfg(unix)]
fn libc_enotdir() -> i32 {
    libc::ENOTDIR
}
#[cfg(not(unix))]
fn libc_eloop() -> i32 {
    -1
}
#[cfg(not(unix))]
fn libc_enotdir() -> i32 {
    -1
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    fn cstr(name: &OsStr) -> Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| Error::new(ErrorKind::InvalidInput, "a file name holds a NUL byte"))
    }

    fn openat(dir: &OwnedFd, name: &OsStr, flags: i32, mode: libc::mode_t) -> Result<OwnedFd> {
        let name = cstr(name)?;
        // SAFETY: a valid directory descriptor and a NUL-terminated name; the result is checked.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC | libc::O_NOFOLLOW, mode as libc::c_uint) };
        if fd < 0 {
            return Err(Error::last_os_error());
        }
        // SAFETY: a fresh descriptor this function owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// The trusted base, opened normally (created first when asked: a fresh runner has no `plain` or `sessions`).
    fn base_dir(base: &Path, create: bool) -> Result<OwnedFd> {
        use std::os::unix::fs::OpenOptionsExt;
        if create {
            std::fs::create_dir_all(base)?;
        }
        let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC).open(base)?;
        Ok(f.into())
    }

    fn sub_dir(dir: &OwnedFd, name: &OsStr, create: bool) -> Result<OwnedFd> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY;
        match openat(dir, name, flags, 0) {
            Err(e) if create && e.kind() == ErrorKind::NotFound => {
                let c = cstr(name)?;
                // SAFETY: as in `openat`.
                if unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o777) } != 0 {
                    let e = Error::last_os_error();
                    if e.kind() != ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                }
                openat(dir, name, flags, 0)
            }
            r => r,
        }
    }

    pub fn open_dir(base: &Path, rel: &Path, create: bool) -> Result<OwnedFd> {
        let mut dir = base_dir(base, create)?;
        for c in components(rel)? {
            dir = sub_dir(&dir, c, create).map_err(|e| refused(rel, e))?;
        }
        Ok(dir)
    }

    /// The folder of a file and its name.
    fn parent(base: &Path, rel: &Path, create: bool) -> Result<(OwnedFd, OsStringName)> {
        let parts = components(rel)?;
        let (name, dirs) = parts.split_last().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?;
        let mut dir = base_dir(base, create)?;
        for c in dirs {
            dir = sub_dir(&dir, c, create).map_err(|e| refused(rel, e))?;
        }
        Ok((dir, name.to_os_string()))
    }
    type OsStringName = std::ffi::OsString;

    pub fn open(base: &Path, rel: &Path) -> Result<std::fs::File> {
        let (dir, name) = parent(base, rel, false)?;
        // Never blocks on a FIFO; the type is checked on the open file.
        let fd = openat(&dir, &name, libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY, 0).map_err(|e| refused(rel, e))?;
        let file = std::fs::File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(Error::new(ErrorKind::InvalidInput, format!("{} is not a regular file", rel.display())));
        }
        Ok(file)
    }

    pub fn write(base: &Path, rel: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
        let (dir, name) = parent(base, rel, true)?;
        let tmp = std::ffi::OsString::from(format!(".lp-write-{}", uuid::Uuid::new_v4().simple()));
        let fd = openat(&dir, &tmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, mode.unwrap_or(0o666) as libc::mode_t)?;
        let mut file = std::fs::File::from(fd);
        let written = (|| {
            file.write_all(data)?;
            if let Some(mode) = mode {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            }
            let (from, to) = (cstr(&tmp)?, cstr(&name)?);
            // SAFETY: as in `openat`; replaces the name itself (a link there is replaced, not followed).
            if unsafe { libc::renameat(dir.as_raw_fd(), from.as_ptr(), dir.as_raw_fd(), to.as_ptr()) } != 0 {
                return Err(Error::last_os_error());
            }
            Ok(())
        })();
        if written.is_err() {
            let c = cstr(&tmp)?;
            // SAFETY: as in `openat`.
            unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) };
        }
        written
    }

    pub fn remove_file(base: &Path, rel: &Path) -> Result<()> {
        let (dir, name) = parent(base, rel, false)?;
        let c = cstr(&name)?;
        // SAFETY: as in `openat`.
        if unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) } != 0 {
            return Err(Error::last_os_error());
        }
        Ok(())
    }
}

/// Without `openat`: a check of each component before the use (agents run only on Linux).
#[cfg(not(unix))]
mod imp {
    use super::*;

    fn checked(base: &Path, rel: &Path) -> Result<PathBuf> {
        let mut path = base.to_path_buf();
        for c in components(rel)? {
            path.push(c);
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(Error::new(ErrorKind::InvalidInput, format!("{} goes through a symbolic link; refused", rel.display())));
            }
        }
        Ok(path)
    }

    pub fn open_dir(base: &Path, rel: &Path, create: bool) -> Result<()> {
        let path = checked(base, rel)?;
        if create {
            std::fs::create_dir_all(&path)?;
            checked(base, rel)?;
        }
        if !std::fs::metadata(&path)?.is_dir() {
            return Err(Error::new(ErrorKind::InvalidInput, "not a folder"));
        }
        Ok(())
    }

    pub fn open(base: &Path, rel: &Path) -> Result<std::fs::File> {
        let file = std::fs::File::open(checked(base, rel)?)?;
        if !file.metadata()?.is_file() {
            return Err(Error::new(ErrorKind::InvalidInput, format!("{} is not a regular file", rel.display())));
        }
        Ok(file)
    }

    pub fn write(base: &Path, rel: &Path, data: &[u8], _mode: Option<u32>) -> Result<()> {
        let path = checked(base, rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(checked(base, rel)?, data)
    }

    pub fn remove_file(base: &Path, rel: &Path) -> Result<()> {
        std::fs::remove_file(checked(base, rel)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_only() {
        assert!(components(Path::new("a/./b")).is_ok_and(|c| c == ["a", "b"]));
        assert!(components(Path::new("../a")).is_err());
        assert!(components(Path::new("a/../../b")).is_err());
        assert!(components(Path::new("/etc/passwd")).is_err());
        let (base, rel) = anchor(Path::new("/data"), Path::new("/data/plain/x/y"));
        assert_eq!((base.as_path(), rel.as_path()), (Path::new("/data/plain"), Path::new("x/y")));
        assert_eq!(anchor(Path::new("/data"), Path::new("/data/tmp")), (PathBuf::from("/data"), PathBuf::from("tmp")));
        assert_eq!(anchor(Path::new("/data"), Path::new("/data/a/../../etc")).0, Path::new("/data/a/../.."));
        assert_eq!(anchor(Path::new("/data"), Path::new("/elsewhere/x")), (PathBuf::from("/elsewhere"), PathBuf::from("x")));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let root = std::env::temp_dir().join(format!("launchpad-safe-fs-{}", uuid::Uuid::new_v4()));
        let (base, outside) = (root.join("base"), root.join("outside"));
        std::fs::create_dir_all(base.join("dir")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, base.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), base.join("dir/file")).unwrap();
        // A linked folder on the way, or the file itself.
        assert!(read(&base, Path::new("link/secret"), 100).is_err());
        assert!(read(&base, Path::new("dir/file"), 100).is_err());
        assert!(write(&base, Path::new("link/new"), b"x", None).is_err());
        assert!(!outside.join("new").exists());
        assert!(!is_dir(&base, Path::new("link")));
        // A link in the file's place is replaced, never written through.
        write(&base, Path::new("dir/file"), b"mine", Some(0o600)).unwrap();
        assert_eq!(std::fs::read(outside.join("secret")).unwrap(), b"secret");
        assert_eq!(read(&base, Path::new("dir/file"), 100).unwrap(), b"mine");
        write(&base, Path::new("new/deep/file"), b"ok", None).unwrap();
        assert_eq!(read(&base, Path::new("new/deep/file"), 100).unwrap(), b"ok");
        assert!(read(&base, Path::new("new/deep/file"), 1).is_err());
        // The data folder reached through a link (a linked home folder), named either way: followed, nothing below.
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&base, &alias).unwrap();
        let (b, r) = anchor(&alias, &base.join("new/deep/file"));
        assert_eq!(read(&b, &r, 100).unwrap(), b"ok");
        std::os::unix::fs::symlink(outside.join("secret"), base.join("dir/other")).unwrap();
        let (b, r) = anchor(&alias, &alias.join("dir/other"));
        assert!(read(&b, &r, 100).is_err());
        remove_file(&base, Path::new("link")).unwrap();
        assert!(outside.join("secret").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

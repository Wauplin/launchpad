//! Daemon Git on repositories that agents can write: a runner's repository caches and worktrees, plain and
//! temporary folders (every jail mounts them read-write).
//!
//! An agent controls everything in those repositories: `.git/config` (hooks path, fsmonitor, filter, diff and
//! merge drivers, `url.*.insteadOf`, `http.*.proxy`, includes), `config.worktree`, `hooks/`, `info/attributes`,
//! submodules, and the `.git` file of a worktree. Overriding keys with `-c` cannot close that: filter driver
//! names cannot be enumerated without a race, and `insteadOf` or `http.<url>.*` keys are not overridden by a
//! later value. So the runner's own `git` never reads the repository's configuration at all: every `git` the
//! daemon starts on a runner (`exec::Cmd`, installed through `UserEnv::with_git_guard`) gets
//!
//! - `GIT_DIR` (the repository found from the folder, as Git would) and `GIT_COMMON_DIR` set to a shadow
//!   directory of the daemon (`<data_dir>/git-safe/<hash>`, outside every jail): its own `config` (written here
//!   from what the daemon knows: Home's URL as `origin` for a cache), an empty `hooks/`, and symlinks to the
//!   repository's objects, refs, packed refs, logs, worktrees and `info/exclude` (data, never code). Without
//!   `extensions.worktreeConfig` in that config, `config.worktree` is ignored too;
//! - no system or global config, `core.hooksPath=/dev/null`, no fsmonitor, credential helper, pager, editor or
//!   askpass, no `ext::` transport and no submodule recursion (child `git` processes inherit the `-c` values).
//!
//! The daemon's Home credential goes only to Home's exact URL (`http.<url>.extraHeader`, `repo_cache::auth`).
//! Home's own mirrors are not agent-writable (Home runs no agents) and keep their hooks (`pre-receive`).
//!
//! Git still follows symlinks and `objects/info/alternates` that an agent plants (a symlinked ref or `HEAD`,
//! another repository's objects, a worktree folder swapped for a link), and no check before the command can
//! close that race. So on a runner whose jail is bubblewrap, daemon Git also runs in a minimal sandbox
//! (`Guard::sandbox_args`): the system's programs and libraries and a few files of `/etc` read-only, the
//! shadow directories, and only the agent-writable folders the command works in (the repository caches and
//! worktrees, or the one plain or temporary folder of its working folder), a private `/tmp`; no home folder or
//! other file of the machine or of Launchpad's data. What a link points at outside does not exist there. The
//! network stays shared: pushes and fetches reach this daemon's loopback Git endpoint (and the folder's remote).
//! A runner without bubblewrap fails these commands. On a Local VM or Burst the machine is the boundary (no
//! owner files there): no sandbox.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// Inherited variables that would point Git elsewhere or run a program.
pub const DROPPED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_INDEX_FILE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_SYSTEM",
    "GIT_CEILING_DIRECTORIES",
    "GIT_EXTERNAL_DIFF",
    "GIT_SSH",
    "GIT_PROXY_COMMAND",
    "GIT_TEMPLATE_DIR",
    "GIT_NAMESPACE",
];

/// `-c` values of every daemon `git` on a runner (also inherited by the `git` processes it starts).
pub const CONFIG: &[&str] = &[
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "core.attributesFile=/dev/null",
    "credential.helper=",
    "protocol.ext.allow=never",
    "submodule.recurse=false",
    // Submodule commits still show; their work trees are never examined (that would run Git there).
    "diff.ignoreSubmodules=dirty",
    "status.submoduleSummary=false",
    "fetch.recurseSubmodules=false",
    "push.recurseSubmodules=no",
];

/// Variables of every daemon `git` on a runner.
pub const ENV: &[(&str, &str)] = &[
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_ASKPASS", ""),
    ("SSH_ASKPASS", ""),
    ("GIT_PAGER", "cat"),
    ("PAGER", "cat"),
    ("GIT_EDITOR", "true"),
    ("GIT_SEQUENCE_EDITOR", "true"),
    ("GIT_SSH_COMMAND", "ssh -oBatchMode=yes"),
];

/// What the shadow links to in the repository's common directory: data only (never `config`, `hooks`, `info/attributes`,
/// `modules`, `remotes` or `branches`).
const LINKED: &[&str] = &["objects", "refs", "packed-refs", "logs", "shallow", "worktrees"];

/// What daemon Git sees of the machine in its sandbox, read-only when it exists: programs and libraries, and the
/// files of `/etc` that user lookups, name resolution and TLS read.
pub const SANDBOX_SYSTEM: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/nix/store",
    "/run/systemd/resolve",
    "/run/resolvconf",
    "/etc/passwd",
    "/etc/group",
    "/etc/nsswitch.conf",
    "/etc/hosts",
    "/etc/host.conf",
    "/etc/resolv.conf",
    "/etc/gai.conf",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/localtime",
    "/etc/alternatives",
    "/etc/ssl",
    "/etc/ca-certificates",
    "/etc/pki",
];

/// Installed on a runner's environment: the daemon's `git` runs through `prepare` (and `sandbox_args`).
#[derive(Debug)]
pub struct Guard {
    data_dir: PathBuf,
    /// `data_dir` with symlinks resolved: repositories outside it are refused.
    real_data_dir: PathBuf,
    port: u16,
    /// Run every command in the bubblewrap sandbox (a runner whose jail is bubblewrap).
    sandbox: bool,
}

/// Arguments (before the subcommand) and variables for one `git` command.
#[derive(Debug, Default)]
pub struct Prepared {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// A repository found from a folder: its work tree (None: bare), its Git directory and its common directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub work_tree: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

impl Guard {
    /// `sandbox`: also run each command in bubblewrap (`sandbox_args`).
    pub fn new(data_dir: &Path, port: u16, sandbox: bool) -> Guard {
        Guard { data_dir: data_dir.to_path_buf(), real_data_dir: std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf()), port, sandbox }
    }

    pub fn sandboxed(&self) -> bool {
        self.sandbox
    }

    fn shadow_root(&self) -> PathBuf {
        self.data_dir.join("git-safe")
    }

    /// A folder of the daemon for files daemon Git reads (a snapshot's temporary index): in its sandbox, never
    /// mounted in a jail.
    pub fn scratch_dir(&self) -> PathBuf {
        self.shadow_root().join("tmp")
    }

    /// The bubblewrap arguments (before `--`) of a daemon `git` (`program`, resolved) started in `cwd`.
    pub fn sandbox_args(&self, cwd: Option<&Path>, program: &Path) -> Vec<std::ffi::OsString> {
        let mut rw = vec![self.shadow_root()];
        rw.extend(self.mounts(cwd));
        // A folder missing here would be created in the sandbox only, and lost (`worktree add`).
        for dir in &rw {
            let _ = std::fs::create_dir_all(dir);
        }
        // Both spellings of each folder: Git writes real paths into `.git` files and `gitdir` links.
        if self.real_data_dir != self.data_dir {
            let real: Vec<PathBuf> = rw.iter().filter_map(|p| p.strip_prefix(&self.data_dir).ok()).map(|r| self.real_data_dir.join(r)).collect();
            rw.extend(real);
        }
        let ro: Vec<PathBuf> = self.install_folder(program).into_iter().collect();
        crate::jail::sandbox_args(SANDBOX_SYSTEM, &ro, &rw, cwd)
    }

    /// The installation of a `git` outside the system folders, mounted read-only: the release folder above its
    /// `bin`, or only `bin` when that folder holds the home folder or Launchpad's data.
    fn install_folder(&self, program: &Path) -> Option<PathBuf> {
        let real = std::fs::canonicalize(program).ok()?;
        if SANDBOX_SYSTEM.iter().any(|s| real.starts_with(s)) {
            return None;
        }
        let parent = real.parent()?;
        let release = parent.parent().filter(|_| parent.ends_with("bin"));
        let home = dirs::home_dir();
        let too_wide = |d: &Path| self.real_data_dir.starts_with(d) || self.data_dir.starts_with(d) || home.as_deref().is_some_and(|h| h.starts_with(d));
        Some(release.filter(|r| !too_wide(r)).unwrap_or(parent).to_path_buf()).filter(|d| !too_wide(d))
    }

    /// The agent-writable folders a command in `cwd` may use, each one a folder no agent can replace by a link
    /// (a jail mounts it or a folder above it): the repository caches and the worktrees (a worktree's `.git`
    /// names its cache, a cache lists its worktrees), or the one plain or temporary folder (`<data_dir>/<kind>/<id>`)
    /// that holds `cwd`. Nothing for a folder outside the data folder: the command fails.
    pub fn mounts(&self, cwd: Option<&Path>) -> Vec<PathBuf> {
        let Some(cwd) = cwd else { return vec![] };
        let cwd = normalize(cwd);
        let Some(rest) = cwd.strip_prefix(&self.data_dir).ok().or_else(|| cwd.strip_prefix(&self.real_data_dir).ok()) else { return vec![] };
        let mut parts = rest.components();
        match (parts.next(), parts.next()) {
            (Some(Component::Normal(kind)), _) if kind == "repos" || kind == "worktrees" => vec![self.data_dir.join("repos"), self.data_dir.join("worktrees")],
            (Some(Component::Normal(kind)), Some(Component::Normal(id))) if kind == "plain" || kind == "tmp" => vec![self.data_dir.join(kind).join(id)],
            _ => vec![],
        }
    }

    /// The arguments and variables of a `git` started in `cwd` with `args`. A failure to set the shadow up leaves
    /// `GIT_DIR` pointing at a folder that does not exist: the command fails instead of reading the repository's config.
    pub fn prepare<S: AsRef<std::ffi::OsStr>>(&self, cwd: Option<&Path>, args: &[S]) -> Prepared {
        let mut out = Prepared::default();
        for c in CONFIG {
            out.args.extend(["-c".to_string(), c.to_string()]);
        }
        out.env.extend(ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let Some(cwd) = cwd else { return out };
        match find(cwd, &self.data_dir) {
            Some(found) => {
                let shadow = match self.shadow(&found) {
                    Ok(dir) => dir,
                    Err(e) => {
                        tracing::warn!("git: could not prepare the safe config for {}: {e:#}", found.git_dir.display());
                        self.shadow_root().join("unavailable")
                    }
                };
                out.env.push(("GIT_DIR".into(), found.git_dir.to_string_lossy().into_owned()));
                out.env.push(("GIT_COMMON_DIR".into(), shadow.to_string_lossy().into_owned()));
                if let Some(w) = &found.work_tree {
                    out.env.push(("GIT_WORK_TREE".into(), w.to_string_lossy().into_owned()));
                }
            }
            // No repository yet (`git init`): never look further up.
            None if creates_repository(args) => {
                if let Some(parent) = cwd.parent() {
                    out.env.push(("GIT_CEILING_DIRECTORIES".into(), parent.to_string_lossy().into_owned()));
                }
            }
            // Any other command fails like Git without a repository would: an agent that swaps `.git` in between
            // must never get its config read by the `git` that starts next.
            None => out.env.push(("GIT_DIR".into(), self.shadow_root().join("unavailable").to_string_lossy().into_owned())),
        }
        out
    }

    /// Creates or refreshes the shadow common directory of a repository.
    fn shadow(&self, found: &Found) -> std::io::Result<PathBuf> {
        // A `.git` file or `commondir` an agent pointed elsewhere: daemon Git would write there.
        for dir in [&found.git_dir, &found.common_dir] {
            if !std::fs::canonicalize(dir).is_ok_and(|d| d.starts_with(&self.real_data_dir)) {
                return Err(std::io::Error::other(format!("{} is outside Launchpad's folders", dir.display())));
            }
        }
        let key = hex::encode(Sha256::digest(found.common_dir.to_string_lossy().as_bytes()));
        let dir = self.shadow_root().join(&key[..24]);
        std::fs::create_dir_all(dir.join("hooks"))?;
        std::fs::create_dir_all(dir.join("info"))?;
        // `git worktree add` writes through the link: the folder must exist in a real repository.
        if found.common_dir.join("objects").is_dir() {
            let _ = std::fs::create_dir(found.common_dir.join("worktrees"));
        }
        let mut links: Vec<(PathBuf, PathBuf)> = LINKED.iter().map(|n| (dir.join(n), found.common_dir.join(n))).collect();
        links.push((dir.join("info/exclude"), found.common_dir.join("info/exclude")));
        for (link, target) in links {
            if std::fs::symlink_metadata(&link).is_err() {
                #[cfg(unix)]
                if let Err(e) = std::os::unix::fs::symlink(&target, &link)
                    && e.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(e);
                }
            }
        }
        let real = read_small(&found.common_dir.join("config")).unwrap_or_default();
        let project = cache_project(&self.data_dir, &found.common_dir);
        let text = shadow_config(project.as_deref(), self.port, found.work_tree.is_none(), &extensions(&real));
        let path = dir.join("config");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            let tmp = dir.join(format!("config.{}.tmp", uuid::Uuid::new_v4()));
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(dir)
    }

    /// After `git worktree add` through the shadow, the new worktree's `.git` file names the shadow: point it at
    /// the repository's own directory, so the agent's Git (in its jail, where the shadow does not exist) works.
    pub fn relink(&self, worktree: &Path) -> std::io::Result<()> {
        let file = worktree.join(".git");
        let text = read_small(&file).ok_or_else(|| std::io::Error::other(format!("{} is not a readable file", file.display())))?;
        let Some(target) = text.strip_prefix("gitdir:").map(str::trim) else { return Ok(()) };
        let Ok(rest) = Path::new(target).strip_prefix(self.shadow_root()) else { return Ok(()) };
        let mut parts = rest.components();
        let (Some(key), Some(Component::Normal(w)), Some(Component::Normal(name)), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(std::io::Error::other(format!("unexpected worktree link {target}")));
        };
        if w != "worktrees" {
            return Err(std::io::Error::other(format!("unexpected worktree link {target}")));
        }
        let real = std::fs::read_link(self.shadow_root().join(key.as_os_str()).join("worktrees"))?;
        std::fs::write(&file, format!("gitdir: {}\n", real.join(name).to_string_lossy()))
    }
}

/// Copies a file an agent may have written (a worktree's index) if it is a regular file, never blocking on a FIFO.
pub fn copy_regular(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    open.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    let mut file = open.open(from)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other(format!("{} is not a regular file", from.display())));
    }
    std::io::copy(&mut file, &mut std::fs::File::create(to)?)?;
    Ok(())
}

/// The repository Git would use in `cwd`, found the same way (the nearest folder with `.git`, or a bare
/// repository), without `core.worktree`, `GIT_DIR` or `commondir` tricks leaving it: the work tree is always the
/// folder that holds `.git`. Never looks at `stop` or above it.
pub fn find(cwd: &Path, stop: &Path) -> Option<Found> {
    if !cwd.join(".git").exists() && cwd.join("HEAD").is_file() && cwd.join("objects").is_dir() && cwd.join("refs").is_dir() {
        let git_dir = normalize(cwd);
        return Some(Found { work_tree: None, common_dir: common_dir(&git_dir), git_dir });
    }
    for dir in cwd.ancestors() {
        if dir == stop || stop.starts_with(dir) {
            return None;
        }
        let dot = dir.join(".git");
        let Ok(meta) = std::fs::metadata(&dot) else { continue };
        let git_dir = if meta.is_dir() {
            normalize(&dot)
        } else {
            let text = read_small(&dot)?;
            normalize(&dir.join(text.strip_prefix("gitdir:")?.trim()))
        };
        return Some(Found { work_tree: Some(normalize(dir)), common_dir: common_dir(&git_dir), git_dir });
    }
    None
}

fn common_dir(git_dir: &Path) -> PathBuf {
    match read_small(&git_dir.join("commondir")) {
        Some(text) if !text.trim().is_empty() => normalize(&git_dir.join(text.trim())),
        _ => git_dir.to_path_buf(),
    }
}

/// Whether a command without a repository is meant to create one (`git init`, `git clone`): the first argument
/// after the global options.
pub fn creates_repository<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> bool {
    let mut it = args.iter().map(|a| a.as_ref().to_str().unwrap_or("-"));
    while let Some(a) = it.next() {
        match a {
            "-c" => {
                it.next();
            }
            a if a.starts_with('-') => {}
            a => return matches!(a, "init" | "clone"),
        }
    }
    false
}

/// A small regular file an agent may have written, read without ever blocking the daemon: a FIFO, a device or a
/// huge file reads as missing.
fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    open.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    let file = open.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(64 * 1024 + 1).read_to_string(&mut text).ok()?;
    (text.len() <= 64 * 1024).then_some(text)
}

/// `a/b/../c` → `a/c`, lexically.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The folder whose repository cache (`<data_dir>/repos/<id>/.git`) this common directory is.
pub fn cache_project(data_dir: &Path, common_dir: &Path) -> Option<String> {
    let rest = common_dir.strip_prefix(data_dir.join("repos")).ok()?;
    let mut parts = rest.components();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Component::Normal(id)), Some(Component::Normal(git)), None) if git == ".git" => {
            let id = id.to_str()?;
            (!id.is_empty() && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| id.to_string())
        }
        _ => None,
    }
}

/// The repository format keys of a config that Git needs to read the repository (object format, ref storage),
/// with safe values only.
pub fn extensions(config: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut section = String::new();
    for line in config.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[') {
            section = s.trim_end_matches(']').trim().to_ascii_lowercase();
        } else if section == "extensions"
            && let Some((k, v)) = line.split_once('=')
        {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
            if matches!(k.as_str(), "objectformat" | "refstorage") && !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric()) {
                out.retain(|(o, _)| *o != k);
                out.push((k, v));
            }
        }
    }
    out
}

/// The shadow's config: nothing from the repository but its format; `origin` is Home's URL for a cache.
pub fn shadow_config(cache_project: Option<&str>, port: u16, bare: bool, extensions: &[(String, String)]) -> String {
    let version = if extensions.is_empty() { 0 } else { 1 };
    let mut text = format!("[core]\n\trepositoryformatversion = {version}\n\tfilemode = true\n\tbare = {bare}\n\tlogallrefupdates = false\n[gc]\n\tauto = 0\n[maintenance]\n\tauto = false\n");
    if !extensions.is_empty() {
        text.push_str("[extensions]\n");
        for (k, v) in extensions {
            text.push_str(&format!("\t{k} = {v}\n"));
        }
    }
    if let Some(id) = cache_project {
        text.push_str(&format!("[remote \"origin\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n", home_url(port, id)));
    }
    text
}

/// Home Git as seen from this daemon (its local Git endpoint).
pub fn home_url(port: u16, project_id: &str) -> String {
    format!("http://127.0.0.1:{port}/git/{project_id}.git")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_are_recognized_only_at_their_place() {
        let data = Path::new("/d");
        assert_eq!(cache_project(data, Path::new("/d/repos/abc-1/.git")).as_deref(), Some("abc-1"));
        assert_eq!(cache_project(data, &normalize(Path::new("/d/repos/abc/.git/worktrees/x/../.."))).as_deref(), Some("abc"));
        assert_eq!(cache_project(data, Path::new("/d/repos/abc")), None);
        assert_eq!(cache_project(data, Path::new("/d/repos/.abc.creating/.git")), None);
        assert_eq!(cache_project(data, Path::new("/d/repos/a b/.git")), None);
        assert_eq!(cache_project(data, Path::new("/d/plain/abc/.git")), None);
        assert_eq!(cache_project(data, Path::new("/d/repos/abc/.git/modules/x")), None);
    }

    #[test]
    fn the_shadow_config_holds_nothing_of_the_repository() {
        let real = "[core]\n\thooksPath = /tmp/x\n\tfsmonitor = sh -c evil\n[extensions]\n\tworktreeConfig = true\n\tobjectFormat = sha256\n\trefStorage = $(evil)\n[filter \"x\"]\n\tclean = evil\n[url \"http://evil/\"]\n\tinsteadOf = http://127.0.0.1:7320/\n[include]\n\tpath = /tmp/y\n";
        let ext = extensions(real);
        assert_eq!(ext, vec![("objectformat".to_string(), "sha256".to_string())]);
        let text = shadow_config(Some("abc"), 7320, false, &ext);
        for bad in ["hooksPath", "fsmonitor", "worktreeConfig", "filter", "insteadOf", "include", "evil"] {
            assert!(!text.to_lowercase().contains(&bad.to_lowercase()), "{bad} in {text}");
        }
        assert!(text.contains("repositoryformatversion = 1"));
        assert!(text.contains("url = http://127.0.0.1:7320/git/abc.git"));
        assert!(!shadow_config(None, 7320, true, &[]).contains("remote"));
    }

    /// Real Git (about 100 ms): what an agent plants in a clone and its worktree never runs in daemon Git.
    #[tokio::test]
    async fn agent_config_never_runs_in_daemon_git() {
        let base = std::env::temp_dir().join(format!("lp-git-safe-{}", uuid::Uuid::new_v4()));
        let data = base.join("data");
        let cache = data.join("repos/p1");
        std::fs::create_dir_all(&cache).unwrap();
        let mark = base.join("ran");
        let path = std::env::var("PATH").unwrap_or_default();
        let plain = crate::shellenv::UserEnv::from_vars([("PATH".to_string(), path), ("HOME".to_string(), base.to_string_lossy().into_owned())].into());
        let guarded = plain.with_git_guard(Guard::new(&data, 7999, false));
        let git = |env: &crate::shellenv::UserEnv, dir: &Path, args: &[&str]| {
            let (env, dir, args) = (env.clone(), dir.to_path_buf(), args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                crate::procs::output(&env, "git", &args, Some(&dir)).await
            }
        };
        let id = ["-c", "user.name=t", "-c", "user.email=t@t"];
        git(&plain, &cache, &["init", "--quiet"]).await.unwrap();
        std::fs::write(cache.join("a.txt"), "a\n").unwrap();
        std::fs::write(cache.join(".gitattributes"), "*.txt filter=x diff=x\n").unwrap();
        git(&plain, &cache, &["add", "--all"]).await.unwrap();
        git(&plain, &cache, &[&id[..], &["commit", "--quiet", "-m", "c"]].concat()).await.unwrap();
        // What an agent writes: hooks, fsmonitor, filter and diff drivers, a redirected Home URL.
        let run = format!("sh -c 'touch {}'; cat", mark.display());
        let config = format!("[core]\n\tfsmonitor = {run}\n\thooksPath = {hooks}\n[filter \"x\"]\n\tclean = {run}\n\tsmudge = {run}\n[diff \"x\"]\n\ttextconv = {run}\n[url \"http://evil.invalid/\"]\n\tinsteadOf = http://127.0.0.1:7999/\n", hooks = base.join("hooks").display());
        let mut text = std::fs::read_to_string(cache.join(".git/config")).unwrap();
        text.push_str(&config);
        std::fs::write(cache.join(".git/config"), text).unwrap();
        std::fs::create_dir_all(base.join("hooks")).unwrap();
        for hook in ["post-checkout", "pre-commit", "reference-transaction"] {
            let file = base.join("hooks").join(hook);
            std::fs::write(&file, format!("#!/bin/sh\ntouch {}\n", mark.display())).unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        // Daemon Git: a worktree, status, a snapshot-like add, a diff and a commit.
        let wt = data.join("worktrees/s1/p1");
        git(&guarded, &cache, &["worktree", "add", "--quiet", "-b", "lp/x", &wt.to_string_lossy()]).await.unwrap();
        Guard::new(&data, 7999, false).relink(&wt).unwrap();
        assert!(std::fs::read_to_string(wt.join(".git")).unwrap().contains("repos/p1/.git/worktrees/"));
        std::fs::write(wt.join("a.txt"), "b\n").unwrap();
        git(&guarded, &wt, &["status", "--porcelain"]).await.unwrap();
        git(&guarded, &wt, &["add", "--all"]).await.unwrap();
        git(&guarded, &wt, &["diff", "--cached"]).await.unwrap();
        git(&guarded, &wt, &[&id[..], &["commit", "--quiet", "-m", "d"]].concat()).await.unwrap();
        assert_eq!(git(&guarded, &wt, &["remote", "get-url", "origin"]).await.unwrap(), "http://127.0.0.1:7999/git/p1.git");
        assert!(!mark.exists(), "agent-written Git config ran in daemon Git");
        // The same config does run without the guard (the test plants a working attack).
        let _ = git(&plain, &wt, &["status", "--porcelain"]).await;
        assert!(mark.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_sandbox_mounts_only_the_folders_of_the_command() {
        let base = std::env::temp_dir().join(format!("lp-git-sandbox-{}", uuid::Uuid::new_v4()));
        let d = base.join("data");
        let guard = Guard::new(&d, 7320, true);
        let d = d.display().to_string();
        let args = |cwd: &str| guard.sandbox_args(Some(Path::new(cwd)), Path::new("/usr/bin/git")).into_iter().map(|a| a.into_string().unwrap()).collect::<Vec<_>>().join(" ");
        let wt = args(&format!("{d}/worktrees/s1/p1"));
        for want in [format!("--bind-try {d}/git-safe {d}/git-safe"), format!("--bind-try {d}/repos {d}/repos"), format!("--bind-try {d}/worktrees {d}/worktrees"), "--tmpfs /tmp".into(), "--unshare-pid".into(), "--ro-bind-try /usr /usr".into(), "--ro-bind-try /etc/resolv.conf /etc/resolv.conf".into()] {
            assert!(wt.contains(&want), "{want} missing in {wt}");
        }
        assert!(wt.ends_with(&format!("--chdir {d}/worktrees/s1/p1")));
        // The private /tmp comes before the folders (a data folder under /tmp stays visible).
        assert!(wt.find("--tmpfs /tmp").unwrap() < wt.find("--bind-try").unwrap());
        // No home folder, no whole /etc, no root, no other data of Launchpad.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/nobody"));
        for bad in [format!(" {} ", home.display()), "/etc /etc".into(), " / / ".into(), format!("{d} {d}"), format!("{d}/plain"), format!("{d}/vault"), "/opt".into()] {
            assert!(!wt.contains(&bad), "{bad} in {wt}");
        }
        let plain = args(&format!("{d}/plain/abc/sub"));
        assert!(plain.contains(&format!("--bind-try {d}/plain/abc {d}/plain/abc")) && !plain.contains(&format!("{d}/repos")) && !plain.contains(&format!("{d}/worktrees")));
        assert!(args(&format!("{d}/tmp/tmp-1111-x")).contains(&format!("--bind-try {d}/tmp/tmp-1111-x {d}/tmp/tmp-1111-x")));
        // Anywhere else: the shadows only (the command fails).
        let none = Vec::<PathBuf>::new();
        for cwd in [format!("{d}/plain"), format!("{d}/vault/x"), format!("{d}/worktrees/../vault"), "/home/me/repo".into()] {
            assert_eq!(guard.mounts(Some(Path::new(&cwd))), none, "{cwd}");
        }
        assert_eq!(guard.mounts(None), none);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn only_init_and_clone_run_without_a_repository() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(creates_repository(&args(&["init", "--quiet"])));
        assert!(creates_repository(&args(&["-c", "init.defaultBranch=main", "--no-pager", "clone", "x"])));
        assert!(!creates_repository(&args(&["-c", "init", "add", "--all"])));
        assert!(!creates_repository(&args(&["status", "init"])));
        assert!(!creates_repository(&args(&[])));
    }

    #[test]
    fn every_command_drops_hooks_and_helpers() {
        let joined = CONFIG.join(" ");
        for key in ["core.hooksPath=/dev/null", "core.fsmonitor=false", "credential.helper=", "protocol.ext.allow=never"] {
            assert!(joined.contains(key));
        }
        assert!(ENV.contains(&("GIT_CONFIG_GLOBAL", "/dev/null")) && ENV.contains(&("GIT_CONFIG_NOSYSTEM", "1")));
    }
}

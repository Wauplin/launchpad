//! The bubblewrap jail every agent runs in on a Linux runner.
//!
//! The jail sees the system folders read-only, the toolchains of the machine read-only, and only the folders of
//! its session read-write: its worktrees and plain folders, its harness home, its own `/tmp` and the build caches
//! of its workspace. Your home folder, `~/.ssh`, `~/.config/gh`, `~/.claude`, Launchpad's own data and the Docker
//! socket do not exist inside. The network is shared: traffic goes through the runner proxy (`proxy.rs`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// System folders mounted read-only when they exist.
const SYSTEM: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/nix", "/run/systemd/resolve", "/run/resolvconf"];

/// Folders of the home folder that hold toolchains (never credentials), mounted read-only when they exist.
pub const HOME_TOOLCHAINS: &[&str] = &[
    ".local/bin",
    ".local/share/claude",
    ".local/share/uv/tools",
    ".local/share/uv/python",
    ".local/share/pnpm",
    ".local/share/fnm",
    ".nvm",
    ".cargo/bin",
    ".rustup",
    ".bun/bin",
    ".deno/bin",
    ".volta",
    ".pyenv",
    ".local/go",
    "go/bin",
    ".opencode/bin",
];

/// Variables of the machine's environment a jailed process keeps. Everything else (tokens, sockets, the
/// machine's logins) is dropped; the session's own variables are added on top.
pub const KEEP_ENV: &[&str] = &[
    "PATH", "LANG", "LANGUAGE", "TERM", "COLORTERM", "TZ", "USER", "LOGNAME", "SHELL", "NVM_DIR", "NVM_BIN", "NVM_INC", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "GOROOT", "JAVA_HOME", "PNPM_HOME", "BUN_INSTALL", "VOLTA_HOME", "PYENV_ROOT", "DENO_INSTALL",
];

pub fn keep_env(name: &str) -> bool {
    KEEP_ENV.contains(&name) || name.starts_with("LC_")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Jail {
    /// Read-write folders, mounted at the same path.
    pub rw: Vec<PathBuf>,
    /// Read-only folders or files, mounted at the same path.
    pub ro: Vec<PathBuf>,
    /// The session's own temporary folder, mounted as `/tmp` and `/var/tmp`.
    pub tmp: PathBuf,
    /// The session's variables (profile and Launchpad), set on top of the kept machine variables.
    pub env: Vec<(String, String)>,
    /// Read-write mounts at another path inside (the ssh-agent socket).
    pub binds: Vec<(PathBuf, PathBuf)>,
}

impl Jail {
    /// `bwrap` arguments before `--`, for a process started in `cwd`.
    pub fn args(&self, cwd: Option<&Path>) -> Vec<OsString> {
        let mut a: Vec<OsString> = vec![];
        let mut push = |items: &[&str]| a.extend(items.iter().map(OsString::from));
        push(&["--die-with-parent", "--unshare-pid", "--unshare-ipc", "--unshare-uts", "--unshare-cgroup-try"]);
        for dir in SYSTEM {
            push(&["--ro-bind-try", dir, dir]);
        }
        push(&["--proc", "/proc", "--dev", "/dev"]);
        let mut bind = |flag: &str, path: &Path, to: &Path| {
            a.push(flag.into());
            a.push(path.as_os_str().into());
            a.push(to.as_os_str().into());
        };
        bind("--bind", &self.tmp, Path::new("/tmp"));
        bind("--bind", &self.tmp, Path::new("/var/tmp"));
        // Read-only first: read-write folders inside a toolchain folder stay writable.
        for p in &self.ro {
            bind("--ro-bind-try", p, p);
        }
        for p in &self.rw {
            bind("--bind-try", p, p);
        }
        for (from, to) in &self.binds {
            bind("--bind-try", from, to);
        }
        if let Some(cwd) = cwd {
            a.push("--chdir".into());
            a.push(cwd.as_os_str().into());
        }
        a
    }

    /// The environment of a jailed process: the kept machine variables, then the session's.
    pub fn environment(&self, base: &std::collections::BTreeMap<String, String>) -> Vec<(String, String)> {
        let mut out = kept(base);
        out.extend(self.env.iter().cloned());
        out
    }
}

/// The machine variables a jailed process keeps.
pub fn kept(base: &std::collections::BTreeMap<String, String>) -> Vec<(String, String)> {
    base.iter().filter(|(k, _)| keep_env(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Whether bubblewrap works on this machine (installed, and user namespaces allowed).
pub async fn check() -> anyhow::Result<()> {
    let out = tokio::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "true"])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|_| anyhow::anyhow!("bubblewrap is not installed: agents run only in a jail (sudo apt install bubblewrap)"))?;
    anyhow::ensure!(
        out.status.success(),
        "bubblewrap cannot create a jail: {}. On Ubuntu 24.04, allow user namespaces for bwrap with an AppArmor profile (Settings → Setup)",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jail_mounts_only_session_folders_read_write() {
        let jail = Jail { rw: vec!["/data/worktrees/s1".into()], ro: vec!["/home/me/.local/bin".into()], tmp: "/data/sessions/s1/tmp".into(), env: vec![], binds: vec![] };
        let args: Vec<String> = jail.args(Some(Path::new("/data/worktrees/s1/repo"))).into_iter().map(|a| a.into_string().unwrap()).collect();
        let joined = args.join(" ");
        assert!(joined.contains("--bind-try /data/worktrees/s1 /data/worktrees/s1"));
        assert!(joined.contains("--ro-bind-try /home/me/.local/bin /home/me/.local/bin"));
        assert!(joined.contains("--bind /data/sessions/s1/tmp /tmp"));
        assert!(joined.ends_with("--chdir /data/worktrees/s1/repo"));
        // Nothing mounts the whole file system or the home folder.
        assert!(!joined.contains("--bind / /") && !joined.contains("--ro-bind / /") && !joined.contains("/home/me "));
        // Read-only mounts come before read-write ones.
        assert!(joined.find("/home/me/.local/bin").unwrap() < joined.find("/data/worktrees/s1 ").unwrap());
    }

    #[test]
    fn jailed_processes_drop_machine_credentials() {
        let base: std::collections::BTreeMap<String, String> =
            [("PATH", "/usr/bin"), ("GH_TOKEN", "x"), ("HF_TOKEN", "y"), ("SSH_AUTH_SOCK", "/s"), ("LC_ALL", "C"), ("ANTHROPIC_API_KEY", "z")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let jail = Jail { env: vec![("GH_TOKEN".into(), "lp-placeholder-github".into())], ..Default::default() };
        let env = jail.environment(&base);
        assert_eq!(env, vec![("LC_ALL".into(), "C".into()), ("PATH".into(), "/usr/bin".into()), ("GH_TOKEN".into(), "lp-placeholder-github".into())]);
    }
}

//! The environment of your login shell, used for every process the daemon starts.
//!
//! Under systemd, the daemon gets a minimal environment (no `~/.local/bin`, no nvm, no cargo).
//! Agents must see the same `PATH` and variables as in a terminal, so the daemon asks the login
//! shell once at startup, like VS Code does.

use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
const START: &str = "__LAUNCHPAD_ENV_START__";

#[derive(Clone)]
pub struct UserEnv {
    vars: Arc<BTreeMap<String, String>>,
    /// Processes started with this environment run in this jail (`jail.rs`).
    jail: Option<Arc<crate::jail::Jail>>,
}

impl UserEnv {
    pub async fn resolve() -> UserEnv {
        let mut vars: BTreeMap<String, String> = std::env::vars().collect();
        match from_login_shell().await {
            Ok(shell_vars) => {
                tracing::info!("environment: {} variables from the login shell", shell_vars.len());
                vars.extend(shell_vars);
            }
            Err(e) => tracing::warn!("environment: login shell failed ({e:#}), using the daemon environment"),
        }
        // Variables set by a parent Claude Code or Codex session must not leak into new sessions:
        // with them, Claude turns transcript saving off ("inherited CLAUDE_CODE_CHILD_SESSION marker").
        vars.retain(|k, _| !is_harness_marker(k));
        // Launchpad's own environment name must not leak either: an agent running `launchpad` commands
        // talks to the instance that started it, which the agent sets explicitly when needed.
        vars.remove("LAUNCHPAD_ENV");
        // A Burst runner's own credential never reaches a process it starts.
        vars.remove("LAUNCHPAD_BURST_CREDENTIAL");
        UserEnv { vars: Arc::new(vars), jail: None }
    }

    /// An environment with exactly these variables (tests).
    pub fn from_vars(vars: BTreeMap<String, String>) -> UserEnv {
        UserEnv { vars: Arc::new(vars), jail: None }
    }

    /// This environment with profile overrides.
    pub fn with(&self, overrides: &[(String, String)]) -> UserEnv {
        let mut vars = (*self.vars).clone();
        vars.extend(overrides.iter().cloned());
        UserEnv { vars: Arc::new(vars), jail: self.jail.clone() }
    }

    /// This environment for processes in `jail` (None: unchanged).
    pub fn jailed(&self, jail: Option<Arc<crate::jail::Jail>>) -> UserEnv {
        let mut env = self.clone();
        if jail.is_some() {
            env.jail = jail;
        }
        env
    }

    pub fn jail(&self) -> Option<&Arc<crate::jail::Jail>> {
        self.jail.as_ref()
    }

    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Absolute path of a program, searched in the user `PATH`.
    pub fn which(&self, program: &str) -> Option<std::path::PathBuf> {
        if program.contains('/') || program.contains('\\') {
            return Some(program.into());
        }
        let path = self.get("PATH").or_else(|| self.get("Path"))?;
        #[cfg(unix)]
        let names = [program.to_string()];
        #[cfg(windows)]
        let mut names = vec![program.to_string()];
        #[cfg(windows)]
        if std::path::Path::new(program).extension().is_none() {
            names.extend(self.get("PATHEXT").unwrap_or(".EXE;.CMD;.BAT").split(';').map(|ext| format!("{program}{ext}")));
        }
        std::env::split_paths(path).flat_map(|d| names.iter().map(move |name| d.join(name))).find(|p| is_executable(p))
    }
}

pub fn is_harness_marker(key: &str) -> bool {
    crate::harness::registrations()
        .iter()
        .any(|registration| (registration.environment_marker)(key))
}

fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(windows)]
    { p.is_file() }
}

#[cfg(unix)]
async fn from_login_shell() -> anyhow::Result<BTreeMap<String, String>> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" }.into());
    let script = format!("printf '%s' '{START}'; env -0");
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.args(["-l", "-i", "-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await??;
    let text = String::from_utf8_lossy(&out.stdout);
    let body = text.split_once(START).map(|(_, b)| b).ok_or_else(|| anyhow::anyhow!("no output from {shell}"))?;
    let mut vars = BTreeMap::new();
    for entry in body.split('\0') {
        if let Some((k, v)) = entry.split_once('=')
            && !k.is_empty() && !k.contains(char::is_whitespace) {
            vars.insert(k.to_string(), v.to_string());
        }
    }
    // Shell-internal variables that make no sense for other processes.
    for k in ["_", "SHLVL", "PWD", "OLDPWD"] {
        vars.remove(k);
    }
    Ok(vars)
}

#[cfg(windows)]
async fn from_login_shell() -> anyhow::Result<BTreeMap<String, String>> {
    Ok(std::env::vars().collect())
}

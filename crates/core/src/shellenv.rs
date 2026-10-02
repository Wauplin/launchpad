//! The environment of your login shell, used for every process the daemon starts.
//!
//! Under systemd, the daemon gets a minimal environment (no `~/.local/bin`, no nvm, no cargo).
//! Agents must see the same `PATH` and variables as in a terminal, so the daemon asks the login
//! shell once at startup, like VS Code does.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const START: &str = "__LAUNCHPAD_ENV_START__";

#[derive(Clone)]
pub struct UserEnv {
    vars: Arc<BTreeMap<String, String>>,
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
        // Launchpad's own profile must not leak either: an agent running `launchpad` commands
        // talks to the instance that started it, which the agent sets explicitly when needed.
        vars.remove("LAUNCHPAD_PROFILE");
        UserEnv { vars: Arc::new(vars) }
    }

    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Absolute path of a program, searched in the user `PATH`.
    pub fn which(&self, program: &str) -> Option<std::path::PathBuf> {
        if program.contains('/') {
            return Some(program.into());
        }
        let path = self.get("PATH")?;
        std::env::split_paths(path).map(|d| d.join(program)).find(|p| is_executable(p))
    }
}

pub fn is_harness_marker(key: &str) -> bool {
    key == "CLAUDECODE"
        || key.starts_with("CLAUDE_CODE_")
        || key == "CLAUDE_PID"
        || key == "CLAUDE_EFFORT"
        || key.starts_with("CODEX_THREAD")
        || key == "CODEX_SANDBOX"
        || key == "CODEX_SANDBOX_NETWORK_DISABLED"
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

async fn from_login_shell() -> anyhow::Result<BTreeMap<String, String>> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
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

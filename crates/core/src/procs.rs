//! Child processes started by the daemon: harnesses and git, on this computer or on an SSH host
//! (`exec.rs`). Integrated terminals use a PTY (`terminals.rs`).

use std::path::Path;

use anyhow::{Context, Result};

use crate::exec::{Cmd, Io};
use crate::shellenv::UserEnv;

/// A command with the user environment, on the host of `cwd` (or of `env`).
pub fn command(env: &UserEnv, program: &str, cwd: Option<&Path>) -> Cmd {
    Cmd::new(env, program, cwd)
}

/// Like [`command`]; on this computer the child gets SIGTERM when the daemon exits, so a restarted dev
/// daemon never leaves orphan agents behind. On a host, the helper stops it when another daemon boot connects.
pub fn child_command(env: &UserEnv, program: &str, cwd: Option<&Path>) -> Cmd {
    Cmd::new(env, program, cwd).child()
}

/// Runs a short command and returns its trimmed stdout.
pub async fn output(env: &UserEnv, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let out = command(env, program, cwd)
        .args(args)
        .stdin(Io::Null)
        .output()
        .await
        .with_context(|| format!("run {program}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

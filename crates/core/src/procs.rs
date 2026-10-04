//! Child processes started by the daemon: harnesses, git, terminals.

use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::shellenv::UserEnv;

/// A command with the user environment. Harness children also die with the daemon (`PR_SET_PDEATHSIG`),
/// so a restarted dev daemon never leaves orphan agents behind.
pub fn command(env: &UserEnv, program: &str, cwd: Option<&Path>) -> Command {
    let resolved = env.which(program).unwrap_or_else(|| program.into());
    let mut cmd = Command::new(resolved);
    cmd.env_clear().envs(env.vars().iter());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.kill_on_drop(true);
    cmd
}

/// Like [`command`], and the child gets SIGTERM when the daemon exits.
pub fn child_command(env: &UserEnv, program: &str, cwd: Option<&Path>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = command(env, program, cwd);
    // SAFETY: prctl is async-signal-safe and only affects the child.
    #[cfg(target_os = "linux")]
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    cmd
}

/// Runs a short command and returns its trimmed stdout.
pub async fn output(env: &UserEnv, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let out = command(env, program, cwd)
        .args(args)
        .stdin(std::process::Stdio::null())
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

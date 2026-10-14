//! Installs the helper on a host: the daemon's own binary, uploaded once per build into
//! `~/.local/share/launchpad-host/bin/launchpad-<build>`. It is the only thing Launchpad installs by itself;
//! harnesses, Git and gh are the owner's (preflight shows the commands).

use std::time::Duration;

use anyhow::{Result, bail};

use super::{Host, Hosts, LinkState, NeedsSetup, helper::BIN_DIR, shell_quote_home, ssh_run};

/// `uname -sm` names of the platform this daemon was built for.
fn local_platform() -> (&'static str, &'static str) {
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        other => other,
    };
    (os, std::env::consts::ARCH)
}

fn same_arch(local: &str, remote: &str) -> bool {
    local == remote || matches!((local, remote), ("aarch64", "arm64") | ("x86_64", "amd64"))
}

/// Ensures the helper of this build is on the host. Returns its path relative to the host's home.
pub(super) async fn ensure(hosts: &Hosts, host: &Host) -> Result<String> {
    let rel = format!("{BIN_DIR}/launchpad-{}", super::build_id());
    let bin = shell_quote_home(&rel);
    // `sh -c`: the login shell of the host may not be POSIX (fish).
    let probe = format!("sh -c 'uname -sm; if [ -x {bin} ]; then echo present; fi'");
    let (ok, out, err) = ssh_run(hosts, host, &probe, None, Duration::from_secs(90)).await?;
    if !ok {
        bail!("{}", if err.is_empty() { "ssh failed".to_string() } else { err });
    }
    let mut lines = out.lines();
    let uname = lines.next().unwrap_or("").trim().to_string();
    let present = lines.any(|l| l.trim() == "present");
    let (os, arch) = uname.split_once(' ').unwrap_or((uname.as_str(), ""));
    if !matches!(os, "Linux" | "Darwin") {
        return Err(NeedsSetup(format!("{} runs {}. Launchpad hosts must be Linux (macOS hosts work when this computer is a Mac).", host.id, if uname.is_empty() { "an unknown system" } else { &uname })).into());
    }
    if !present {
        let (local_os, local_arch) = local_platform();
        if os != local_os || !same_arch(local_arch, arch) {
            return Err(NeedsSetup(format!(
                "{} is {os} {arch}, but this Launchpad build is {local_os} {local_arch}: it can only upload its own binary. Copy a launchpad binary of the same build for {os} {arch} to ~/{rel} on the host.",
                host.id
            )).into());
        }
        let exe = std::env::current_exe()?;
        if let Some(c) = hosts.conns.read().unwrap().get(&host.id) {
            c.set_state(LinkState::Connecting, Some("Uploading the Launchpad helper".into()));
        }
        let dir = shell_quote_home(BIN_DIR);
        let upload = format!("sh -c 'set -e; mkdir -p {dir}; t={bin}.upload-$$; cat > \"$t\"; chmod 755 \"$t\"; mv \"$t\" {bin}'");
        let (ok, _, err) = ssh_run(hosts, host, &upload, Some(exe), Duration::from_secs(30 * 60)).await?;
        if !ok {
            bail!("could not upload the helper: {err}");
        }
        tracing::info!("uploaded the helper to {}", host.id);
    }
    let (ok, out, err) = ssh_run(hosts, host, &format!("{bin} host-helper version"), None, Duration::from_secs(60)).await?;
    if !ok || out.trim() != super::build_id() {
        return Err(NeedsSetup(format!("the Launchpad helper does not run on {}: {}", host.id, if err.is_empty() { out.trim().to_string() } else { err })).into());
    }
    Ok(rel)
}

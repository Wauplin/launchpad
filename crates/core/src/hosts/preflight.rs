//! Read-only checks of a host (Settings → Hosts): connection, platform, helper, Git, GitHub CLI and each
//! harness (found, version, signed in). Never signs in, installs or refreshes a token. Each failed check
//! names the command to run, which the owner can start in a visible terminal on the host.

use serde::Serialize;
use ts_rs::TS;

use crate::App;
use crate::shellenv::UserEnv;

#[derive(Clone, Debug, Serialize, TS)]
pub struct Check {
    /// `ssh`, `system`, `helper`, `git`, `gh` or a harness ID.
    pub id: String,
    pub name: String,
    /// None: a warning or unknown.
    pub ok: Option<bool>,
    pub detail: String,
    /// What to run on the host to fix it (an install or sign-in command).
    pub command: Option<String>,
    /// Harness checks: the CLI is on the login-shell PATH and signed in.
    pub found: Option<bool>,
    pub signed_in: Option<bool>,
}

fn check(id: &str, name: &str, ok: Option<bool>, detail: impl Into<String>) -> Check {
    Check { id: id.into(), name: name.into(), ok, detail: detail.into(), command: None, found: None, signed_in: None }
}

async fn first_line(env: &UserEnv, program: &str, args: &[&str]) -> Result<String, String> {
    match crate::signin::status_output(env, program, args).await {
        Ok((true, out, err)) => Ok(if out.trim().is_empty() { err } else { out }.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()),
        Ok((false, out, err)) => Err(format!("{}{}", out.trim(), err.trim()).lines().next().unwrap_or("failed").to_string()),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// Runs every check and stores the result in the host status.
pub async fn run(app: &App, id: &str) -> anyhow::Result<Vec<Check>> {
    let hosts = crate::hosts::global().ok_or_else(|| anyhow::anyhow!("hosts are not available"))?;
    let status = hosts.status(id).ok_or_else(|| anyhow::anyhow!("unknown host {id}"))?;
    let mut checks = vec![];
    if status.state != crate::hosts::LinkState::Connected {
        checks.push(Check { command: None, ..check("ssh", "SSH connection", Some(false), status.error.unwrap_or_else(|| "Not connected".into())) });
        hosts.set_checks(id, checks.clone());
        return Ok(checks);
    }
    let host = status.host.clone();
    let config = {
        let mut args: Vec<String> = vec!["-G".into()];
        if let Some(p) = host.port { args.extend(["-p".into(), p.to_string()]); }
        args.push(host.destination.clone());
        let ssh = hosts.env.which("ssh").unwrap_or_else(|| "ssh".into());
        tokio::process::Command::new(ssh).args(&args).output().await.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    let jump = config.lines().find_map(|l| l.strip_prefix("proxyjump ")).map(str::trim).filter(|j| !j.is_empty() && *j != "none");
    let latency = status.latency_ms.map(|l| format!(" · {l} ms")).unwrap_or_default();
    checks.push(check("ssh", "SSH connection", Some(true), format!("{}{latency} · system OpenSSH with your ~/.ssh/config", jump.map(|j| format!("via {j} (ProxyJump)")).unwrap_or_else(|| format!("to {}", host.destination)))));
    let info = status.info.clone().unwrap_or_default();
    let linux = info.os == "Linux";
    let root = info.user == "root";
    checks.push(check("system", "System", Some(linux && !root).filter(|ok| *ok), format!("{} {}{}{}", info.os, info.arch, if info.user.is_empty() { String::new() } else { format!(" · user {}", info.user) }, if !linux { " · Linux hosts are the supported first step" } else if root { " · Claude Code refuses to skip permission prompts as root: use a regular account, or set IS_SANDBOX=1 in the login environment of a disposable sandbox" } else { "" })));
    checks.push(check("helper", "Launchpad helper", Some(true), format!("Installed in ~/.local/share/launchpad-host, build {} (same as this daemon)", crate::hosts::build_id())));
    let _ = crate::hosts::conn(id)?.request(crate::hosts::proto::Op::RefreshEnv).await;
    let env = UserEnv::for_host(id);
    let git = match first_line(&env, "git", &["--version"]).await {
        Ok(v) => check("git", "Git", Some(true), v),
        Err(e) => Check { command: Some("sudo apt install git".into()), ..check("git", "Git", Some(false), format!("Not found: {e}")) },
    };
    checks.push(git);
    let gh = match crate::signin::status_output(&env, "gh", &["auth", "status", "--hostname", "github.com"]).await {
        Ok((ok, out, err)) => {
            let text = format!("{out}\n{err}");
            let account = text.lines().find_map(|l| l.split_once("Logged in to github.com account ").map(|(_, rest)| rest.trim().to_string()));
            if ok {
                check("gh", "GitHub CLI", Some(true), format!("Signed in{} · publication and merges push from this host", account.map(|a| format!(" as {a}")).unwrap_or_default()))
            } else {
                Check { command: Some("gh auth login".into()), ..check("gh", "GitHub CLI", None, "Not signed in: pre-draft Publish needs push access from this host") }
            }
        }
        Err(_) => Check { command: Some("Install the GitHub CLI: https://cli.github.com".into()), ..check("gh", "GitHub CLI", None, "Not installed: pre-draft Publish needs it on this host") },
    };
    checks.push(gh);
    let programs: Vec<String> = crate::harness::REGISTRATIONS.iter().map(|r| app.harnesses.get(r.descriptor.id).sign_in_program().to_string()).collect();
    let found: Vec<Option<String>> = crate::hosts::conn(id)?
        .request(crate::hosts::proto::Op::Which { programs: programs.clone() })
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(|| vec![None; programs.len()]);
    for (r, path) in crate::harness::REGISTRATIONS.iter().zip(found) {
        let adapter = app.harnesses.get(r.descriptor.id);
        let name = r.descriptor.name;
        let hid = r.descriptor.id.as_str();
        let Some(_) = path else {
            checks.push(Check {
                command: adapter.install_command().map(str::to_string),
                found: Some(false),
                ..check(hid, name, Some(false), "Not installed, or not on the PATH of a login shell (nvm or .bashrc-only installs are not seen)")
            });
            continue;
        };
        let version = first_line(&env, adapter.sign_in_program(), &["--version"]).await.unwrap_or_default();
        let (remote, local) = tokio::join!(adapter.sign_in(&env), adapter.sign_in(&app.env));
        let account = match (&remote.detail, &local.detail, remote.signed_in, local.signed_in) {
            (Some(r), Some(l), Some(true), Some(true)) if r == l => " · same account as this computer".to_string(),
            (_, _, Some(true), Some(true)) if r.descriptor.capabilities.quota => " · possibly another account: quota meters show this computer's account".to_string(),
            _ => String::new(),
        };
        let detail = match remote.signed_in {
            Some(true) => format!("{version} · signed in{}{account}", remote.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default()),
            Some(false) => format!("{version} · not signed in"),
            None => format!("{version} · sign-in unknown{}", remote.detail.map(|d| format!(": {d}")).unwrap_or_default()),
        };
        checks.push(Check {
            command: (remote.signed_in != Some(true)).then(|| adapter.sign_in_command().to_string()),
            found: Some(true),
            signed_in: remote.signed_in,
            ..check(hid, name, remote.signed_in.filter(|s| *s), detail)
        });
    }
    hosts.set_checks(id, checks.clone());
    Ok(checks)
}

/// Checks each host again after it (re)connects, so the composer knows which harnesses it can run.
pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            for h in app.hosts.snapshot() {
                if h.state == crate::hosts::LinkState::Connected && h.checked_at.is_none_or(|at| at < h.since)
                    && let Err(e) = run(&app, &h.host.id).await
                {
                    tracing::info!("host {} checks: {e:#}", h.host.id);
                }
            }
        }
    });
}

//! Daemon roles. Home holds every record and runs no agents; a runner runs agents and terminals and keeps only
//! caches and events not yet saved by Home.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Home,
    Runner,
}

impl Role {
    pub fn parse(s: &str) -> anyhow::Result<Role> {
        match s {
            "home" => Ok(Role::Home),
            "runner" => Ok(Role::Runner),
            "all" => anyhow::bail!("the all role is gone: `launchpad daemon` starts Home (no agents); a machine that runs agents is a runner (`launchpad daemon --role runner`, paired with `launchpad connect <Home address>`)"),
            _ => anyhow::bail!("unknown role {s} (home or runner)"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Home => "home",
            Role::Runner => "runner",
        }
    }

    /// Agents and terminals run here.
    pub fn runs_agents(self) -> bool {
        self == Role::Runner
    }
}

/// Version of the Home ↔ runner protocol. A runner with another protocol cannot connect.
pub const PROTOCOL: u32 = 2;

/// How a runner's build relates to Home's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Compat {
    Same,
    /// Same protocol, another build: running sessions continue, new sessions are refused (Update this runner).
    Outdated,
    /// Another protocol: the connection is refused.
    Incompatible,
}

pub fn compat(home_protocol: u32, home_commit: &str, runner_protocol: u32, runner_commit: &str) -> Compat {
    if home_protocol != runner_protocol {
        Compat::Incompatible
    } else if home_commit != runner_commit {
        Compat::Outdated
    } else {
        Compat::Same
    }
}

/// Refusal for work that needs a runner, on Home, and for every agent or terminal off Linux.
pub fn ensure_runs_agents(role: Role) -> anyhow::Result<()> {
    anyhow::ensure!(role.runs_agents(), "Home runs no agents or terminals: connect a runner");
    ensure_agent_os()
}

/// Agents and terminals run only on Linux (a Linux machine, a Local VM or Burst), always contained. On macOS,
/// `packaging/local-vm.sh` creates a Linux VM runner. Home itself runs no agents and may run elsewhere.
pub fn ensure_agent_os() -> anyhow::Result<()> {
    anyhow::ensure!(cfg!(target_os = "linux"), "{}", os_refusal(std::env::consts::OS).unwrap_or_default());
    Ok(())
}

/// Why Home refuses a runner that reports `os` (`std::env::consts::OS`): None for Linux.
pub fn os_refusal(os: &str) -> Option<String> {
    (os != "linux").then(|| format!("agents and terminals run only on Linux, not on {os}: on a Mac, create a Linux VM runner with packaging/local-vm.sh"))
}

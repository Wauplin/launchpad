//! Daemon roles. Home holds every record and runs no agents; a runner runs agents and terminals and keeps only
//! caches and events not yet saved by Home; `all` keeps the single-process mode (development, today's installs).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Home,
    Runner,
    #[default]
    All,
}

impl Role {
    pub fn parse(s: &str) -> anyhow::Result<Role> {
        match s {
            "home" => Ok(Role::Home),
            "runner" => Ok(Role::Runner),
            "all" => Ok(Role::All),
            _ => anyhow::bail!("unknown role {s} (home, runner or all)"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Home => "home",
            Role::Runner => "runner",
            Role::All => "all",
        }
    }

    /// Agents and terminals run here.
    pub fn runs_agents(self) -> bool {
        self != Role::Home
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

/// Refusal for work that needs a runner, on Home.
pub fn ensure_runs_agents(role: Role) -> anyhow::Result<()> {
    anyhow::ensure!(role.runs_agents(), "Home runs no agents or terminals: connect a runner");
    Ok(())
}

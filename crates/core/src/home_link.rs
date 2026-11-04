//! A runner's link to its Home: the credential from `launchpad connect`, the runner guard (local settings that
//! never sync to Home), and the state of the tunnel shown in the status.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::paths::Paths;
use crate::role::Compat;
use crate::runners::Guard;

/// `<data_dir>/home.json`, mode 0600.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    /// Home's address, for example `https://wauplin-launchpad.hf.space`.
    pub url: String,
    pub id: String,
    pub secret: String,
}

impl Link {
    pub fn credential(&self) -> String {
        format!("{}.{}", self.id, self.secret)
    }
}

fn link_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir.join("home.json")
}

fn guard_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir.join("guard.json")
}

pub fn load(paths: &Paths) -> Option<Link> {
    serde_json::from_slice(&std::fs::read(link_file(paths)).ok()?).ok()
}

pub fn save(paths: &Paths, link: &Link) -> Result<()> {
    paths.ensure_dirs()?;
    crate::paths::write_private(&link_file(paths), &serde_json::to_string_pretty(link)?).context("save the Home link")
}

pub fn guard(paths: &Paths) -> Guard {
    std::fs::read(guard_file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn set_guard(app: &App, guard: &Guard) -> Result<()> {
    std::fs::write(guard_file(&app.paths), serde_json::to_vec_pretty(guard)?)?;
    app.status.update(|s| {
        if let Some(h) = s.home.as_mut() {
            h.guard = guard.clone();
        }
    });
    app.bus.publish("runner", "guard", serde_json::to_value(guard)?);
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    /// No `launchpad connect` yet.
    NotConnected,
    Connecting,
    Connected,
    /// Home refused the runner (pending approval, removed, incompatible): `message` says why.
    Refused,
    /// Home is unreachable; the runner retries.
    Offline,
}

/// The runner's view of its Home, in the status.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct HomeStatus {
    pub url: Option<String>,
    pub state: TunnelState,
    pub message: Option<String>,
    pub compat: Option<Compat>,
    pub guard: Guard,
}

pub fn set_state(app: &App, url: Option<String>, state: TunnelState, message: Option<String>, compat: Option<Compat>) {
    let guard = guard(&app.paths);
    app.status.update(|s| s.home = Some(HomeStatus { url, state, message, compat, guard }));
}

/// The HF token sent to a Home in a private Space (the Space's sign-in), when there is one: `HF_TOKEN`, or the
/// `hf auth login` token of this machine.
pub fn space_token(url: &str) -> Option<String> {
    let host = url.split("://").nth(1)?.split('/').next()?;
    if !host.ends_with(".hf.space") {
        return None;
    }
    if let Ok(t) = std::env::var("HF_TOKEN").map(|t| t.trim().to_string())
        && !t.is_empty()
    {
        return Some(t);
    }
    let home = std::env::var("HF_HOME").map(std::path::PathBuf::from).ok().or_else(|| dirs::home_dir().map(|h| h.join(".cache/huggingface")))?;
    std::fs::read_to_string(home.join("token")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

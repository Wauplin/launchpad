//! Setup checklist of a runner (`/api/setup/checklist`, shown in the runner's own window): its link to Home, the
//! jail, the network capture, the agent CLIs of the profiles, and the vault logins and Git identity agents use.
//! "Detect first, never redo": every check only reads (version commands, a test jail, Home's vault). A missing part
//! shows its fixed command for the owner to copy into a terminal on that machine; Launchpad never runs it. Home's
//! own setup is computed by the UI (`HomeSetup`), or by the Space gateway.

use serde::Serialize;
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::home_link::TunnelState;
use launchpad_core::shellenv::UserEnv;
use launchpad_core::signin::status_output;

#[derive(Clone, Copy, Debug, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Ready,
    /// Configured, but Home could not confirm it now (unreachable). Counts as done.
    Unverified,
    Missing,
}

#[derive(Clone, Copy, Debug, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Required,
    Recommended,
    Optional,
}

/// A detected value and where it comes from. Never a secret.
#[derive(Clone, Debug, Serialize, TS)]
pub struct Found {
    pub label: String,
    pub value: String,
    /// A file, command or account the value comes from.
    pub source: Option<String>,
}

/// One missing part, with the fixed command that adds it (when there is one), to copy into a terminal there.
#[derive(Clone, Debug, Serialize, TS)]
pub struct MissingPart {
    /// `connect`, `install`, `apparmor`, `check`, `login:<profile>`, `identity`, …
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    /// The command asks for the owner's password (sudo).
    pub sudo: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct SetupItem {
    /// `home`, `jail`, `netcap`, `harnesses`, `vault_login` or `vault_git`.
    pub id: String,
    pub title: String,
    pub importance: Importance,
    pub state: ItemState,
    pub why: String,
    pub found: Vec<Found>,
    pub missing: Vec<MissingPart>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct Checklist {
    pub items: Vec<SetupItem>,
    /// Required items that are neither ready nor unverified.
    pub required_left: u32,
}

fn item(id: &str, title: &str, importance: Importance, why: &str) -> SetupItem {
    SetupItem { id: id.into(), title: title.into(), importance, state: ItemState::Missing, why: why.into(), found: vec![], missing: vec![] }
}

fn found(label: &str, value: impl Into<String>, source: Option<String>) -> Found {
    Found { label: label.into(), value: value.into(), source }
}

fn part(id: &str, label: &str, detail: Option<String>, command: Option<String>, sudo: bool) -> MissingPart {
    MissingPart { id: id.into(), label: label.into(), detail, command, sudo }
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

/// Runs a command; `None` when it is missing or timed out.
async fn run(env: &UserEnv, program: &str, args: &[&str]) -> Option<(bool, String, String)> {
    status_output(env, program, args).await.ok()
}

// ---------------------------------------------------------------- Home

/// The runner's link to Home (`launchpad connect`), from the tunnel state.
fn home(app: &App) -> SetupItem {
    let mut it = item("home", "Home", Importance::Required, "Home keeps every record and credential and sends this runner its sessions.");
    let status = app.status.get().home;
    let Some(h) = status else {
        it.missing.push(part("connect", "Pair with Home", Some("Approve the runner in Home's Settings → Home and runners when the code shows up there.".into()), Some("launchpad connect <Home address>".into()), false));
        return it;
    };
    if let Some(url) = &h.url {
        it.found.push(found("Address", url.clone(), Some("launchpad connect".into())));
    }
    match h.state {
        TunnelState::Connected => it.state = ItemState::Ready,
        TunnelState::Connecting | TunnelState::Offline if h.url.is_some() => {
            it.state = ItemState::Unverified;
            it.found.push(found("Connection", h.message.clone().unwrap_or_else(|| "connecting".into()), None));
        }
        TunnelState::Refused => it.missing.push(part("refused", "Home refuses this runner", h.message.clone(), Some("launchpad connect <Home address>".into()), false)),
        _ => it.missing.push(part("connect", "Pair with Home", Some("Approve the runner in Home's Settings → Home and runners when the code shows up there.".into()), Some("launchpad connect <Home address>".into()), false)),
    }
    it
}

// ---------------------------------------------------------------- Jail and network capture

/// Ubuntu 24.04 restricts unprivileged user namespaces through AppArmor (read-only check).
fn apparmor_restricts_userns() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").is_ok_and(|v| v.trim() == "1")
}

/// The AppArmor profile that lets bwrap create user namespaces. Never overwrites an existing file.
const APPARMOR_FIX: &str = r#"if [ -e /etc/apparmor.d/bwrap ]; then echo '/etc/apparmor.d/bwrap already exists and was not changed.'; else printf '%s\n' 'abi <abi/4.0>,' 'include <tunables/global>' 'profile bwrap /usr/bin/bwrap flags=(unconfined) {' '  userns,' '  include if exists <local/bwrap>' '}' | sudo tee /etc/apparmor.d/bwrap >/dev/null && sudo apparmor_parser -r /etc/apparmor.d/bwrap; fi"#;

async fn jail(env: &UserEnv) -> SetupItem {
    let mut it = item("jail", "Agent jail", Importance::Required, "Every agent on Linux runs in a bubblewrap jail: it sees only its own folders, never your home folder or logins.");
    let Some((true, out, _)) = run(env, "bwrap", &["--version"]).await else {
        it.missing.push(part("install", "Install bubblewrap", Some("Ubuntu and Debian (asks for your password).".into()), Some("sudo apt update && sudo apt install bubblewrap -y".into()), true));
        return it;
    };
    it.found.push(found("Version", first_line(&out).trim_start_matches("bubblewrap ").to_string(), Some("bwrap --version".into())));
    match launchpad_core::jail::check().await {
        Ok(()) => {
            it.state = ItemState::Ready;
            it.found.push(found("Jail", "works", Some("a test jail ran true".into())));
        }
        Err(e) if apparmor_restricts_userns() => {
            it.found.push(found("Jail", format!("{e:#}"), Some("kernel.apparmor_restrict_unprivileged_userns = 1".into())));
            it.missing.push(part("apparmor", "Allow user namespaces for bwrap", Some("Ubuntu 24.04 blocks them with AppArmor. This adds the profile /etc/apparmor.d/bwrap (asks for your password); an existing file is never changed.".into()), Some(APPARMOR_FIX.into()), true));
        }
        Err(e) => it.missing.push(part("check", "bubblewrap cannot create a jail", Some(format!("{e:#}")), None, false)),
    }
    it
}

/// Every agent runs under the network capture (`netcap::check`): a runner where it cannot start runs no agents.
async fn netcap() -> SetupItem {
    let mut it = item("netcap", "Network capture", Importance::Required, "Every agent runs under the network capture, which sends its connections through the runner proxy. A runner where it cannot start runs no agents.");
    match launchpad_core::netcap::check().await {
        Ok(()) => {
            it.state = ItemState::Ready;
            it.found.push(found("Capture", "works", Some("a test capture ran".into())));
        }
        Err(e) => it.missing.push(part("check", "The network capture cannot start", Some(format!("{e:#}")), Some("launchpad netcap --self-test".into()), false)),
    }
    it
}

// ---------------------------------------------------------------- Agent CLIs

/// The agent CLI of each profile's harness, installed on this machine (the jail runs this machine's CLI).
fn harnesses(app: &App) -> SetupItem {
    let mut it = item("harnesses", "Agent CLIs", Importance::Required, "Agents run the CLI installed on this machine, inside the jail, with a login from Home's vault.");
    let profiles = app.config.get().profiles;
    if profiles.is_empty() {
        it.state = ItemState::Unverified;
        it.found.push(found("Profiles", "none yet: Home sends them once you add one in Settings → Profiles there", None));
        return it;
    }
    let status = app.status.get();
    let mut kinds: Vec<launchpad_core::model::HarnessKind> = profiles.iter().map(|p| p.harness).collect();
    kinds.sort_by_key(|k| k.as_str());
    kinds.dedup();
    for kind in kinds {
        match status.harnesses.get(kind.as_str()).and_then(|h| h.version.clone()) {
            Some(v) => it.found.push(found(kind.display(), v, None)),
            None => {
                let command = app.harnesses.get(kind).install_command().map(str::to_string);
                it.missing.push(part(&format!("install:{}", kind.as_str()), &format!("Install {}", kind.display()), Some("A profile on Home uses it; this runner takes its sessions once it is installed.".into()), command, false));
            }
        }
    }
    it.state = if it.missing.is_empty() { ItemState::Ready } else { ItemState::Missing };
    it
}

// ---------------------------------------------------------------- Vault

/// Home's vault (entries without values) and agents' Git identity. `None`: Home is unreachable.
async fn vault_view() -> Option<(Vec<launchpad_core::vault::VaultEntry>, String, String)> {
    let v = launchpad_core::home_link::home_call("vault-view", serde_json::Value::Null).await.ok()?;
    let entries = serde_json::from_value(v["entries"].clone()).ok()?;
    Some((entries, v["git_name"].as_str().unwrap_or_default().to_string(), v["git_email"].as_str().unwrap_or_default().to_string()))
}

const UNREACHABLE: &str = "Home is unreachable: checked again when it answers";

/// Contained sessions take their harness login only from the vault, never from this machine.
fn vault_logins(app: &App, vault: Option<&[launchpad_core::vault::VaultEntry]>) -> SetupItem {
    let mut it = item("vault_login", "Agent logins in the vault", Importance::Required, "Agents in the jail never use this machine's sign-ins: each profile's agent needs a login in Home's vault (Settings → Credentials and tools on Home).");
    let Some(entries) = vault else {
        it.state = ItemState::Unverified;
        it.found.push(found("Vault", UNREACHABLE, None));
        return it;
    };
    let profiles = app.config.get().profiles;
    if profiles.is_empty() {
        it.missing.push(part("profile", "Create a profile first", Some("On Home: Settings → Profiles, then add the login of its agent to the vault.".into()), None, false));
        return it;
    }
    for p in &profiles {
        let Some(kind) = launchpad_core::vault::Kind::login_of(p.harness) else { continue };
        let login = match &p.login {
            Some(id) => entries.iter().find(|e| &e.id == id && e.kind == kind),
            None => launchpad_core::vault::default_login(entries, kind),
        };
        match login {
            Some(e) => it.found.push(found(&p.name, format!("{} · {}", e.name, e.detail), Some(format!("vault entry {}", e.id)))),
            None => {
                let what = match &p.login { Some(id) => format!("The login {id} that this profile names is not in the vault."), None => format!("The vault has no {} login.", p.harness.display()) };
                it.missing.push(part(&format!("login:{}", p.id), &format!("Add a {} login for {}", p.harness.display(), p.name), Some(what), None, false));
            }
        }
    }
    it.state = if it.missing.is_empty() { ItemState::Ready } else { ItemState::Missing };
    it
}

/// The author of agents' commits in the jail: the vault's identity, never this machine's Git config.
fn vault_git(vault: Option<&(Vec<launchpad_core::vault::VaultEntry>, String, String)>) -> SetupItem {
    let mut it = item("vault_git", "Git identity of agents", Importance::Required, "Agents commit with this name and email on every machine; the jail never reads your Git configuration.");
    let Some((_, name, email)) = vault else {
        it.state = ItemState::Unverified;
        it.found.push(found("Vault", UNREACHABLE, None));
        return it;
    };
    let source = Some("Home: Settings → Credentials and tools".to_string());
    if name.trim().is_empty() || email.trim().is_empty() {
        it.missing.push(part("identity", "Set the Git identity of agents", Some("Name and email for agents' commits, on Home: Settings → Credentials and tools.".into()), None, false));
    } else {
        it.state = ItemState::Ready;
    }
    if !name.trim().is_empty() { it.found.push(found("Name", name.clone(), source.clone())); }
    if !email.trim().is_empty() { it.found.push(found("Email", email.clone(), source)); }
    it
}

// ---------------------------------------------------------------- Checklist

fn required_left(items: &[SetupItem]) -> u32 {
    items.iter().filter(|i| i.importance == Importance::Required && i.state == ItemState::Missing).count() as u32
}

/// Every item of this runner. A Local VM contains its agents itself: no jail item.
pub async fn checklist(app: &App) -> Checklist {
    let env = app.env.clone();
    let vm = app.config.get().runner.boundary == launchpad_core::config::Boundary::Vm;
    let (jail_item, capture, vault) = tokio::join!(async { if vm { None } else { Some(jail(&env).await) } }, netcap(), vault_view());
    let mut items = vec![home(app)];
    items.extend(jail_item);
    items.push(capture);
    items.push(harnesses(app));
    items.push(vault_logins(app, vault.as_ref().map(|v| v.0.as_slice())));
    items.push(vault_git(vault.as_ref()));
    let left = required_left(&items);
    Checklist { items, required_left: left }
}

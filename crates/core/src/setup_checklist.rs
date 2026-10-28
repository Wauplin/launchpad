//! Setup checklist (`/setup`, onboarding plan section 8): what this computer or an SSH host already has, and
//! only the missing parts. "Detect first, never redo": every check only reads (version and status commands,
//! Git config reads, file existence, `ssh -T` in batch mode). Any working method counts and is shown with its
//! source; fixes are fixed commands the owner starts in a visible terminal, offered only for missing parts.
//! Notifications are a browser permission, checked by the UI.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use ts_rs::TS;

use crate::App;
use crate::shellenv::UserEnv;
use crate::signin::{SignInCheck, status_output};

#[derive(Clone, Copy, Debug, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Ready,
    /// Configured, but proving it works would need a side effect (a push). Counts as done.
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

/// One missing part, with the fixed command that adds it (when there is one).
#[derive(Clone, Debug, Serialize, TS)]
pub struct MissingPart {
    /// `install`, `sign_in`, `scopes`, `user.name`, `user.email`, `access_gh`, `access_ssh`, `enable`, `register`, …
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    /// `POST /api/setup/fix/{item}` can run `command` in a setup terminal.
    pub runnable: bool,
    /// The command asks for the owner's password (sudo): shown with Copy and Run in terminal.
    pub sudo: bool,
}

/// A signed-in harness login without a profile: "Create a profile from this login".
#[derive(Clone, Debug, Serialize, TS)]
pub struct LoginSuggestion {
    pub harness: String,
    pub name: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct SetupItem {
    /// `github`, `git`, `profile`, `folder`, `presets`, `devices`, `huggingface`, `hosts` or `extension`.
    pub id: String,
    pub title: String,
    pub importance: Importance,
    pub state: ItemState,
    pub why: String,
    pub found: Vec<Found>,
    pub missing: Vec<MissingPart>,
    /// Profile item only.
    pub logins: Vec<LoginSuggestion>,
    /// First folder item only: Git repositories of the folders root that are not registered yet.
    pub suggestions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct Checklist {
    /// SSH host, or null for this computer.
    pub host: Option<String>,
    pub items: Vec<SetupItem>,
    /// Required items that are neither ready nor unverified.
    pub required_left: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ChecklistSummary {
    pub required_left: u32,
}

fn item(id: &str, title: &str, importance: Importance, why: &str) -> SetupItem {
    SetupItem { id: id.into(), title: title.into(), importance, state: ItemState::Missing, why: why.into(), found: vec![], missing: vec![], logins: vec![], suggestions: vec![] }
}

fn found(label: &str, value: impl Into<String>, source: Option<String>) -> Found {
    Found { label: label.into(), value: value.into(), source }
}

fn part(id: &str, label: &str, detail: Option<String>, command: Option<String>, runnable: bool, sudo: bool) -> MissingPart {
    MissingPart { id: id.into(), label: label.into(), detail, command, runnable, sudo }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    Linux,
    Mac,
    Windows,
}

fn os(app: &App, host: Option<&str>) -> Os {
    match host {
        Some(h) => match app.hosts.status(h).and_then(|s| s.info).map(|i| i.os) {
            Some(o) if o == "Darwin" => Os::Mac,
            Some(o) if o.starts_with("Windows") || o.starts_with("MINGW") => Os::Windows,
            _ => Os::Linux,
        },
        None if cfg!(target_os = "macos") => Os::Mac,
        None if cfg!(windows) => Os::Windows,
        None => Os::Linux,
    }
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

/// Replaces the home folder with `~` in displayed paths.
fn tilde(path: &str, home: Option<&str>) -> String {
    match home.map(|h| h.trim_end_matches(['/', '\\'])).filter(|h| !h.is_empty()) {
        Some(h) if path == h => "~".into(),
        Some(h) if path.starts_with(h) && path[h.len()..].starts_with(['/', '\\']) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

fn home_of(app: &App, host: Option<&str>) -> Option<String> {
    match host {
        Some(h) => app.hosts.home(h),
        None => dirs::home_dir().map(|p| p.to_string_lossy().into_owned()),
    }
}

/// Runs a command; `None` when it is missing or timed out.
async fn run(env: &UserEnv, program: &str, args: &[&str]) -> Option<(bool, String, String)> {
    status_output(env, program, args).await.ok()
}

// ---------------------------------------------------------------- GitHub CLI

/// Scopes of `gh auth status` (`- Token scopes: 'gist', 'read:org', 'repo'`); `None` when not printed
/// (fine-grained or environment tokens).
pub fn parse_scopes(status: &str) -> Option<Vec<String>> {
    let line = status.lines().find_map(|l| l.trim().trim_start_matches(['-', '✓', ' ']).strip_prefix("Token scopes:"))?;
    Some(line.split(',').map(|s| s.trim().trim_matches(['\'', '"']).to_string()).filter(|s| !s.is_empty() && s != "none").collect())
}

/// Required scopes the token lacks. `repo` also grants notifications; `write:org`/`admin:org` include `read:org`.
pub fn missing_scopes(scopes: &[String]) -> Vec<&'static str> {
    let has = |s: &str| scopes.iter().any(|x| x == s);
    let mut out = vec![];
    if !has("repo") { out.push("repo"); }
    if !(has("read:org") || has("write:org") || has("admin:org")) { out.push("read:org"); }
    if !(has("notifications") || has("repo")) { out.push("notifications"); }
    out
}

fn gh_install(os: Os) -> MissingPart {
    match os {
        Os::Mac => part("install", "Install the GitHub CLI", Some("With Homebrew.".into()), Some("brew install gh".into()), true, false),
        Os::Windows => part("install", "Install the GitHub CLI", Some("With winget.".into()), Some("winget install --id GitHub.cli".into()), true, false),
        Os::Linux => part(
            "install",
            "Install the GitHub CLI",
            Some("Ubuntu and Debian: the official apt repository (asks for your password). Other systems: https://github.com/cli/cli#installation".into()),
            Some(r#"(type -p wget >/dev/null || (sudo apt update && sudo apt install wget -y)) && sudo mkdir -p -m 755 /etc/apt/keyrings && out=$(mktemp) && wget -nv -O$out https://cli.github.com/packages/githubcli-archive-keyring.gpg && cat $out | sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null && sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg && sudo mkdir -p -m 755 /etc/apt/sources.list.d && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null && sudo apt update && sudo apt install gh -y"#.into()),
            true,
            true,
        ),
    }
}

/// GitHub CLI state, also used by the Git item (gh credential helper, `gh auth setup-git`).
struct Gh {
    installed: bool,
    signed_in: bool,
    account: Option<String>,
}

async fn github(app: &App, env: &UserEnv, host: Option<&str>) -> (SetupItem, Gh) {
    let mut it = item("github", "GitHub CLI", Importance::Required, "The Inbox, issues, pull requests, Clone from GitHub and publication all use gh.");
    let mut gh = Gh { installed: false, signed_in: false, account: None };
    let Some((true, out, _)) = run(env, "gh", &["--version"]).await else {
        it.missing.push(gh_install(os(app, host)));
        return (it, gh);
    };
    gh.installed = true;
    let version = first_line(&out);
    it.found.push(found("Version", version.trim_start_matches("gh version ").to_string(), Some("gh --version".into())));
    match run(env, "gh", &["auth", "status", "--hostname", "github.com"]).await {
        Some((ok, out, err)) => {
            // The token itself is masked by gh; only the account, its storage and scopes are read.
            let text = format!("{out}\n{err}");
            let account_line = text.lines().find_map(|l| l.split_once("Logged in to github.com account ").map(|(_, rest)| rest.trim().to_string()));
            if ok {
                gh.signed_in = true;
                let (account, storage) = match account_line.as_deref().and_then(|a| a.split_once(' ')) {
                    Some((a, storage)) => (a.to_string(), Some(storage.trim_matches(['(', ')']).to_string())),
                    None => (account_line.clone().unwrap_or_else(|| "signed in".into()), None),
                };
                gh.account = Some(account.clone());
                it.found.push(found("Account", account, Some(match storage { Some(s) => format!("gh auth status · token in {s}"), None => "gh auth status".into() })));
                match parse_scopes(&text) {
                    Some(scopes) => {
                        let missing = missing_scopes(&scopes);
                        it.found.push(found("Token scopes", if scopes.is_empty() { "none".into() } else { scopes.join(", ") }, None));
                        if missing.is_empty() {
                            it.state = ItemState::Ready;
                        } else {
                            let list = missing.join(",");
                            it.missing.push(part("scopes", &format!("Add the {} scope{}", missing.join(", "), if missing.len() > 1 { "s" } else { "" }), Some("gh asks GitHub for the extra permission in your browser.".into()), Some(format!("gh auth refresh --hostname github.com --scopes {list}")), true, false));
                        }
                    }
                    // Fine-grained or environment tokens print no scopes: nothing to prove without a request.
                    None => it.state = ItemState::Unverified,
                }
            } else {
                it.missing.push(part("sign_in", "Sign in to GitHub", Some("Opens github.com in your browser with a one-time code.".into()), Some("gh auth login --hostname github.com --web".into()), true, false));
            }
        }
        None => it.missing.push(part("sign_in", "Sign in to GitHub", None, Some("gh auth login --hostname github.com --web".into()), true, false)),
    }
    (it, gh)
}

// ---------------------------------------------------------------- Git

fn git_install(os: Os) -> MissingPart {
    match os {
        Os::Mac => part("install", "Install Git", Some("Installs the Xcode command line tools, which include Git.".into()), Some("xcode-select --install".into()), true, false),
        Os::Windows => part("install", "Install Git", Some("With winget.".into()), Some("winget install --id Git.Git -e".into()), true, false),
        Os::Linux => part("install", "Install Git", Some("Ubuntu and Debian (asks for your password).".into()), Some("sudo apt update && sudo apt install git -y".into()), true, true),
    }
}

/// `git config --show-origin --get <key>`: (value, origin) with the `file:` prefix removed.
async fn git_key(env: &UserEnv, key: &str) -> Option<(String, String)> {
    let (ok, out, _) = run(env, "git", &["config", "--show-origin", "--get", key]).await?;
    if !ok { return None; }
    let line = out.lines().next()?;
    let (origin, value) = line.split_once('\t')?;
    Some((value.trim().to_string(), origin.trim_start_matches("file:").to_string()))
}

/// `ssh -v -T git@github.com` output: the accepted key and the GitHub account, when it authenticated.
pub fn parse_ssh(output: &str) -> Option<(Option<String>, String)> {
    let account = output.lines().find_map(|l| l.trim().strip_prefix("Hi ").and_then(|r| r.split_once('!')).map(|(a, _)| a.to_string()))?;
    let key = output.lines().find_map(|l| l.split_once("Server accepts key: ").map(|(_, r)| r.split_whitespace().next().unwrap_or("").to_string())).filter(|k| !k.is_empty());
    Some((key, account))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

async fn git(app: &App, env: &UserEnv, host: Option<&str>, gh: &Gh) -> SetupItem {
    let mut it = item("git", "Git", Importance::Required, "Agents commit in worktrees; Publish pushes branches to GitHub.");
    let Some((true, out, _)) = run(env, "git", &["--version"]).await else {
        it.missing.push(git_install(os(app, host)));
        return it;
    };
    let home = home_of(app, host);
    let home = home.as_deref();
    it.found.push(found("Version", first_line(&out).trim_start_matches("git version ").to_string(), Some("git --version".into())));
    let (name, email) = tokio::join!(git_key(env, "user.name"), git_key(env, "user.email"));
    match &name {
        Some((v, origin)) => it.found.push(found("Name", v.clone(), Some(format!("from {}", tilde(origin, home))))),
        None => it.missing.push(part("user.name", "Your name for commits", Some("Saved with git config --global user.name.".into()), None, false, false)),
    }
    match &email {
        Some((v, origin)) => it.found.push(found("Email", v.clone(), Some(format!("from {}", tilde(origin, home))))),
        None => it.missing.push(part("user.email", "Your email for commits", Some("Saved with git config --global user.email.".into()), None, false, false)),
    }

    // GitHub access: the protocol of registered GitHub remotes decides which method is checked first.
    let projects = crate::projects::list(app).await.unwrap_or_default();
    let remotes: Vec<&str> = projects.iter().filter(|p| crate::hosts::loc::host_of(&p.path) == host).filter_map(|p| p.remote_url.as_deref()).filter(|u| u.contains("github.com")).collect();
    let https = remotes.iter().filter(|u| u.starts_with("https://")).count();
    let ssh_remotes = remotes.len() - https;
    if !remotes.is_empty() {
        it.found.push(found("Folder remotes", format!("{ssh_remotes} SSH, {https} HTTPS"), Some("registered folders".into())));
    }
    let ssh = async {
        let out = run(env, "ssh", &["-v", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "StrictHostKeyChecking=yes", "git@github.com"]).await;
        out.and_then(|(_, o, e)| parse_ssh(&format!("{o}\n{e}")))
    };
    let helpers = async {
        let mut list = vec![];
        for key in ["credential.helper", "credential.https://github.com.helper"] {
            if let Some((true, out, _)) = run(env, "git", &["config", "--show-origin", "--get-all", key]).await {
                list.extend(out.lines().filter_map(|l| l.split_once('\t')).filter(|(_, v)| !v.trim().is_empty()).map(|(o, v)| (v.trim().to_string(), o.trim_start_matches("file:").to_string())));
            }
        }
        list
    };
    let (ssh, helpers) = tokio::join!(ssh, helpers);
    let ssh_found = ssh.map(|(key, account)| found("GitHub access", format!("SSH key {} authenticates as {account}", key.map(|k| tilde(&k, home)).unwrap_or_else(|| "(from the SSH agent)".into())), Some("ssh -T git@github.com".into())));
    let gh_helper = helpers.iter().find(|(v, _)| v.contains("gh auth git-credential")).cloned();
    let helper_found = match (&gh_helper, helpers.first()) {
        (Some((_, origin)), _) if gh.signed_in => Some((ItemState::Ready, found("GitHub access", format!("GitHub CLI credential helper{}", gh.account.as_ref().map(|a| format!(" · signed in as {a}")).unwrap_or_default()), Some(format!("from {}", tilde(origin, home)))))),
        (_, Some((v, origin))) => Some((ItemState::Unverified, found("GitHub access", format!("Credential helper {v} · Configured, not verified"), Some(format!("from {}", tilde(origin, home)))))),
        _ => None,
    };
    let access = if https > ssh_remotes {
        helper_found.or(ssh_found.map(|f| (ItemState::Ready, f)))
    } else {
        ssh_found.map(|f| (ItemState::Ready, f)).or(helper_found)
    };
    let access_state = match access {
        Some((state, f)) => {
            it.found.push(f);
            state
        }
        None => {
            if gh.signed_in {
                it.missing.push(part("access_gh", "Use GitHub CLI for Git", Some("Git then signs in to GitHub with your gh login (HTTPS).".into()), Some("gh auth setup-git".into()), true, false));
            }
            let os = os(app, host);
            if os != Os::Windows {
                let comment = email.as_ref().map(|(e, _)| format!(" -C {}", shell_quote(e))).unwrap_or_default();
                // Never overwrite an existing key: the command stops when the file exists.
                let add = if gh.signed_in { " && gh ssh-key add ~/.ssh/id_ed25519.pub --title Launchpad" } else { "" };
                let command = format!("if [ -e ~/.ssh/id_ed25519 ]; then echo '~/.ssh/id_ed25519 already exists and was not changed.'{}; else ssh-keygen -t ed25519{comment} -f ~/.ssh/id_ed25519{add}; fi", if gh.signed_in { "; echo 'Add ~/.ssh/id_ed25519.pub to GitHub: gh ssh-key add ~/.ssh/id_ed25519.pub'" } else { "" });
                it.missing.push(part("access_ssh", "Use an SSH key", Some(format!("Creates ~/.ssh/id_ed25519 (you choose a passphrase){}.", if gh.signed_in { " and adds it to your GitHub account" } else { "; add the .pub file in GitHub → Settings → SSH keys" })), Some(command), true, false));
            }
            if !gh.signed_in && os == Os::Windows {
                it.missing.push(part("access_gh", "Sign in to the GitHub CLI first", Some("Then Git can use it: gh auth setup-git.".into()), None, false, false));
            }
            ItemState::Missing
        }
    };
    it.state = if name.is_none() || email.is_none() { ItemState::Missing } else { access_state };
    it
}

// ---------------------------------------------------------------- Agent profile

async fn profile(app: &App, host: Option<&str>) -> SetupItem {
    let mut it = item("profile", "Agent profile", Importance::Required, "A profile says which agent runs and how it is paid. No profile, no agent.");
    let checks = crate::signin::check_all_on(app, host).await;
    let profiles = app.config.get().profiles;
    let default_check = |harness: &str| checks.iter().find(|c| c.profile.is_none() && c.id == harness);
    let state_of = |p: &crate::config::Profile| -> Option<&SignInCheck> {
        if p.env.is_empty() { default_check(p.harness.as_str()) } else { checks.iter().find(|c| c.profile.as_deref() == Some(&p.id)) }
    };
    let mut any_ready = false;
    let mut any_unknown = false;
    for p in &profiles {
        let c = state_of(p);
        let (value, ready) = match c {
            Some(c) if !c.cli_found => (format!("{} is not installed", p.harness.display()), Some(false)),
            Some(c) if c.signed_in == Some(true) => (format!("signed in{}", c.detail.as_deref().map(|d| format!(" · {d}")).unwrap_or_default()), Some(true)),
            Some(c) if c.signed_in == Some(false) => ("not signed in".into(), Some(false)),
            _ => ("sign-in unknown".into(), None),
        };
        match ready {
            Some(true) => any_ready = true,
            None => any_unknown = true,
            Some(false) => it.missing.push(part(&format!("sign_in:{}", p.id), &format!("Sign in {}", p.name), Some(value.clone()), None, false, false)),
        }
        it.found.push(found(&p.name, value, Some(format!("profile {} · {}", p.id, p.harness.display()))));
    }
    // Signed-in default logins that no profile uses yet: one click creates a profile, without a new sign-in.
    for c in checks.iter().filter(|c| c.profile.is_none() && c.id != "github" && c.signed_in == Some(true)) {
        if profiles.iter().any(|p| p.harness.as_str() == c.id && p.env.is_empty()) { continue; }
        it.logins.push(LoginSuggestion { harness: c.id.clone(), name: c.name.clone(), detail: c.detail.clone() });
    }
    it.state = if any_ready { ItemState::Ready } else if any_unknown { ItemState::Unverified } else { ItemState::Missing };
    if profiles.is_empty() {
        it.missing.push(part("create", "Create a profile", Some(if it.logins.is_empty() { "No signed-in agent login was found.".into() } else { "Use a login found on this machine, or start from How do you pay for agents?".into() }), None, false, false));
    }
    if it.state != ItemState::Missing { it.missing.clear(); }
    it
}

// ---------------------------------------------------------------- First folder

async fn folder(app: &App, host: Option<&str>) -> SetupItem {
    let mut it = item("folder", "First folder", Importance::Required, "Agents run in folders: register one you work in.");
    let projects = crate::projects::list(app).await.unwrap_or_default();
    let mine: Vec<_> = projects.iter().filter(|p| !p.is_temp && crate::hosts::loc::host_of(&p.path) == host).collect();
    let home = home_of(app, host);
    for p in mine.iter().take(5) {
        it.found.push(found(&p.name, tilde(crate::hosts::loc::native(&p.path), home.as_deref()), p.gh_repo.clone()));
    }
    if mine.len() > 5 {
        it.found.push(found("More", format!("{} other folders", mine.len() - 5), None));
    }
    if !mine.is_empty() {
        it.state = ItemState::Ready;
        return it;
    }
    let root = match host {
        Some(h) => app.hosts.projects_root(h).ok(),
        None => Some(app.config.get().projects_root().to_string_lossy().into_owned()),
    };
    if let Some(root) = root
        && let Ok(entries) = crate::hostfs::read_dir(&root).await
    {
        let mut names: Vec<String> = entries.into_iter().filter(|e| e.kind == "dir" && !e.name.starts_with('.')).map(|e| e.name).collect();
        names.sort_by_key(|n| n.to_lowercase());
        for name in names.into_iter().take(300) {
            let path = format!("{}/{name}", root.trim_end_matches('/'));
            if crate::hostfs::exists(format!("{path}/.git")).await && !projects.iter().any(|p| p.path == path) {
                it.suggestions.push(path);
                if it.suggestions.len() >= 12 { break; }
            }
        }
    }
    it.missing.push(part("add", "Add a folder", Some(if it.suggestions.is_empty() { "Browse to a folder or clone a repository from GitHub.".into() } else { "Git repositories found in your folders root:".into() }), None, false, false));
    it
}

// ---------------------------------------------------------------- Optional items (this computer)

fn presets(app: &App) -> SetupItem {
    let mut it = item("presets", "Starter presets", Importance::Optional, "Reusable prompt instructions you add to any launch with one click.");
    let presets = app.config.get().presets;
    if presets.is_empty() {
        it.missing.push(part("templates", "Pick starter presets", Some("Write tests, Small commits, Open a Local PR when done. Edit them later in Settings → Presets.".into()), None, false, false));
    } else {
        it.state = ItemState::Ready;
        it.found.push(found("Presets", presets.iter().map(|p| p.label.as_str()).collect::<Vec<_>>().join(", "), Some("config.toml".into())));
    }
    it
}

/// Phone & other devices, built by the server from its front door (`remote`, `origin`) and Tailscale state.
pub fn devices(remote: bool, origin: &str, tailscale: Option<bool>) -> SetupItem {
    let mut it = item("devices", "Phone & other devices", Importance::Optional, "Answer agents and review work from your phone or another computer.");
    if remote {
        it.state = ItemState::Ready;
        it.found.push(found("Address", origin, Some(if origin.starts_with("https://") { "HTTPS front door".into() } else { "trusted network listener".into() })));
    } else {
        it.missing.push(part("pair", "Set up remote access", Some(match tailscale {
            Some(true) => "Tailscale is installed: Settings → Devices can create a private HTTPS route to Launchpad.".into(),
            _ => "Install Tailscale, then use Settings → Devices to create a private HTTPS route.".into(),
        }), None, false, false));
    }
    it
}

async fn huggingface(app: &App) -> SetupItem {
    let mut it = item("huggingface", "Hugging Face trace backup", Importance::Optional, "Keep agent traces in a private Hugging Face bucket.");
    let (check, account) = crate::trace_backup::check(&app.env).await;
    if !check.cli_found {
        let command = if cfg!(windows) { r#"powershell -ExecutionPolicy ByPass -c "irm https://hf.co/cli/install.ps1 | iex""# } else { "curl -LsSf https://hf.co/cli/install.sh | bash" };
        it.missing.push(part("install", "Install the hf CLI", Some("Other ways: brew install hf, or pip install -U huggingface_hub.".into()), Some(command.into()), true, false));
        return it;
    }
    match account {
        Some(a) => it.found.push(found("Account", a, Some("hf auth whoami".into()))),
        None => {
            it.missing.push(part("sign_in", "Sign in to Hugging Face", None, Some("hf auth login".into()), true, false));
            return it;
        }
    }
    let config = app.config.get().trace_backup;
    if config.enabled {
        it.state = ItemState::Ready;
        it.found.push(found("Backup", "enabled", Some("Settings → Hugging Face".into())));
    } else {
        it.missing.push(part("enable", "Turn on trace backup", Some("In Settings → Hugging Face.".into()), None, false, false));
    }
    it
}

fn hosts(app: &App) -> SetupItem {
    let mut it = item("hosts", "SSH machines", Importance::Optional, "Run agents on another computer you reach with SSH.");
    let list = app.hosts.snapshot();
    for h in &list {
        let state = match h.state {
            crate::hosts::LinkState::Connected => "connected",
            crate::hosts::LinkState::Connecting => "connecting",
            crate::hosts::LinkState::Unreachable => "unreachable",
            crate::hosts::LinkState::NeedsSetup => "needs setup",
        };
        it.found.push(found(&h.host.id, state, Some(h.host.destination.clone())));
    }
    if list.is_empty() {
        it.missing.push(part("add", "Add an SSH machine", Some("In Settings → Machines. Each machine then gets its own checks here.".into()), None, false, false));
    } else {
        it.state = ItemState::Ready;
    }
    it
}

fn native_host_manifest() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts/dev.launchpad.browser.json")
    } else if cfg!(windows) {
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Launchpad/NativeMessagingHosts/dev.launchpad.browser.json")
    } else {
        home.join(".config/google-chrome/NativeMessagingHosts/dev.launchpad.browser.json")
    })
}

async fn extension(app: &App) -> SetupItem {
    let mut it = item("extension", "Chrome extension", Importance::Optional, "Browser tasks: agents read and act in the tabs you share.");
    let home = home_of(app, None);
    match native_host_manifest() {
        Some(m) if tokio::fs::metadata(&m).await.is_ok() => {
            it.state = ItemState::Ready;
            it.found.push(found("Native host", "registered", Some(tilde(&m.to_string_lossy(), home.as_deref()))));
        }
        _ => {
            let source = std::path::Path::new(crate::SOURCE);
            let script = source.join("extension/register-native-host.py");
            let binary = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
            let (command, detail) = match (tokio::fs::metadata(&script).await.is_ok(), binary) {
                (true, Some(binary)) => {
                    let env = if app.paths.app_env.name() == "dev" { " --environment dev" } else { "" };
                    let python = if cfg!(windows) { "python" } else { "python3" };
                    (Some(format!("{python} {} --binary {}{env}", shell_quote(&script.to_string_lossy()), shell_quote(&binary))), "Build and load the extension first (extension/README.md), then register its native host.".to_string())
                }
                _ => (None, "Needs a Launchpad source checkout: see extension/README.md.".to_string()),
            };
            let runnable = command.is_some() && !cfg!(windows);
            it.missing.push(part("register", "Register the native host", Some(detail), command, runnable, false));
        }
    }
    it
}

// ---------------------------------------------------------------- Checklist

fn required_left(items: &[SetupItem]) -> u32 {
    items.iter().filter(|i| i.importance == Importance::Required && i.state == ItemState::Missing).count() as u32
}

static SUMMARY: Mutex<Option<(Instant, u32)>> = Mutex::new(None);
const SUMMARY_TTL: Duration = Duration::from_secs(300);

/// The required items of a machine: GitHub CLI, Git, Agent profile, First folder.
async fn required(app: &App, host: Option<&str>) -> Vec<SetupItem> {
    let env = app.env.on(host);
    let ((gh_item, gh), profile, folder) = tokio::join!(github(app, &env, host), profile(app, host), folder(app, host));
    let git = git(app, &env, host, &gh).await;
    vec![gh_item, git, profile, folder]
}

/// Every item of a machine. On this computer, `devices` (from the server's front door) is added too.
pub async fn checklist(app: &App, host: Option<&str>, devices: Option<SetupItem>) -> Checklist {
    let items = if host.is_some() {
        required(app, host).await
    } else {
        let (mut required, hf, ext) = tokio::join!(required(app, None), huggingface(app), extension(app));
        required.push(presets(app));
        required.extend(devices);
        required.push(hf);
        required.push(hosts(app));
        required.push(ext);
        required
    };
    let left = required_left(&items);
    if host.is_none() {
        *SUMMARY.lock().unwrap() = Some((Instant::now(), left));
    }
    Checklist { host: host.map(str::to_string), items, required_left: left }
}

/// One item, checked again (fixes refuse parts that are already set up).
pub async fn item_state(app: &App, host: Option<&str>, id: &str) -> Option<SetupItem> {
    let env = app.env.on(host);
    Some(match id {
        "github" => github(app, &env, host).await.0,
        "git" => {
            let (_, gh) = github(app, &env, host).await;
            git(app, &env, host, &gh).await
        }
        "profile" => profile(app, host).await,
        "folder" => folder(app, host).await,
        "huggingface" if host.is_none() => huggingface(app).await,
        "extension" if host.is_none() => extension(app).await,
        _ => return None,
    })
}

/// Required items left on this computer, cached for five minutes (the sidebar entry).
pub async fn summary(app: &App) -> ChecklistSummary {
    if let Some((at, left)) = *SUMMARY.lock().unwrap()
        && at.elapsed() < SUMMARY_TTL
    {
        return ChecklistSummary { required_left: left };
    }
    let left = required_left(&required(app, None).await);
    *SUMMARY.lock().unwrap() = Some((Instant::now(), left));
    ChecklistSummary { required_left: left }
}

/// Forgets the cached summary after a change (identity saved, profile created).
pub fn invalidate() {
    *SUMMARY.lock().unwrap() = None;
}

/// The fixed command of a missing part, or an error when the part is set up (or has no command).
pub async fn fix_command(app: &App, host: Option<&str>, item: &str, part_id: &str) -> anyhow::Result<(String, String)> {
    let it = item_state(app, host, item).await.ok_or_else(|| anyhow::anyhow!("no fix for {item}"))?;
    let p = it.missing.iter().find(|p| p.id == part_id).ok_or_else(|| anyhow::anyhow!("{} is already set up on this machine; nothing was changed", it.title))?;
    let command = p.command.clone().filter(|_| p.runnable).ok_or_else(|| anyhow::anyhow!("{} has no command to run", p.label))?;
    Ok((format!("{} · {}", it.title, p.label), command))
}

/// Sets only the missing identity keys with `git config --global`; refuses a key that is already set.
pub async fn set_identity(app: &App, host: Option<&str>, name: Option<&str>, email: Option<&str>) -> anyhow::Result<()> {
    let env = app.env.on(host);
    let mut keys = vec![];
    for (key, value) in [("user.name", name), ("user.email", email)] {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else { continue };
        anyhow::ensure!(!value.contains(['\n', '\r', '\0']) && value.chars().count() <= 200, "{key}: use one line of at most 200 characters");
        if key == "user.email" { anyhow::ensure!(value.contains('@'), "enter an email address"); }
        if let Some((existing, origin)) = git_key(&env, key).await {
            anyhow::bail!("{key} is already set to {existing} in {origin}; nothing was changed");
        }
        keys.push((key, value.to_string()));
    }
    anyhow::ensure!(!keys.is_empty(), "nothing to save");
    for (key, value) in keys {
        crate::procs::output(&env, "git", &["config", "--global", key, &value], None).await?;
    }
    invalidate();
    Ok(())
}

/// Creates a profile for a signed-in default login of `harness` (no new sign-in). Subscription billing
/// when the harness has monitored quotas, pay-as-you-go otherwise.
pub async fn profile_from_login(app: &App, harness: &str) -> anyhow::Result<crate::config::Profile> {
    let kind = crate::model::HarnessKind::parse(harness).ok_or_else(|| anyhow::anyhow!("unknown harness {harness}"))?;
    let item = profile(app, None).await;
    let login = item.logins.iter().find(|l| l.harness == harness).ok_or_else(|| anyhow::anyhow!("no signed-in {} login without a profile was found", kind.display()))?;
    let mut config = app.config.get();
    let base = harness.to_string();
    let id = (1..100).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| config.profile(id).is_none()).ok_or_else(|| anyhow::anyhow!("no free profile ID"))?;
    let descriptor = crate::harness::registration(kind).descriptor;
    let p = crate::config::Profile {
        id,
        name: login.name.clone(),
        harness: kind,
        env: Default::default(),
        model: None,
        effort: None,
        billing: if descriptor.capabilities.quota { crate::config::Billing::Subscription } else { crate::config::Billing::PayAsYouGo },
        budget_usd: None,
        models_url: None,
        spend_limits: vec![],
        send_credentials: false,
    };
    config.profiles.push(p.clone());
    config.validate()?;
    app.config.save(config)?;
    crate::hosts::preflight::recheck_all(app);
    invalidate();
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_and_ssh() {
        let status = "github.com\n  ✓ Logged in to github.com account demo (keyring)\n  - Token: gho_****\n  - Token scopes: 'gist', 'read:org', 'repo'\n";
        let scopes = parse_scopes(status).unwrap();
        assert_eq!(scopes, ["gist", "read:org", "repo"]);
        assert!(missing_scopes(&scopes).is_empty());
        assert_eq!(missing_scopes(&["gist".into()]), ["repo", "read:org", "notifications"]);
        assert_eq!(parse_scopes("  - Token scopes: none\n").unwrap(), Vec::<String>::new());
        assert!(parse_scopes("Logged in").is_none());
        let ssh = "debug1: Server accepts key: /home/u/.ssh/id_rsa RSA SHA256:x agent\nHi demo! You've successfully authenticated, but GitHub does not provide shell access.";
        assert_eq!(parse_ssh(ssh), Some((Some("/home/u/.ssh/id_rsa".into()), "demo".into())));
        assert_eq!(parse_ssh("Permission denied (publickey)."), None);
    }
}

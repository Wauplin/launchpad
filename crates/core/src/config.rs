//! `config.toml`: loading with defaults, watching, and rewriting from the Settings screen.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::Bus;
use crate::model::HarnessKind;
use crate::paths::Paths;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Discovery, temporary folders and worktrees. `~` is expanded.
    pub projects_root: String,
    /// Remove a clean pre-draft worktree after successful publication.
    pub remove_published_worktree: bool,
    pub server: ServerConfig,
    pub commands: Commands,
    pub github: GitHub,
    pub agents: Agents,
    pub worktrees: Worktrees,
    pub presets: Vec<Preset>,
    /// Named harness settings, selectable in the composer beside the base harnesses.
    pub profiles: Vec<Profile>,
}

/// Network front door. File edits require restart; pairing can apply verified HTTPS live.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub public_origin: Option<String>,
    /// Discover and listen on this machine's Tailscale address when no origin is configured.
    pub auto_tailnet: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1".into(), public_origin: None, auto_tailnet: true }
    }
}

impl ServerConfig {
    pub fn origin(&self) -> Result<Option<String>> {
        let Some(origin) = &self.public_origin else { return Ok(None) };
        let url = reqwest::Url::parse(origin)?;
        anyhow::ensure!(matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            && url.username().is_empty() && url.password().is_none() && url.path() == "/"
            && url.query().is_none() && url.fragment().is_none(),
            "server.public_origin must be an HTTP(S) origin without credentials, path, query or fragment");
        Ok(Some(url.origin().ascii_serialization()))
    }
}

/// A registered harness with its own environment, defaults and billing mode.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Profile {
    /// Stable ID stored in launch specs: lowercase letters, digits and dashes.
    pub id: String,
    pub name: String,
    pub harness: HarnessKind,
    /// Added to the user environment of the harness process, its terminal TUI and title requests.
    /// A leading `~/` in a value is expanded.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub billing: Billing,
    /// Default cost limit (USD) for new sessions of this profile.
    #[serde(default)]
    pub budget_usd: Option<f64>,
    /// OpenAI-compatible model list (`GET …/v1/models`) offered instead of the base harness models,
    /// for profiles pointing the harness at another provider.
    #[serde(default)]
    pub models_url: Option<String>,
    /// Rolling spending limits of a pay-as-you-go profile, summed over all its sessions.
    #[serde(default)]
    pub spend_limits: Vec<SpendLimit>,
    /// On SSH hosts, resolve `${NAME}` references on this computer and send the values with the process
    /// start. Otherwise the host resolves them from its own login shell. Literal values are always sent.
    #[serde(default)]
    pub send_credentials: bool,
}

/// At most `usd` of reported cost in any `hours`-long rolling window.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct SpendLimit {
    pub usd: f64,
    pub hours: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// Included subscription quota, protected with the base harness quota monitor (same account only).
    #[default]
    Subscription,
    /// Billed per use: no subscription quota protection; cost limits apply.
    PayAsYouGo,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Commands {
    pub browser: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct GitHub {
    pub poll_interval_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Agents {
    /// Notifications for completion, automatic stops and automatic quota resumption.
    pub notify: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Worktrees {
    /// Clean task worktrees of idle sessions kept on disk; older ones are removed and restored on resume. 0: keep all.
    pub keep: u32,
}

impl Default for Worktrees {
    fn default() -> Self {
        Worktrees { keep: 15 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub text: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            projects_root: "~/projects".into(),
            remove_published_worktree: false,
            server: ServerConfig::default(),
            commands: Commands::default(),
            github: GitHub::default(),
            agents: Agents::default(),
            worktrees: Worktrees::default(),
            profiles: vec![],
            presets: vec![],
        }
    }
}

impl Default for Commands {
    fn default() -> Self {
        Commands { browser: if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer.exe" } else { "xdg-open" }.into() }
    }
}

impl Default for GitHub {
    fn default() -> Self {
        GitHub { poll_interval_seconds: 60 }
    }
}

impl Default for Agents {
    fn default() -> Self {
        Agents { notify: true }
    }
}

/// Written on first start. Missing keys in a user file fall back to these values.
/// Presets are user-defined only; missing presets and `presets = []` both mean none.
pub const DEFAULT_CONFIG: &str = r#"projects_root = "~/projects"        # discovery, temporary folders, worktrees
remove_published_worktree = false   # keep local PR worktrees after publication
presets = []                       # add your own prompt instructions in Settings

[server]
bind = "127.0.0.1"                 # local desktop and CLI listener
auto_tailnet = true               # also listen on a discovered Tailscale IP
# public_origin = "https://launchpad.example.com"  # restart daemon after changes

[commands]
browser = "xdg-open"

[github]
poll_interval_seconds = 60          # GitHub's X-Poll-Interval wins when larger

[agents]
notify = true                       # completion, automatic stops and automatic quota resumption

[worktrees]
keep = 15                           # clean task worktrees of idle agents kept on disk (0: keep all)

"#;

impl Config {
    pub fn projects_root(&self) -> PathBuf {
        crate::paths::expand_home(&self.projects_root)
    }

    pub fn validate(&self) -> Result<()> {
        self.server.bind.parse::<std::net::IpAddr>().context("server.bind must be an IP address")?;
        self.server.origin()?;
        let mut ids = std::collections::HashSet::new();
        for v in &self.profiles {
            anyhow::ensure!(
                !v.id.is_empty() && v.id.len() <= 40 && v.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "profile ID {:?}: use 1 to 40 lowercase letters, digits or dashes", v.id
            );
            anyhow::ensure!(ids.insert(v.id.as_str()), "profile ID {:?} is used twice", v.id);
            anyhow::ensure!(!v.name.trim().is_empty() && v.name.chars().count() <= 40, "profile {:?}: use a name of 1 to 40 characters", v.id);
            for (key, value) in &v.env {
                anyhow::ensure!(
                    !key.is_empty() && !key.starts_with(|c: char| c.is_ascii_digit()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "profile {:?}: invalid environment variable name {key:?}", v.id
                );
                anyhow::ensure!(!value.contains('\0'), "profile {:?}: {key} contains a null byte", v.id);
                anyhow::ensure!(!key.starts_with("LAUNCHPAD_"), "profile {:?}: {key} is set by Launchpad", v.id);
                crate::profiles::references(value).with_context(|| format!("profile {:?}: {key}", v.id))?;
            }
            if let Some(url) = &v.models_url {
                anyhow::ensure!(reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")),
                    "profile {:?}: the model list must be an HTTP(S) URL", v.id);
            }
            if let Some(b) = v.budget_usd {
                anyhow::ensure!(b.is_finite() && b > 0.0, "profile {:?}: the cost limit must be a positive amount", v.id);
                anyhow::ensure!(crate::harness::registration(v.harness).descriptor.capabilities.cost,
                    "profile {:?}: {} does not report costs, so a cost limit cannot apply", v.id, v.harness.display());
            }
            anyhow::ensure!(v.billing == Billing::PayAsYouGo || crate::harness::registration(v.harness).descriptor.capabilities.quota,
                "profile {:?}: {} has no monitored subscription, so use pay as you go", v.id, v.harness.display());
            if !v.spend_limits.is_empty() {
                anyhow::ensure!(v.billing == Billing::PayAsYouGo, "profile {:?}: spending limits apply only to pay-as-you-go billing", v.id);
                anyhow::ensure!(crate::harness::registration(v.harness).descriptor.capabilities.cost,
                    "profile {:?}: {} does not report costs, so a spending limit cannot apply", v.id, v.harness.display());
                anyhow::ensure!(v.spend_limits.len() <= 4, "profile {:?}: use at most 4 spending limits", v.id);
                for l in &v.spend_limits {
                    anyhow::ensure!(l.usd.is_finite() && l.usd > 0.0, "profile {:?}: a spending limit must be a positive amount", v.id);
                    anyhow::ensure!((1..=crate::spend::MAX_HOURS).contains(&l.hours), "profile {:?}: a spending limit window must be 1 hour to 31 days", v.id);
                }
            }
        }
        Ok(())
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|v| v.id == id)
    }
}

/// The current config, reloaded when the file changes.
#[derive(Clone)]
pub struct ConfigHandle {
    inner: Arc<RwLock<Config>>,
    path: PathBuf,
    bus: Bus,
    /// Set when the last load failed (the previous config stays in use).
    error: Arc<RwLock<Option<String>>>,
}

impl ConfigHandle {
    pub fn load(paths: &Paths, bus: Bus) -> Result<ConfigHandle> {
        let path = paths.config_file();
        if !path.exists() {
            std::fs::write(&path, DEFAULT_CONFIG.replace(r#"browser = "xdg-open""#, &format!("browser = {:?}", Commands::default().browser))).with_context(|| format!("write {}", path.display()))?;
        }
        let (config, error) = match read(&path) {
            Ok(c) => (c, None),
            Err(e) => {
                tracing::error!("config: {e:#}, using defaults");
                (Config::default(), Some(browser_error(&e)))
            }
        };
        let handle = ConfigHandle {
            inner: Arc::new(RwLock::new(config)),
            path,
            bus,
            error: Arc::new(RwLock::new(error)),
        };
        handle.watch();
        Ok(handle)
    }

    pub fn get(&self) -> Config {
        self.inner.read().unwrap().clone()
    }

    pub fn error(&self) -> Option<String> {
        self.error.read().unwrap().clone()
    }

    /// Rewrites the file (comments are lost) and applies the new config at once.
    pub fn save(&self, config: Config) -> Result<()> {
        config.validate()?;
        let text = toml::to_string_pretty(&config)?;
        let tmp = self.path.with_extension("toml.tmp");
        // Restrict the temporary file before writing profile credentials.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        // Profile environments can hold API keys.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        use std::io::Write;
        file.write_all(text.as_bytes())?;
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        self.apply(Ok(config));
        Ok(())
    }

    fn apply(&self, loaded: Result<Config>) {
        match loaded {
            Ok(c) => {
                let changed = *self.inner.read().unwrap() != c;
                *self.inner.write().unwrap() = c;
                *self.error.write().unwrap() = None;
                if changed {
                    tracing::info!("config reloaded");
                    self.bus.publish("config", "changed", serde_json::Value::Null);
                }
            }
            Err(e) => {
                tracing::error!("config: {e:#}, keeping the previous config");
                *self.error.write().unwrap() = Some(browser_error(&e));
                self.bus.publish("config", "error", serde_json::json!(browser_error(&e)));
            }
        }
    }

    /// Watches the config directory (editors replace the file, so the file itself is not watched).
    fn watch(&self) {
        let handle = self.clone();
        let dir = self.path.parent().unwrap().to_path_buf();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| p.file_name().is_some_and(|n| n == "config.toml")) {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("config watch disabled: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            tracing::warn!("config watch disabled: {e}");
            return;
        }
        tokio::spawn(async move {
            let _watcher = watcher;
            while rx.recv().await.is_some() {
                // Debounce bursts of events (write, rename, chmod).
                tokio::time::sleep(Duration::from_millis(150)).await;
                while rx.try_recv().is_ok() {}
                if handle.path.exists() {
                    handle.apply(read(&handle.path));
                }
            }
        });
    }
}

// TOML diagnostics include source lines, which may contain profile credentials.
// Keep full diagnostics in host logs only, including for config bus events.
fn browser_error(error: &anyhow::Error) -> String {
    if let Some(parse) = error.downcast_ref::<toml::de::Error>() {
        let location = parse.span().map(|span| format!(" at byte {}", span.start + 1)).unwrap_or_default();
        format!("Invalid config.toml{location}. See the host logs for details.")
    } else {
        "Could not load config.toml. See the host logs for details.".into()
    }
}

fn read(path: &PathBuf) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let config: Config = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    config.validate().with_context(|| format!("check {}", path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parse_errors_never_expose_source_in_browser_diagnostics() {
        let error = toml::from_str::<Config>("[profiles.env]\nSECRET = 'private-value' broken").unwrap_err();
        assert!(error.to_string().contains("private-value"));
        let error = anyhow::Error::new(error).context("parse config.toml");
        let safe = browser_error(&error);
        assert!(!safe.contains("private-value"));
        assert!(safe.contains("at byte"));
        let bus = Bus::new();
        let mut events = bus.subscribe(None).rx;
        let handle = ConfigHandle {
            inner: Arc::new(RwLock::new(Config::default())),
            path: PathBuf::from("unused"),
            bus,
            error: Arc::new(RwLock::new(None)),
        };
        handle.apply(Err(error));
        assert_eq!(handle.error().unwrap(), safe);
        assert!(!events.try_recv().unwrap().text.contains("private-value"));
    }

    #[test]
    fn presets_are_only_user_defined() {
        assert!(Config::default().presets.is_empty());
        for text in ["", "presets = []", DEFAULT_CONFIG] {
            let config: Config = toml::from_str(text).unwrap();
            assert!(config.presets.is_empty());
        }
        let config: Config = toml::from_str(
            r#"
[[presets]]
id = "draft-pr"
label = "My review"
text = "Review the changes."
"#,
        )
        .unwrap();
        assert_eq!(
            config.presets,
            vec![Preset {
                id: "draft-pr".into(),
                label: "My review".into(),
                text: "Review the changes.".into(),
            }]
        );
    }
}

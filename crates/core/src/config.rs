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
    /// UI zoom position: 0..4 (100%, 110%, 120%, 135%, 150%).
    pub zoom: u8,
    /// Remove a clean pre-draft worktree after successful publication.
    pub remove_published_worktree: bool,
    pub commands: Commands,
    pub github: GitHub,
    pub agents: Agents,
    pub presets: Vec<Preset>,
    /// Named harness settings, selectable in the composer beside the base harnesses.
    pub variants: Vec<HarnessVariant>,
}

/// A registered harness with its own environment, defaults and billing mode.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct HarnessVariant {
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
    /// Default cost limit (USD) for new sessions of this variant.
    #[serde(default)]
    pub budget_usd: Option<f64>,
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
    pub editor: String,
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
    /// Desktop notification when a session becomes done.
    pub notify: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub text: String,
}

impl Default for Config {
    fn default() -> Self {
        let preset = |id: &str, label: &str, text: &str| Preset { id: id.into(), label: label.into(), text: text.into() };
        Config {
            projects_root: "~/projects".into(),
            zoom: 2,
            remove_published_worktree: false,
            commands: Commands::default(),
            github: GitHub::default(),
            agents: Agents::default(),
            variants: vec![],
            presets: vec![
                preset("worktree", "worktree", "Please use a local git worktree and delete it once you're done."),
                preset("no-unit-test", "no unit test", "No need to add any unit tests for that PR."),
            ],
        }
    }
}

impl Default for Commands {
    fn default() -> Self {
        Commands { editor: "code".into(), browser: if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer.exe" } else { "xdg-open" }.into() }
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

/// Written on first start. Missing keys in a user file fall back to these values
/// (`presets` included: write `presets = []` to have none).
pub const DEFAULT_CONFIG: &str = r#"zoom = 2                           # 100%, 110%, 120% (default), 135%, 150%
projects_root = "~/projects"        # discovery, temporary folders, worktrees
remove_published_worktree = false   # keep local PR worktrees after publication

[commands]
editor = "code"
browser = "xdg-open"

[github]
poll_interval_seconds = 60          # GitHub's X-Poll-Interval wins when larger

[agents]
notify = true                       # desktop notification when a session becomes done

[[presets]]
id = "worktree"
label = "worktree"
text = "Please use a local git worktree and delete it once you're done."

[[presets]]
id = "no-unit-test"
label = "no unit test"
text = "No need to add any unit tests for that PR."
"#;

impl Config {
    pub fn projects_root(&self) -> PathBuf {
        crate::paths::expand_home(&self.projects_root)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.zoom <= 4, "zoom must be between 0 and 4");
        let mut ids = std::collections::HashSet::new();
        for v in &self.variants {
            anyhow::ensure!(
                !v.id.is_empty() && v.id.len() <= 40 && v.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "variant ID {:?}: use 1 to 40 lowercase letters, digits or dashes", v.id
            );
            anyhow::ensure!(ids.insert(v.id.as_str()), "variant ID {:?} is used twice", v.id);
            anyhow::ensure!(!v.name.trim().is_empty() && v.name.chars().count() <= 40, "variant {:?}: use a name of 1 to 40 characters", v.id);
            for (key, value) in &v.env {
                anyhow::ensure!(
                    !key.is_empty() && !key.starts_with(|c: char| c.is_ascii_digit()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "variant {:?}: invalid environment variable name {key:?}", v.id
                );
                anyhow::ensure!(!value.contains('\0'), "variant {:?}: {key} contains a null byte", v.id);
                anyhow::ensure!(!key.starts_with("LAUNCHPAD_"), "variant {:?}: {key} is set by Launchpad", v.id);
            }
            if let Some(b) = v.budget_usd {
                anyhow::ensure!(b.is_finite() && b > 0.0, "variant {:?}: the cost limit must be a positive amount", v.id);
                anyhow::ensure!(crate::harness::registration(v.harness).descriptor.capabilities.cost,
                    "variant {:?}: {} does not report costs, so a cost limit cannot apply", v.id, v.harness.display());
            }
        }
        Ok(())
    }

    pub fn variant(&self, id: &str) -> Option<&HarnessVariant> {
        self.variants.iter().find(|v| v.id == id)
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
                (Config::default(), Some(format!("{e:#}")))
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
        // Restrict the temporary file before writing variant credentials.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        // Variant environments can hold API keys.
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
                *self.error.write().unwrap() = Some(format!("{e:#}"));
                self.bus.publish("config", "error", serde_json::json!(format!("{e:#}")));
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

fn read(path: &PathBuf) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let config: Config = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    config.validate().with_context(|| format!("check {}", path.display()))?;
    Ok(config)
}

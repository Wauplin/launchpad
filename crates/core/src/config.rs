//! `config.toml`: loading with defaults, watching, and rewriting from the Settings screen.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::Bus;
use crate::paths::Paths;

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
    pub terminal: Terminal,
    pub github: GitHub,
    pub agents: Agents,
    pub presets: Vec<Preset>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Commands {
    pub editor: String,
    pub browser: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Terminal {
    /// `terminator`, `ghostty`, `gnome-terminal` or `custom`.
    pub app: String,
    /// Only used when `app = "custom"`. `{cwd}` and `{command}` are replaced.
    pub custom: Vec<String>,
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
            terminal: Terminal::default(),
            github: GitHub::default(),
            agents: Agents::default(),
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

impl Default for Terminal {
    fn default() -> Self {
        Terminal {
            app: if cfg!(target_os = "macos") { "terminal" } else if cfg!(windows) { "windows-terminal" } else { "terminator" }.into(),
            custom: vec!["kitty".into(), "--directory".into(), "{cwd}".into(), "{command}".into()],
        }
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

[terminal]                          # used by "Open in terminal"
app = "terminator"                  # terminator | ghostty | gnome-terminal | custom
custom = ["kitty", "--directory", "{cwd}", "{command}"]   # only used when app = "custom"

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
            std::fs::write(&path, DEFAULT_CONFIG.replace(r#"browser = "xdg-open""#, &format!("browser = {:?}", Commands::default().browser)).replace(r#"app = "terminator""#, &format!("app = {:?}", Terminal::default().app))).with_context(|| format!("write {}", path.display()))?;
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
        anyhow::ensure!(config.zoom <= 4, "zoom must be between 0 and 4");
        let text = toml::to_string_pretty(&config)?;
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
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
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

//! Environments (prod and dev) and the files and ports they use.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEnv {
    Prod,
    Dev,
}

impl AppEnv {
    /// Reads `LAUNCHPAD_ENV` (`dev` or `prod`, default `prod`).
    pub fn from_env() -> AppEnv {
        match std::env::var("LAUNCHPAD_ENV").as_deref() {
            Ok("dev") => AppEnv::Dev,
            _ => AppEnv::Prod,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AppEnv::Prod => "prod",
            AppEnv::Dev => "dev",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub app_env: AppEnv,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub role: crate::role::Role,
}

impl Paths {
    pub fn for_app_env(app_env: AppEnv) -> Paths {
        Paths::for_role(app_env, crate::role::Role::All)
    }

    /// A runner keeps its own folders (`launchpad-runner`): it never shares data with a single-process install.
    pub fn for_role(app_env: AppEnv, role: crate::role::Role) -> Paths {
        let home = dirs::home_dir().expect("no home directory");
        let suffix = match (app_env, role) {
            (AppEnv::Prod, crate::role::Role::Runner) => "launchpad-runner",
            (AppEnv::Dev, crate::role::Role::Runner) => "launchpad-runner-dev",
            (AppEnv::Prod, _) => "launchpad",
            (AppEnv::Dev, _) => "launchpad-dev",
        };
        Paths {
            app_env,
            config_dir: home.join(".config").join(suffix),
            data_dir: home.join(".local/share").join(suffix),
            // `LAUNCHPAD_PORT` lets isolated checks (with their own `HOME`) run next to the dev daemon.
            port: std::env::var("LAUNCHPAD_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(match app_env {
                AppEnv::Prod => 7317,
                AppEnv::Dev => 7318,
            }),
            role,
        }
    }

    /// `LAUNCHPAD_PORT` overrides the environment's port (isolated test daemons). `LAUNCHPAD_ROLE` selects the
    /// folders of a runner for CLI commands run next to it.
    pub fn from_env() -> Paths {
        let role = std::env::var("LAUNCHPAD_ROLE").ok().and_then(|r| crate::role::Role::parse(&r).ok()).unwrap_or_default();
        let mut paths = Paths::for_role(AppEnv::from_env(), role);
        if let Some(port) = std::env::var("LAUNCHPAD_PORT").ok().and_then(|p| p.parse().ok()) {
            paths.port = port;
        }
        paths
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir).with_context(|| format!("create {}", self.config_dir.display()))?;
        std::fs::create_dir_all(&self.data_dir).with_context(|| format!("create {}", self.data_dir.display()))?;
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn state_db(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }
    pub fn cache_db(&self) -> PathBuf {
        self.data_dir.join("cache.db")
    }
    pub fn token_file(&self) -> PathBuf {
        self.data_dir.join("token")
    }

    /// Cookie name, per environment: cookies are not isolated by port, so prod and dev need different names.
    pub fn cookie_name(&self) -> &'static str {
        match self.app_env {
            AppEnv::Prod => "lp_session",
            AppEnv::Dev => "lp_dev_session",
        }
    }

    /// Shell invocation included in agent prompts, with this daemon's environment and exact CLI.
    pub fn cli_command(&self) -> String {
        let cli = std::env::current_exe().unwrap_or_else(|_| "launchpad".into());
        let cli = cli.to_string_lossy();
        let cli = cli.trim_end_matches(" (deleted)");
        if cfg!(windows) {
            format!("$env:LAUNCHPAD_ENV = '{}'; & '{}'", self.app_env.name(), cli.replace('\'', "''"))
        } else {
            format!("LAUNCHPAD_ENV={} '{}'", self.app_env.name(), cli.replace('\'', "'\\''"))
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Reads the API token, creating it (256 random bits, mode 0600) on first use.
    pub fn token(&self) -> Result<String> {
        let path = self.token_file();
        if let Ok(t) = std::fs::read_to_string(&path) {
            let t = t.trim().to_string();
            if t.len() == 64 {
                return Ok(t);
            }
        }
        self.ensure_dirs()?;
        let bytes: [u8; 32] = rand::random();
        let token = hex::encode(bytes);
        write_private(&path, &token)?;
        Ok(token)
    }
}

pub(crate) fn write_private(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Expands a leading `~` to the home directory.
pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_default();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir().unwrap_or_default().join(rest);
    }
    PathBuf::from(p)
}

/// Replaces the home directory prefix with `~` for display.
pub fn tilde(p: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = p.strip_prefix(&home) {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

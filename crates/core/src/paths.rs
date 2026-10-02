//! Profiles (prod and dev) and the files and ports they use.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Prod,
    Dev,
}

impl Profile {
    /// Reads `LAUNCHPAD_PROFILE` (`dev` or `prod`, default `prod`).
    pub fn from_env() -> Profile {
        match std::env::var("LAUNCHPAD_PROFILE").as_deref() {
            Ok("dev") => Profile::Dev,
            _ => Profile::Prod,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Profile::Prod => "prod",
            Profile::Dev => "dev",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub profile: Profile,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
}

impl Paths {
    pub fn for_profile(profile: Profile) -> Paths {
        let home = dirs::home_dir().expect("no home directory");
        let suffix = match profile {
            Profile::Prod => "launchpad",
            Profile::Dev => "launchpad-dev",
        };
        Paths {
            profile,
            config_dir: home.join(".config").join(suffix),
            data_dir: home.join(".local/share").join(suffix),
            port: match profile {
                Profile::Prod => 7317,
                Profile::Dev => 7318,
            },
        }
    }

    pub fn from_env() -> Paths {
        Paths::for_profile(Profile::from_env())
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

    /// Cookie name, per profile: cookies are not isolated by port, so prod and dev need different names.
    pub fn cookie_name(&self) -> &'static str {
        match self.profile {
            Profile::Prod => "lp_session",
            Profile::Dev => "lp_dev_session",
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

fn write_private(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
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

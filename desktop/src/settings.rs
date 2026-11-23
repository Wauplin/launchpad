//! The companion's settings: `$XDG_CONFIG_HOME/launchpad-desktop/settings.toml`. No secret is ever written here (the
//! device secret and Hugging Face tokens live in the keyring, `secrets.rs`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DIR: &str = "launchpad-desktop";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HomeKind {
    /// Home on a private Hugging Face Space: every call also carries the owner's Hugging Face token.
    #[default]
    Space,
    /// A dev or self-hosted Home reached by its address.
    Address,
}

/// The linked Home (one per companion, decision D-MULTI-HOME).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct HomeLink {
    pub kind: HomeKind,
    /// `https://<subdomain>.hf.space`, or the address the owner typed.
    pub origin: String,
    /// `owner/name` of the Space.
    #[serde(default)]
    pub space: Option<String>,
    /// The name this device has on Home (Settings → Devices).
    #[serde(default)]
    pub device_name: String,
    /// The Hugging Face login: OAuth (with a refresh token in the keyring) or a pasted token.
    #[serde(default)]
    pub hf: Option<HfLogin>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HfLogin {
    OAuth,
    Token,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub home: Option<HomeLink>,
    /// The workspace the companion follows (decision D-WORKSPACE); the oldest when unset or deleted.
    pub workspace: Option<String>,
    /// Desktop notifications in needs-you mode (decision D-NOTIFY); off keeps the badge and the menu.
    pub notifications: bool,
    /// Start at login (an XDG autostart entry). None until the first link turns it on (the folio's default).
    pub autostart: Option<bool>,
    /// The Hugging Face OAuth app's client ID, when the build has none (`LAUNCHPAD_HF_CLIENT_ID`).
    pub hf_client_id: Option<String>,
    /// The spotlight's default Ask target on this device (a bot ID); the web's rule when unset or gone.
    pub ask_bot: Option<String>,
    /// The quick launch's last folder and profile on this device.
    pub last_folder: Option<String>,
    pub last_profile: Option<String>,
    /// `/dnd`: no notifications until this time (Unix ms).
    pub dnd_until: Option<i64>,
    /// The global shortcut asked of the portal (its format, `CTRL+ALT+space`).
    pub shortcut: String,
    /// `portal` (the default), `custom` (the GNOME custom shortcut the owner chose) or `off`.
    pub shortcut_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            home: None,
            workspace: None,
            notifications: true,
            autostart: None,
            hf_client_id: None,
            ask_bot: None,
            last_folder: None,
            last_profile: None,
            dnd_until: None,
            shortcut: crate::shortcut::DEFAULT_TRIGGER.into(),
            shortcut_mode: "portal".into(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(DIR)
}

fn path() -> PathBuf {
    config_dir().join(DIR).join("settings.toml")
}

impl Settings {
    /// Notifications are on and not paused by `/dnd`.
    pub fn notifying(&self, now: i64) -> bool {
        self.notifications && self.dnd_until.is_none_or(|t| now >= t)
    }

    /// The saved settings; defaults when there is no file, or when it cannot be read (logged).
    pub fn load() -> Settings {
        match std::fs::read_to_string(path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("settings unreadable, using defaults: {e}");
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    /// Writes the file whole (a temporary file renamed into place).
    pub fn save(&self) -> anyhow::Result<()> {
        let path = path();
        std::fs::create_dir_all(path.parent().expect("settings file has a folder"))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// The OAuth client ID: the setting, else the build's `LAUNCHPAD_HF_CLIENT_ID`.
    pub fn client_id(&self) -> Option<String> {
        self.hf_client_id
            .clone()
            .filter(|c| !c.trim().is_empty())
            .or_else(|| option_env!("LAUNCHPAD_HF_CLIENT_ID").map(str::to_string).filter(|c| !c.trim().is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let s = Settings { home: Some(HomeLink { kind: HomeKind::Address, origin: "http://127.0.0.1:7317".into(), device_name: "Desktop · pc".into(), ..Default::default() }), ..Default::default() };
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
        let empty: Settings = toml::from_str("").unwrap();
        assert!(empty.notifications && empty.home.is_none() && empty.autostart.is_none());
        assert_eq!((empty.shortcut.as_str(), empty.shortcut_mode.as_str()), ("CTRL+ALT+space", "portal"));
        let dnd = Settings { dnd_until: Some(100), ..Settings::default() };
        assert!(!dnd.notifying(99) && dnd.notifying(100));
    }
}

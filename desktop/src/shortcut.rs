//! The global shortcut (decision D-HOTKEY, Ctrl+Alt+Space by default). First the GlobalShortcuts portal: GNOME 48+
//! has a backend (xdg-desktop-portal-gnome NEWS), version 2 at xdg-desktop-portal 1.21 with `ConfigureShortcuts`
//! and an `activation_token` in `Activated`. A host (unsandboxed) app must first register its app ID on its D-Bus
//! connection with `org.freedesktop.host.portal.Registry.Register`, once and before any other portal call (1.21's
//! org.freedesktop.host.portal.Registry.xml), so the companion uses one connection for both. Otherwise, on the
//! owner's explicit choice only, a GNOME custom shortcut (`org.gnome.settings-daemon.plugins.media-keys`) that runs
//! `launchpad-desktop --toggle`; it touches only its own entry. `--toggle` always works, so any binding can call it.

use futures_util::StreamExt;

pub const ID: &str = "spotlight";
pub const DEFAULT_TRIGGER: &str = "CTRL+ALT+space";

/// Where the shortcut comes from, for the menu and the logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    /// Bound through the portal; GNOME's own description of the keys.
    Portal(String),
    /// The GNOME custom shortcut runs `launchpad-desktop --toggle`.
    Custom(String),
    /// Neither: why, and that `--toggle` can be bound by hand.
    Unavailable(String),
}

impl State {
    pub fn label(&self) -> String {
        match self {
            State::Starting => "Shortcut: setting up…".into(),
            State::Portal(keys) => format!("Shortcut: {keys}"),
            State::Custom(keys) => format!("Shortcut: {keys} (GNOME custom shortcut)"),
            State::Unavailable(_) => "Shortcut: none (bind launchpad-desktop --toggle)".into(),
        }
    }
}

pub enum Event {
    State(State),
    /// The shortcut was pressed; the portal's activation token lets the window take the focus under Wayland.
    Activated(Option<String>),
}

pub enum PortalCmd {
    /// "Change shortcut…": GNOME's own dialog (`ConfigureShortcuts`, portal version 2).
    Configure,
}

/// `CTRL+ALT+space` (the portal's shortcut format) as a GTK accelerator for GNOME settings (`<Control><Alt>space`).
pub fn gnome_accel(trigger: &str) -> String {
    let mut parts: Vec<&str> = trigger.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let key = parts.pop().unwrap_or("space");
    let mods: String = parts
        .iter()
        .map(|m| match m.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => "<Control>",
            "ALT" => "<Alt>",
            "SHIFT" => "<Shift>",
            "LOGO" | "SUPER" => "<Super>",
            _ => "",
        })
        .collect();
    format!("{mods}{key}")
}

/// Human words for a trigger (`Ctrl+Alt+Space`).
pub fn words(trigger: &str) -> String {
    trigger
        .split('+')
        .map(|p| match p.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => "Ctrl".to_string(),
            "ALT" => "Alt".into(),
            "SHIFT" => "Shift".into(),
            "LOGO" | "SUPER" => "Super".into(),
            other => {
                let mut c = other.to_ascii_lowercase();
                if let Some(f) = c.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                c
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The portal path: register the app ID, open a session, bind the shortcut unless GNOME already has it (so the
/// consent dialog shows once), then forward each activation. Ends with an error when the portal is missing or the
/// owner refused; the caller then says so in the menu.
pub async fn portal(trigger: String, events: async_channel::Sender<Event>, mut cmds: tokio::sync::mpsc::UnboundedReceiver<PortalCmd>) -> anyhow::Result<()> {
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    let conn = ashpd::zbus::Connection::session().await?;
    let app_id = ashpd::AppID::try_from(crate::APP_ID)?;
    if let Err(e) = ashpd::register_host_app_with_connection(conn.clone(), app_id).await {
        // A portal older than 1.19 has no Registry: GNOME then names the app from its process.
        tracing::info!("portal registry: {e}");
    }
    let gs = GlobalShortcuts::with_connection(conn).await?;
    tracing::info!("GlobalShortcuts portal version {}", gs.version());
    let session = gs.create_session(Default::default()).await?;
    let listed = gs.list_shortcuts(&session, Default::default()).await?.response()?;
    let mut keys = listed.shortcuts().iter().find(|s| s.id() == ID).map(|s| s.trigger_description().to_string());
    if keys.is_none() {
        let shortcut = NewShortcut::new(ID, "Open the Launchpad spotlight").preferred_trigger(Some(trigger.as_str()));
        let bound = gs.bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default()).await?.response()?;
        keys = bound.shortcuts().iter().find(|s| s.id() == ID).map(|s| s.trigger_description().to_string());
        if keys.is_none() {
            anyhow::bail!("the shortcut was not bound (refused in GNOME's dialog)");
        }
    }
    let shown = keys.filter(|k| !k.is_empty()).unwrap_or_else(|| words(&trigger));
    tracing::info!("global shortcut through the portal: {shown}");
    let _ = events.send(Event::State(State::Portal(shown))).await;
    let mut activated = gs.receive_activated().await?;
    let mut changed = gs.receive_shortcuts_changed().await?;
    loop {
        tokio::select! {
            a = activated.next() => {
                let Some(a) = a else { anyhow::bail!("the portal closed the session") };
                if a.shortcut_id() != ID {
                    continue;
                }
                let token = a.options().get("activation_token").and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok());
                let _ = events.send(Event::Activated(token)).await;
            }
            c = changed.next() => {
                if let Some(c) = c && let Some(s) = c.shortcuts().iter().find(|s| s.id() == ID) {
                    let _ = events.send(Event::State(State::Portal(s.trigger_description().to_string()))).await;
                }
            }
            cmd = cmds.recv() => match cmd {
                Some(PortalCmd::Configure) => {
                    if let Err(e) = gs.configure_shortcuts(&session, None, Default::default()).await {
                        tracing::warn!("configure shortcuts: {e}");
                    }
                }
                None => return Ok(()),
            },
        }
    }
}

/// The GNOME custom shortcut (GSettings, relocatable schema). Main thread only.
pub mod custom {
    use gtk::gio;
    use gtk::prelude::*;

    const LIST_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const ENTRY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
    const PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/launchpad-desktop/";

    /// Both schemas are installed (GNOME's settings daemon): creating `gio::Settings` without them would abort.
    pub fn available() -> bool {
        gio::SettingsSchemaSource::default().is_some_and(|src| src.lookup(LIST_SCHEMA, true).is_some() && src.lookup(ENTRY_SCHEMA, true).is_some())
    }

    pub fn installed() -> Option<String> {
        if !available() {
            return None;
        }
        let list = gio::Settings::new(LIST_SCHEMA);
        if !list.strv("custom-keybindings").iter().any(|p| p.as_str() == PATH) {
            return None;
        }
        Some(gio::Settings::with_path(ENTRY_SCHEMA, PATH).string("binding").to_string())
    }

    /// Adds (or updates) only the companion's entry; the owner's other shortcuts stay as they are.
    pub fn install(accel: &str, command: &str) -> Result<(), String> {
        if !available() {
            return Err("GNOME's custom shortcuts are not available here".into());
        }
        let entry = gio::Settings::with_path(ENTRY_SCHEMA, PATH);
        for (k, v) in [("name", "Launchpad spotlight"), ("command", command), ("binding", accel)] {
            entry.set_string(k, v).map_err(|e| e.to_string())?;
        }
        let list = gio::Settings::new(LIST_SCHEMA);
        let mut paths: Vec<String> = list.strv("custom-keybindings").iter().map(|p| p.to_string()).collect();
        if !paths.iter().any(|p| p == PATH) {
            paths.push(PATH.into());
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            list.set_strv("custom-keybindings", refs.as_slice()).map_err(|e| e.to_string())?;
        }
        gio::Settings::sync();
        Ok(())
    }

    /// Removes only the companion's entry.
    pub fn remove() -> Result<(), String> {
        if !available() {
            return Ok(());
        }
        let list = gio::Settings::new(LIST_SCHEMA);
        let paths: Vec<String> = list.strv("custom-keybindings").iter().map(|p| p.to_string()).filter(|p| p != PATH).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        list.set_strv("custom-keybindings", refs.as_slice()).map_err(|e| e.to_string())?;
        let entry = gio::Settings::with_path(ENTRY_SCHEMA, PATH);
        for k in ["name", "command", "binding"] {
            entry.reset(k);
        }
        gio::Settings::sync();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trigger_formats() {
        assert_eq!(super::gnome_accel("CTRL+ALT+space"), "<Control><Alt>space");
        assert_eq!(super::gnome_accel("LOGO+SHIFT+l"), "<Super><Shift>l");
        assert_eq!(super::words("CTRL+ALT+space"), "Ctrl+Alt+Space");
    }
}

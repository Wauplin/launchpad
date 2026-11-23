//! The desktop integration of the GTK side: opening addresses in the default browser, notifications
//! (GNotification: GNOME shows them under the app of the desktop file `dev.launchpad.Desktop.desktop`), the dock
//! badge (the Ubuntu Dock's `com.canonical.Unity.LauncherEntry` signal) and the XDG autostart entry.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::rules::Note;
use crate::settings::config_dir;

/// Opens an address in the default browser (`xdg-open`'s handler; the OpenURI portal when sandboxed). Addresses never
/// carry a token. Only web addresses: the `app.open-url` action is reachable by any program of the session.
pub fn open(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    if let Err(e) = gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>) {
        tracing::warn!("could not open {url}: {e}");
    }
}

/// A notification; the same `key` replaces the previous one (one per session). Clicking it, or Open, opens `url`.
pub fn notify(app: &gtk::Application, note: &Note, url: &str) {
    let n = gio::Notification::new(&note.title);
    if !note.body.is_empty() {
        n.set_body(Some(&note.body));
    }
    if !url.is_empty() {
        n.set_default_action_and_target_value("app.open-url", Some(&url.to_variant()));
        n.add_button_with_target_value("Open", "app.open-url", Some(&url.to_variant()));
    }
    app.send_notification(Some(&note.key), &n);
}

/// The dock badge: the needs-you count on the companion's dock icon (hidden at 0, offline or signed out).
pub fn dock_badge(app: &gtk::Application, count: Option<usize>) {
    let Some(bus) = app.dbus_connection() else { return };
    let props = glib::VariantDict::new(None);
    props.insert("count", count.unwrap_or(0) as i64);
    props.insert("count-visible", count.is_some_and(|c| c > 0));
    let params = glib::Variant::tuple_from_iter([format!("application://{}.desktop", crate::APP_ID).to_variant(), props.end()]);
    if let Err(e) = bus.emit_signal(None, "/dev/launchpad/Desktop/launcher", "com.canonical.Unity.LauncherEntry", "Update", Some(&params)) {
        tracing::debug!("dock badge: {e}");
    }
}

fn autostart_file() -> std::path::PathBuf {
    config_dir().join("autostart").join(format!("{}.desktop", crate::APP_ID))
}

/// The command the autostart entry runs: the installed `launchpad-desktop`, or this binary's path in a build (only a
/// path that needs no quoting in a desktop file's `Exec`).
fn command() -> String {
    match std::env::current_exe() {
        Ok(path) if !path.starts_with("/usr/") && path.to_str().is_some_and(|p| p.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+".contains(&b))) => path.display().to_string(),
        _ => "launchpad-desktop".into(),
    }
}

pub fn autostart_on() -> bool {
    autostart_file().exists()
}

/// Writes or removes `~/.config/autostart/dev.launchpad.Desktop.desktop`.
pub fn set_autostart(on: bool) -> std::io::Result<()> {
    let path = autostart_file();
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(path.parent().expect("autostart folder"))?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Launchpad\nComment=Launchpad companion: top-bar status, notifications and the global spotlight\nExec={} --background\nIcon={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        command(),
        crate::APP_ID
    );
    std::fs::write(path, entry)
}

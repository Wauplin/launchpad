//! Notifications, sent by the daemon so they work when every window is closed: Web Push to paired
//! browsers (`push.rs`) and a desktop notification on the host. Clicking a desktop notification
//! opens the window on the item (`launchpad-desktop --open <route>`).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::App;
use crate::model::{Session, SessionStatus};

/// Threads waiting for a click. GNOME keeps unanswered notifications in its tray, so the waits are
/// capped; beyond the cap, notifications are shown without the click action.
static WAITING: AtomicUsize = AtomicUsize::new(0);
const MAX_WAITING: usize = 32;

/// The session needs you: it finished a turn or asked a question, and no window in use shows it.
pub fn session_done(app: &App, s: &Session) {
    if !app.config.get().agents.notify {
        return;
    }
    app.push.session(app, s);
    let project = std::path::Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let summary = match (s.status, s.open_question) {
        (_, true) => format!("{} has a question in {project}", s.harness.display()),
        (SessionStatus::Background, _) => format!("{} waits on a background command in {project}", s.harness.display()),
        _ => format!("{} is waiting in {project}", s.harness.display()),
    };
    let body = s.title.clone();
    let route = format!("/agents?session={}", s.id);
    let desktop = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(format!("launchpad-desktop{}", std::env::consts::EXE_SUFFIX)))).filter(|p| p.exists());
    let env = app.env.clone();
    let profile = app.paths.profile;
    std::thread::spawn(move || {
        let handle = notify_rust::Notification::new()
            .appname("Launchpad")
            .summary(&summary)
            .body(&body)
            .icon("launchpad")
            .action("default", "Open")
            .show();
        match handle {
            Ok(_) if WAITING.load(Ordering::Relaxed) >= MAX_WAITING => {}
            Ok(h) => {
                WAITING.fetch_add(1, Ordering::Relaxed);
                h.wait_for_action(|action| {
                if action == "default" {
                    let exe = desktop.clone().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "launchpad-desktop".into());
                    let mut cmd = std::process::Command::new(exe);
                    cmd.args(["--open", &route]).env_clear().envs(env.vars().iter());
                    if profile == crate::paths::Profile::Dev {
                        cmd.env("LAUNCHPAD_PROFILE", "dev");
                    }
                    let _ = cmd.spawn();
                }
                });
                WAITING.fetch_sub(1, Ordering::Relaxed);
            }
            // A headless host has no notification server: expected, not worth a warning.
            Err(e) if cfg!(target_os = "linux") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() => tracing::debug!("desktop notification: {e}"),
            Err(e) => tracing::warn!("desktop notification: {e}"),
        }
    });
}

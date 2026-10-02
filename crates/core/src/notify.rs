//! Desktop notifications, sent by the daemon so they work when the window is closed.
//! Clicking one opens the window on the item (`launchpad-desktop --open <route>`).

use crate::App;
use crate::model::{Session, SessionStatus};

pub fn session_pending(app: &App, s: &Session) {
    if !app.config.get().agents.notify {
        return;
    }
    let project = std::path::Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let summary = match (s.status, s.open_question) {
        (_, true) => format!("{} has a question in {project}", s.harness.display()),
        (SessionStatus::Background, _) => format!("{} waits on a background command in {project}", s.harness.display()),
        _ => format!("{} is waiting in {project}", s.harness.display()),
    };
    let body = s.title.clone();
    let route = format!("/agents?session={}", s.id);
    let desktop = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("launchpad-desktop"))).filter(|p| p.exists());
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
            Ok(h) => h.wait_for_action(|action| {
                if action == "default" {
                    let exe = desktop.clone().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "launchpad-desktop".into());
                    let mut cmd = std::process::Command::new(exe);
                    cmd.args(["--open", &route]).env_clear().envs(env.vars().iter());
                    if profile == crate::paths::Profile::Dev {
                        cmd.env("LAUNCHPAD_PROFILE", "dev");
                    }
                    let _ = cmd.spawn();
                }
            }),
            Err(e) => tracing::warn!("notification: {e}"),
        }
    });
}

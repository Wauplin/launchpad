//! The Launchpad desktop companion's library: everything but the GTK application glue of `main.rs`. The pure parts
//! (`conn`, `sse`, `model`, `links`, `rules`, `oauth`, `redact`, `settings`) have small tests; `engine` talks to Home.

pub mod conn;
pub mod desktop;
pub mod engine;
pub mod hf;
pub mod home;
pub mod links;
pub mod linkwin;
pub mod logging;
pub mod model;
pub mod oauth;
pub mod redact;
pub mod rules;
pub mod secrets;
pub mod settings;
pub mod sse;
pub mod tray;

/// The application ID: the D-Bus name, the desktop file (`dev.launchpad.Desktop.desktop`), the notifications' app
/// and the portal registration all use it.
pub const APP_ID: &str = "dev.launchpad.Desktop";

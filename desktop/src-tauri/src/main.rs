//! Launchpad desktop shell: one native window that loads the UI from the daemon.
//!
//! On start it checks `GET /api/health`. If the daemon is down, it runs `systemctl --user start launchpad`
//! (prod) or spawns `launchpad daemon` directly (dev, or when the unit is missing), then opens the window
//! through `/auth?t=<token>`, which sets the session cookie. The UI itself has no Tauri dependency.
//!
//! Prod keeps one shell process warm, so the window opens instantly from the dock:
//! - closing the window hides it; the page stays loaded and connected;
//! - `launchpad-desktop --hidden` (autostart at login) starts without showing the window;
//! - starting `launchpad-desktop` again hands over to the running process (single instance), which shows
//!   the window. `--open <route>` also navigates (used by notifications), `--quit` stops the shell.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

struct Profile {
    dev: bool,
    port: u16,
    data_dir: PathBuf,
}

impl Profile {
    fn from_env() -> Profile {
        let dev = std::env::var("LAUNCHPAD_PROFILE").as_deref() == Ok("dev");
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Profile {
            dev,
            port: if dev { 7318 } else { 7317 },
            data_dir: home.join(".local/share").join(if dev { "launchpad-dev" } else { "launchpad" }),
        }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// `GET /api/health` with a plain socket: no HTTP client needed for one request.
fn healthy(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
    let req = format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 12];
    s.read_exact(&mut buf).is_ok() && buf.starts_with(b"HTTP/1.1 200")
}

fn wait_healthy(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if healthy(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Starts the daemon: through systemd for prod, as a detached process otherwise.
fn start_daemon(p: &Profile) -> Result<(), String> {
    if !p.dev {
        let ok = std::process::Command::new("systemctl")
            .args(["--user", "start", "launchpad.service"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok && wait_healthy(p.port, Duration::from_secs(10)) {
            return Ok(());
        }
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("launchpad")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("launchpad"));
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // A new session, so the daemon outlives the window.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    if wait_healthy(p.port, Duration::from_secs(15)) {
        Ok(())
    } else {
        Err(format!("the daemon did not answer on port {} after 15 s", p.port))
    }
}

/// The sign-in address, which redirects to `route` with the shell start time for the startup log.
fn app_url(p: &Profile, route: &str, t0: u128) -> Result<url::Url, String> {
    let token = std::fs::read_to_string(p.data_dir.join("token")).map_err(|e| format!("cannot read the token: {e}"))?;
    let sep = if route.contains('?') { '&' } else { '?' };
    let next = format!("{route}{sep}lp_t0={t0}");
    let mut url = url::Url::parse(&format!("{}/auth", p.origin())).unwrap();
    url.query_pairs_mut().append_pair("t", token.trim()).append_pair("next", &next);
    Ok(url)
}

fn has_flag(argv: &[String], flag: &str) -> bool {
    argv.iter().any(|a| a == flag)
}

/// Shows the hidden window and logs how long it took since the second instance started (`--t0 <ms>`).
fn show_window(w: &WebviewWindow, argv: &[String]) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    if let Some(route) = open_route(argv) {
        let _ = w.eval(navigate_js(&route));
    }
    if let Some(t0) = argv.iter().position(|a| a == "--t0").and_then(|i| argv.get(i + 1)).and_then(|v| v.parse::<u128>().ok()) {
        let _ = w.eval(format!(
            "requestAnimationFrame(() => requestAnimationFrame(() => fetch('/api/log', {{method: 'POST', headers: {{'Content-Type': 'application/json'}}, body: JSON.stringify({{message: 'window shown: ' + (Date.now() - {t0}) + ' ms after the second instance started'}})}})));"
        ));
    }
}

fn open_route(argv: &[String]) -> Option<String> {
    let i = argv.iter().position(|a| a == "--open")?;
    argv.get(i + 1).filter(|r| r.starts_with('/')).cloned()
}

/// Client-side navigation, so the open app does not reload.
fn navigate_js(route: &str) -> String {
    let r = serde_json_string(route);
    format!("history.pushState(null, '', {r}); dispatchEvent(new PopStateEvent('popstate'));")
}

fn serde_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn show_error(w: &WebviewWindow, msg: &str) {
    let m = serde_json_string(&format!("Launchpad could not start.\n\n{msg}\n\nLogs: journalctl --user -u launchpad"));
    let _ = w.eval(format!(
        "document.getElementById('msg').outerHTML = '<pre id=\"msg\"></pre>'; document.getElementById('msg').textContent = {m};"
    ));
}

fn main() {
    let t0: u128 = std::env::var("LP_T_LAUNCH").ok().and_then(|v| v.parse().ok()).unwrap_or_else(now_ms);
    let profile = Profile::from_env();
    let args: Vec<String> = std::env::args().collect();
    let route = open_route(&args).unwrap_or_else(|| "/".into());

    let hidden = has_flag(&args, "--hidden");
    let keep_warm = !profile.dev;

    let mut builder = tauri::Builder::default();
    if keep_warm {
        // Dev windows are not single-instance, so a dev window can run next to the prod one.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if has_flag(&argv, "--quit") {
                app.exit(0);
                return;
            }
            if let Some(w) = app.get_webview_window("main") {
                show_window(&w, &argv);
            }
        }));
    } else if has_flag(&args, "--quit") {
        return;
    }
    builder
        // Size and position only: visibility is decided by `--hidden`.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE)
                .build(),
        )
        .setup(move |app| {
            let warm = healthy(profile.port);
            let origin = profile.origin();
            let first_url = match (warm, app_url(&profile, &route, t0)) {
                (true, Ok(url)) => WebviewUrl::External(url),
                _ => WebviewUrl::App("index.html".into()),
            };
            let window = WebviewWindowBuilder::new(app, "main", first_url)
                .title(if profile.dev { "Launchpad (dev)" } else { "Launchpad" })
                .inner_size(1440.0, 920.0)
                .min_inner_size(960.0, 600.0)
                .disable_drag_drop_handler()
                .visible(!hidden)
                .on_navigation(move |url| {
                    let internal = url.as_str().starts_with(&origin) || url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost");
                    if !internal && matches!(url.scheme(), "http" | "https" | "mailto") {
                        // Links to GitHub and elsewhere open in the browser, never inside the app window.
                        let _ = std::process::Command::new("xdg-open").arg(url.as_str()).spawn();
                    }
                    internal
                })
                .build()?;
            if keep_warm {
                let w = window.clone();
                window.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        // Hide instead of quitting: the next dock click shows the window at once.
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }
            if !warm {
                std::thread::spawn(move || match start_daemon(&profile).and_then(|_| app_url(&profile, &route, t0)) {
                    Ok(url) => {
                        let _ = window.navigate(url);
                    }
                    Err(e) => show_error(&window, &e),
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Launchpad");
}

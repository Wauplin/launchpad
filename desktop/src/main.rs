//! Launchpad desktop companion for Ubuntu GNOME (Wayland): a thin client of Home, like the Chrome extension. It shows
//! the top-bar menu with live status, sends desktop notifications and hands work over to the browser, which keeps the
//! full UI. One GApplication (`dev.launchpad.Desktop`, single instance: a second launch hands its arguments to the
//! running one) runs GTK on the main thread and a tokio runtime for Home (`engine`), joined by channels.


use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use gtk::{gio, glib};
use ksni::TrayMethods;
use tokio::sync::mpsc;

use launchpad_desktop::engine::{Cmd, Engine, LinkMsg, Shared, Ui, View};
use launchpad_desktop::tray::{self, Action, LpTray};
use launchpad_desktop::{APP_ID, conn, desktop, linkwin, logging, settings};

const USAGE: &str = "Usage: launchpad-desktop [--toggle | --action <ask|goto|launch|open|today|connect|quit> | --background | --quit]
  --toggle      Open or close the spotlight (bind it to a global shortcut)
  --action      Run one menu action in the running companion
  --background  Start without opening a window (autostart)
  --quit        Quit the running companion
  --version     Print the version";

struct Ctx {
    app: gtk::Application,
    rt: tokio::runtime::Handle,
    cmd: mpsc::UnboundedSender<Cmd>,
    settings: Shared,
    tray: Arc<Mutex<Option<ksni::Handle<LpTray>>>>,
    view: RefCell<View>,
    link: RefCell<Option<Rc<linkwin::LinkWindow>>>,
    badge: Cell<Option<usize>>,
    /// The first launch asked for the link window when not linked (not `--background`).
    want_link: Cell<bool>,
    seen_view: Cell<bool>,
    /// An action asked for before the engine's first view (the shortcut started the companion): run once it knows
    /// whether a Home is linked.
    pending: RefCell<Option<Action>>,
}

impl Ctx {
    fn send(&self, cmd: Cmd) {
        let _ = self.cmd.send(cmd);
    }

    /// Pushes the view and the switches to the tray icon and its menu, and the count to the dock badge.
    fn refresh(&self) {
        let view = self.view.borrow().clone();
        let (notifications, autostart) = (self.settings.lock().unwrap().notifications, desktop::autostart_on());
        let count = view.live.then_some(view.status.count);
        let icon = tray::icons(count.unwrap_or(0), view.live);
        if self.badge.get() != count {
            self.badge.set(count);
            desktop::dock_badge(&self.app, count);
        }
        let slot = self.tray.clone();
        self.rt.spawn(async move {
            let handle = slot.lock().unwrap().clone();
            if let Some(handle) = handle {
                handle
                    .update(move |t| {
                        t.view = view;
                        t.icon = icon;
                        t.notifications = notifications;
                        t.autostart = autostart;
                    })
                    .await;
            }
        });
    }

    fn open_link_window(self: &Rc<Self>) {
        if let Some(w) = self.link.borrow().as_ref()
            && w.window.is_visible()
        {
            w.show();
            return;
        }
        let ctx = self.clone();
        let send: Rc<dyn Fn(Cmd)> = Rc::new(move |cmd| ctx.send(cmd));
        let open: Rc<dyn Fn(&str)> = Rc::new(desktop::open);
        let client_id = self.settings.lock().unwrap().client_id().is_some();
        let reauth = matches!(self.view.borrow().conn, Some(conn::Conn::AuthExpired));
        let window = linkwin::LinkWindow::new(&self.app, client_id, reauth, send, open);
        window.show();
        *self.link.borrow_mut() = Some(window);
    }

    fn on_ui(self: &Rc<Self>, msg: Ui) {
        match msg {
            Ui::View(view) => {
                let first = !self.seen_view.replace(true);
                let linked = view.linked;
                *self.view.borrow_mut() = *view;
                self.refresh();
                if first && !linked && self.want_link.get() {
                    self.open_link_window();
                }
                if first && let Some(action) = self.pending.take() {
                    self.run(action);
                }
            }
            Ui::Notify { note, url } => desktop::notify(&self.app, &note, &url),
            Ui::Link(msg) => {
                // A pairing code: open Settings → Devices, where the owner approves it in a signed-in window.
                if let LinkMsg::Code { approve, .. } = &msg {
                    desktop::open(approve);
                }
                if let Some(w) = self.link.borrow().as_ref() {
                    w.update(msg.clone());
                }
                if matches!(msg, LinkMsg::Done) {
                    *self.link.borrow_mut() = None;
                }
            }
            Ui::Open(url) => desktop::open(&url),
            Ui::Linked => {
                // The folio's default: Start at login is on once the companion is linked.
                let mut s = self.settings.lock().unwrap();
                if s.autostart.is_none() {
                    if let Err(e) = desktop::set_autostart(true) {
                        tracing::warn!("autostart: {e}");
                    }
                    s.autostart = Some(true);
                    let _ = s.save();
                }
                drop(s);
                self.refresh();
            }
        }
    }

    fn on_action(self: &Rc<Self>, action: Action) {
        let view = self.view.borrow().clone();
        match action {
            Action::Open(url) => desktop::open(&url),
            // Until the spotlight (step 3): the browser's own Ask, Go to (Today) and launch composer.
            Action::Ask => desktop::open(&view.ask_url),
            Action::GoTo | Action::OpenHome => desktop::open(&view.today_url),
            Action::QuickLaunch => desktop::open(&view.launch_url),
            Action::Workspace(id) => self.send(Cmd::SetWorkspace(id)),
            Action::Notifications(on) => {
                let mut s = self.settings.lock().unwrap();
                s.notifications = on;
                let _ = s.save();
                drop(s);
                self.refresh();
            }
            Action::Autostart(on) => {
                if let Err(e) = desktop::set_autostart(on) {
                    tracing::warn!("autostart: {e}");
                }
                let mut s = self.settings.lock().unwrap();
                s.autostart = Some(on);
                let _ = s.save();
                drop(s);
                self.refresh();
            }
            Action::Connect => self.open_link_window(),
            Action::Retry => self.send(Cmd::RetryNow),
            Action::SignOut => self.send(Cmd::SignOut),
            Action::Update => {}
            Action::Quit => self.app.quit(),
        }
    }

    /// The arguments of a launch (this one, or a second instance that handed them over). Exit code 2 for unknown ones.
    fn on_args(self: &Rc<Self>, args: &[String], first: bool) -> i32 {
        let Some(action) = valid_args(args) else { return 2 };
        let background = args.iter().any(|a| a == "--background");
        if first {
            self.want_link.set(!background && action.is_none());
        }
        let Some(name) = action else {
            // A plain second launch (the dock icon or the app grid): the link window when not linked, else Home.
            if !first {
                if self.view.borrow().linked { self.on_action(Action::OpenHome) } else { self.open_link_window() }
            }
            return 0;
        };
        let action = match name.as_str() {
            "ask" => Action::Ask,
            "goto" => Action::GoTo,
            "launch" => Action::QuickLaunch,
            "open" | "today" => Action::OpenHome,
            "connect" => Action::Connect,
            "quit" => Action::Quit,
            _ => return 2,
        };
        // Before the first view the engine does not know the link yet: actions other than connect and quit wait for it.
        match action {
            Action::Connect | Action::Quit => self.on_action(action),
            _ if !self.seen_view.get() => *self.pending.borrow_mut() = Some(action),
            _ => self.run(action),
        }
        0
    }

    /// A launch's action once the link is known: the link window when not linked.
    fn run(self: &Rc<Self>, action: Action) {
        if self.view.borrow().linked { self.on_action(action) } else { self.open_link_window() }
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("launchpad-desktop {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    // Unknown arguments are refused here, before they reach the running instance.
    if valid_args(&args).is_none() {
        eprintln!("{USAGE}");
        return glib::ExitCode::from(2);
    }
    logging::init();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("launchpad-desktop").build().expect("tokio runtime");
    let rt = runtime.handle().clone();
    let app = gtk::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    let ctx: Rc<RefCell<Option<Rc<Ctx>>>> = Rc::default();

    let slot = ctx.clone();
    app.connect_startup(move |app| {
        tracing::info!("launchpad-desktop {} starting", env!("CARGO_PKG_VERSION"));
        let settings: Shared = Arc::new(Mutex::new(settings::Settings::load()));
        let (ui_tx, ui_rx) = async_channel::unbounded::<Ui>();
        let (engine, cmd) = Engine::new(settings.clone(), ui_tx);
        rt.spawn(engine.run());
        let (act_tx, act_rx) = async_channel::unbounded::<Action>();
        let tray_slot: Arc<Mutex<Option<ksni::Handle<LpTray>>>> = Arc::default();
        let (notifications, autostart) = (settings.lock().unwrap().notifications, desktop::autostart_on());
        let t = tray_slot.clone();
        rt.spawn(async move {
            let tray = LpTray { view: View::default(), notifications, autostart, icon: tray::icons(0, false), tx: act_tx };
            // Keep running without a tray host: the icon appears when one starts (the AppIndicator extension).
            match tray.assume_sni_available(true).spawn().await {
                Ok(handle) => *t.lock().unwrap() = Some(handle),
                Err(e) => tracing::warn!("no top-bar icon: {e}"),
            }
        });
        let open_url = gio::SimpleAction::new("open-url", Some(glib::VariantTy::STRING));
        open_url.connect_activate(|_, param| {
            if let Some(url) = param.and_then(|p| p.get::<String>()) {
                desktop::open(&url);
            }
        });
        app.add_action(&open_url);
        let c = Rc::new(Ctx {
            app: app.clone(),
            rt: rt.clone(),
            cmd,
            settings,
            tray: tray_slot,
            view: RefCell::new(View::default()),
            link: RefCell::new(None),
            badge: Cell::new(None),
            want_link: Cell::new(false),
            seen_view: Cell::new(false),
            pending: RefCell::new(None),
        });
        let ui = c.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = ui_rx.recv().await {
                ui.on_ui(msg);
            }
        });
        let actions = c.clone();
        glib::spawn_future_local(async move {
            while let Ok(action) = act_rx.recv().await {
                actions.on_action(action);
            }
        });
        *slot.borrow_mut() = Some(c);
        // The companion lives in the top bar: no window keeps it running.
        std::mem::forget(app.hold());
    });

    let slot = ctx.clone();
    app.connect_command_line(move |_, line| {
        let Some(ctx) = slot.borrow().clone() else { return glib::ExitCode::FAILURE };
        let args: Vec<String> = line.arguments().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        glib::ExitCode::from(ctx.on_args(&args, !line.is_remote()) as u8)
    });
    let code = app.run();
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}

/// The action the arguments ask for (None: no action), or None at the outer level when they are invalid.
fn valid_args(args: &[String]) -> Option<Option<String>> {
    let mut it = args.iter().skip(1);
    let mut action = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--toggle" => action = Some("ask".to_string()),
            "--action" => {
                let name = it.next()?;
                ["ask", "goto", "launch", "open", "today", "connect", "quit"].contains(&name.as_str()).then_some(())?;
                action = Some(name.clone());
            }
            "--background" => {}
            "--quit" => action = Some("quit".into()),
            _ => return None,
        }
    }
    Some(action)
}

#[cfg(test)]
mod tests {
    #[test]
    fn arguments() {
        let a = |s: &str| std::iter::once("launchpad-desktop").chain(s.split_whitespace()).map(String::from).collect::<Vec<_>>();
        assert_eq!(super::valid_args(&a("")), Some(None));
        assert_eq!(super::valid_args(&a("--toggle")), Some(Some("ask".into())));
        assert_eq!(super::valid_args(&a("--background --action goto")), Some(Some("goto".into())));
        assert_eq!(super::valid_args(&a("--action")), None);
        assert_eq!(super::valid_args(&a("--action rm")), None);
        assert_eq!(super::valid_args(&a("--nope")), None);
    }

    #[test]
    fn app_id_is_a_valid_bus_and_desktop_id() {
        assert!(gtk::gio::Application::id_is_valid(launchpad_desktop::APP_ID));
        // Desktop file IDs and D-Bus names: reverse DNS, letters, digits and dots, no dash.
        assert!(launchpad_desktop::APP_ID.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()) && !p.starts_with(|c: char| c.is_ascii_digit())));
    }

    #[test]
    fn tray_icon_draws_the_count() {
        let icon = launchpad_desktop::tray::icon(32, 3, true).expect("icon");
        assert_eq!((icon.width, icon.height, icon.data.len()), (32, 32, 32 * 32 * 4));
        // The amber badge sits top right; the corner bottom left is transparent.
        let px = |x: usize, y: usize| &icon.data[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(px(0, 31)[0], 0);
        assert!(px(31 - 2, 8)[1] > 150, "{:?}", px(29, 8));
    }
}

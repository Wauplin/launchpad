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

use launchpad_desktop::engine::{AskView, Cmd, Engine, LinkMsg, Shared, SpotData, Ui, View};
use launchpad_desktop::shortcut::{self, State as ShortcutState};
use launchpad_desktop::tray::{self, Action, LpTray, ShortcutMenu};
use launchpad_desktop::{APP_ID, conn, desktop, linkwin, logging, prefs, quicklaunch, rules, settings, spotlight, update};

const USAGE: &str = "Usage: launchpad-desktop [--toggle | --action <ask|goto|launch|open|today|connect|quit> | --background | --quit]
  --toggle      Open or close the spotlight (bind any shortcut to it)
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
    spot_data: RefCell<Arc<SpotData>>,
    /// The spotlight: created once at start, then shown and hidden (warm).
    spotlight: RefCell<Option<Rc<spotlight::Spotlight>>>,
    quick: RefCell<Option<Rc<quicklaunch::QuickLaunch>>>,
    prefs: RefCell<Option<gtk::Window>>,
    shortcut: RefCell<ShortcutState>,
    portal: RefCell<Option<mpsc::UnboundedSender<shortcut::PortalCmd>>>,
    /// Where the shortcut paths report (kept to start the portal again after the custom shortcut is removed).
    shortcut_events: RefCell<Option<async_channel::Sender<shortcut::Event>>>,
    /// Asks already notified as answered (by key).
    answered: RefCell<std::collections::HashSet<String>>,
    /// The companion's own update (None: up to date, or not checked yet).
    update: RefCell<Option<update::State>>,
    updates: async_channel::Sender<Upd>,
}

/// Messages of the update tasks to the main loop.
enum Upd {
    Available(update::Candidate),
    State(update::State),
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Ctx {
    fn send(&self, cmd: Cmd) {
        let _ = self.cmd.send(cmd);
    }

    fn save(&self, change: impl FnOnce(&mut settings::Settings)) {
        let mut s = self.settings.lock().unwrap();
        change(&mut s);
        if let Err(e) = s.save() {
            tracing::warn!("settings not saved: {e}");
        }
    }

    fn shortcut_menu(&self) -> ShortcutMenu {
        let state = self.shortcut.borrow().clone();
        let available = shortcut::custom::available();
        ShortcutMenu { label: state.label(), portal: matches!(state, ShortcutState::Portal(_)), custom_available: available, custom_installed: available && shortcut::custom::installed().is_some() }
    }

    /// Pushes the view and the switches to the tray icon and its menu, and the count to the dock badge.
    fn refresh(&self) {
        let mut view = self.view.borrow().clone();
        view.update = self.update.borrow().as_ref().map(update::State::menu);
        let (notifications, autostart) = (self.settings.lock().unwrap().notifying(now_ms()), desktop::autostart_on());
        let count = view.live.then_some(view.status.count);
        let icon = tray::icons(count.unwrap_or(0), view.live);
        if self.badge.get() != count {
            self.badge.set(count);
            desktop::dock_badge(&self.app, count);
        }
        let shortcut = self.shortcut_menu();
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
                        t.shortcut = shortcut;
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

    fn spotlight(self: &Rc<Self>) -> Rc<spotlight::Spotlight> {
        if let Some(s) = self.spotlight.borrow().as_ref() {
            return s.clone();
        }
        let ctx = self.clone();
        let spot = spotlight::Spotlight::new(&self.app, Rc::new(move |out| ctx.on_spot(out)));
        spot.set_data(self.spot_data.borrow().clone(), self.settings.lock().unwrap().ask_bot.clone());
        *self.spotlight.borrow_mut() = Some(spot.clone());
        spot
    }

    fn quick(self: &Rc<Self>) -> Rc<quicklaunch::QuickLaunch> {
        if let Some(q) = self.quick.borrow().as_ref() {
            return q.clone();
        }
        let ctx = self.clone();
        let q = quicklaunch::QuickLaunch::new(&self.app, Rc::new(move |out| ctx.on_quick(out)));
        *self.quick.borrow_mut() = Some(q.clone());
        q
    }

    fn open_quick(self: &Rc<Self>, prompt: &str) {
        let (folder, profile) = {
            let s = self.settings.lock().unwrap();
            (s.last_folder.clone(), s.last_profile.clone())
        };
        self.quick().open(self.spot_data.borrow().clone(), prompt, folder.as_deref(), profile.as_deref());
    }

    fn open_prefs(self: &Rc<Self>) {
        if let Some(w) = self.prefs.borrow().as_ref() {
            w.close();
        }
        let s = self.settings.lock().unwrap().clone();
        let available = shortcut::custom::available();
        let data = self.spot_data.borrow().clone();
        let state = self.shortcut.borrow().clone();
        let now = prefs::Now { shortcut: &state, custom_available: available, custom_installed: available && shortcut::custom::installed().is_some(), data: &data, ask_bot: s.ask_bot.as_deref(), notifications: s.notifications, autostart: desktop::autostart_on() };
        let ctx = self.clone();
        let p = prefs::Prefs::new(&self.app, now, Rc::new(move |out| ctx.on_prefs(out)));
        p.window.present();
        *self.prefs.borrow_mut() = Some(p.window);
    }

    fn on_prefs(self: &Rc<Self>, out: prefs::Out) {
        match out {
            prefs::Out::ConfigureShortcut => self.on_action(Action::ChangeShortcut),
            prefs::Out::CustomShortcut(on) => {
                self.on_action(Action::CustomShortcut(on));
                self.open_prefs();
            }
            prefs::Out::AskBot(bot) => {
                self.save(|s| s.ask_bot = bot.clone());
                if let Some(s) = self.spotlight.borrow().as_ref() {
                    s.set_data(self.spot_data.borrow().clone(), bot);
                }
            }
            prefs::Out::Notifications(on) => self.on_action(Action::Notifications(on)),
            prefs::Out::Autostart(on) => self.on_action(Action::Autostart(on)),
        }
    }

    fn on_spot(self: &Rc<Self>, out: spotlight::Out) {
        match out {
            spotlight::Out::Ask { bot, text, thread, session, key, files } => self.send(Cmd::Ask { bot, text, thread, session, key, files }),
            spotlight::Out::Answer { session, request, count, text } => self.send(Cmd::Answer { session, request, count, text }),
            spotlight::Out::Open(url) => desktop::open(&url),
            spotlight::Out::Launch(text) => self.open_quick(&text),
            spotlight::Out::Handoff { text, bot: None } => self.send(Cmd::Handoff(text)),
            // A bot's chat has no Home-side draft yet (gap G-DRAFT): the text goes to the clipboard and the bot's
            // new chat opens, where the owner pastes it.
            spotlight::Out::Handoff { text, bot: Some(bot) } => {
                if !text.trim().is_empty()
                    && let Some(display) = gtk::gdk::Display::default()
                {
                    display.clipboard().set_text(&text);
                }
                let url = self.links_page(&format!("/bots/{}", bot.id), &[("thread", "new")]);
                desktop::open(&url);
            }
            spotlight::Out::Resolve(text) => self.send(Cmd::Resolve(text)),
            spotlight::Out::Command { name, rest } => match name.as_str() {
                "launch" => self.open_quick(&rest),
                "workspace" => self.send(Cmd::NextWorkspace),
                "today" => desktop::open(&self.view.borrow().today_url),
                "settings" => self.open_prefs(),
                "dnd" => {
                    self.save(|s| s.dnd_until = Some(now_ms() + 60 * 60_000));
                    self.refresh();
                }
                _ => {}
            },
        }
    }

    /// An address of the linked Home in the companion's workspace.
    fn links_page(&self, path: &str, query: &[(&str, &str)]) -> String {
        let data = self.spot_data.borrow();
        launchpad_desktop::links::Links::new(&data.origin, data.workspace.as_deref().unwrap_or_default()).page(path, query)
    }

    fn on_quick(self: &Rc<Self>, out: quicklaunch::Out) {
        match out {
            quicklaunch::Out::Launch { spec, key, folder, profile } => {
                self.save(|s| {
                    s.last_folder = folder;
                    s.last_profile = Some(profile);
                });
                self.send(Cmd::Launch { spec, key });
            }
            quicklaunch::Out::Handoff(text) => self.send(Cmd::Handoff(text)),
        }
    }

    fn on_ask(self: &Rc<Self>, view: AskView) {
        let showing = self.spotlight.borrow().as_ref().is_some_and(|s| s.showing(&view.key));
        if let Some(s) = self.spotlight.borrow().as_ref() {
            s.on_ask(&view);
        }
        // An answer that arrives while the spotlight is closed: one notification (Open), when notifications are on.
        let finished = view.answer.done || view.answer.question.is_some();
        if finished && !showing && self.settings.lock().unwrap().notifying(now_ms()) && self.answered.borrow_mut().insert(view.key.clone()) {
            let what = if view.answer.question.is_some() { "asks you" } else { "answered" };
            let body = view.answer.question.clone().unwrap_or_else(|| view.answer.text.clone());
            let note = rules::Note { key: format!("ask-{}", view.session), title: format!("{} {what}", view.bot.name), body: launchpad_desktop::model::cut(&body, rules::BODY_CHARS), item: None };
            desktop::notify(&self.app, &note, &view.thread_url);
        }
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
            Ui::Spot(data) => {
                *self.spot_data.borrow_mut() = data.clone();
                if let Some(s) = self.spotlight.borrow().as_ref() {
                    s.set_data(data, self.settings.lock().unwrap().ask_bot.clone());
                }
            }
            Ui::Ask(view) => self.on_ask(*view),
            Ui::Launched(result) => {
                if let Some(q) = self.quick.borrow().as_ref() {
                    q.launched(&result);
                }
                if let Ok(url) = &result {
                    desktop::open(url);
                }
            }
            // An empty message clears the footer's error (an answer went through) and never opens the spotlight.
            Ui::SpotError(e) if e.is_empty() => {
                if let Some(s) = self.spotlight.borrow().as_ref() {
                    s.error("");
                }
            }
            Ui::SpotError(e) => {
                tracing::info!("spotlight: {e}");
                let spot = self.spotlight();
                if !spot.window.is_visible() {
                    spot.show(None, None);
                }
                spot.error(&e);
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

    fn on_shortcut(self: &Rc<Self>, event: shortcut::Event) {
        match event {
            shortcut::Event::State(state) => {
                tracing::info!("{}", state.label());
                *self.shortcut.borrow_mut() = state;
                self.refresh();
            }
            shortcut::Event::Activated(token) => self.toggle(token.as_deref()),
        }
    }

    /// The shortcut (or `--toggle`): the spotlight when linked, else the link window.
    fn toggle(self: &Rc<Self>, token: Option<&str>) {
        if !self.view.borrow().linked {
            self.open_link_window();
            return;
        }
        self.spotlight().toggle(token, None);
    }

    /// Starts the portal path (unless the owner chose the custom shortcut or none); its failure says why in the menu.
    fn start_shortcut(self: &Rc<Self>, events: async_channel::Sender<shortcut::Event>) {
        *self.shortcut_events.borrow_mut() = Some(events.clone());
        let s = self.settings.lock().unwrap().clone();
        if s.shortcut_mode != "portal" {
            let state = match shortcut::custom::installed() {
                Some(accel) => ShortcutState::Custom(accel),
                None => ShortcutState::Unavailable("No global shortcut is set".into()),
            };
            *self.shortcut.borrow_mut() = state;
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        *self.portal.borrow_mut() = Some(tx);
        self.rt.spawn(async move {
            if let Err(e) = shortcut::portal(s.shortcut.clone(), events.clone(), rx).await {
                tracing::warn!("global shortcut portal: {e}");
                let why = format!("The shortcut portal is not available ({e})");
                let _ = events.send(shortcut::Event::State(ShortcutState::Unavailable(why))).await;
            }
        });
    }

    fn on_action(self: &Rc<Self>, action: Action) {
        let view = self.view.borrow().clone();
        match action {
            Action::Open(url) => desktop::open(&url),
            Action::Ask => self.spotlight().show(None, None),
            Action::GoTo => self.spotlight().show(None, Some(">")),
            Action::QuickLaunch => self.open_quick(""),
            Action::OpenHome => desktop::open(&view.today_url),
            Action::Workspace(id) => self.send(Cmd::SetWorkspace(id)),
            Action::Notifications(on) => {
                self.save(|s| {
                    s.notifications = on;
                    s.dnd_until = None;
                });
                self.refresh();
            }
            Action::Autostart(on) => {
                if let Err(e) = desktop::set_autostart(on) {
                    tracing::warn!("autostart: {e}");
                }
                self.save(|s| s.autostart = Some(on));
                self.refresh();
            }
            Action::Connect => self.open_link_window(),
            Action::Retry => self.send(Cmd::RetryNow),
            Action::SignOut => self.send(Cmd::SignOut),
            Action::Update => self.install_update(),
            Action::Quit => self.app.quit(),
            Action::Settings => self.open_prefs(),
            Action::ChangeShortcut => {
                if let Some(tx) = self.portal.borrow().as_ref() {
                    let _ = tx.send(shortcut::PortalCmd::Configure);
                }
            }
            // Only on the owner's explicit choice; the owner's other shortcuts are never touched.
            Action::CustomShortcut(on) => {
                let trigger = self.settings.lock().unwrap().shortcut.clone();
                let result = if on { shortcut::custom::install(&shortcut::gnome_accel(&trigger), "launchpad-desktop --toggle") } else { shortcut::custom::remove() };
                match result {
                    // The custom shortcut replaces the portal's (both on the same keys would toggle twice): dropping
                    // the portal's sender ends its session. Removing it goes back to the portal.
                    Ok(()) if on => {
                        self.portal.borrow_mut().take();
                        self.save(|s| s.shortcut_mode = "custom".into());
                        let state = ShortcutState::Custom(shortcut::words(&trigger));
                        tracing::info!("{}", state.label());
                        *self.shortcut.borrow_mut() = state;
                    }
                    Ok(()) => {
                        self.save(|s| s.shortcut_mode = "portal".into());
                        *self.shortcut.borrow_mut() = ShortcutState::Starting;
                        let events = self.shortcut_events.borrow().clone();
                        if let Some(events) = events {
                            self.start_shortcut(events);
                        }
                    }
                    Err(e) => tracing::warn!("GNOME custom shortcut: {e}"),
                }
                self.refresh();
            }
        }
    }

    /// The update channel: the setting, else this build's (`dev` checks nothing, except a debug build pointed at a
    /// fake release server).
    fn update_channel(&self) -> Option<String> {
        let chosen = self.settings.lock().unwrap().update_channel.clone();
        let channel = chosen.unwrap_or_else(|| update::channel().to_string());
        match channel.as_str() {
            "release" | "main" => Some(channel),
            _ if cfg!(debug_assertions) && std::env::var_os("LAUNCHPAD_DESKTOP_UPDATE_API").is_some() => Some("release".into()),
            _ => None,
        }
    }

    /// The check at start, then every 6 hours; failures back off from 15 minutes. It only reads public release data.
    fn start_update_checks(&self) {
        let Some(channel) = self.update_channel() else {
            tracing::info!("update checks off for this build (channel {})", update::channel());
            return;
        };
        let tx = self.updates.clone();
        self.rt.spawn(async move {
            let http = update::client();
            let mut failures = 0u32;
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            loop {
                let wait = match update::check(&http, &channel).await {
                    Ok(found) => {
                        failures = 0;
                        if let Some(c) = found {
                            tracing::info!("update available: {}", c.version);
                            let _ = tx.send(Upd::Available(c)).await;
                        }
                        6 * 3600
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::info!("update check: {e}");
                        (900u64 << failures.min(4)).min(6 * 3600)
                    }
                };
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        });
    }

    fn on_update(self: &Rc<Self>, msg: Upd) {
        match msg {
            Upd::Available(c) => {
                // A pending or running install is not replaced by a newer check.
                let busy = matches!(*self.update.borrow(), Some(update::State::Downloading(_) | update::State::Verifying(_) | update::State::Installing(_) | update::State::Restart(_)));
                if busy {
                    return;
                }
                let first = self.settings.lock().unwrap().update_notified.as_deref() != Some(c.version.as_str());
                if first {
                    self.save(|s| s.update_notified = Some(c.version.clone()));
                    let note = rules::Note { key: "update".into(), title: format!("Launchpad companion {} is available", c.version), body: "Update from the Launchpad menu in the top bar.".into(), item: None };
                    desktop::notify(&self.app, &note, "");
                }
                *self.update.borrow_mut() = Some(update::State::Available(c));
            }
            Upd::State(state) => {
                let restart = matches!(state, update::State::Restart(_));
                *self.update.borrow_mut() = Some(state);
                if restart {
                    self.refresh();
                    restart_companion();
                    self.app.quit();
                    return;
                }
            }
        }
        self.refresh();
    }

    /// The owner's click: download, verify, install (polkit asks), restart. Never without this click.
    fn install_update(self: &Rc<Self>) {
        let candidate = match self.update.borrow().as_ref() {
            Some(update::State::Available(c)) | Some(update::State::Failed { candidate: c, .. }) => c.clone(),
            _ => return,
        };
        // Set at once (not through the channel), so a second click before the main loop catches up starts nothing.
        *self.update.borrow_mut() = Some(update::State::Downloading(candidate.version.clone()));
        self.refresh();
        let tx = self.updates.clone();
        let say = move |s: update::State| {
            let _ = tx.try_send(Upd::State(s));
        };
        self.rt.spawn(async move {
            let http = update::client();
            let c = candidate.clone();
            let verify_say = say.clone();
            let result = async {
                let path = update::download(&http, &c, || verify_say(update::State::Verifying(c.version.clone()))).await?;
                say(update::State::Installing(c.version.clone()));
                update::install(&path).await
            }
            .await;
            match result {
                Ok(()) => {
                    tracing::info!("installed {}", candidate.version);
                    say(update::State::Restart(candidate.version.clone()));
                }
                Err(e) => {
                    tracing::warn!("update {} failed: {e}", candidate.version);
                    say(update::State::Failed { candidate, reason: e.to_string() });
                }
            }
        });
    }

    /// The arguments of a launch (this one, or a second instance that handed them over). Exit code 2 for unknown ones.
    fn on_args(self: &Rc<Self>, args: &[String], first: bool) -> i32 {
        let Some(action) = valid_args(args) else { return 2 };
        let background = args.iter().any(|a| a == "--background");
        if first {
            self.want_link.set(!background && action.is_none());
        }
        let Some(name) = action else {
            // A plain second launch (the dock icon or the app grid): the link window when not linked, else the
            // spotlight.
            if !first {
                if self.view.borrow().linked { self.toggle(None) } else { self.open_link_window() }
            }
            return 0;
        };
        let action = match name.as_str() {
            "toggle" => Action::Ask,
            "ask" => Action::Ask,
            "goto" => Action::GoTo,
            "launch" => Action::QuickLaunch,
            "open" | "today" => Action::OpenHome,
            "connect" => Action::Connect,
            "quit" => Action::Quit,
            _ => return 2,
        };
        let toggle = name == "toggle";
        // Before the first view the engine does not know the link yet: actions other than connect and quit wait for it.
        match action {
            Action::Connect | Action::Quit => self.on_action(action),
            _ if !self.seen_view.get() => *self.pending.borrow_mut() = Some(action),
            _ if toggle => self.toggle(None),
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
        println!("launchpad-desktop {} ({})", update::version(), update::channel());
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
        tracing::info!("launchpad-desktop {} ({}) starting", update::version(), update::channel());
        let (upd_tx, upd_rx) = async_channel::unbounded::<Upd>();
        let settings: Shared = Arc::new(Mutex::new(settings::Settings::load()));
        let (ui_tx, ui_rx) = async_channel::unbounded::<Ui>();
        let (engine, cmd) = Engine::new(settings.clone(), ui_tx);
        rt.spawn(engine.run());
        let (act_tx, act_rx) = async_channel::unbounded::<Action>();
        let tray_slot: Arc<Mutex<Option<ksni::Handle<LpTray>>>> = Arc::default();
        let (notifications, autostart) = (settings.lock().unwrap().notifications, desktop::autostart_on());
        let t = tray_slot.clone();
        rt.spawn(async move {
            let tray = LpTray { view: View::default(), notifications, autostart, icon: tray::icons(0, false), shortcut: ShortcutMenu::default(), tx: act_tx };
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
            spot_data: RefCell::new(Arc::default()),
            spotlight: RefCell::new(None),
            quick: RefCell::new(None),
            prefs: RefCell::new(None),
            shortcut: RefCell::new(ShortcutState::Starting),
            portal: RefCell::new(None),
            shortcut_events: RefCell::new(None),
            answered: RefCell::new(Default::default()),
            update: RefCell::new(None),
            updates: upd_tx,
        });
        let up = c.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = upd_rx.recv().await {
                up.on_update(msg);
            }
        });
        c.start_update_checks();
        // The spotlight is built now, hidden, so the shortcut shows it at once.
        c.spotlight();
        let (sc_tx, sc_rx) = async_channel::unbounded::<shortcut::Event>();
        c.start_shortcut(sc_tx);
        let sc = c.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = sc_rx.recv().await {
                sc.on_shortcut(event);
            }
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

/// Starts the newly installed companion once this one has quit (the single instance would otherwise take it).
fn restart_companion() {
    let exe = if std::path::Path::new("/usr/bin/launchpad-desktop").exists() { "/usr/bin/launchpad-desktop".to_string() } else { std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default() };
    // Wait until this process is gone (at most 30 s), else the new one would hand its arguments to this instance,
    // still registered on the bus, and exit.
    let script = "i=0; while kill -0 \"$1\" 2>/dev/null && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done; exec \"$0\" --background";
    let spawned = std::process::Command::new("/bin/sh").arg("-c").arg(script).arg(&exe).arg(std::process::id().to_string()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    if let Err(e) = spawned {
        tracing::warn!("restart: {e}");
    }
}

/// The action the arguments ask for (None: no action), or None at the outer level when they are invalid.
fn valid_args(args: &[String]) -> Option<Option<String>> {
    let mut it = args.iter().skip(1);
    let mut action = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--toggle" => action = Some("toggle".to_string()),
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
        assert_eq!(super::valid_args(&a("--toggle")), Some(Some("toggle".into())));
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

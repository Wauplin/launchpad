//! The companion's settings window (`/settings`, the menu's Settings…): the global shortcut and where it comes
//! from, the default Ask target of this device, notifications and Start at login.

use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use crate::engine::SpotData;
use crate::shortcut::State;

pub enum Out {
    ConfigureShortcut,
    /// Use (true) or remove (false) the GNOME custom shortcut.
    CustomShortcut(bool),
    AskBot(Option<String>),
    Notifications(bool),
    Autostart(bool),
}

pub struct Prefs {
    pub window: gtk::Window,
}

pub struct Now<'a> {
    pub shortcut: &'a State,
    pub custom_available: bool,
    pub custom_installed: bool,
    pub data: &'a Arc<SpotData>,
    pub ask_bot: Option<&'a str>,
    pub notifications: bool,
    pub autostart: bool,
}

fn row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = gtk::Label::with_mnemonic(label);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_mnemonic_widget(Some(widget));
    b.append(&l);
    b.append(widget);
    b
}

impl Prefs {
    pub fn new(app: &gtk::Application, now: Now, out: Rc<dyn Fn(Out)>) -> Prefs {
        let window = gtk::Window::builder().application(app).title("Launchpad companion settings").default_width(520).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        root.set_margin_start(18);
        root.set_margin_end(18);
        let shortcut = gtk::Label::new(Some(&now.shortcut.label()));
        shortcut.set_xalign(0.0);
        shortcut.set_wrap(true);
        root.append(&shortcut);
        if let State::Unavailable(why) = now.shortcut {
            let l = gtk::Label::new(Some(&format!("{why}. Any shortcut can run: launchpad-desktop --toggle")));
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_selectable(true);
            l.add_css_class("dim-label");
            root.append(&l);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if matches!(now.shortcut, State::Portal(_)) {
            let b = gtk::Button::with_mnemonic("_Change shortcut…");
            let o = out.clone();
            b.connect_clicked(move |_| o(Out::ConfigureShortcut));
            buttons.append(&b);
        }
        if now.custom_available {
            let b = gtk::Button::with_mnemonic(if now.custom_installed { "_Remove GNOME custom shortcut" } else { "Use a _GNOME custom shortcut" });
            let (o, on) = (out.clone(), !now.custom_installed);
            b.connect_clicked(move |_| o(Out::CustomShortcut(on)));
            buttons.append(&b);
        }
        root.append(&buttons);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let mut names = vec!["The workspace's Ask bot (like the web app)".to_string()];
        names.extend(now.data.bots.iter().map(|b| b.name.clone()));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let ask = gtk::DropDown::from_strings(&refs);
        ask.set_selected(now.ask_bot.and_then(|id| now.data.bots.iter().position(|b| b.id == id)).map_or(0, |i| i + 1) as u32);
        let (o, bots) = (out.clone(), now.data.bots.clone());
        ask.connect_selected_notify(move |d| o(Out::AskBot((d.selected() as usize).checked_sub(1).and_then(|i| bots.get(i)).map(|b| b.id.clone()))));
        root.append(&row("Default _Ask target", &ask));
        let notify = gtk::Switch::builder().active(now.notifications).valign(gtk::Align::Center).build();
        let o = out.clone();
        notify.connect_active_notify(move |s| o(Out::Notifications(s.is_active())));
        root.append(&row("_Notifications (asks and failures)", &notify));
        let auto = gtk::Switch::builder().active(now.autostart).valign(gtk::Align::Center).build();
        let o = out.clone();
        auto.connect_active_notify(move |s| o(Out::Autostart(s.is_active())));
        root.append(&row("_Start at login", &auto));
        let close = gtk::Button::with_mnemonic("_Close");
        close.set_halign(gtk::Align::End);
        let w = window.clone();
        close.connect_clicked(move |_| w.close());
        root.append(&close);
        window.set_child(Some(&root));
        Prefs { window }
    }
}

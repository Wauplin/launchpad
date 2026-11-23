//! The spotlight window (folio sketch S-SPOTLIGHT): one undecorated GTK window, created once and shown or hidden.
//! GNOME places it (Wayland gives apps no positions). The rows, the footer and the answer come from `spot.rs`; this
//! file draws them and maps keys (folio "Keyboard map"): ↑ ↓, Enter, Ctrl+Enter, Tab / Shift+Tab, Ctrl+D, Ctrl+W,
//! Ctrl+O, Ctrl+N, Esc (keeps the text 15 minutes) and Shift+Esc (discards it).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use crate::engine::{AskView, SpotData};
use crate::spot::{self, Act, BotInfo, Kept, Row, Target};

/// What the spotlight asks of the application.
pub enum Out {
    /// `key`: the message's Idempotency-Key (a new thread's session ID).
    Ask { bot: BotInfo, text: String, thread: Option<String>, session: Option<String>, key: String },
    /// Typed text that answers the open question of `session` (`session.answer`, like the web chat's box).
    Answer { session: String, request: String, count: usize, text: String },
    Open(String),
    /// Opens the quick launch form with this prompt.
    Launch(String),
    /// Continue in the browser: a launch draft (`bot` None) or a bot's new chat.
    Handoff { text: String, bot: Option<BotInfo> },
    Resolve(String),
    Command { name: String, rest: String },
}

#[derive(Default)]
struct State {
    data: Arc<SpotData>,
    /// The device's default Ask target (a bot ID).
    ask_bot: Option<String>,
    targets: Vec<Target>,
    target: usize,
    rows: Vec<Row>,
    kept: Kept,
    /// The last objects opened from the spotlight (label, URL), newest first.
    recent: Vec<(String, String)>,
    /// The ask the spotlight shows (answer mode); None in input mode.
    ask: Option<AskView>,
    /// The key of the last message the spotlight sent: it follows that ask only.
    sent: Option<String>,
    /// Ctrl+Enter on a new ask: open its thread once Home names it (the key of that ask).
    open_when_known: Option<String>,
    error: Option<String>,
}

pub struct Spotlight {
    pub window: gtk::Window,
    entry: gtk::Entry,
    target: gtk::Label,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    answer: gtk::Box,
    answer_head: gtk::Label,
    answer_text: gtk::Label,
    footer: gtk::Label,
    state: RefCell<State>,
    out: Rc<dyn Fn(Out)>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Spotlight {
    pub fn new(app: &gtk::Application, out: Rc<dyn Fn(Out)>) -> Rc<Spotlight> {
        let window = gtk::Window::builder().application(app).title("Launchpad spotlight").decorated(false).resizable(false).default_width(640).hide_on_close(true).build();
        window.add_css_class("launchpad-spotlight");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(12);
        root.set_margin_bottom(10);
        root.set_margin_start(12);
        root.set_margin_end(12);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let target = gtk::Label::with_mnemonic("_Ask");
        target.add_css_class("dim-label");
        target.set_tooltip_text(Some("Tab: next target, Shift+Tab: previous"));
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Ask, > go to, / commands, @ a bot"));
        target.set_mnemonic_widget(Some(&entry));
        top.append(&target);
        top.append(&entry);
        root.append(&top);

        let answer = gtk::Box::new(gtk::Orientation::Vertical, 4);
        answer.set_visible(false);
        let answer_head = gtk::Label::new(None);
        answer_head.set_xalign(0.0);
        answer_head.add_css_class("heading");
        let answer_text = gtk::Label::new(None);
        answer_text.set_xalign(0.0);
        answer_text.set_wrap(true);
        answer_text.set_selectable(true);
        answer.append(&answer_head);
        answer.append(&answer_text);
        root.append(&answer);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("navigation-sidebar");
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).max_content_height(420).child(&list).build();
        root.append(&scroller);
        let footer = gtk::Label::builder().xalign(0.0).wrap(true).accessible_role(gtk::AccessibleRole::Status).build();
        footer.add_css_class("dim-label");
        footer.add_css_class("caption");
        root.append(&footer);
        window.set_child(Some(&root));

        let spot = Rc::new(Spotlight { window, entry, target, list, scroller, answer, answer_head, answer_text, footer, state: RefCell::new(State::default()), out });

        let s = spot.clone();
        spot.entry.connect_changed(move |_| s.refresh_rows());
        let s = spot.clone();
        spot.entry.connect_activate(move |_| s.enter(false));
        let s = spot.clone();
        spot.list.connect_row_activated(move |_, row| {
            s.select(row.index() as usize);
            s.enter(false);
        });
        let s = spot.clone();
        spot.list.connect_row_selected(move |_, _| s.update_footer());
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let s = spot.clone();
        keys.connect_key_pressed(move |_, key, _, mods| s.key(key, mods));
        spot.window.add_controller(keys);
        // Clicking elsewhere closes it like Esc (GNOME takes the focus away).
        let s = spot.clone();
        spot.window.connect_is_active_notify(move |w| {
            if !w.is_active() && w.is_visible() {
                s.hide(false);
            }
        });
        spot
    }

    pub fn set_data(&self, data: Arc<SpotData>, ask_bot: Option<String>) {
        {
            let mut st = self.state.borrow_mut();
            let current = st.targets.get(st.target).cloned();
            st.targets = spot::targets(&data.bots, ask_bot.as_deref());
            // Keep the chosen target while it still exists.
            st.target = current.and_then(|c| st.targets.iter().position(|t| *t == c)).unwrap_or(0);
            st.data = data;
            st.ask_bot = ask_bot;
        }
        if self.window.is_visible() {
            self.refresh_rows();
        }
    }

    /// Shows it (with the portal's activation token, so GNOME gives it the focus), or hides it when it is shown.
    pub fn toggle(&self, token: Option<&str>, prefill: Option<&str>) {
        if self.window.is_visible() && self.window.is_active() && prefill.is_none() {
            self.hide(false);
            return;
        }
        self.show(token, prefill);
    }

    pub fn show(&self, token: Option<&str>, prefill: Option<&str>) {
        let text = match prefill {
            Some(p) => p.to_string(),
            None => self.state.borrow_mut().kept.restore(now()),
        };
        {
            let mut st = self.state.borrow_mut();
            // Each opening starts from the device's default target.
            st.target = 0;
            st.error = None;
        }
        self.entry.set_text(&text);
        self.entry.set_position(-1);
        self.refresh_rows();
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            self.window.set_startup_id(t);
        }
        self.window.present();
        self.entry.grab_focus();
    }

    /// Esc keeps the text 15 minutes; Shift+Esc (`discard`) drops it and leaves answer mode.
    pub fn hide(&self, discard: bool) {
        {
            let mut st = self.state.borrow_mut();
            if discard {
                st.kept.discard();
                st.ask = None;
            } else {
                let text = self.entry.text().to_string();
                st.kept.hide(&text, now());
            }
        }
        self.window.set_visible(false);
    }

    fn current_target(&self) -> Option<Target> {
        let st = self.state.borrow();
        st.targets.get(st.target).cloned()
    }

    fn refresh_rows(&self) {
        let text = self.entry.text().to_string();
        let rows = {
            let st = self.state.borrow();
            let target = st.targets.get(st.target);
            let cx = spot::Context { bots: &st.data.bots, target, entries: &st.data.entries, needs: &st.data.needs, recent: &st.recent, live: st.data.live };
            if st.ask.is_some() && !text.starts_with(['>', '/', '@']) { Vec::new() } else { spot::rows(&text, &cx) }
        };
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for r in &rows {
            let label = gtk::Label::new(Some(&r.label));
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            if r.header {
                label.add_css_class("dim-label");
                label.add_css_class("caption-heading");
            }
            if r.act.is_none() {
                row.set_selectable(false);
                row.set_activatable(false);
            }
            self.list.append(&row);
        }
        self.scroller.set_visible(!rows.is_empty());
        let first = spot::first_action(&rows);
        self.state.borrow_mut().rows = rows;
        match first {
            Some(i) => self.select(i),
            None => self.list.unselect_all(),
        }
        self.update_target();
        self.update_footer();
    }

    fn select(&self, i: usize) {
        if let Some(row) = self.list.row_at_index(i as i32) {
            // The focus stays in the field; the footer (a status) names the selected row for screen readers.
            self.list.select_row(Some(&row));
        }
    }

    fn selected(&self) -> Option<Act> {
        let i = self.list.selected_row()?.index() as usize;
        self.state.borrow().rows.get(i).and_then(|r| r.act.clone())
    }

    fn update_target(&self) {
        let st = self.state.borrow();
        let label = match (&st.ask, st.targets.get(st.target)) {
            (Some(a), _) => format!("Reply to {}", a.bot.name),
            (None, Some(Target::Bot(b))) => format!("Ask {}", b.name),
            (None, Some(Target::NewAgent)) => "New agent".into(),
            (None, None) => "Go to".into(),
        };
        let ws = if st.data.workspace_name.is_empty() { String::new() } else { format!(" · {}", st.data.workspace_name) };
        self.target.set_text(&format!("{label}{ws}"));
    }

    fn update_footer(&self) {
        let st = self.state.borrow();
        let text = if let Some(e) = &st.error {
            e.clone()
        } else if st.ask.is_some() && self.entry.text().is_empty() {
            "Type to reply in the same thread · Ctrl+O: open the thread · Ctrl+N: new ask · Esc: close (the thread keeps running)".into()
        } else if st.ask.is_some() && !self.entry.text().starts_with(['>', '/', '@']) {
            "Enter: reply in the same thread · Ctrl+Enter: reply and open the thread · Ctrl+N: new ask".into()
        } else {
            drop(st);
            let selected = self.list.selected_row().and_then(|r| r.child()).and_then(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.text().to_string());
            let keys = spot::footer(self.selected().as_ref());
            match selected {
                Some(label) => format!("{label} — {keys}"),
                None => keys,
            }
        };
        self.footer.set_text(&text);
    }

    /// Moves the selection, skipping headings.
    fn step(&self, delta: i32) {
        let st = self.state.borrow();
        let len = st.rows.len() as i32;
        let mut i = self.list.selected_row().map_or(-1, |r| r.index());
        for _ in 0..len {
            i = (i + delta).rem_euclid(len.max(1));
            if st.rows.get(i as usize).is_some_and(|r| r.act.is_some()) {
                drop(st);
                self.select(i as usize);
                self.update_footer();
                return;
            }
        }
    }

    fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        match key {
            gdk::Key::Escape => self.hide(shift),
            gdk::Key::Up => self.step(-1),
            gdk::Key::Down => self.step(1),
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                {
                    let mut st = self.state.borrow_mut();
                    let len = st.targets.len();
                    st.target = spot::cycle(st.target, len, if shift || key == gdk::Key::ISO_Left_Tab { -1 } else { 1 });
                    st.ask = None;
                }
                self.answer.set_visible(false);
                self.refresh_rows();
            }
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => self.enter(true),
            gdk::Key::d | gdk::Key::D if ctrl => self.handoff(),
            gdk::Key::w | gdk::Key::W if ctrl => (self.out)(Out::Command { name: "workspace".into(), rest: String::new() }),
            gdk::Key::o | gdk::Key::O if ctrl => self.open_thread(),
            gdk::Key::n | gdk::Key::N if ctrl => {
                self.state.borrow_mut().ask = None;
                self.answer.set_visible(false);
                self.entry.set_text("");
                self.refresh_rows();
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn open_thread(&self) {
        let url = self.state.borrow().ask.as_ref().map(|a| a.thread_url.clone());
        if let Some(url) = url {
            (self.out)(Out::Open(url));
            self.hide(false);
        }
    }

    fn handoff(&self) {
        let text = self.entry.text().to_string();
        let bot = match (&self.state.borrow().ask, self.current_target()) {
            (Some(a), _) => Some(a.bot.clone()),
            (None, Some(Target::Bot(b))) => Some(b),
            _ => None,
        };
        (self.out)(Out::Handoff { text, bot });
        self.entry.set_text("");
        self.hide(true);
    }

    /// Enter (or a click) runs the selected row; in answer mode plain text replies in the thread.
    fn enter(&self, ctrl: bool) {
        let text = self.entry.text().to_string();
        let reply = self.state.borrow().ask.clone().filter(|_| !text.trim().is_empty() && !text.starts_with(['>', '/', '@']));
        if let Some(a) = reply {
            // An open question takes the typed text as its answer (a new message would wait behind it); a
            // permission request never does.
            if let Some(r) = a.answer.request.clone() {
                if r.permission {
                    self.error("This is a permission request: Allow or Deny it in the browser (Ctrl+O)");
                    return;
                }
                (self.out)(Out::Answer { session: a.session.clone(), request: r.id, count: r.count, text: text.trim().to_string() });
                self.entry.set_text("");
                if ctrl {
                    self.open_thread();
                }
                return;
            }
            self.send_ask(a.bot.clone(), text.trim().to_string(), a.thread.clone(), Some(a.session.clone()));
            if ctrl {
                self.open_thread();
            }
            return;
        }
        if self.state.borrow().ask.is_some() && text.trim().is_empty() {
            if ctrl {
                self.open_thread();
            }
            return;
        }
        let Some(act) = self.selected() else { return };
        match act {
            Act::Ask { bot, text } => {
                let key = self.send_ask(bot, text, None, None);
                // The new thread is known only once Home accepted the message (`on_ask`).
                self.state.borrow_mut().open_when_known = ctrl.then_some(key);
            }
            Act::Open(url) => {
                let label = self.list.selected_row().and_then(|r| r.child()).and_then(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.text().to_string()).unwrap_or_default();
                {
                    let mut st = self.state.borrow_mut();
                    st.recent.retain(|(_, u)| *u != url);
                    st.recent.insert(0, (label, url.clone()));
                    st.recent.truncate(5);
                }
                (self.out)(Out::Open(url));
                self.entry.set_text("");
                self.hide(true);
            }
            Act::Launch(text) => {
                (self.out)(Out::Launch(text));
                self.entry.set_text("");
                self.hide(true);
            }
            Act::Handoff(_) => self.handoff(),
            Act::PickBot(slug) => {
                self.entry.set_text(&format!("@{slug} "));
                self.entry.set_position(-1);
            }
            Act::Command { name, rest } => {
                let keep_open = name == "workspace";
                (self.out)(Out::Command { name, rest });
                if !keep_open {
                    self.entry.set_text("");
                    self.hide(true);
                } else {
                    self.entry.set_text("");
                }
            }
            Act::Resolve(text) => {
                (self.out)(Out::Resolve(text));
                self.entry.set_text("");
                self.hide(true);
            }
        }
    }

    fn send_ask(&self, bot: BotInfo, text: String, thread: Option<String>, session: Option<String>) -> String {
        let key = spot::new_key();
        {
            let mut st = self.state.borrow_mut();
            st.error = None;
            st.sent = Some(key.clone());
            st.open_when_known = None;
        }
        (self.out)(Out::Ask { bot, text, thread, session, key: key.clone() });
        self.entry.set_text("");
        key
    }

    /// A new state of an ask (from the engine). The spotlight follows the latest one it sent.
    pub fn on_ask(&self, view: &AskView) {
        {
            let mut st = self.state.borrow_mut();
            if st.sent.as_deref() != Some(view.key.as_str()) {
                return;
            }
            st.ask = Some(view.clone());
        }
        let open = view.thread.is_some() && self.state.borrow().open_when_known.as_deref() == Some(view.key.as_str());
        if open {
            self.state.borrow_mut().open_when_known = None;
            self.open_thread();
            return;
        }
        let a = &view.answer;
        let state = match (&view.error, a.done, &a.question, a.received) {
            (Some(e), _, _, _) => format!("not sent: {e}"),
            (None, _, Some(_), _) => "asks you".into(),
            (None, true, None, _) => "answered".into(),
            (None, false, None, true) => "working…".into(),
            (None, false, None, false) => "sending…".into(),
        };
        self.answer_head.set_text(&format!("{} · {state}", view.bot.name));
        let mut text = match (&a.question, a.text.is_empty()) {
            (Some(q), _) if a.request.as_ref().is_some_and(|r| r.permission) => format!("{q}\n\nA permission request: Allow or Deny it in the browser (Ctrl+O)."),
            (Some(q), _) => format!("{q}\n\nType your answer, or answer in the browser (Ctrl+O)."),
            (None, true) if a.done => a.error.clone().map(|e| format!("The turn ended with an error: {e}")).unwrap_or_else(|| "The bot ended its turn without an answer.".into()),
            (None, true) => String::new(),
            (None, false) => a.text.clone(),
        };
        if text.chars().count() > spot::ANSWER_CHARS {
            text = format!("{}… (the rest is in the browser: Ctrl+O)", text.chars().take(spot::ANSWER_CHARS).collect::<String>());
        }
        self.answer_text.set_text(&text);
        self.answer_text.set_visible(!text.is_empty());
        self.answer.set_visible(true);
        self.refresh_rows();
    }

    /// An error for the footer; an empty one clears it.
    pub fn error(&self, message: &str) {
        self.state.borrow_mut().error = Some(message.to_string()).filter(|m| !m.is_empty());
        self.update_footer();
    }

    /// Whether the spotlight still shows this ask (no notification is needed then).
    pub fn showing(&self, key: &str) -> bool {
        self.window.is_visible() && self.state.borrow().ask.as_ref().is_some_and(|a| a.key == key)
    }
}

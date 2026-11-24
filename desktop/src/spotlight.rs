//! The spotlight window (folio sketch S-SPOTLIGHT): one undecorated GTK window, created once and shown or hidden.
//! GNOME places it (Wayland gives apps no positions). The rows, the footer and the answer come from `spot.rs`; this
//! file draws them and maps keys (folio "Keyboard map"): ↑ ↓, Enter, Ctrl+Enter, Tab / Shift+Tab, Ctrl+D, Ctrl+W,
//! Ctrl+O, Ctrl+N, Esc (keeps the text 15 minutes) and Shift+Esc (discards it). Pasting an image (a screenshot: an
//! image in the clipboard and no plain text) attaches it to the next Ask, shown as a thumbnail above the rows with a
//! remove button; Backspace in an empty field removes the last one.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gtk::gdk;
use gtk::gdk_pixbuf;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::engine::{AskView, SpotData};
use crate::spot::{self, Act, BotInfo, Kept, Pasted, Row, Target};

/// Pasted PNGs at most this large are sent as they are (the web's `KEEP_BELOW`); larger ones become a JPEG of at most
/// `MAX_SIDE` px when that is smaller.
const KEEP_PNG: usize = 1536 * 1024;
const MAX_SIDE: i32 = 2048;

/// The bytes and type to send for a pasted image, like the web's `normalizeImage`.
fn encode(texture: &gdk::Texture) -> (Vec<u8>, &'static str) {
    let png = texture.save_to_png_bytes().to_vec();
    if png.len() <= KEEP_PNG {
        return (png, "image/png");
    }
    let jpeg = gdk::pixbuf_get_from_texture(texture).and_then(|pixbuf| {
        let (w, h) = (pixbuf.width(), pixbuf.height());
        let scale = (f64::from(MAX_SIDE) / f64::from(w.max(h))).min(1.0);
        let small = if scale < 1.0 {
            let size = |v: i32| ((f64::from(v) * scale).round() as i32).max(1);
            pixbuf.scale_simple(size(w), size(h), gdk_pixbuf::InterpType::Bilinear)?
        } else {
            pixbuf
        };
        small.save_to_bufferv("jpeg", &[("quality", "88")]).ok()
    });
    match jpeg {
        Some(j) if j.len() < png.len() => (j, "image/jpeg"),
        _ => (png, "image/png"),
    }
}

/// What the spotlight asks of the application.
pub enum Out {
    /// `key`: the message's Idempotency-Key (a new thread's session ID); `files`: the pasted images.
    /// Always a new thread (like the browser's top-bar Ask), also to the bot whose answer shows.
    Ask { bot: BotInfo, text: String, key: String, files: Vec<Pasted> },
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
    /// The ask whose answer the spotlight shows; None in input mode. Only its open question takes typed text (as an
    /// answer); a message never continues its thread.
    ask: Option<AskView>,
    /// The key of the last message the spotlight sent: it follows that ask only.
    sent: Option<String>,
    /// Ctrl+Enter on a new ask: open its thread once Home names it (the key of that ask).
    open_when_known: Option<String>,
    error: Option<String>,
    /// Pasted images for the next Ask, with their textures for the thumbnails.
    pasted: Vec<(Pasted, gdk::Texture)>,
}

pub struct Spotlight {
    me: Weak<Spotlight>,
    pub window: gtk::Window,
    entry: gtk::Entry,
    /// The thumbnails of the pasted images (hidden when there are none).
    shots: gtk::Box,
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
        let shots = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        shots.set_visible(false);
        root.append(&shots);

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

        let spot = Rc::new_cyclic(|me| Spotlight { me: me.clone(), window, entry, shots, target, list, scroller, answer, answer_head, answer_text, footer, state: RefCell::new(State::default()), out });

        // Ctrl+V, Shift+Insert and the field's Paste menu all emit the inner text's `paste-clipboard`: an image without
        // plain text attaches; anything else continues to GTK's own text paste.
        if let Some(text) = spot.entry.delegate().and_then(|d| d.downcast::<gtk::Text>().ok()) {
            let s = spot.clone();
            text.connect_paste_clipboard(move |t| {
                let clipboard = t.clipboard();
                let mimes: Vec<String> = clipboard.formats().mime_types().iter().map(|m| m.to_string()).collect();
                if !spot::pastes_image(&mimes) {
                    return;
                }
                t.stop_signal_emission_by_name("paste-clipboard");
                let s = s.clone();
                clipboard.read_texture_async(None::<&gio::Cancellable>, move |read| s.on_pasted(read));
            });
        }

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
        // Pasted images expire with the kept text (15 minutes after Esc).
        if !self.window.is_visible() && !self.state.borrow().kept.fresh(now()) && !self.state.borrow().pasted.is_empty() {
            self.state.borrow_mut().pasted.clear();
            self.refresh_shots();
        }
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
                st.pasted.clear();
            } else {
                let text = self.entry.text().to_string();
                st.kept.hide(&text, now());
            }
        }
        self.window.set_visible(false);
        self.refresh_shots();
    }

    /// A pasted image read from the clipboard: encoded, checked against Home's limits and shown as a thumbnail.
    fn on_pasted(&self, read: Result<Option<gdk::Texture>, glib::Error>) {
        let texture = match read {
            Ok(Some(t)) => t,
            Ok(None) => return self.error("The clipboard holds no image"),
            Err(e) => return self.error(&format!("Could not read the pasted image: {e}")),
        };
        let (bytes, mime) = encode(&texture);
        let sizes: Vec<usize> = self.state.borrow().pasted.iter().map(|(p, _)| p.bytes.len()).collect();
        if let Some(why) = spot::attach_refusal(&sizes, bytes.len()) {
            return self.error(why);
        }
        let ext = if mime == "image/png" { "png" } else { "jpg" };
        let name = match sizes.len() {
            0 => format!("Pasted image.{ext}"),
            n => format!("Pasted image {}.{ext}", n + 1),
        };
        {
            let mut st = self.state.borrow_mut();
            st.pasted.push((Pasted { name, mime: mime.into(), bytes }, texture));
            st.error = None;
        }
        self.refresh_shots();
        self.refresh_rows();
    }

    fn remove_pasted(&self, index: usize) {
        {
            let mut st = self.state.borrow_mut();
            if index < st.pasted.len() {
                st.pasted.remove(index);
            }
            st.error = None;
        }
        self.refresh_shots();
        self.refresh_rows();
        self.entry.grab_focus();
    }

    /// Draws the thumbnails of the pasted images, each with its remove button.
    fn refresh_shots(&self) {
        while let Some(child) = self.shots.first_child() {
            self.shots.remove(&child);
        }
        let st = self.state.borrow();
        for (i, (p, texture)) in st.pasted.iter().enumerate() {
            let picture = gtk::Picture::for_paintable(texture);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_size_request(72, 48);
            picture.set_alternative_text(Some(&p.name));
            picture.set_tooltip_text(Some(&p.name));
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("circular");
            remove.add_css_class("osd");
            remove.set_halign(gtk::Align::End);
            remove.set_valign(gtk::Align::Start);
            remove.set_focus_on_click(false);
            remove.set_tooltip_text(Some(&format!("Remove {}", p.name)));
            let me = self.me.clone();
            remove.connect_clicked(move |_| {
                if let Some(s) = me.upgrade() {
                    s.remove_pasted(i);
                }
            });
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&remove);
            self.shots.append(&overlay);
        }
        self.shots.set_visible(!st.pasted.is_empty());
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
            if st.ask.as_ref().is_some_and(|a| a.answer.request.is_some()) && !text.starts_with(['>', '/', '@']) {
                Vec::new()
            } else if !st.pasted.is_empty() && text.trim().is_empty() {
                // Pasted images alone are an Ask of the current bot.
                let images = if st.pasted.len() == 1 { "1 image".to_string() } else { format!("{} images", st.pasted.len()) };
                let row = |label: String, act: Option<Act>| Row { header: false, label, act };
                vec![match (target, st.data.live) {
                    (_, false) => row("Home is not connected: Ask and Launch wait for it".into(), None),
                    (Some(Target::Bot(b)), true) => row(format!("Ask {}: {images}", b.name), Some(Act::Ask { bot: b.clone(), text: String::new() })),
                    _ => row("Pasted images go with an Ask: Tab to a bot".into(), None),
                }]
            } else {
                spot::rows(&text, &cx)
            }
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
        let label = match (st.ask.as_ref().filter(|a| a.answer.request.is_some()), st.targets.get(st.target)) {
            (Some(a), _) => format!("Answer {}", a.bot.name),
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
        } else if st.ask.as_ref().is_some_and(|a| a.answer.request.is_some()) && self.entry.text().is_empty() && st.pasted.is_empty() {
            "Type your answer to the question · Ctrl+O: open the thread · Ctrl+N: new ask · Esc: close (the thread keeps running)".into()
        } else if st.ask.as_ref().is_some_and(|a| a.answer.request.is_some()) && !self.entry.text().starts_with(['>', '/', '@']) {
            "Enter: answer the question · Ctrl+Enter: answer and open the thread · Ctrl+N: new ask".into()
        } else if st.ask.is_some() && self.entry.text().is_empty() && st.pasted.is_empty() {
            "Type to ask again (a new thread) · Ctrl+O: open this thread · Ctrl+N: clear · Esc: close (the thread keeps running)".into()
        } else {
            drop(st);
            let selected = self.list.selected_row().and_then(|r| r.child()).and_then(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.text().to_string());
            let keys = spot::footer(self.selected().as_ref());
            match selected {
                Some(label) => format!("{label} — {keys}"),
                None => keys,
            }
        };
        let text = if self.state.borrow().pasted.is_empty() || self.state.borrow().error.is_some() { text } else { format!("{text} · Backspace: remove the last image") };
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
            gdk::Key::BackSpace if !ctrl && self.entry.text().is_empty() && !self.state.borrow().pasted.is_empty() => {
                let last = self.state.borrow().pasted.len() - 1;
                self.remove_pasted(last);
            }
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

    /// Pasted images go only with an Ask: the browser hand-off and the quick launch would drop them.
    fn refuse_with_images(&self) -> bool {
        let has = !self.state.borrow().pasted.is_empty();
        if has {
            self.error("Pasted images go with an Ask only: remove them (Backspace) or send them to a bot");
        }
        has
    }

    fn handoff(&self) {
        if self.refuse_with_images() {
            return;
        }
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
        let images = !self.state.borrow().pasted.is_empty();
        // An open question takes the typed text as its answer (a permission request never does); any other text is
        // a new ask in a new thread (the selected row), even to the bot whose answer shows.
        let shown = self.state.borrow().ask.clone();
        let question = shown.as_ref().and_then(|a| a.answer.request.clone());
        match spot::on_enter(question.as_ref().map(|r| r.permission), &text, images) {
            spot::OnEnter::Refuse(why) => return self.error(why),
            spot::OnEnter::Answer => {
                let (Some(a), Some(r)) = (shown, question) else { return };
                (self.out)(Out::Answer { session: a.session.clone(), request: r.id, count: r.count, text: text.trim().to_string() });
                self.entry.set_text("");
                if ctrl {
                    self.open_thread();
                }
                return;
            }
            spot::OnEnter::Nothing => {
                if ctrl {
                    self.open_thread();
                }
                return;
            }
            spot::OnEnter::Row => {}
        }
        if shown.is_some() && text.trim().is_empty() && !images {
            if ctrl {
                self.open_thread();
            }
            return;
        }
        let Some(act) = self.selected() else { return };
        match act {
            Act::Ask { bot, text } => {
                let key = self.send_ask(bot, text);
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
                if self.refuse_with_images() {
                    return;
                }
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

    /// A new thread of `bot` for this text and the pasted images, under a fresh key.
    fn send_ask(&self, bot: BotInfo, text: String) -> String {
        let key = spot::new_key();
        let files = {
            let mut st = self.state.borrow_mut();
            st.error = None;
            st.sent = Some(key.clone());
            st.open_when_known = None;
            std::mem::take(&mut st.pasted).into_iter().map(|(p, _)| p).collect()
        };
        (self.out)(Out::Ask { bot, text, key: key.clone(), files });
        self.refresh_shots();
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

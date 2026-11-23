//! Quick launch (folio sketch S-QUICKLAUNCH, minimal): the prompt, a folder of the workspace (or a temporary one),
//! When done for a Git folder, and a profile. Nothing starts without the Launch button (or Ctrl+Enter); Ctrl+D edits
//! the launch in the browser composer instead. Model, effort, presets, schedules and attachments stay in the browser.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use crate::actions::{self, WHEN_DONE};
use crate::engine::SpotData;

pub enum Out {
    /// Launch this spec with this Idempotency-Key; remember the folder and profile.
    Launch { spec: serde_json::Value, key: String, folder: Option<String>, profile: String },
    Handoff(String),
}

pub struct QuickLaunch {
    pub window: gtk::Window,
    prompt: gtk::TextView,
    folder: gtk::DropDown,
    when_done: gtk::DropDown,
    profile: gtk::DropDown,
    launch: gtk::Button,
    status: gtk::Label,
    data: RefCell<Arc<SpotData>>,
    /// The Idempotency-Key of this form: a second click (or a retry) launches once.
    key: RefCell<String>,
    out: Rc<dyn Fn(Out)>,
}

fn strings(items: &[String]) -> gtk::StringList {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    gtk::StringList::new(&refs)
}

fn labelled(text: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::with_mnemonic(text);
    label.set_xalign(0.0);
    label.set_width_chars(10);
    label.set_mnemonic_widget(Some(widget));
    row.append(&label);
    widget.set_hexpand(true);
    row.append(widget);
    row
}

impl QuickLaunch {
    pub fn new(app: &gtk::Application, out: Rc<dyn Fn(Out)>) -> Rc<QuickLaunch> {
        let window = gtk::Window::builder().application(app).title("Quick launch").default_width(560).hide_on_close(true).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.set_margin_top(16);
        root.set_margin_bottom(16);
        root.set_margin_start(16);
        root.set_margin_end(16);
        let prompt = gtk::TextView::builder().wrap_mode(gtk::WrapMode::WordChar).top_margin(6).bottom_margin(6).left_margin(6).right_margin(6).build();
        let frame = gtk::ScrolledWindow::builder().min_content_height(120).child(&prompt).build();
        frame.add_css_class("frame");
        let prompt_label = gtk::Label::with_mnemonic("_Prompt");
        prompt_label.set_xalign(0.0);
        prompt_label.set_mnemonic_widget(Some(&prompt));
        root.append(&prompt_label);
        root.append(&frame);
        let folder = gtk::DropDown::from_strings(&[]);
        let when_done = gtk::DropDown::from_strings(&WHEN_DONE.iter().map(|(_, l)| *l).collect::<Vec<_>>());
        let profile = gtk::DropDown::from_strings(&[]);
        root.append(&labelled("_Folder", &folder));
        root.append(&labelled("_When done", &when_done));
        root.append(&labelled("P_rofile", &profile));
        let status = gtk::Label::builder().xalign(0.0).wrap(true).accessible_role(gtk::AccessibleRole::Status).build();
        status.add_css_class("dim-label");
        root.append(&status);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_mnemonic("_Cancel");
        let browser = gtk::Button::with_mnemonic("_Edit in browser");
        browser.set_tooltip_text(Some("Ctrl+D: continue in the browser composer with this prompt"));
        let launch = gtk::Button::with_mnemonic("_Launch");
        launch.add_css_class("suggested-action");
        launch.set_tooltip_text(Some("Ctrl+Enter"));
        buttons.append(&cancel);
        buttons.append(&browser);
        buttons.append(&launch);
        root.append(&buttons);
        window.set_child(Some(&root));
        let q = Rc::new(QuickLaunch { window, prompt, folder, when_done, profile, launch, status, data: RefCell::new(Arc::default()), key: RefCell::new(crate::spot::new_key()), out });

        let w = q.window.clone();
        cancel.connect_clicked(move |_| w.set_visible(false));
        let s = q.clone();
        browser.connect_clicked(move |_| s.handoff());
        let s = q.clone();
        q.launch.connect_clicked(move |_| s.launch());
        // Changing any field makes a new launch (a new key); the same form clicked twice launches once.
        let s = q.clone();
        q.prompt.buffer().connect_changed(move |_| s.renew());
        for dd in [&q.folder, &q.when_done, &q.profile] {
            let s = q.clone();
            dd.connect_selected_notify(move |_| s.renew());
        }
        let s = q.clone();
        q.folder.connect_selected_notify(move |_| {
            s.select_folder_default();
            s.update_when_done();
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let s = q.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Escape => s.window.set_visible(false),
                gdk::Key::Return | gdk::Key::KP_Enter if ctrl => s.launch(),
                gdk::Key::d | gdk::Key::D if ctrl => s.handoff(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        q.window.add_controller(keys);
        q
    }

    fn renew(&self) {
        *self.key.borrow_mut() = crate::spot::new_key();
        self.status.set_text("");
    }

    /// The chosen folder's own When done (the web composer's default).
    fn select_folder_default(&self) {
        let i = self.folder.selected() as usize;
        let wd = i.checked_sub(1).and_then(|i| self.data.borrow().folders.get(i).and_then(|f| f.when_done.clone()));
        self.when_done.set_selected(actions::default_when_done(wd.as_deref()) as u32);
    }

    fn update_when_done(&self) {
        let git = self.selected_folder().is_none_or(|(_, git)| git);
        // A temporary folder has no worktree to keep or merge.
        self.when_done.set_sensitive(git && self.selected_folder().is_some());
    }

    /// The folder chosen: None for a temporary folder (the first entry).
    fn selected_folder(&self) -> Option<(String, bool)> {
        let i = self.folder.selected() as usize;
        let data = self.data.borrow();
        i.checked_sub(1).and_then(|i| data.folders.get(i)).map(|f| (f.id.clone(), f.git))
    }

    fn text(&self) -> String {
        let b = self.prompt.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }

    /// Opens the form with the workspace's choices: the device's last folder and profile, else the folder of the
    /// newest agent and the first profile (the web composer's default).
    pub fn open(&self, data: Arc<SpotData>, prompt: &str, last_folder: Option<&str>, last_profile: Option<&str>) {
        let folders: Vec<String> = std::iter::once("Temporary folder".to_string()).chain(data.folders.iter().map(|f| f.name.clone())).collect();
        self.folder.set_model(Some(&strings(&folders)));
        let pick = last_folder.or(data.newest_folder.as_deref()).and_then(|id| data.folders.iter().position(|f| f.id == id)).map_or(0, |i| i + 1);
        self.folder.set_selected(pick as u32);
        let profiles: Vec<String> = data.profiles.iter().map(|p| if p.name.is_empty() { p.id.clone() } else { format!("{} ({})", p.name, p.harness) }).collect();
        self.profile.set_model(Some(&strings(&profiles)));
        self.profile.set_selected(last_profile.and_then(|id| data.profiles.iter().position(|p| p.id == id)).unwrap_or(0) as u32);
        *self.data.borrow_mut() = data;
        self.select_folder_default();
        self.prompt.buffer().set_text(prompt);
        self.update_when_done();
        let ready = !self.data.borrow().profiles.is_empty();
        self.launch.set_sensitive(ready);
        self.status.set_text(if ready { "" } else { "No profile on Home yet: add one in Settings → Profiles." });
        self.renew();
        self.window.present();
        self.prompt.grab_focus();
    }

    fn launch(&self) {
        let text = self.text();
        if text.trim().is_empty() {
            self.status.set_text("Type a prompt first.");
            return;
        }
        let data = self.data.borrow().clone();
        if !data.live {
            self.status.set_text("Home is not connected: try again once it is.");
            return;
        }
        let Some(profile) = data.profiles.get(self.profile.selected() as usize) else { return };
        let folder = self.selected_folder();
        let when_done = WHEN_DONE.get(self.when_done.selected() as usize).map_or("keep", |(k, _)| *k);
        let spec = actions::launch_spec(text.trim(), profile, folder.as_ref().map(|(id, git)| (id.as_str(), *git)), when_done, data.workspace.as_deref().unwrap_or_default());
        self.launch.set_sensitive(false);
        self.status.set_text("Launching…");
        (self.out)(Out::Launch { spec, key: self.key.borrow().clone(), folder: folder.map(|(id, _)| id), profile: profile.id.clone() });
    }

    fn handoff(&self) {
        (self.out)(Out::Handoff(self.text()));
        self.window.set_visible(false);
    }

    /// Home's answer: the session's address (the window closes), or why it refused (the window stays, Launch again
    /// retries with the same key).
    pub fn launched(&self, result: &Result<String, String>) {
        self.launch.set_sensitive(true);
        match result {
            Ok(_) => {
                self.prompt.buffer().set_text("");
                self.window.set_visible(false);
            }
            Err(e) => self.status.set_text(&format!("Not launched: {e}")),
        }
    }
}

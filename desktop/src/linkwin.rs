//! The first-connection window (folio sketch S-CONNECT): sign in with Hugging Face (or paste a token) and pick the
//! Launchpad Space, or connect a dev or self-hosted Home by its address with a pairing code approved in a signed-in
//! window. The work runs in the engine (`LinkCmd`); this window only shows its progress (`LinkMsg`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::engine::{Cmd, LinkCmd, LinkMsg};

pub struct LinkWindow {
    pub window: gtk::Window,
    status: gtk::Label,
    spaces: gtk::ListBox,
    spaces_title: gtk::Label,
    code_box: gtk::Box,
    code: gtk::Label,
    approve: Rc<RefCell<String>>,
    send: Rc<dyn Fn(Cmd)>,
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>{}</b>", gtk::glib::markup_escape_text(text)));
    l.set_xalign(0.0);
    l
}

fn note(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.add_css_class("dim-label");
    l
}

impl LinkWindow {
    /// `client_id`: Sign in with Hugging Face is configured (else only the pasted token). `reauth`: the linked Space
    /// needs a new Hugging Face sign-in (the device stays).
    pub fn new(app: &gtk::Application, client_id: bool, reauth: bool, send: Rc<dyn Fn(Cmd)>, open: Rc<dyn Fn(&str)>) -> Rc<LinkWindow> {
        let window = gtk::Window::builder().application(app).title(if reauth { "Sign in to Launchpad again" } else { "Connect Launchpad" }).default_width(480).resizable(false).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        root.set_margin_start(18);
        root.set_margin_end(18);
        root.append(&heading(if reauth { "Hugging Face no longer accepts this sign-in" } else { "Connect to your Launchpad Home" }));

        // Home on a Hugging Face Space.
        root.append(&heading("Home on a Hugging Face Space"));
        if client_id {
            let sign_in = gtk::Button::with_label("Sign in with Hugging Face");
            sign_in.add_css_class("suggested-action");
            sign_in.set_tooltip_text(Some("Opens Hugging Face in your browser; approve Launchpad there"));
            let s = send.clone();
            sign_in.connect_clicked(move |_| s(Cmd::Link(LinkCmd::SignIn)));
            root.append(&sign_in);
        } else {
            root.append(&note("This build has no Hugging Face sign-in app: paste a Hugging Face access token with read access (huggingface.co/settings/tokens). It stays in your keyring."));
        }
        let token_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let token = gtk::PasswordEntry::new();
        token.set_hexpand(true);
        token.set_show_peek_icon(true);
        token.set_property("placeholder-text", "hf_…");
        let use_token = gtk::Button::with_label("Use token");
        let (s, t) = (send.clone(), token.clone());
        use_token.connect_clicked(move |_| {
            let text = t.text().to_string();
            if !text.trim().is_empty() {
                t.set_text("");
                s(Cmd::Link(LinkCmd::Token(text)));
            }
        });
        let token_label = gtk::Label::with_mnemonic("_Token");
        token_label.set_mnemonic_widget(Some(&token));
        token_row.append(&token_label);
        token_row.append(&token);
        token_row.append(&use_token);
        if client_id {
            let expander = gtk::Expander::new(Some("Use a Hugging Face token instead"));
            expander.set_child(Some(&token_row));
            root.append(&expander);
        } else {
            root.append(&token_row);
        }
        let spaces_title = heading("Choose your Launchpad Space");
        spaces_title.set_visible(false);
        let spaces = gtk::ListBox::new();
        spaces.add_css_class("boxed-list");
        spaces.set_visible(false);
        root.append(&spaces_title);
        root.append(&spaces);

        if !reauth {
            root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            root.append(&heading("Dev or self-hosted Home"));
            let address_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let address = gtk::Entry::new();
            address.set_hexpand(true);
            address.set_placeholder_text(Some("http://127.0.0.1:7317"));
            let connect = gtk::Button::with_label("Get a pairing code");
            let (s, a) = (send.clone(), address.clone());
            let go = move || {
                let text = a.text().to_string();
                if !text.trim().is_empty() {
                    s(Cmd::Link(LinkCmd::Address(text)));
                }
            };
            let go = Rc::new(go);
            let g = go.clone();
            connect.connect_clicked(move |_| g());
            address.connect_activate(move |_| go());
            let address_label = gtk::Label::with_mnemonic("_Address");
            address_label.set_mnemonic_widget(Some(&address));
            address_row.append(&address_label);
            address_row.append(&address);
            address_row.append(&connect);
            root.append(&address_row);
        }
        let code_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        code_box.set_visible(false);
        let code = gtk::Label::new(None);
        code.set_selectable(true);
        code.set_xalign(0.0);
        code_box.append(&note("Approve this code in Settings → Devices of a signed-in Launchpad window. It expires after five minutes."));
        code_box.append(&code);
        let approve = Rc::new(RefCell::new(String::new()));
        let open_page = gtk::Button::with_label("Open Settings → Devices");
        let (u, o) = (approve.clone(), open.clone());
        open_page.connect_clicked(move |_| o(&u.borrow()));
        code_box.append(&open_page);
        root.append(&code_box);

        let status = note("");
        status.set_selectable(true);
        root.append(&status);
        let close = gtk::Button::with_label("Cancel");
        close.set_halign(gtk::Align::End);
        let (s, w) = (send.clone(), window.clone());
        close.connect_clicked(move |_| {
            s(Cmd::Link(LinkCmd::Cancel));
            w.close();
        });
        root.append(&close);
        let s = send.clone();
        window.connect_close_request(move |_| {
            s(Cmd::Link(LinkCmd::Cancel));
            gtk::glib::Propagation::Proceed
        });
        let keys = gtk::EventControllerKey::new();
        let w = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                w.close();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        window.add_controller(keys);
        window.set_child(Some(&root));
        Rc::new(LinkWindow { window, status, spaces, spaces_title, code_box, code, approve, send })
    }

    pub fn show(&self) {
        self.window.present();
    }

    pub fn update(&self, msg: LinkMsg) {
        match msg {
            LinkMsg::Busy(text) => self.status.set_text(&text),
            LinkMsg::Error(text) => self.status.set_text(&text),
            LinkMsg::Spaces(found) => {
                while let Some(child) = self.spaces.first_child() {
                    self.spaces.remove(&child);
                }
                self.status.set_text(match found.len() {
                    0 => "No Launchpad Space found on your account or its organizations. Deploy Home on a Space first (deploy/hf-home), or connect by address below.",
                    1 => "Found your Launchpad Space.",
                    _ => "Several Launchpad Spaces found: choose one.",
                });
                for home in found {
                    let state = match (home.answers, home.stage.as_deref()) {
                        (true, _) => "running".to_string(),
                        (false, Some(stage)) => stage.to_lowercase(),
                        (false, None) => "unknown".into(),
                    };
                    let label = format!("{} · {}{}", home.space, state, if home.own { "" } else { " · organization" });
                    let button = gtk::Button::with_label(&label);
                    let (send, space, origin) = (self.send.clone(), home.space.clone(), home.origin.clone());
                    button.connect_clicked(move |_| send(Cmd::Link(LinkCmd::Space { space: space.clone(), origin: origin.clone() })));
                    self.spaces.append(&button);
                }
                let any = self.spaces.first_child().is_some();
                self.spaces.set_visible(any);
                self.spaces_title.set_visible(any);
            }
            LinkMsg::Code { code, approve } => {
                self.code.set_markup(&format!("<span size=\"xx-large\" font_family=\"monospace\" weight=\"bold\">{}</span>", gtk::glib::markup_escape_text(&code)));
                *self.approve.borrow_mut() = approve;
                self.code_box.set_visible(true);
                self.status.set_text("Waiting for approval…");
            }
            LinkMsg::Done => self.window.close(),
        }
    }
}

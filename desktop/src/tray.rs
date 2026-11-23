//! The top-bar icon and its menu: a StatusNotifierItem (ksni, plain D-Bus). On Ubuntu the "Ubuntu AppIndicators"
//! Shell extension (`ubuntu-appindicators@ubuntu.com`, enabled by the Ubuntu session mode) draws it in the top bar.
//! The menu is rebuilt from the engine's `View` on every change (folio sketch S-STATUS). Menu clicks become `Action`s
//! for the GTK side; the menu itself never calls Home.

use crate::engine::View;
use crate::model::{Group, Row};

#[derive(Clone, Debug)]
pub enum Action {
    Open(String),
    Ask,
    GoTo,
    QuickLaunch,
    OpenHome,
    Workspace(String),
    Notifications(bool),
    Autostart(bool),
    Connect,
    Retry,
    SignOut,
    Update,
    Quit,
    /// The companion's settings window.
    Settings,
    /// GNOME's dialog for the portal shortcut.
    ChangeShortcut,
    /// Add (true) or remove the GNOME custom shortcut.
    CustomShortcut(bool),
}

/// The shortcut part of the menu.
#[derive(Clone, Debug, Default)]
pub struct ShortcutMenu {
    pub label: String,
    /// Bound through the portal (Change shortcut… works).
    pub portal: bool,
    pub custom_available: bool,
    pub custom_installed: bool,
}

pub struct LpTray {
    pub view: View,
    pub notifications: bool,
    pub autostart: bool,
    pub icon: Vec<ksni::Icon>,
    pub shortcut: ShortcutMenu,
    pub tx: async_channel::Sender<Action>,
}

impl LpTray {
    fn item(&self, label: impl Into<String>, action: Option<Action>) -> ksni::MenuItem<Self> {
        let enabled = action.is_some();
        ksni::menu::StandardItem {
            label: escape(&label.into()),
            enabled,
            activate: Box::new(move |t: &mut Self| {
                if let Some(a) = &action {
                    let _ = t.tx.try_send(a.clone());
                }
            }),
            ..Default::default()
        }
        .into()
    }

    fn rows(&self, menu: &mut Vec<ksni::MenuItem<Self>>, title: &str, group: &Group, more: &str) {
        if group.rows.is_empty() {
            return;
        }
        let count = group.rows.len() + group.more;
        menu.push(self.item(format!("{title} ({count})"), None));
        for Row { label, url } in &group.rows {
            menu.push(self.item(format!("    {label}"), Some(Action::Open(url.clone()))));
        }
        if group.more > 0 {
            menu.push(self.item(format!("    {} more…", group.more), Some(Action::Open(more.to_string()))));
        }
    }
}

/// DBusMenu labels use `_` as the mnemonic marker.
fn escape(label: &str) -> String {
    label.replace('_', "__")
}

impl ksni::Tray for LpTray {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    fn title(&self) -> String {
        match self.view.status.count {
            0 => "Launchpad".into(),
            n => format!("Launchpad · {n} waiting for you"),
        }
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.title(), description: self.view.header.clone(), ..Default::default() }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use crate::conn::Conn;
        let v = &self.view;
        let mut menu = Vec::new();
        let head = if v.home_name.is_empty() { v.header.clone() } else { format!("{} · {}", short_home(&v.home_name), v.header) };
        menu.push(self.item(head, None));
        if let Some(problem) = &v.problem {
            menu.push(self.item(problem.chars().take(90).collect::<String>(), None));
        }
        if let Some(version) = &v.update {
            menu.push(self.item(format!("Update to {version}"), Some(Action::Update)));
        }
        match &v.conn {
            Some(Conn::NotLinked | Conn::Revoked) | None => {
                menu.push(self.item("Connect to Home…", Some(Action::Connect)));
                menu.push(ksni::MenuItem::Separator);
                menu.push(self.item("Quit", Some(Action::Quit)));
                return menu;
            }
            Some(Conn::AuthExpired) => menu.push(self.item("Sign in again…", Some(Action::Connect))),
            Some(Conn::Offline { .. } | Conn::Starting { .. } | Conn::Updating { .. } | Conn::Connecting { .. } | Conn::Incompatible(_)) => menu.push(self.item("Retry now", Some(Action::Retry))),
            Some(Conn::Live) => {}
        }
        if v.live {
            menu.push(ksni::MenuItem::Separator);
            let s = &v.status;
            if s.needs.rows.is_empty() {
                menu.push(self.item("Nothing waits for you", None));
            }
            self.rows(&mut menu, "Needs you", &s.needs, &v.today_url);
            self.rows(&mut menu, "Running", &s.running, &v.agents_url);
            self.rows(&mut menu, "Quota paused", &s.quota, &v.agents_url);
            self.rows(&mut menu, "Sleeping", &s.sleeping, &v.agents_url);
            self.rows(&mut menu, "Bots", &s.bots, &v.chats_url);
        }
        menu.push(ksni::MenuItem::Separator);
        menu.push(self.item("Ask…", v.live.then_some(Action::Ask)));
        menu.push(self.item("Go to…", Some(Action::GoTo)));
        menu.push(self.item("Quick launch…", v.live.then_some(Action::QuickLaunch)));
        menu.push(self.item("Open Launchpad", Some(Action::OpenHome)));
        menu.push(ksni::MenuItem::Separator);
        if !self.shortcut.label.is_empty() {
            menu.push(self.item(self.shortcut.label.clone(), None));
        }
        if self.shortcut.portal {
            menu.push(self.item("Change shortcut…", Some(Action::ChangeShortcut)));
        } else if self.shortcut.custom_available && !self.shortcut.custom_installed {
            menu.push(self.item("Use a GNOME custom shortcut", Some(Action::CustomShortcut(true))));
        }
        menu.push(self.item("Settings…", Some(Action::Settings)));
        if v.workspaces.len() > 1 {
            let selected = v.workspaces.iter().position(|(id, _)| Some(id) == v.workspace.as_ref()).unwrap_or(0);
            let ids: Vec<String> = v.workspaces.iter().map(|(id, _)| id.clone()).collect();
            menu.push(
                ksni::menu::SubMenu {
                    label: "Workspace".into(),
                    submenu: vec![
                        ksni::menu::RadioGroup {
                            selected,
                            select: Box::new(move |t: &mut Self, i| {
                                if let Some(id) = ids.get(i) {
                                    let _ = t.tx.try_send(Action::Workspace(id.clone()));
                                }
                            }),
                            options: v.workspaces.iter().map(|(_, name)| ksni::menu::RadioItem { label: escape(name), ..Default::default() }).collect(),
                        }
                        .into(),
                    ],
                    ..Default::default()
                }
                .into(),
            );
        }
        let notifications = self.notifications;
        menu.push(
            ksni::menu::CheckmarkItem {
                label: "Notifications".into(),
                checked: notifications,
                activate: Box::new(move |t: &mut Self| {
                    let _ = t.tx.try_send(Action::Notifications(!notifications));
                }),
                ..Default::default()
            }
            .into(),
        );
        let autostart = self.autostart;
        menu.push(
            ksni::menu::CheckmarkItem {
                label: "Start at login".into(),
                checked: autostart,
                activate: Box::new(move |t: &mut Self| {
                    let _ = t.tx.try_send(Action::Autostart(!autostart));
                }),
                ..Default::default()
            }
            .into(),
        );
        menu.push(ksni::MenuItem::Separator);
        menu.push(self.item("Sign out", Some(Action::SignOut)));
        menu.push(self.item("Quit", Some(Action::Quit)));
        menu
    }
}

fn short_home(name: &str) -> String {
    name.trim_start_matches("https://").trim_start_matches("http://").chars().take(40).collect()
}

/// The tray icon at `size` px: a teal disc with a white launch arrow, grey when not live, and the needs-you count in
/// an amber badge (`99+` past 99). ARGB32 in network byte order, as StatusNotifierItem wants it.
pub fn icon(size: i32, count: usize, live: bool) -> Option<ksni::Icon> {
    use gtk::cairo;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let s = size as f64;
        let (r, g, b) = if live { (0.06, 0.46, 0.43) } else { (0.45, 0.47, 0.50) };
        cr.set_source_rgb(r, g, b);
        cr.arc(s / 2.0, s / 2.0, s * 0.44, 0.0, std::f64::consts::TAU);
        cr.fill().ok()?;
        // A launch arrow pointing up-right.
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(s * 0.30, s * 0.70);
        cr.line_to(s * 0.66, s * 0.34);
        cr.set_line_width(s * 0.09);
        cr.stroke().ok()?;
        cr.move_to(s * 0.44, s * 0.30);
        cr.line_to(s * 0.72, s * 0.28);
        cr.line_to(s * 0.70, s * 0.56);
        cr.close_path();
        cr.fill().ok()?;
        if count > 0 {
            let text = if count > 99 { "99+".to_string() } else { count.to_string() };
            let radius = s * 0.27;
            let (cx, cy) = (s - radius, radius);
            cr.set_source_rgb(0.85, 0.47, 0.02);
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            cr.fill().ok()?;
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(radius * if text.len() > 2 { 0.9 } else if text.len() > 1 { 1.1 } else { 1.4 });
            let ext = cr.text_extents(&text).ok()?;
            cr.move_to(cx - ext.width() / 2.0 - ext.x_bearing(), cy - ext.height() / 2.0 - ext.y_bearing());
            cr.show_text(&text).ok()?;
        }
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?;
    let mut argb = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size as usize {
        for x in 0..size as usize {
            let px = u32::from_ne_bytes(data[y * stride + x * 4..y * stride + x * 4 + 4].try_into().ok()?);
            argb.extend_from_slice(&px.to_be_bytes());
        }
    }
    Some(ksni::Icon { width: size, height: size, data: argb })
}

pub fn icons(count: usize, live: bool) -> Vec<ksni::Icon> {
    [22, 32, 48].into_iter().filter_map(|s| icon(s, count, live)).collect()
}

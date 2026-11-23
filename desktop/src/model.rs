//! What the companion knows of Home: workspaces, folders, bots, light sessions (with Home's `group`) and Today, kept
//! fresh from the event stream; and the pure views the menu and the dock badge show. Only the fields the companion
//! uses are read; Home's records carry more.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::links::Links;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
}

/// A folder (`/api/projects`): which workspace its sessions belong to.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Folder {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub is_temp: bool,
    /// A plain folder (no Git): launches get no worktree.
    #[serde(default)]
    pub plain: bool,
    /// The folder's own When done, the web composer's default for a launch there.
    #[serde(default)]
    pub when_done: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BotSettings {
    #[serde(default)]
    pub name: String,
    /// Its Launchpad CLI access: `read`, `propose` or `admin` (the web's `askTarget` prefers admin).
    #[serde(default)]
    pub launchpad: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Bot {
    pub id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub settings: BotSettings,
    #[serde(default)]
    pub paused: bool,
}

/// A bot thread (`GET /api/chat-threads?workspace=`): a chat for Go to.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Thread {
    pub id: String,
    pub bot_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub archived_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Threads {
    #[serde(default)]
    pub threads: Vec<Thread>,
}

/// A bot channel or DM (`GET /api/channels`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub last_at: Option<i64>,
}

/// A folio of the workspace (`GET /api/folios/{workspace}`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Folio {
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub updated_at: i64,
}

/// A Local PR (`GET /api/predrafts?workspace=`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LocalPr {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub created_at: i64,
}

/// A profile (`GET /api/context/profiles`: never its environment): the quick launch's choices.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub harness: String,
}

impl Bot {
    pub fn name(&self) -> &str {
        if self.settings.name.is_empty() { &self.slug } else { &self.settings.name }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BotLaunch {
    pub id: String,
    #[serde(default)]
    pub thread: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LaunchSpec {
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub target: Option<Value>,
    #[serde(default)]
    pub bot: Option<BotLaunch>,
}

/// A light session record (`GET /api/sessions`, `sessions`/`updated` events).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: String,
    /// Home's group (`Session::group`): needs_you, running, quota_paused, sleeping, done, stopped, interrupted,
    /// failed, in_terminal, archived.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub status_since: i64,
    #[serde(default)]
    pub launch_spec: LaunchSpec,
}

impl Session {
    /// Home's group; a record without one (never from a Home that passed the version check) reads by status.
    pub fn group(&self) -> &str {
        if let Some(g) = &self.group {
            return g;
        }
        match self.status.as_str() {
            "starting" | "working" | "background" => "running",
            "quota_paused" => "quota_paused",
            "closed" => "archived",
            "error" => "failed",
            "waiting" => "done",
            other => other,
        }
    }

    fn target_folder(&self) -> Option<&str> {
        let t = self.launch_spec.target.as_ref()?;
        matches!(t.get("kind")?.as_str()?, "project" | "predraft").then(|| t.get("project_id")?.as_str()).flatten()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct TodayItem {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Today {
    #[serde(default)]
    pub items: Vec<TodayItem>,
}

/// One delivered bus event (`{"bus", "seq", "topic", "type", "payload"}`), or a control line (`hello`, `replayed`,
/// `resync`) with `type` only.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct BusEvent {
    #[serde(default)]
    pub bus: u64,
    #[serde(default)]
    pub topic: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub boot: Option<String>,
}

/// What an event asks the engine to load again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    Today,
    Sessions,
    Bots,
    Workspaces,
    Folders,
    /// The workspace's chats, channels, folios and Local PRs (Go to's index).
    Threads,
    Channels,
    Folios,
    LocalPrs,
    Profiles,
}

#[derive(Default)]
pub struct Model {
    pub workspaces: Vec<Workspace>,
    pub folders: Vec<Folder>,
    pub bots: Vec<Bot>,
    pub sessions: HashMap<String, Session>,
    pub today: Vec<TodayItem>,
    pub threads: Vec<Thread>,
    pub channels: Vec<Channel>,
    pub folios: Vec<Folio>,
    pub local_prs: Vec<LocalPr>,
    pub profiles: Vec<Profile>,
    /// Bumped by every change: the spotlight's index is rebuilt only when it moved.
    pub version: u64,
    /// All of it was loaded once since the stream (re)opened.
    pub loaded: bool,
}

impl Model {
    /// Applies an event of the stream; returns what must be loaded again (none for `sessions`/`updated`, which
    /// carries the light record).
    pub fn apply(&mut self, ev: &BusEvent) -> Option<Reload> {
        match (ev.topic.as_str(), ev.kind.as_str()) {
            ("sessions", "updated") => match serde_json::from_value::<Session>(ev.payload.clone()) {
                Ok(s) => {
                    self.sessions.insert(s.id.clone(), s);
                    self.version += 1;
                    None
                }
                Err(_) => Some(Reload::Sessions),
            },
            ("sessions", "changed") => Some(Reload::Sessions),
            ("today", _) => Some(Reload::Today),
            ("bots", "changed") => Some(Reload::Bots),
            ("bots", "threads") => Some(Reload::Threads),
            ("workspaces", _) => Some(Reload::Workspaces),
            ("projects", _) => Some(Reload::Folders),
            ("channels", "changed") => Some(Reload::Channels),
            ("folios", "changed") => Some(Reload::Folios),
            ("predrafts", _) => Some(Reload::LocalPrs),
            ("config", "changed") => Some(Reload::Profiles),
            _ => None,
        }
    }

    /// The workspace to follow: the chosen one while it exists, else the oldest (like the extension).
    pub fn workspace(&self, chosen: Option<&str>) -> Option<String> {
        if let Some(w) = chosen.filter(|c| self.workspaces.iter().any(|w| w.id == *c)) {
            return Some(w.to_string());
        }
        self.workspaces.iter().min_by_key(|w| w.created_at).map(|w| w.id.clone())
    }

    /// A Today item counts in a workspace when it names it, or names none (shared), like the web's `inWorkspace`.
    pub fn today_in<'a>(&'a self, ws: &'a str) -> impl Iterator<Item = &'a TodayItem> + 'a {
        self.today.iter().filter(move |i| i.workspaces.is_empty() || i.workspaces.iter().any(|w| w == ws))
    }

    /// The web's `scopeOf(…).hasSession`: the session's folder (or its path) or its launch's workspace or folder.
    pub fn session_in(&self, s: &Session, ws: &str) -> bool {
        let mine = |id: &str| self.folders.iter().any(|f| f.id == id && f.workspace_id.as_deref() == Some(ws));
        s.project_id.as_deref().is_some_and(mine)
            || (!s.cwd.is_empty()
                && self.folders.iter().any(|f| f.workspace_id.as_deref() == Some(ws) && !f.path.is_empty() && (s.cwd == f.path || s.cwd.starts_with(&format!("{}/", f.path)))))
            || s.launch_spec.workspace_id.as_deref() == Some(ws)
            || s.target_folder().is_some_and(mine)
    }
}

/// One clickable row of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub url: String,
}

/// Rows of one group, the first `MAX_ROWS`, and how many more there are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub rows: Vec<Row>,
    pub more: usize,
}

pub const MAX_ROWS: usize = 4;
const LABEL: usize = 60;

/// The live part of the menu for one workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Today's count of the workspace: the needs-you number of the icon and the dock badge.
    pub count: usize,
    /// Today's items, newest decision first as Home orders them.
    pub needs: Group,
    /// Running agents that are not bot threads.
    pub running: Group,
    pub quota: Group,
    pub sleeping: Group,
    /// Bots with working or asking threads ("Name · 1 asking, 2 working").
    pub bots: Group,
}

pub fn cut(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    format!("{}…", line.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
}

fn group(rows: impl Iterator<Item = Row>) -> Group {
    let all: Vec<Row> = rows.collect();
    let more = all.len().saturating_sub(MAX_ROWS);
    Group { rows: all.into_iter().take(MAX_ROWS).collect(), more }
}

/// The menu's live groups (folio page Status, "Sessions versus bots").
pub fn status(model: &Model, ws: &str, links: &Links) -> Status {
    let needs: Vec<&TodayItem> = model.today_in(ws).collect();
    let mut sessions: Vec<&Session> = model.sessions.values().filter(|s| model.session_in(s, ws)).collect();
    sessions.sort_by(|a, b| b.status_since.cmp(&a.status_since).then(a.id.cmp(&b.id)));
    let agents = |g: &'static str| {
        let sessions = &sessions;
        group(sessions.iter().filter(move |s| s.launch_spec.bot.is_none() && s.group() == g).map(|s| Row { label: cut(&s.title, LABEL), url: links.session(s) }))
    };
    let mut bots: Vec<(&Bot, usize, usize)> = model
        .bots
        .iter()
        .filter(|b| b.workspace_id == ws)
        .map(|b| {
            let threads = sessions.iter().filter(|s| s.launch_spec.bot.as_ref().is_some_and(|l| l.id == b.id));
            let (mut asking, mut working) = (0, 0);
            for s in threads {
                match s.group() {
                    "needs_you" => asking += 1,
                    "running" => working += 1,
                    _ => {}
                }
            }
            (b, asking, working)
        })
        .filter(|(_, a, w)| a + w > 0)
        .collect();
    bots.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.name().to_lowercase().cmp(&b.0.name().to_lowercase())));
    Status {
        count: needs.len(),
        needs: group(needs.iter().map(|i| Row { label: cut(&i.title, LABEL), url: links.today_item(&i.id) })),
        running: agents("running"),
        quota: agents("quota_paused"),
        sleeping: agents("sleeping"),
        bots: group(bots.into_iter().map(|(b, asking, working)| {
            let parts: Vec<String> = [(asking, "asking"), (working, "working")].into_iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!("{n} {w}")).collect();
            let paused = if b.paused { " · paused" } else { "" };
            Row { label: format!("{} · {}{paused}", cut(b.name(), 40), parts.join(", ")), url: links.bot(&b.id) }
        })),
    }
}

/// The workspace's bots, as the spotlight's targets.
pub fn bot_infos(model: &Model, ws: &str) -> Vec<crate::spot::BotInfo> {
    model
        .bots
        .iter()
        .filter(|b| b.workspace_id == ws)
        .map(|b| crate::spot::BotInfo { id: b.id.clone(), name: b.name().to_string(), slug: b.slug.clone(), admin: b.settings.launchpad == "admin", created_at: b.created_at, paused: b.paused })
        .collect()
}

/// Fixed pages of Go to.
const PAGES: &[(&str, &str)] = &[
    ("Today", "/"),
    ("Agents", "/agents"),
    ("Chats", "/chats"),
    ("Folios", "/folios"),
    ("Schedules", "/schedules"),
    ("Knowledge", "/knowledge"),
    ("Settings", "/settings"),
    ("Settings · Devices", "/settings/devices"),
];

/// Go to's index of the workspace: fixed pages, Today cards, agents (not bot threads, not archived), open chats,
/// bots, channels and DMs, folios, open Local PRs and folders, each with its browser address (`ws=` included).
pub fn index(model: &Model, ws: &str, links: &Links) -> Vec<crate::search::Entry> {
    use crate::search::{Entry, Kind};
    let mut out: Vec<Entry> = PAGES.iter().map(|(t, p)| Entry::new(Kind::Page, t, links.page(p, &[]), false, 0)).collect();
    let needs: std::collections::HashSet<&str> = model.today_in(ws).filter_map(|i| i.session_id.as_deref()).collect();
    out.extend(model.today_in(ws).map(|i| Entry::new(Kind::Today, &i.title, links.today_item(&i.id), true, i.at)));
    out.extend(
        model
            .sessions
            .values()
            .filter(|s| s.launch_spec.bot.is_none() && s.group() != "archived" && model.session_in(s, ws))
            .map(|s| Entry::new(Kind::Session, &s.title, links.session(s), needs.contains(s.id.as_str()), s.status_since)),
    );
    let bots: HashMap<&str, &Bot> = model.bots.iter().filter(|b| b.workspace_id == ws).map(|b| (b.id.as_str(), b)).collect();
    for t in model.threads.iter().filter(|t| t.archived_at.is_none()) {
        if let Some(b) = bots.get(t.bot_id.as_str()) {
            let title = if t.title.is_empty() { "New chat" } else { &t.title };
            out.push(Entry::new(Kind::Chat, &format!("{title} · {}", b.name()), links.page(&format!("/bots/{}", t.bot_id), &[("thread", &t.id)]), false, t.created_at));
        }
    }
    out.extend(bots.values().map(|b| Entry::new(Kind::Bot, b.name(), links.bot(&b.id), false, b.created_at)));
    out.extend(model.channels.iter().filter(|c| c.workspace_id == ws && !c.archived).map(|c| {
        let title = if c.title.is_empty() { format!("#{}", c.name) } else { c.title.clone() };
        Entry::new(Kind::Channel, &title, links.page(&format!("/channels/{}", c.id), &[]), false, c.last_at.unwrap_or(0))
    }));
    out.extend(model.folios.iter().map(|f| Entry::new(Kind::Folio, if f.title.is_empty() { &f.slug } else { &f.title }, links.page(&format!("/folios/{}", f.slug), &[]), false, f.updated_at)));
    out.extend(model.local_prs.iter().filter(|p| !p.archived).map(|p| Entry::new(Kind::LocalPr, &p.title, links.page(&format!("/predrafts/{}", p.id), &[]), false, p.created_at)));
    out.extend(model.folders.iter().filter(|f| !f.is_temp && f.workspace_id.as_deref() == Some(ws)).map(|f| Entry::new(Kind::Folder, &f.name, links.page(&format!("/projects/{}", f.id), &[]), false, 0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model() -> Model {
        let mut m = Model { workspaces: vec![Workspace { id: "w2".into(), name: "Side".into(), created_at: 5 }, Workspace { id: "w1".into(), name: "Work".into(), created_at: 1 }], ..Default::default() };
        m.folders = vec![Folder { id: "f1".into(), path: "/data/app".into(), workspace_id: Some("w1".into()), ..Default::default() }, Folder { id: "f2".into(), path: "/data/side".into(), workspace_id: Some("w2".into()), ..Default::default() }];
        m.bots = vec![Bot { id: "b1".into(), workspace_id: "w1".into(), slug: "hubert".into(), settings: BotSettings { name: "Hubert".into(), ..Default::default() }, ..Default::default() }];
        let session = |id: &str, group: &str, folder: &str, bot: bool, at: i64| -> Session {
            serde_json::from_value(json!({"id": id, "title": format!("Task {id}"), "status": "working", "group": group, "project_id": folder, "status_since": at,
                "launch_spec": {"bot": if bot { json!({"id": "b1", "name": "Hubert", "thread": "t1"}) } else { Value::Null }}})).unwrap()
        };
        for s in [session("s1", "running", "f1", false, 3), session("s2", "running", "f2", false, 4), session("s3", "needs_you", "f1", true, 5), session("s4", "running", "f1", true, 6), session("s5", "sleeping", "f1", false, 7)] {
            m.sessions.insert(s.id.clone(), s);
        }
        m.today = vec![
            TodayItem { id: "session:s3".into(), kind: "question".into(), title: "Hubert asks about the release".into(), workspaces: vec!["w1".into()], ..Default::default() },
            TodayItem { id: "runner:r1".into(), kind: "runner".into(), title: "Runner outdated".into(), ..Default::default() },
            TodayItem { id: "session:s2".into(), kind: "failed".into(), title: "Other".into(), workspaces: vec!["w2".into()], ..Default::default() },
        ];
        m
    }

    #[test]
    fn groups_sessions_and_bots_per_workspace() {
        let m = model();
        assert_eq!(m.workspace(None).as_deref(), Some("w1"));
        assert_eq!(m.workspace(Some("w2")).as_deref(), Some("w2"));
        assert_eq!(m.workspace(Some("gone")).as_deref(), Some("w1"));
        let links = Links::new("https://me-home.hf.space", "w1");
        let s = status(&m, "w1", &links);
        assert_eq!(s.count, 2);
        assert_eq!(s.needs.rows[0].url, "https://me-home.hf.space/?card=session%3As3&ws=w1");
        assert_eq!(s.running.rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Task s1"]);
        assert_eq!(s.sleeping.rows.len(), 1);
        assert_eq!(s.bots.rows, vec![Row { label: "Hubert · 1 asking, 1 working".into(), url: "https://me-home.hf.space/bots/b1?ws=w1".into() }]);
        let other = status(&m, "w2", &links);
        assert_eq!((other.count, other.running.rows.len(), other.bots.rows.len()), (2, 1, 0));
    }

    #[test]
    fn events_update_or_ask_for_reloads() {
        let mut m = model();
        let ev = |topic: &str, kind: &str, payload: Value| BusEvent { topic: topic.into(), kind: kind.into(), payload, ..Default::default() };
        assert_eq!(m.apply(&ev("sessions", "updated", json!({"id": "s1", "title": "Renamed", "status": "waiting", "group": "done", "project_id": "f1"}))), None);
        assert_eq!(m.sessions["s1"].group(), "done");
        assert_eq!(m.apply(&ev("today", "changed", Value::Null)), Some(Reload::Today));
        assert_eq!(m.apply(&ev("bots", "threads", Value::Null)), Some(Reload::Threads));
        assert_eq!(m.apply(&ev("bots", "changed", Value::Null)), Some(Reload::Bots));
        assert_eq!(m.apply(&ev("session/s1", "event", Value::Null)), None);
        assert_eq!(m.apply(&ev("sessions", "updated", json!("broken"))), Some(Reload::Sessions));
    }

    #[test]
    fn index_of_the_workspace() {
        let mut m = model();
        m.threads = vec![Thread { id: "t1".into(), bot_id: "b1".into(), title: "Release".into(), ..Default::default() }, Thread { id: "t2".into(), bot_id: "b1".into(), archived_at: Some(1), ..Default::default() }];
        m.folders[0].name = "app".into();
        let links = Links::new("http://h", "w1");
        let idx = index(&m, "w1", &links);
        let urls: Vec<&str> = idx.iter().map(|e| e.url.as_str()).collect();
        assert!(urls.contains(&"http://h/bots/b1?thread=t1&ws=w1"));
        assert!(urls.contains(&"http://h/projects/f1?ws=w1"));
        assert!(urls.contains(&"http://h/agents?session=s1&ws=w1"));
        // Bot threads are chats, not agents; another workspace's agents and archived threads are left out.
        assert!(!urls.iter().any(|u| u.contains("session=s3") || u.contains("session=s2") || u.contains("thread=t2")));
        assert_eq!(bot_infos(&m, "w1").len(), 1);
    }

    #[test]
    fn groups_cap_rows_and_cut_labels() {
        let g = group((0..6).map(|i| Row { label: i.to_string(), url: String::new() }));
        assert_eq!((g.rows.len(), g.more), (MAX_ROWS, 2));
        assert_eq!(cut("A very long title\nsecond line", 10), "A very lo…");
        assert_eq!(cut("short", 10), "short");
    }
}

//! The engine: a tokio task that owns the link to Home, the event stream (`GET /api/events`, like
//! extension/src/background/events.ts), the model and the connection state. It sends finished views and
//! notifications to the GTK side (`Ui`), and takes the menu's and the link window's commands (`Cmd`).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::conn::{self, Conn, Failure};
use crate::hf::{self, FoundHome, HfAuth, HomeStatus};
use crate::home::{self, Home, HomeError};
use crate::links::Links;
use crate::model::{self, BusEvent, Model, Reload, Status};
use crate::rules::{Note, Rules};
use crate::secrets;
use crate::settings::{HfLogin, HomeKind, HomeLink, Settings};
use crate::search::Entry;
use crate::spot::{self, Answer, BotInfo};
use crate::sse;
use crate::actions;
use std::collections::HashMap;
use serde_json::Value;

pub type Shared = Arc<Mutex<Settings>>;

/// Home sends a keepalive comment every 15 s: a stream silent for this long is dead.
const SILENT: Duration = Duration::from_secs(45);
/// Bursts of `today` events make one request.
const TODAY_DEBOUNCE: Duration = Duration::from_millis(300);
/// Offline or starting this long: one notification.
const SAY_OFFLINE_MS: i64 = 2 * 60_000;
/// A stream that was live at least this long resets the retry delays; shorter ones count as failures, so a Home or
/// proxy that drops every stream at once is not asked again every second.
const STEADY: Duration = Duration::from_secs(30);
/// Reverse pairing: Home allows one poll every 2 s.
const POLL: Duration = Duration::from_millis(2500);

/// What the menu and the link window ask the engine.
#[derive(Debug)]
pub enum Cmd {
    SetWorkspace(String),
    RetryNow,
    SignOut,
    Link(LinkCmd),
    /// Internal: a link task finished signing in with Hugging Face (`reauth`: for the linked Space).
    HfReady { auth: HfAuth, spaces: Vec<FoundHome>, reauth: bool },
    /// Internal: a link task got a device.
    Linked { link: HomeLink, device: String, hf: Option<HfAuth> },
    /// The spotlight's `/workspace` and Ctrl+W.
    NextWorkspace,
    /// Ask a bot in a new thread; `key` is the Idempotency-Key and the new session's ID. `files`: pasted images,
    /// uploaded before the message.
    Ask { bot: BotInfo, text: String, key: String, files: Vec<spot::Pasted> },
    /// Typed text answering the open question `request` of `session` (`POST /api/sessions/{id}/answer`).
    Answer { session: String, request: String, count: usize, text: String },
    /// Quick launch (`POST /api/sessions`); `key` is the Idempotency-Key.
    Launch { spec: Value, key: String },
    /// Continue in the browser composer with this text.
    Handoff(String),
    /// Open a pasted link or short ID.
    Resolve(String),
    /// Internal: Home accepted an ask (its thread), or refused it.
    AskSent { session: String, thread: Result<String, String> },
}

impl std::fmt::Debug for HfAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HfAuth")
    }
}

#[derive(Debug)]
pub enum LinkCmd {
    /// Sign in with Hugging Face in the browser (OAuth loopback).
    SignIn,
    /// A pasted Hugging Face token (when no OAuth client ID is configured).
    Token(String),
    Space { space: String, origin: String },
    /// Reverse pairing with a dev or self-hosted Home.
    Address(String),
    Cancel,
}

/// What the link window shows.
#[derive(Clone, Debug)]
pub enum LinkMsg {
    Busy(String),
    Error(String),
    Spaces(Vec<FoundHome>),
    /// A reverse-pairing code, and the page where the owner approves it.
    Code { code: String, approve: String },
    Done,
}

/// The menu's whole content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub conn: Option<Conn>,
    pub header: String,
    pub linked: bool,
    /// The Home's address and the space name (for the menu header).
    pub home_name: String,
    pub workspace: Option<String>,
    pub workspaces: Vec<(String, String)>,
    pub status: Status,
    /// Today, Agents, Chats and Home's root, with `ws=`.
    pub today_url: String,
    pub agents_url: String,
    pub chats_url: String,
    /// The browser's Ask (the workspace's Ask bot) and launch composer (`?open=ask`, `?open=launch`): the hand-offs
    /// until the spotlight (step 3) answers in place.
    pub ask_url: String,
    pub launch_url: String,
    /// Writes (Ask, launch) are possible: Live.
    pub live: bool,
    /// The keyring or another local problem.
    pub problem: Option<String>,
    /// A newer companion, as the menu line and whether a click installs it (`update::State::menu`); set by the GTK
    /// side, which owns updates.
    pub update: Option<(String, bool)>,
}

/// A folder the quick launch offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderChoice {
    pub id: String,
    pub name: String,
    /// A Git folder: the launch gets a worktree and When done.
    pub git: bool,
    /// The folder's own When done (the web composer's default there).
    pub when_done: Option<String>,
}

/// What the spotlight and the quick launch need, rebuilt when the model or the workspace changes.
#[derive(Clone, Debug, Default)]
pub struct SpotData {
    pub origin: String,
    pub workspace: Option<String>,
    pub workspace_name: String,
    pub live: bool,
    pub entries: Vec<Entry>,
    pub bots: Vec<BotInfo>,
    /// Today's needs-you rows (label, URL).
    pub needs: Vec<(String, String)>,
    pub folders: Vec<FolderChoice>,
    pub profiles: Vec<model::Profile>,
    /// The folder of the workspace's newest agent (the quick launch's default after the device's last one).
    pub newest_folder: Option<String>,
}

/// One ask as the spotlight shows it.
#[derive(Clone, Debug)]
pub struct AskView {
    /// The Idempotency-Key of the latest message (the spotlight follows only its own latest ask).
    pub key: String,
    pub session: String,
    pub bot: BotInfo,
    pub thread: Option<String>,
    /// The thread in the browser (`/bots/<bot>?thread=<thread>&ws=`).
    pub thread_url: String,
    pub answer: Answer,
    pub error: Option<String>,
}

pub enum Ui {
    View(Box<View>),
    Spot(Arc<SpotData>),
    Ask(Box<AskView>),
    /// A quick launch started (the session's address) or was refused.
    Launched(Result<String, String>),
    /// A spotlight request failed (shown in its footer).
    SpotError(String),
    Notify { note: Note, url: String },
    Link(LinkMsg),
    /// Open this address in the default browser.
    Open(String),
    /// Linked to a new Home (the first link turns Start at login on).
    Linked,
}

pub struct Engine {
    settings: Shared,
    ui: async_channel::Sender<Ui>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    tx: mpsc::UnboundedSender<Cmd>,
    http: reqwest::Client,
    home: Option<Home>,
    model: Model,
    conn: Conn,
    rules: Rules,
    position: Option<(String, u64)>,
    last_live: Option<i64>,
    problem: Option<String>,
    link_task: Option<AbortHandle>,
    hf_pending: Option<HfAuth>,
    /// The state a notification was last sent about (offline, revoked…), so each says it once.
    said: Option<String>,
    /// Streams in a row that ended within `STEADY` of opening.
    drops: u32,
    last_view: Option<View>,
    /// Asks followed through their session's events, by session ID.
    watches: HashMap<String, Watch>,
    /// What the last `SpotData` was built from (model version, workspace, live).
    spot_key: Option<(u64, Option<String>, bool)>,
}

/// An ask followed through its session: the session's events since it was sent, and where it stands.
struct Watch {
    key: String,
    bot: BotInfo,
    thread: Option<String>,
    events: Vec<Value>,
    last_seq: i64,
    error: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn clock(ms: i64) -> String {
    let secs = ms / 1000;
    let local = gtk::glib::DateTime::from_unix_local(secs).ok();
    local.and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

#[derive(Deserialize)]
struct Version {
    api: u32,
    #[serde(default)]
    min_client: u32,
    #[serde(default)]
    capabilities: Vec<String>,
}

enum Outcome {
    /// The stream ended or went silent after it was live; `steady`: it stayed live at least `STEADY`.
    Ended { steady: bool },
    Failed(HomeError),
    /// A command changed the link: start over.
    Changed,
}

impl Engine {
    pub fn new(settings: Shared, ui: async_channel::Sender<Ui>) -> (Engine, mpsc::UnboundedSender<Cmd>) {
        let (tx, cmds) = mpsc::unbounded_channel();
        let engine = Engine {
            settings,
            ui,
            cmds,
            tx: tx.clone(),
            http: home::http(),
            home: None,
            model: Model::default(),
            conn: Conn::NotLinked,
            rules: Rules::default(),
            position: None,
            last_live: None,
            problem: None,
            link_task: None,
            hf_pending: None,
            said: None,
            drops: 0,
            last_view: None,
            watches: HashMap::new(),
            spot_key: None,
        };
        (engine, tx)
    }

    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn save(&self, change: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.lock().unwrap();
        change(&mut s);
        if let Err(e) = s.save() {
            tracing::warn!("settings not saved: {e}");
        }
    }

    fn send(&self, msg: Ui) {
        let _ = self.ui.try_send(msg);
    }

    /// The linked Home from the settings and the keyring.
    async fn load(&mut self) {
        let settings = self.settings();
        let Some(link) = settings.home.clone() else { return };
        match secrets::load(secrets::DEVICE, &link.origin).await {
            Ok(Some(device)) => {
                let hf = if link.kind == HomeKind::Space { HfAuth::from_keyring(&link.origin, settings.client_id()).await.ok().flatten() } else { None };
                self.conn = if link.kind == HomeKind::Space && hf.is_none() { Conn::AuthExpired } else { Conn::Connecting { attempt: 0 } };
                self.home = Some(Home::new(self.http.clone(), link, device, hf));
                self.problem = None;
            }
            Ok(None) => {
                tracing::info!("no device secret in the keyring: not linked");
                self.conn = Conn::NotLinked;
            }
            Err(e) => {
                self.problem = Some(e.to_string());
                self.conn = Conn::NotLinked;
            }
        }
    }

    pub async fn run(mut self) {
        self.load().await;
        loop {
            self.emit();
            if self.home.is_none() || !self.conn.retries() {
                let Some(cmd) = self.cmds.recv().await else { return };
                self.command(cmd).await;
                continue;
            }
            match self.stream().await {
                // Incompatible says so once (`say_state`); a changed link says nothing.
                Outcome::Changed => {
                    self.say_state();
                    continue;
                }
                Outcome::Ended { steady } => {
                    self.drops = if steady { 0 } else { self.drops + 1 };
                    self.conn = Conn::Connecting { attempt: self.drops };
                }
                Outcome::Failed(e) => {
                    tracing::info!("Home: {}", e.message);
                    self.conn = self.conn.failed(&e.failure, now(), self.last_live);
                    if self.conn == Conn::Revoked {
                        self.revoked().await;
                    }
                }
            }
            self.say_state();
            self.emit();
            if !self.conn.retries() {
                continue;
            }
            // Wait before the next attempt; commands still work, Retry now cuts the wait.
            let wait = tokio::time::sleep(conn::backoff(self.conn.attempt()));
            tokio::pin!(wait);
            loop {
                tokio::select! {
                    _ = &mut wait => break,
                    cmd = self.cmds.recv() => {
                        let Some(cmd) = cmd else { return };
                        let retry = matches!(cmd, Cmd::RetryNow);
                        if self.command(cmd).await || retry { break }
                        self.emit();
                    }
                }
            }
        }
    }

    /// Handles a command; true when the link changed (the stream must start over).
    async fn command(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::SetWorkspace(id) => {
                self.switch_workspace(id).await;
                false
            }
            Cmd::NextWorkspace => {
                let mut list: Vec<(String, String)> = self.model.workspaces.iter().map(|w| (w.name.to_lowercase(), w.id.clone())).collect();
                list.sort();
                let current = self.workspace();
                let at = list.iter().position(|(_, id)| Some(id) == current.as_ref()).map_or(0, |i| (i + 1) % list.len().max(1));
                if let Some((_, id)) = list.get(at).cloned() {
                    self.switch_workspace(id).await;
                }
                false
            }
            Cmd::Ask { bot, text, key, files } => {
                self.ask(bot, text, key, files);
                false
            }
            Cmd::AskSent { session, thread } => {
                if let Some(w) = self.watches.get_mut(&session) {
                    match thread {
                        Ok(t) if !t.is_empty() => w.thread = Some(t),
                        Ok(_) => {}
                        Err(e) => w.error = Some(e),
                    }
                }
                self.ask_view(&session);
                false
            }
            Cmd::Answer { session, request, count, text } => {
                self.spawn_write(move |home, _| async move {
                    match actions::answer(&home, &session, &request, count, &text).await {
                        Ok(()) => Ui::SpotError(String::new()),
                        Err(e) => Ui::SpotError(format!("Not answered: {}", e.message)),
                    }
                });
                false
            }
            Cmd::Launch { spec, key } => {
                self.spawn_write(move |home, links| async move {
                    match actions::launch(&home, &spec, &key).await {
                        Ok(s) => Ui::Launched(Ok(links.session(&s))),
                        Err(e) => Ui::Launched(Err(e.message)),
                    }
                });
                false
            }
            Cmd::Handoff(text) => {
                let ws = self.workspace().unwrap_or_default();
                self.spawn_write(move |home, links| async move {
                    match actions::handoff(&home, &links, &ws, &text).await {
                        Ok(url) => Ui::Open(url),
                        Err(e) => match actions::share_url(&links, &text) {
                            Some(url) => {
                                tracing::info!("composer draft not saved ({}): the text goes in the address", e.message);
                                Ui::Open(url)
                            }
                            None => Ui::SpotError(format!("The composer draft was not saved: {}", e.message)),
                        },
                    }
                });
                false
            }
            Cmd::Resolve(text) => {
                self.spawn_write(move |home, _| async move {
                    match actions::resolve(&home, &text).await {
                        Ok(url) => Ui::Open(url),
                        Err(e) => Ui::SpotError(e.message),
                    }
                });
                false
            }
            Cmd::RetryNow => {
                if self.home.is_none() && self.settings().home.is_some() {
                    self.load().await;
                    return true;
                }
                if matches!(self.conn, Conn::Incompatible(_)) {
                    self.conn = Conn::Connecting { attempt: 0 };
                    return true;
                }
                false
            }
            Cmd::SignOut => {
                self.sign_out().await;
                true
            }
            Cmd::Link(link) => {
                self.link(link);
                false
            }
            Cmd::HfReady { auth, spaces, reauth } => {
                if reauth && let Some(home) = &mut self.home {
                    home.hf = Some(auth);
                    self.conn = Conn::Connecting { attempt: 0 };
                    self.send(Ui::Link(LinkMsg::Done));
                    return true;
                }
                self.hf_pending = Some(auth);
                self.send(Ui::Link(LinkMsg::Spaces(spaces)));
                false
            }
            Cmd::Linked { link, device, hf } => {
                let first = self.settings().autostart.is_none();
                self.save(|s| s.home = Some(link.clone()));
                self.home = Some(Home::new(self.http.clone(), link, device, hf));
                self.conn = Conn::Connecting { attempt: 0 };
                self.model = Model::default();
                self.rules = Rules::default();
                self.position = None;
                self.problem = None;
                self.said = None;
                self.drops = 0;
                self.send(Ui::Link(LinkMsg::Done));
                if first {
                    self.send(Ui::Linked);
                }
                true
            }
        }
    }

    /// Home refused this device: forget its secrets (the owner links again).
    async fn revoked(&mut self) {
        if let Some(home) = self.home.take() {
            let _ = secrets::delete_all(&home.link.origin).await;
        }
        self.save(|s| s.home = None);
        self.model = Model::default();
    }

    async fn sign_out(&mut self) {
        if let Some(home) = self.home.take() {
            // Best effort: delete this device on Home, then every secret here.
            match tokio::time::timeout(Duration::from_secs(5), home.delete("/api/devices/self")).await {
                Ok(Ok(())) => tracing::info!("device deleted on Home"),
                Ok(Err(e)) => tracing::warn!("device not deleted on Home: {e}"),
                Err(_) => tracing::warn!("device not deleted on Home: no answer"),
            }
            if let Err(e) = secrets::delete_all(&home.link.origin).await {
                tracing::warn!("keyring: {e}");
            }
        }
        self.save(|s| s.home = None);
        self.model = Model::default();
        self.conn = Conn::NotLinked;
        self.position = None;
        self.said = None;
    }

    fn workspace(&self) -> Option<String> {
        self.model.workspace(self.settings().workspace.as_deref())
    }

    /// Follows another workspace: its notifications start fresh (no burst for what was already there) and Go to's
    /// lists of that workspace load again.
    async fn switch_workspace(&mut self, id: String) {
        self.save(|s| s.workspace = Some(id));
        self.rules.reset();
        self.rules.on_today(&self.today_items(), now(), false);
        if self.home.is_some() && self.conn == Conn::Live {
            for what in [Reload::Threads, Reload::Folios, Reload::LocalPrs] {
                self.reload_soft(what).await;
            }
        }
    }

    /// Runs a write in its own task (the stream keeps flowing) and sends its result to the GTK side.
    fn spawn_write<F, Fut>(&self, work: F)
    where
        F: FnOnce(Home, Links) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Ui> + Send + 'static,
    {
        let (Some(home), Some(links)) = (self.home.clone(), self.links()) else {
            self.send(Ui::SpotError("Not connected to a Launchpad Home".into()));
            return;
        };
        let ui = self.ui.clone();
        tokio::spawn(async move {
            let _ = ui.send(work(home, links).await).await;
        });
    }

    /// Registers the ask before sending it, so no event of its session is missed: the new thread's session takes the
    /// message's key as its ID.
    fn ask(&mut self, bot: BotInfo, text: String, key: String, files: Vec<spot::Pasted>) {
        let session = key.clone();
        let watch = self.watches.entry(session.clone()).or_insert_with(|| Watch { key: key.clone(), bot: bot.clone(), thread: None, events: Vec::new(), last_seq: 0, error: None });
        watch.key = key.clone();
        watch.error = None;
        // Keep the newest asks only.
        if self.watches.len() > 20 {
            let oldest: Vec<String> = self.watches.keys().filter(|k| **k != session).take(self.watches.len() - 20).cloned().collect();
            for k in oldest {
                self.watches.remove(&k);
            }
        }
        self.ask_view(&session);
        let (tx, bot_id) = (self.tx.clone(), bot.id.clone());
        let Some(home) = self.home.clone() else {
            let _ = tx.send(Cmd::AskSent { session, thread: Err("Not connected to a Launchpad Home".into()) });
            return;
        };
        tokio::spawn(async move {
            let thread = actions::ask(&home, &bot_id, &text, &key, &files).await.map_err(|e| e.message);
            let _ = tx.send(Cmd::AskSent { session, thread });
        });
    }

    fn ask_view(&self, session: &str) {
        let (Some(w), Some(links)) = (self.watches.get(session), self.links()) else { return };
        let thread_url = match &w.thread {
            Some(t) => links.page(&format!("/bots/{}", w.bot.id), &[("thread", t)]),
            None => links.bot(&w.bot.id),
        };
        let view = AskView { key: w.key.clone(), session: session.to_string(), bot: w.bot.clone(), thread: w.thread.clone(), thread_url, answer: spot::answer(&w.events, &w.key), error: w.error.clone() };
        self.send(Ui::Ask(Box::new(view)));
    }

    /// An event of a followed session (`session/<id>`); also learns a new thread's ID from its session record.
    fn watch_event(&mut self, ev: &BusEvent) {
        if let Some(sid) = ev.topic.strip_prefix("session/")
            && let Some(w) = self.watches.get_mut(sid)
        {
            if let Some(body) = ev.payload.get("body") {
                w.events.push(body.clone());
            }
            w.last_seq = w.last_seq.max(ev.payload.get("seq").and_then(Value::as_i64).unwrap_or(0));
            let sid = sid.to_string();
            self.ask_view(&sid);
        } else if ev.topic == "sessions" && ev.kind == "updated"
            && let Some(id) = ev.payload.get("id").and_then(Value::as_str)
            && let Some(w) = self.watches.get_mut(id)
            && w.thread.is_none()
            && let Some(t) = ev.payload.pointer("/launch_spec/bot/thread").and_then(Value::as_str).filter(|t| !t.is_empty())
        {
            w.thread = Some(t.to_string());
            let id = id.to_string();
            self.ask_view(&id);
        }
    }

    /// After a reopened stream: the events each unfinished ask missed (`GET /api/sessions/{id}/events?after=`).
    async fn catch_up_asks(&mut self) {
        let Some(home) = self.home.clone() else { return };
        let open: Vec<(String, i64)> = self.watches.iter().filter(|(_, w)| !spot::answer(&w.events, &w.key).done).map(|(k, w)| (k.clone(), w.last_seq)).collect();
        for (sid, after) in open {
            let Ok(events) = home.get::<Vec<Value>>(&format!("/api/sessions/{sid}/events?after={after}")).await else { continue };
            if let Some(w) = self.watches.get_mut(&sid) {
                for e in events {
                    w.last_seq = w.last_seq.max(e.get("seq").and_then(Value::as_i64).unwrap_or(0));
                    if let Some(body) = e.get("body") {
                        w.events.push(body.clone());
                    }
                }
            }
            self.ask_view(&sid);
        }
    }

    /// The spotlight's data, sent again when the model, the workspace or the link state changed.
    fn spot(&mut self) {
        let ws = self.workspace();
        let live = self.conn == Conn::Live && self.model.loaded;
        let key = (self.model.version, ws.clone(), live);
        if self.spot_key.as_ref() == Some(&key) {
            return;
        }
        self.spot_key = Some(key);
        let Some(links) = self.links() else {
            self.send(Ui::Spot(Arc::new(SpotData::default())));
            return;
        };
        let origin = self.home.as_ref().map(|h| h.link.origin.clone()).unwrap_or_default();
        let data = match &ws {
            Some(ws) => {
                let mut newest: Vec<&model::Session> = self.model.sessions.values().filter(|s| s.launch_spec.bot.is_none() && self.model.session_in(s, ws) && s.project_id.is_some()).collect();
                newest.sort_by_key(|s| std::cmp::Reverse(s.status_since));
                SpotData {
                    origin,
                    workspace: Some(ws.clone()),
                    workspace_name: self.model.workspaces.iter().find(|w| &w.id == ws).map(|w| w.name.clone()).unwrap_or_default(),
                    live,
                    entries: model::index(&self.model, ws, &links),
                    bots: model::bot_infos(&self.model, ws),
                    needs: model::status(&self.model, ws, &links).needs.rows.into_iter().map(|r| (r.label, r.url)).collect(),
                    folders: {
                        let mut f: Vec<FolderChoice> = self.model.folders.iter().filter(|f| !f.is_temp && f.workspace_id.as_deref() == Some(ws.as_str())).map(|f| FolderChoice { id: f.id.clone(), name: f.name.clone(), git: !f.plain, when_done: f.when_done.clone() }).collect();
                        f.sort_by_key(|f| f.name.to_lowercase());
                        f
                    },
                    profiles: self.model.profiles.clone(),
                    newest_folder: newest.first().and_then(|s| s.project_id.clone()),
                }
            }
            None => SpotData { origin, live, ..Default::default() },
        };
        self.send(Ui::Spot(Arc::new(data)));
    }

    fn today_items(&self) -> Vec<model::TodayItem> {
        match self.workspace() {
            Some(ws) => self.model.today_in(&ws).cloned().collect(),
            None => Vec::new(),
        }
    }

    fn links(&self) -> Option<Links> {
        Some(Links::new(&self.home.as_ref()?.link.origin, &self.workspace().unwrap_or_default()))
    }

    fn view(&self) -> View {
        let settings = self.settings();
        let ws = self.workspace();
        let mut workspaces: Vec<(String, String)> = self.model.workspaces.iter().map(|w| (w.id.clone(), w.name.clone())).collect();
        workspaces.sort_by_key(|(_, name)| name.to_lowercase());
        let live = self.conn == Conn::Live && self.model.loaded;
        let links = self.links();
        let status = match (&links, &ws, self.model.loaded) {
            (Some(l), Some(ws), true) => model::status(&self.model, ws, l),
            _ => Status::default(),
        };
        let home_name = settings.home.as_ref().map(|h| h.space.clone().unwrap_or_else(|| h.origin.clone())).unwrap_or_default();
        View {
            conn: Some(self.conn.clone()),
            header: self.conn.header(clock),
            linked: self.home.is_some(),
            home_name,
            workspace: ws,
            workspaces,
            status,
            today_url: links.as_ref().map(|l| l.today()).unwrap_or_default(),
            agents_url: links.as_ref().map(|l| l.agents()).unwrap_or_default(),
            chats_url: links.as_ref().map(|l| l.chats()).unwrap_or_default(),
            ask_url: links.as_ref().map(|l| l.page("/", &[("open", "ask")])).unwrap_or_default(),
            launch_url: links.as_ref().map(|l| l.page("/", &[("open", "launch")])).unwrap_or_default(),
            live,
            problem: self.problem.clone(),
            update: None,
        }
    }

    fn emit(&mut self) {
        self.spot();
        let view = self.view();
        if self.last_view.as_ref() != Some(&view) {
            self.last_view = Some(view.clone());
            self.send(Ui::View(Box::new(view)));
        }
    }

    fn notify(&self, note: Note) {
        if !self.settings().notifying(now()) {
            return;
        }
        let url = match (self.links(), &note.item) {
            (Some(l), Some(item)) => l.today_item(item),
            (Some(l), None) => l.today(),
            (None, _) => String::new(),
        };
        self.send(Ui::Notify { note, url });
    }

    /// One notification per connection problem (folio: offline over 2 min, starting over 2 min, sign in, revoked,
    /// incompatible), and "Back online" after an offline one.
    fn say_state(&mut self) {
        let (key, title, body) = match &self.conn {
            Conn::Offline { since, .. } if now() - since >= SAY_OFFLINE_MS => ("offline", "Launchpad is offline", "Home does not answer. The companion keeps trying."),
            Conn::Starting { .. } if self.said.as_deref() != Some("starting") && self.last_live.is_some_and(|t| now() - t >= SAY_OFFLINE_MS) => ("starting", "Home is starting", "Its Space is building or waking up."),
            Conn::AuthExpired => ("auth", "Sign in to Launchpad again", "Hugging Face no longer accepts the companion's sign-in."),
            Conn::Revoked => ("revoked", "This device was removed from Home", "Connect the companion again from its menu."),
            Conn::Incompatible(which) => ("incompatible", "Launchpad companion", which.as_str()),
            Conn::Live if self.said.as_deref() == Some("offline") => {
                self.said = None;
                self.notify(Note { key: "connection".into(), title: "Launchpad is back online".into(), body: String::new(), item: None });
                return;
            }
            Conn::Live => {
                self.said = None;
                return;
            }
            _ => return,
        };
        if self.said.as_deref() == Some(key) {
            return;
        }
        self.said = Some(key.to_string());
        self.notify(Note { key: "connection".into(), title: title.into(), body: body.into(), item: None });
    }

    async fn reload(&mut self, what: Reload) -> Result<(), HomeError> {
        let home = self.home.clone().expect("linked");
        match what {
            Reload::Workspaces => self.model.workspaces = home.get("/api/workspaces").await?,
            Reload::Folders => self.model.folders = home.get("/api/projects").await?,
            Reload::Bots => self.model.bots = home.get("/api/bots").await?,
            Reload::Sessions => {
                let list: Vec<model::Session> = home.get("/api/sessions").await?;
                self.model.sessions = list.into_iter().map(|s| (s.id.clone(), s)).collect();
            }
            Reload::Today => {
                let today: model::Today = home.get("/api/today").await?;
                self.model.today = today.items;
            }
            Reload::Channels => self.model.channels = home.get("/api/channels").await?,
            // Like the extension: `/api/context/profiles` (id, name, harness, never their environment), not
            // `/api/config`.
            Reload::Profiles => {
                let list: Value = home.get("/api/context/profiles").await?;
                self.model.profiles = list.get("profiles").cloned().and_then(|p| serde_json::from_value(p).ok()).unwrap_or_default();
            }
            Reload::Threads | Reload::Folios | Reload::LocalPrs => {
                let Some(ws) = self.workspace() else { return Ok(()) };
                let q: String = url::form_urlencoded::byte_serialize(ws.as_bytes()).collect();
                match what {
                    Reload::Threads => self.model.threads = home.get::<model::Threads>(&format!("/api/chat-threads?workspace={q}&archived=0")).await?.threads,
                    Reload::Folios => self.model.folios = home.get(&format!("/api/folios/{q}")).await?,
                    _ => self.model.local_prs = home.get(&format!("/api/predrafts?workspace={q}")).await?,
                }
            }
        }
        self.model.version += 1;
        Ok(())
    }

    /// Go to's extra lists: a failure is logged and leaves the list as it was (never fails the stream).
    async fn reload_soft(&mut self, what: Reload) {
        if let Err(e) = self.reload(what).await {
            tracing::info!("{what:?} not loaded: {}", e.message);
        }
    }

    fn today_notes(&mut self, catch_up: bool) {
        let notes = self.rules.on_today(&self.today_items(), now(), catch_up);
        for note in notes {
            self.notify(note);
        }
    }

    /// One stream: version check, `GET /api/events`, then events until it ends, fails or a command changes the link.
    async fn stream(&mut self) -> Outcome {
        let home = self.home.clone().expect("linked");
        // Compatibility first: outside the range nothing else is called.
        match home.get::<Version>("/api/version").await {
            Ok(v) => {
                if let Some(which) = conn::compatibility(Some(v.api), v.min_client, &v.capabilities) {
                    self.conn = Conn::Incompatible(which);
                    return Outcome::Changed;
                }
            }
            // No `/api/version` (Home's 404 for an unknown route): Home is older than the companion. Other failures
            // (a 500, a proxy's page) are retried, never taken as an old Home.
            // On a Space only the daemon's own 404 counts (the Hub hides a Space it refuses behind its own 404 page).
            Err(HomeError { status: 404, message, .. }) if home.link.kind != HomeKind::Space || message == "no such route" => {
                self.conn = Conn::Incompatible(conn::compatibility(None, 0, &[]).unwrap_or_default());
                return Outcome::Changed;
            }
            Err(e) => return Outcome::Failed(e),
        }
        let response = match home.events(self.position.as_ref()).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e),
        };
        // Not Home's stream (a Space's waking page answered 200): retried, never read as events.
        if !response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream")) {
            let failure = if home.link.kind == HomeKind::Space { Failure::Starting } else { Failure::Other };
            return Outcome::Failed(HomeError { failure, message: "Home answered something other than its event stream".into(), status: response.status().as_u16() });
        }
        let mut live_at: Option<Instant> = None;
        let catch_up = self.rules_primed_after_gap();
        let mut body = response.bytes_stream();
        let mut parser = sse::Parser::default();
        let mut offered: Option<(String, u64)> = None;
        let mut today_due: Option<Instant> = None;
        let mut first_load = true;
        let silent = tokio::time::sleep(SILENT);
        tokio::pin!(silent);
        loop {
            let today_wait = async {
                match today_due {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                chunk = body.next() => {
                    let chunk = match chunk {
                        Some(Ok(c)) => c,
                        // Before the stream went live, an end is a failed attempt (backoff), not a drop.
                        Some(Err(e)) => return ended(live_at, e.to_string()),
                        None => return ended(live_at, "the stream ended".into()),
                    };
                    silent.as_mut().reset(tokio::time::Instant::now() + SILENT);
                    for frame in parser.push_bytes(&chunk) {
                        let Ok(ev) = serde_json::from_str::<BusEvent>(&frame.data) else { continue };
                        match ev.kind.as_str() {
                            "hello" => offered = Some((ev.boot.clone().unwrap_or_default(), ev.bus)),
                            "replayed" | "resync" => {
                                if let Some(o) = offered.take() { self.position = Some(o) }
                                // Every (re)opened stream reloads everything: cheap, and the companion may have slept.
                                for what in [Reload::Workspaces, Reload::Folders, Reload::Bots, Reload::Sessions, Reload::Today] {
                                    if let Err(e) = self.reload(what).await { return Outcome::Failed(e) }
                                }
                                for what in [Reload::Threads, Reload::Channels, Reload::Folios, Reload::LocalPrs, Reload::Profiles] {
                                    self.reload_soft(what).await;
                                }
                                self.catch_up_asks().await;
                                self.model.loaded = true;
                                self.conn = Conn::Live;
                                live_at.get_or_insert_with(Instant::now);
                                self.last_live = Some(now());
                                self.today_notes(first_load && catch_up);
                                first_load = false;
                                self.say_state();
                            }
                            _ => {
                                // The stream's position: the bus number of the last event applied.
                                if let Some(p) = &mut self.position { p.1 = frame.id.as_deref().and_then(sse::position).map_or(ev.bus, |(_, bus)| bus) }
                                self.last_live = Some(now());
                                self.watch_event(&ev);
                                match self.model.apply(&ev) {
                                    Some(Reload::Today) => today_due = Some(Instant::now() + TODAY_DEBOUNCE),
                                    Some(what @ (Reload::Threads | Reload::Channels | Reload::Folios | Reload::LocalPrs | Reload::Profiles)) => self.reload_soft(what).await,
                                    Some(what) => if let Err(e) = self.reload(what).await { return Outcome::Failed(e) },
                                    None => {}
                                }
                            }
                        }
                    }
                    self.emit();
                }
                _ = today_wait => {
                    today_due = None;
                    if let Err(e) = self.reload(Reload::Today).await { return Outcome::Failed(e) }
                    self.today_notes(false);
                    self.emit();
                }
                _ = &mut silent => return ended(live_at, "the stream went silent".into()),
                cmd = self.cmds.recv() => {
                    let Some(cmd) = cmd else { return Outcome::Changed };
                    if self.command(cmd).await { return Outcome::Changed }
                    self.emit();
                }
            }
        }
    }

    /// The stream reopens after a gap (offline, a dropped stream): new items then make a summary.
    fn rules_primed_after_gap(&self) -> bool {
        self.position.is_some()
    }

    fn link(&mut self, cmd: LinkCmd) {
        if let LinkCmd::Cancel = cmd {
            if let Some(task) = self.link_task.take() {
                task.abort();
            }
            return;
        }
        if let Some(task) = self.link_task.take() {
            task.abort();
        }
        let (ui, tx, http) = (self.ui.clone(), self.tx.clone(), self.http.clone());
        let say = move |m: LinkMsg| {
            let _ = ui.try_send(Ui::Link(m));
        };
        let reauth = self.home.as_ref().is_some_and(|h| h.link.kind == HomeKind::Space) && self.conn == Conn::AuthExpired;
        let reauth_origin = self.home.as_ref().map(|h| h.link.origin.clone()).unwrap_or_default();
        let client_id = self.settings().client_id();
        let pending = self.hf_pending.clone();
        let open_ui = self.ui.clone();
        let task = tokio::spawn(async move {
            let result: anyhow::Result<()> = async {
                secrets::available().await?;
                match cmd {
                    LinkCmd::SignIn | LinkCmd::Token(_) => {
                        let auth = match cmd {
                            LinkCmd::Token(token) => {
                                say(LinkMsg::Busy("Checking the token…".into()));
                                HfAuth::token(token.trim().to_string())
                            }
                            _ => {
                                let client_id = client_id.ok_or_else(|| anyhow::anyhow!("Sign in with Hugging Face is not configured: paste a token instead"))?;
                                say(LinkMsg::Busy("Approve Launchpad in your browser…".into()));
                                hf::sign_in(&http, &client_id, |url| {
                                    let _ = open_ui.try_send(Ui::Open(url));
                                })
                                .await?
                            }
                        };
                        let me = hf::whoami(&http, &auth).await?;
                        tracing::info!("signed in to Hugging Face as {}", me.name);
                        if reauth {
                            auth.keep_for(&reauth_origin).await?;
                            let _ = tx.send(Cmd::HfReady { auth, spaces: Vec::new(), reauth: true });
                            return Ok(());
                        }
                        say(LinkMsg::Busy("Looking for your Launchpad Space…".into()));
                        let spaces = hf::discover(&http, &auth).await?;
                        let _ = tx.send(Cmd::HfReady { auth, spaces, reauth: false });
                    }
                    LinkCmd::Space { space, origin } => {
                        let auth = pending.ok_or_else(|| anyhow::anyhow!("Sign in with Hugging Face first"))?;
                        say(LinkMsg::Busy(format!("Connecting to {space}…")));
                        match hf::home_status(&http, &auth, &origin).await {
                            HomeStatus::Home { complete: true } => {}
                            HomeStatus::Home { complete: false } => anyhow::bail!("Finish Home's setup on its Space first"),
                            HomeStatus::Starting => anyhow::bail!("Home is starting (its Space builds or wakes up). Try again in a minute."),
                            HomeStatus::Unreachable(why) => anyhow::bail!(why),
                        }
                        let bearer = auth.bearer(&http).await?;
                        let name = device_name();
                        let data = home::post_unlinked(&http, &format!("{origin}/_home/desktop/connect"), &serde_json::json!({"name": name}), Some(&bearer)).await.map_err(|(_, e)| anyhow::anyhow!(e))?;
                        let device = data.get("device").and_then(|d| d.as_str()).ok_or_else(|| anyhow::anyhow!("Home gave no device"))?.to_string();
                        secrets::store(secrets::DEVICE, &origin, &device).await?;
                        auth.keep_for(&origin).await?;
                        let login = if auth.oauth { HfLogin::OAuth } else { HfLogin::Token };
                        let link = HomeLink { kind: HomeKind::Space, origin, space: Some(space), device_name: name, hf: Some(login) };
                        let _ = tx.send(Cmd::Linked { link, device, hf: Some(auth) });
                    }
                    LinkCmd::Address(input) => {
                        let origin = home::address_origin(&input).map_err(anyhow::Error::msg)?;
                        let name = device_name();
                        say(LinkMsg::Busy("Asking Home for a pairing code…".into()));
                        let pair = home::post_unlinked(&http, &format!("{origin}/auth/reverse/start"), &serde_json::json!({"name": name, "kind": "desktop"}), None).await.map_err(|(_, e)| anyhow::anyhow!(e))?;
                        let code = pair.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                        let secret = pair.get("secret").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                        let expires = pair.get("expires_at").and_then(|c| c.as_i64()).unwrap_or(0) * 1000;
                        say(LinkMsg::Code { code: format!("{} {}", &code[..code.len().min(4)], &code[code.len().min(4)..]), approve: Links::new(&origin, "").devices() });
                        loop {
                            tokio::time::sleep(POLL).await;
                            if now() >= expires {
                                anyhow::bail!("The pairing code expired. Connect again for a new one.");
                            }
                            match home::post_unlinked(&http, &format!("{origin}/auth/reverse/poll"), &serde_json::json!({"code": code, "secret": secret}), None).await {
                                Ok(r) => {
                                    if let Some(device) = r.get("device").and_then(|d| d.as_str()) {
                                        secrets::store(secrets::DEVICE, &origin, device).await?;
                                        let link = HomeLink { kind: HomeKind::Address, origin, space: None, device_name: name, hf: None };
                                        let _ = tx.send(Cmd::Linked { link, device: device.to_string(), hf: None });
                                        return Ok(());
                                    }
                                }
                                Err((429, _)) => {}
                                Err((_, e)) => anyhow::bail!(e),
                            }
                        }
                    }
                    LinkCmd::Cancel => {}
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                tracing::info!("link: {e}");
                say(LinkMsg::Error(e.to_string()));
            }
        });
        self.link_task = Some(task.abort_handle());
    }
}

/// How a stream ended: a drop once it was live, else a failed attempt.
fn ended(live_at: Option<Instant>, why: String) -> Outcome {
    match live_at {
        Some(at) => Outcome::Ended { steady: at.elapsed() >= STEADY },
        None => Outcome::Failed(HomeError { failure: Failure::Network, message: why, status: 0 }),
    }
}

/// "Desktop · <hostname>", the device's name in Settings → Devices.
pub fn device_name() -> String {
    let host = gtk::glib::host_name().to_string();
    let host: String = host.chars().take(60).collect();
    format!("Desktop · {}", if host.is_empty() { "Linux".into() } else { host })
}

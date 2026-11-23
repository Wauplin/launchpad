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
use crate::sse;

pub type Shared = Arc<Mutex<Settings>>;

/// Home sends a keepalive comment every 15 s: a stream silent for this long is dead.
const SILENT: Duration = Duration::from_secs(45);
/// Bursts of `today` events make one request.
const TODAY_DEBOUNCE: Duration = Duration::from_millis(300);
/// Offline or starting this long: one notification.
const SAY_OFFLINE_MS: i64 = 2 * 60_000;
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
    /// Step 4: a newer companion ("Update to <version>").
    pub update: Option<String>,
}

pub enum Ui {
    View(Box<View>),
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
    last_view: Option<View>,
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
    /// The stream ended or went silent after it was open.
    Ended,
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
            last_view: None,
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
                Outcome::Changed => continue,
                Outcome::Ended => self.conn = Conn::Connecting { attempt: 0 },
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
                self.save(|s| s.workspace = Some(id));
                self.rules.reset();
                self.rules.on_today(&self.today_items(), now(), false);
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
        let view = self.view();
        if self.last_view.as_ref() != Some(&view) {
            self.last_view = Some(view.clone());
            self.send(Ui::View(Box::new(view)));
        }
    }

    fn notify(&self, note: Note) {
        if !self.settings().notifications {
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
        }
        Ok(())
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
            // No `/api/version` (a 404, or an older Home's page): Home is older than the companion.
            Err(HomeError { failure: Failure::Other, .. }) => {
                self.conn = Conn::Incompatible(conn::compatibility(None, 0, &[]).unwrap_or_default());
                return Outcome::Changed;
            }
            Err(e) => return Outcome::Failed(e),
        }
        let response = match home.events(self.position.as_ref()).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e),
        };
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
                        Some(Err(e)) => return if self.conn == Conn::Live { Outcome::Ended } else { Outcome::Failed(HomeError { failure: Failure::Network, message: e.to_string() }) },
                        None => return Outcome::Ended,
                    };
                    silent.as_mut().reset(tokio::time::Instant::now() + SILENT);
                    for frame in parser.push(&String::from_utf8_lossy(&chunk)) {
                        let Ok(ev) = serde_json::from_str::<BusEvent>(&frame.data) else { continue };
                        match ev.kind.as_str() {
                            "hello" => offered = Some((ev.boot.clone().unwrap_or_default(), ev.bus)),
                            "replayed" | "resync" => {
                                if let Some(o) = offered.take() { self.position = Some(o) }
                                // Every (re)opened stream reloads everything: cheap, and the companion may have slept.
                                for what in [Reload::Workspaces, Reload::Folders, Reload::Bots, Reload::Sessions, Reload::Today] {
                                    if let Err(e) = self.reload(what).await { return Outcome::Failed(e) }
                                }
                                self.model.loaded = true;
                                self.conn = Conn::Live;
                                self.last_live = Some(now());
                                self.today_notes(first_load && catch_up);
                                first_load = false;
                                self.say_state();
                            }
                            _ => {
                                // The stream's position: the bus number of the last event applied.
                                if let Some(p) = &mut self.position { p.1 = frame.id.as_deref().and_then(sse::position).map_or(ev.bus, |(_, bus)| bus) }
                                self.last_live = Some(now());
                                match self.model.apply(&ev) {
                                    Some(Reload::Today) => today_due = Some(Instant::now() + TODAY_DEBOUNCE),
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
                _ = &mut silent => return Outcome::Ended,
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

/// "Desktop · <hostname>", the device's name in Settings → Devices.
pub fn device_name() -> String {
    let host = gtk::glib::host_name().to_string();
    let host: String = host.chars().take(60).collect();
    format!("Desktop · {}", if host.is_empty() { "Linux".into() } else { host })
}

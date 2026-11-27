//! Notifications on the owner's phone (the Android app), on Home. Home never sends text to the phone through Google:
//! it sends a **wake-up** with Firebase Cloud Messaging (FCM, HTTP v1), a data message that holds only
//! `{kind: "wake", ws: <workspace id>, n: <a counter>}`. The app then reads `GET /api/phone/notifications` from Home
//! over its own connection and builds the notifications itself.
//!
//! - **The list** (`notifications`) is what the phone should show now: Today's waiting items that concern a phone
//!   (questions, failed and interrupted agents, failed routines, channel asks, bot answers, finished agents, report
//!   cards, other cards) and the automatic quota or cost stops and resumptions of the last hours (`session_event`,
//!   from `notify::session`). Today stays the only source of what waits.
//! - **A wake-up** goes out when the list gains an item (high priority when it is one that needs the owner, so it
//!   passes Doze) or loses one (normal priority: the phone removes its notification). An item that appears while a
//!   window the owner used in the last minute shows its session never notifies (`Sessions::is_watched`).
//! - **The sender is inert** until the owner saves a Firebase service-account key (Settings → Devices; vault entry
//!   `channel-push-fcm`) and a phone registered its token (`device_push`, written by `PUT /api/devices/self/push`): no
//!   key or no token means no request at all. Requests to Google follow the exit routes (`egress::Clients`).
//!
//! Runners forward their sessions' events to Home (`POST /api/phone/notify`), which owns the key and the tokens.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use launchpad_core::App;
use launchpad_core::model::Session;
use launchpad_core::today::{Filter, Item, Kind};
use launchpad_core::util::now_ms;

/// The vault entry of the Firebase service-account key (JSON). `channel-…` entries are not listed in the vault page.
const KEY_ID: &str = "channel-push-fcm";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// Quota stops and resumptions stay in the list this long.
const EVENT_TTL_MS: i64 = 12 * 3_600_000;

/// One notification for the phone. The app derives its buttons from `kind` with the action list: a question's
/// options (`session.answer`), Reply (`bot.message`, `channel.post`), Seen (`session.seen`) and Open; never Merge or
/// Publish.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct PhoneNotification {
    /// Stable: Today's item ID (`session:<id>`, `channel:<id>`, `proposal:<id>`…) or `quota:<session id>`.
    pub id: String,
    /// question, failed, routine, report_question, channel_ask, bot_answer, card, finished, report, quota_stop or
    /// quota_resumed.
    pub kind: String,
    /// The Android channel: needs_you, chats or done.
    pub channel: String,
    /// The workspace it belongs to; None: every workspace.
    pub workspace_id: Option<String>,
    pub title: String,
    pub text: String,
    /// Unix milliseconds: since when. A newer time of the same ID is a new notification.
    #[ts(type = "number")]
    pub at: i64,
    pub session_id: Option<String>,
    /// A bot's thread: the bot and the thread a reply goes to.
    pub bot_id: Option<String>,
    pub thread_id: Option<String>,
    /// A channel ask: the channel and the message a reply answers.
    pub channel_id: Option<String>,
    #[ts(type = "number | null")]
    pub message: Option<i64>,
    /// A card: its proposal.
    pub proposal_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct PhoneNotifications {
    /// Raised at every change of the list; the wake-up's `n`.
    #[ts(type = "number")]
    pub seq: u64,
    pub items: Vec<PhoneNotification>,
}

/// Settings → Devices → Phone notifications.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct PushStatus {
    /// A service-account key is saved.
    pub configured: bool,
    pub project_id: Option<String>,
    pub client_email: Option<String>,
    /// Phones that registered a push token.
    pub phones: u32,
    #[ts(type = "number | null")]
    pub last_sent_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct State {
    seq: u64,
    /// The list as last computed: `(id, at)` of each item, and whether a first list exists (Home's start notifies
    /// nothing).
    known: Option<Vec<(String, i64)>>,
    /// Items that appeared while the owner was looking at their session: never notified.
    suppressed: HashSet<String>,
    events: Vec<PhoneNotification>,
    access: Option<(String, i64)>,
    last_sent_at: Option<i64>,
    last_error: Option<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
static POKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------- the list

fn note(item: &Item, kind: &str, channel: &str) -> PhoneNotification {
    PhoneNotification {
        id: item.id.clone(),
        kind: kind.into(),
        channel: channel.into(),
        workspace_id: item.workspaces.first().cloned(),
        title: item.title.clone(),
        text: item.detail.clone(),
        at: item.at,
        session_id: item.session_id.clone(),
        bot_id: None,
        thread_id: None,
        channel_id: None,
        message: None,
        proposal_id: None,
    }
}

/// What a waiting item of Today is on the phone; None: it does not notify (Local PRs, the Outbox, suggestions,
/// runners, updates and oracles wait in Today only).
fn of_item(app: &App, item: &Item) -> Option<PhoneNotification> {
    Some(match item.kind {
        Kind::Question => note(item, "question", "needs_you"),
        Kind::Failed => note(item, "failed", "needs_you"),
        Kind::Routine => note(item, "routine", "needs_you"),
        Kind::ChannelAsk => PhoneNotification { channel_id: Some(item.record.clone()), message: item.message, ..note(item, "channel_ask", "chats") },
        Kind::Finished => {
            // A bot's finished turn is its answer in a chat; an agent's is finished work.
            match item.session_id.as_ref().and_then(|id| app.sessions.get(id)).and_then(|s| s.launch_spec.bot) {
                Some(bot) => PhoneNotification { bot_id: Some(bot.id.clone()), thread_id: Some(bot.thread_id().to_string()), ..note(item, "bot_answer", "chats") },
                None => note(item, "finished", "done"),
            }
        }
        Kind::Proposal => {
            let (kind, channel) = match (item.action.as_deref(), item.filter) {
                (Some("proposal.reply"), _) => ("report_question", "needs_you"),
                (Some("proposal.dismiss"), Filter::Review) => ("report", "done"),
                _ => ("card", "chats"),
            };
            PhoneNotification { proposal_id: Some(item.record.clone()), ..note(item, kind, channel) }
        }
        _ => return None,
    })
}

/// Home: the notifications the phone should show now (`GET /api/phone/notifications`).
pub async fn notifications(app: &App) -> Result<PhoneNotifications> {
    crate::connections::ensure_home(app)?;
    let today = launchpad_core::today::latest(app).await?;
    let mut items: Vec<PhoneNotification> = today.items.iter().filter_map(|i| of_item(app, i)).collect();
    let mut st = state();
    let now = now_ms();
    st.events.retain(|e| now - e.at < EVENT_TTL_MS);
    items.extend(st.events.iter().cloned());
    items.retain(|i| !st.suppressed.contains(&i.id));
    Ok(PhoneNotifications { seq: st.seq, items })
}

/// Pure: the IDs that are new (or waiting since a newer time) in `now` against `before`, and whether any left.
fn changes(before: &[(String, i64)], now: &[(String, i64)]) -> (Vec<String>, bool) {
    let old: HashMap<&str, i64> = before.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    let added = now.iter().filter(|(id, at)| old.get(id.as_str()).is_none_or(|was| at > was)).map(|(id, _)| id.clone()).collect();
    let ids: HashSet<&str> = now.iter().map(|(id, _)| id.as_str()).collect();
    (added, before.iter().any(|(id, _)| !ids.contains(id.as_str())))
}

/// Home: recomputes the list and wakes the phones when it changed.
async fn refresh(app: &App) {
    let Ok(today) = launchpad_core::today::latest(app).await else { return };
    let mut items: Vec<PhoneNotification> = today.items.iter().filter_map(|i| of_item(app, i)).collect();
    let wake = {
        let mut st = state();
        let now = now_ms();
        st.events.retain(|e| now - e.at < EVENT_TTL_MS);
        items.extend(st.events.iter().cloned());
        let all: Vec<(String, i64)> = items.iter().map(|i| (i.id.clone(), i.at)).collect();
        let Some(before) = st.known.replace(all.clone()) else { return };
        let (added, removed) = changes(&before, &all);
        // An item the owner is looking at never notifies, now or later; one that left the list is forgotten.
        let ids: HashSet<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
        st.suppressed.retain(|id| ids.contains(id.as_str()));
        let mut news = vec![];
        for id in added {
            let Some(item) = items.iter().find(|i| i.id == id) else { continue };
            if item.session_id.as_deref().is_some_and(|s| app.sessions.is_watched(s)) {
                st.suppressed.insert(id);
            } else {
                st.suppressed.remove(&id);
                news.push(item.clone());
            }
        }
        if news.is_empty() && !removed {
            return;
        }
        st.seq += 1;
        let high = news.iter().any(|n| n.channel == "needs_you");
        let workspace = news.first().and_then(|n| n.workspace_id.clone()).unwrap_or_default();
        (st.seq, workspace, high)
    };
    app.bus.publish("phone", "notifications", json!({"seq": wake.0}));
    if let Err(e) = send_wake(app, wake.0, &wake.1, wake.2).await {
        tracing::warn!("phone push: {e:#}");
        state().last_error = Some(format!("{e:#}"));
    }
}

/// Home: follows Today (`today`/`changed` on the bus) and the session events, and wakes the phones.
pub fn spawn(app: &App) {
    use tokio::sync::broadcast::error::RecvError;
    if !launchpad_core::vault::here(app) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut rx = app.bus.subscribe(None).rx;
        // The first list is the baseline: what already waits when Home starts wakes no phone.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        refresh(&app).await;
        loop {
            tokio::select! {
                _ = POKE.notified() => {}
                event = rx.recv() => match event {
                    Ok(e) if e.text.contains("\"topic\":\"today\"") => {}
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
            refresh(&app).await;
        }
    });
}

fn event_note(event: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match event {
        "quota_paused" => ("quota_stop", "needs_you", "Quota reached: the agent is paused"),
        "cost_limit" => ("quota_stop", "needs_you", "Cost limit reached: the agent was stopped"),
        "quota_resumed" => ("quota_resumed", "done", "Quota available: the agent resumed"),
        _ => return None,
    })
}

/// From `notify::session`: an eligible session notification. Finished, failed and interrupted sessions are waiting
/// items of Today, which wakes the phones; the automatic stops and resumptions are kept here for a few hours. A
/// runner forwards the event to Home.
pub fn session_event(app: &App, s: &Session, event: &'static str) {
    let (app, id) = (app.clone(), s.id.clone());
    tokio::spawn(async move {
        let result = if launchpad_core::vault::here(&app) {
            record_event(&app, &id, event).await
        } else {
            launchpad_core::home_link::home_request("POST", "/api/phone/notify", Some(json!({"session_id": id, "event": event}))).await.map(|_| ())
        };
        if let Err(e) = result {
            tracing::debug!("phone notification: {e:#}");
        }
    });
}

/// Home: records a session event (`POST /api/phone/notify` for a runner's) and looks at the list again.
pub async fn record_event(app: &App, session_id: &str, event: &str) -> Result<()> {
    crate::connections::ensure_home(app)?;
    if let Some((kind, channel, text)) = event_note(event) {
        let s = app.sessions.get(session_id).context("session not found")?;
        let workspace_id = launchpad_core::kb::workspace_of_session(app, session_id).await;
        let note = PhoneNotification {
            id: format!("quota:{session_id}"),
            kind: kind.into(),
            channel: channel.into(),
            workspace_id,
            title: if s.title.is_empty() { "Agent".into() } else { s.title.clone() },
            text: text.into(),
            at: now_ms(),
            session_id: Some(session_id.into()),
            bot_id: None,
            thread_id: None,
            channel_id: None,
            message: None,
            proposal_id: None,
        };
        let mut st = state();
        st.events.retain(|e| e.id != note.id);
        st.events.push(note);
    }
    POKE.notify_one();
    Ok(())
}

/// The owner marked a kept event (a quota stop or resumption) seen on the phone: it leaves the list. Today's items
/// leave it through their own actions.
pub fn dismiss(id: &str) {
    state().events.retain(|e| e.id != id);
    POKE.notify_one();
}

// ---------------------------------------------------------------- the key and the tokens

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
}

async fn key(app: &App) -> Option<ServiceAccount> {
    let (_, text) = launchpad_core::vault::secret(app, KEY_ID).await.ok().flatten()?;
    serde_json::from_str(&text).ok()
}

fn signing_key(account: &ServiceAccount) -> Result<aws_lc_rs::signature::RsaKeyPair> {
    let body: String = account.private_key.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body.trim()).context("the private key is not valid base64")?;
    aws_lc_rs::signature::RsaKeyPair::from_pkcs8(&der).map_err(|_| anyhow!("the private key is not a PKCS#8 RSA key"))
}

/// The registered push tokens of the phones that are still signed in.
async fn tokens(app: &App) -> Result<Vec<(String, String)>> {
    let now = now_ms() / 1000;
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT p.device_id, p.token FROM device_push p JOIN browser_device d ON d.id = p.device_id WHERE d.expires_at > ?1")?;
            let rows = st.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

pub async fn status(app: &App) -> Result<PushStatus> {
    crate::connections::ensure_home(app)?;
    let key = key(app).await;
    let phones = tokens(app).await?.len() as u32;
    let st = state();
    Ok(PushStatus {
        configured: key.is_some(),
        project_id: key.as_ref().map(|k| k.project_id.clone()),
        client_email: key.as_ref().map(|k| k.client_email.clone()),
        phones,
        last_sent_at: st.last_sent_at,
        last_error: st.last_error.clone(),
    })
}

/// Saves the Firebase service-account key (the JSON file of Firebase → Project settings → Service accounts).
pub async fn set_key(app: &App, text: &str) -> Result<PushStatus> {
    crate::connections::ensure_home(app)?;
    let account: ServiceAccount = serde_json::from_str(text.trim()).context("paste the whole service-account JSON file (project_id, client_email, private_key)")?;
    ensure!(!account.project_id.is_empty() && !account.client_email.is_empty(), "the key has no project_id or client_email");
    signing_key(&account)?;
    launchpad_core::vault::put(app, KEY_ID, launchpad_core::vault::Kind::Token, "Phone notifications (Firebase)", text.trim()).await?;
    {
        let mut st = state();
        st.access = None;
        st.last_error = None;
    }
    status(app).await
}

pub async fn remove_key(app: &App) -> Result<PushStatus> {
    launchpad_core::vault::delete(app, KEY_ID).await?;
    state().access = None;
    status(app).await
}

/// Settings → Devices → Send a test wake-up: the phones answer with a "Test" notification of their own.
pub async fn test(app: &App) -> Result<u32> {
    crate::connections::ensure_home(app)?;
    ensure!(key(app).await.is_some(), "save the Firebase service-account key first");
    ensure!(!tokens(app).await?.is_empty(), "no phone registered for notifications yet: sign in on the phone and allow notifications");
    let seq = state().seq;
    let sent = send(app, json!({"kind": "test", "ws": "", "n": seq.to_string()}), true).await;
    if let Err(e) = &sent {
        state().last_error = Some(format!("{e:#}"));
    }
    sent
}

// ---------------------------------------------------------------- the sender

static CLIENTS: launchpad_core::egress::Clients = launchpad_core::egress::Clients::new(|b| b.timeout(std::time::Duration::from_secs(20)));
static DIRECT: LazyLock<reqwest::Client> = LazyLock::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default());

/// The client for a Google address: direct, or through an exit route when the owner routes it.
fn client(url: &str) -> Result<reqwest::Client> {
    let url = reqwest::Url::parse(url)?;
    CLIENTS.pick(&DIRECT, &url).map(|(c, _)| c).map_err(|e| anyhow!(e))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// An OAuth access token of the service account (a signed JWT exchanged at Google), kept until shortly before it
/// expires.
async fn access_token(account: &ServiceAccount) -> Result<String> {
    let now = now_ms() / 1000;
    if let Some((token, expires)) = state().access.clone()
        && expires - 300 > now
    {
        return Ok(token);
    }
    let claims = json!({"iss": account.client_email, "scope": SCOPE, "aud": TOKEN_URL, "iat": now, "exp": now + 3600});
    let unsigned = format!("{}.{}", b64(br#"{"alg":"RS256","typ":"JWT"}"#), b64(claims.to_string().as_bytes()));
    let pair = signing_key(account)?;
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(&aws_lc_rs::signature::RSA_PKCS1_SHA256, &aws_lc_rs::rand::SystemRandom::new(), unsigned.as_bytes(), &mut signature).map_err(|_| anyhow!("could not sign the request to Google"))?;
    let assertion = format!("{unsigned}.{}", b64(&signature));
    let res = client(TOKEN_URL)?
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        // The assertion is base64url text and dots: nothing to escape.
        .body(format!("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}"))
        .send()
        .await?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    let Some(token) = body["access_token"].as_str() else {
        bail!("Google refused the service-account key ({status}): {}", body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("no access token"));
    };
    state().access = Some((token.to_string(), now + body["expires_in"].as_i64().unwrap_or(3600)));
    Ok(token.to_string())
}

/// A wake-up for a change of the list. Inert without a key or a registered phone.
async fn send_wake(app: &App, seq: u64, workspace: &str, high: bool) -> Result<()> {
    send(app, json!({"kind": "wake", "ws": workspace, "n": seq.to_string()}), high).await.map(|_| ())
}

/// Sends one data message to every registered phone; answers how many FCM accepted. A token FCM no longer knows
/// is forgotten.
async fn send(app: &App, data: Value, high: bool) -> Result<u32> {
    let Some(account) = key(app).await else { return Ok(0) };
    let tokens = tokens(app).await?;
    if tokens.is_empty() {
        return Ok(0);
    }
    let access = access_token(&account).await?;
    let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", account.project_id);
    let http = client(&url)?;
    let (mut sent, mut failure) = (0, None);
    for (device, token) in tokens {
        // The whole payload: no title, no text, nothing of the work.
        let message = json!({"message": {"token": token, "data": data, "android": {"priority": if high { "HIGH" } else { "NORMAL" }, "ttl": "3600s", "collapse_key": "wake"}}});
        tracing::info!("phone push to device {device}: {} ({})", data, if high { "high" } else { "normal" });
        let res = http.post(&url).bearer_auth(&access).json(&message).send().await?;
        let status = res.status();
        if status.is_success() {
            sent += 1;
            continue;
        }
        let body: Value = res.json().await.unwrap_or(Value::Null);
        let why = body["error"]["message"].as_str().unwrap_or("no reason").to_string();
        if status == reqwest::StatusCode::NOT_FOUND || body["error"]["details"].to_string().contains("UNREGISTERED") {
            // The app was uninstalled or its token replaced.
            let _ = app.state_db.call(move |c| Ok(c.execute("DELETE FROM device_push WHERE device_id = ?1", [device])?)).await;
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            state().access = None;
        }
        failure = Some(format!("FCM answered {status}: {why}"));
    }
    let mut st = state();
    if sent > 0 {
        st.last_sent_at = Some(now_ms());
    }
    st.last_error = failure.clone();
    match failure {
        Some(why) if sent == 0 => bail!(why),
        _ => Ok(sent),
    }
}

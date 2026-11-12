//! Dictation (voice input in the message boxes), served by Home to the owner's windows only.
//!
//! `GET /api/voice`: the settings and the sources, in fallback order, with their state (Settings → Voice and the
//! mic's tooltip). `GET /api/voice/ws`: one WebSocket per window. In: `{"type":"warm"}` (the pointer is on a mic or
//! a message box has the focus: connect the first source now, kept 30 s), `{"type":"start","keyterms":[…]}`,
//! binary frames (16 kHz mono 16-bit PCM), `{"type":"stop"}` (the last words, then `closed`) and
//! `{"type":"cancel"}`. Out: `source` (the login that listens, with a `note` after a failover), `delta` (the
//! phrase so far, replacing the previous one), `done` (a settled phrase to insert), `error` (`reason`) and `closed`.
//!
//! Home picks the first ready source (`[voice] profiles`: subscription profiles of a harness with voice, skipping
//! logins with a used-up window), opens it with Home's copy of the vault login (`Harness::open_voice`), and keeps
//! the audio since the last settled phrase, so a source that refuses, fails or does not answer within 3 s is
//! replaced by the next one with that audio played again: no words are lost. One dictation at a time per
//! window, 5 minutes at most. Agents, runners and the CLI have no part in it; logs carry sizes and timings, never
//! text or audio.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use launchpad_core::App;
use launchpad_core::config::{Billing, Profile};
use launchpad_core::harness::{VoiceCall, VoiceEvent, VoiceInput, VoiceOpts, VoiceRefused};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use ts_rs::TS;

use crate::Server;
use crate::auth::Access;

const MAX_DICTATION: Duration = Duration::from_secs(5 * 60);
const WARM_FOR: Duration = Duration::from_secs(30);
/// A source that has not connected by then is replaced by the next one.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);
/// Audio replayed into a new source goes in frames of 1 s, like Claude Code's own buffer.
const REPLAY_FRAME: usize = 32_000;

pub fn router() -> Router<Server> {
    Router::new().route("/api/voice", get(status)).route("/api/voice/ws", get(socket))
}

pub fn gen_types(cfg: &ts_rs::Config) -> anyhow::Result<()> {
    VoiceStatus::export_all(cfg)?;
    Ok(())
}

/// One profile as a voice source.
#[derive(Clone, Debug, Serialize, TS)]
pub struct VoiceSource {
    pub profile: String,
    pub name: String,
    pub harness: String,
    /// In the fallback order (`[voice] profiles`, or every eligible profile when that list is empty).
    pub used: bool,
    /// `ready`, `used_up` (a window is used up until `resets_at`), `refused` (the login needs a new sign-in),
    /// `no_login` (no login of its harness in Home's vault), `pay_as_you_go` or `unsupported` (its harness has no voice).
    pub state: String,
    /// The vault login it uses, with its quota in words.
    pub detail: String,
    /// Unix seconds.
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct VoiceStatus {
    pub enabled: bool,
    pub language: String,
    pub silence_stop_s: u32,
    /// Used sources first, in fallback order, then the other profiles.
    pub sources: Vec<VoiceSource>,
    /// Why the mic is off (voice off, no source, every login used up), or None when a source is ready.
    pub unavailable: Option<String>,
    /// Every used login is used up: the first reset (Unix seconds).
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
}

/// A source Home may open: the profile and Home's copy of its login.
struct Candidate {
    name: String,
    harness: launchpad_core::model::HarnessKind,
    env: Vec<(String, String)>,
}

fn pct(w: &Option<launchpad_core::status::QuotaWindow>) -> Option<String> {
    w.as_ref().map(|w| format!("{:.0}%", w.used_percent))
}

async fn sources(app: &App, with_env: bool) -> (VoiceStatus, Vec<Candidate>) {
    let config = app.config.get();
    let voice = config.voice.clone();
    let eligible = |p: &Profile| launchpad_core::harness::registrations().iter().any(|r| r.descriptor.id == p.harness && r.descriptor.capabilities.voice) && p.billing == Billing::Subscription;
    let mut order: Vec<&Profile> = if voice.profiles.is_empty() {
        config.profiles.iter().filter(|p| eligible(p)).collect()
    } else {
        voice.profiles.iter().filter_map(|id| config.profile(id)).filter(|p| eligible(p)).collect()
    };
    let used = order.len();
    let listed: std::collections::HashSet<String> = order.iter().map(|p| p.id.clone()).collect();
    order.extend(config.profiles.iter().filter(|p| !listed.contains(&p.id)));
    let now = launchpad_core::util::now_ms();
    let mut out = vec![];
    let mut ready = vec![];
    let mut seen_logins = std::collections::HashSet::new();
    let mut used_up_until: Option<i64> = None;
    for (i, p) in order.into_iter().enumerate() {
        let descriptor = launchpad_core::harness::registration(p.harness).descriptor;
        let mut source = VoiceSource { profile: p.id.clone(), name: p.name.clone(), harness: descriptor.name.to_string(), used: i < used, state: "ready".into(), detail: String::new(), resets_at: None };
        if !descriptor.capabilities.voice {
            source.state = "unsupported".into();
            source.detail = "Not supported for voice yet".into();
        } else if p.billing != Billing::Subscription {
            source.state = "pay_as_you_go".into();
            source.detail = "Pay-as-you-go profile: voice uses subscription logins only".into();
        } else {
            let logins = launchpad_core::vault::logins(app, p.harness).await.unwrap_or_default();
            let entry = match &p.login {
                Some(id) => logins.iter().find(|e| &e.id == id),
                None => descriptor.vault_login.and_then(|l| launchpad_core::vault::default_login(&logins, l.kind)),
            };
            match entry {
                None => {
                    source.state = "no_login".into();
                    source.detail = format!("No {} login in Home's vault", descriptor.name);
                }
                Some(e) => {
                    let q = launchpad_core::quota::reading(app, &launchpad_core::quota::vault_key(p.harness, Some(&e.id)));
                    let blocked = launchpad_core::quota::exhausted(&q).into_iter().filter(|b| b.resets_at.is_none_or(|r| r > now / 1000)).max_by_key(|b| b.resets_at);
                    let quota = match (pct(&q.five_hour), pct(&q.weekly)) {
                        (Some(f), Some(w)) => format!("5 h window {f}, week {w}"),
                        (Some(f), None) => format!("5 h window {f}"),
                        _ => "quota not read yet".into(),
                    };
                    source.detail = format!("Login {} · {quota}", e.name);
                    if q.refused {
                        source.state = "refused".into();
                        source.detail = format!("Login {} needs a new sign-in", e.name);
                    } else if let Some(b) = blocked {
                        source.state = "used_up".into();
                        source.resets_at = b.resets_at;
                        source.detail = format!("Login {} · {} window used up", e.name, b.window.to_lowercase());
                        if let Some(r) = b.resets_at { used_up_until = Some(used_up_until.map_or(r, |u| u.min(r))); }
                    } else if source.used && seen_logins.insert(e.id.clone()) && with_env {
                        match launchpad_core::vault::login_home(app, &e.id).await {
                            Ok((_, env)) => ready.push(Candidate { name: format!("{} ({})", descriptor.name, p.name), harness: p.harness, env }),
                            Err(err) => tracing::warn!("voice: login copy of {}: {err:#}", e.id),
                        }
                    }
                }
            }
        }
        out.push(source);
    }
    let any_ready = out.iter().any(|s| s.used && s.state == "ready");
    let unavailable = if !voice.enabled {
        Some("Voice is off (Settings → Voice)".to_string())
    } else if app.paths.role != launchpad_core::role::Role::Home {
        Some("Voice runs on Home".to_string())
    } else if used == 0 {
        Some("No voice source: add a Claude subscription profile (Settings → Profiles)".to_string())
    } else if !any_ready {
        Some(if used_up_until.is_some() { "Every voice login is used up".to_string() } else { "No voice source is ready: check the logins in Settings → Voice".to_string() })
    } else {
        None
    };
    (VoiceStatus { enabled: voice.enabled, language: voice.language, silence_stop_s: voice.silence_stop_s, sources: out, unavailable, resets_at: if any_ready { None } else { used_up_until } }, ready)
}

/// Only the owner's windows and CLI: never an agent's token.
fn owner(access: &Option<Extension<Access>>, agent: &Option<Extension<crate::home_client::AgentCall>>) -> Option<Response> {
    (agent.is_some() || access.is_none()).then(|| (StatusCode::FORBIDDEN, Json(json!({"error": "voice input is for the owner's windows only"}))).into_response())
}

async fn status(State(s): State<Server>, access: Option<Extension<Access>>, agent: Option<Extension<crate::home_client::AgentCall>>) -> Response {
    if let Some(r) = owner(&access, &agent) { return r }
    Json(sources(&s.app, false).await.0).into_response()
}

async fn socket(State(s): State<Server>, access: Option<Extension<Access>>, agent: Option<Extension<crate::home_client::AgentCall>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if let Some(r) = owner(&access, &agent) { return r }
    let Some(Extension(access)) = access else { return StatusCode::FORBIDDEN.into_response() };
    // A browser window of Home: signed in as a device, from an allowed Origin (checked exactly by the front door).
    if s.app.paths.role != launchpad_core::role::Role::Home || access.device.is_none() || !headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "voice input runs in Home's browser windows only"}))).into_response();
    }
    ws.max_message_size(1 << 20).on_upgrade(move |socket| run(s.app, socket, access))
}

struct Live {
    started: Instant,
    /// Audio since the last settled phrase, played again into the next source after a failover.
    buffer: Vec<u8>,
    call: Option<(VoiceCall, String)>,
    /// The sources not tried yet.
    rest: Vec<Candidate>,
    attempt: u64,
    finishing: bool,
    keyterms: Vec<String>,
    /// What went wrong with the previous source, for the note of the next one.
    note: Option<String>,
    bytes: usize,
}

type Opened = (u64, String, anyhow::Result<VoiceCall>);

fn open(app: &App, attempt: u64, c: Candidate, opts: VoiceOpts, tx: mpsc::Sender<Opened>) {
    let app = app.clone();
    tokio::spawn(async move {
        let harness = app.harnesses.get(c.harness).clone();
        let result = match tokio::time::timeout(ANSWER_WITHIN, harness.open_voice(&app, &c.env, opts)).await {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!("did not answer within {} s", ANSWER_WITHIN.as_secs())),
        };
        let _ = tx.send((attempt, c.name, result)).await;
    });
}

fn keyterms_of(v: &Value) -> Vec<String> {
    v["keyterms"].as_array().map(|a| a.iter().filter_map(|t| t.as_str()).take(200).map(str::to_string).collect()).unwrap_or_default()
}

async fn recv_call(live: &mut Option<Live>) -> Option<VoiceEvent> {
    match live.as_mut().and_then(|l| l.call.as_mut()) {
        Some((call, _)) => call.events.recv().await,
        None => std::future::pending().await,
    }
}

async fn send(socket: &mut WebSocket, v: Value) -> bool {
    socket.send(Message::Text(v.to_string().into())).await.is_ok()
}

/// Why a source failed, in a few words for the window.
fn failure(name: &str, e: &anyhow::Error) -> String {
    if e.downcast_ref::<VoiceRefused>().is_some() || launchpad_core::vault::login_refused(&format!("{e:#}")) {
        format!("{name}: this login can't use voice")
    } else {
        format!("{name}: {e:#}")
    }
}

async fn run(app: App, mut socket: WebSocket, access: Access) {
    let (opened_tx, mut opened) = mpsc::channel::<Opened>(8);
    let mut live: Option<Live> = None;
    let mut warm: Option<(VoiceCall, String, Instant, Vec<String>)> = None;
    let mut warm_terms: Vec<String> = vec![];
    let mut warming: Option<u64> = None;
    let mut attempts = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let opts = |app: &App, keyterms: &[String]| VoiceOpts { language: app.config.get().voice.language, keyterms: keyterms.to_vec() };
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if !access.valid() { break }
                if warm.as_ref().is_some_and(|(_, _, at, _)| at.elapsed() > WARM_FOR) { warm = None }
                if let Some(l) = live.as_mut() && !l.finishing && l.started.elapsed() > MAX_DICTATION {
                    l.finishing = true;
                    if let Some((call, _)) = &l.call { let _ = call.input.send(VoiceInput::Finish).await; }
                }
            }
            msg = socket.recv() => {
                let msg = match msg { Some(Ok(m)) => m, _ => break };
                match msg {
                    Message::Binary(frame) => {
                        if let Some(l) = live.as_mut() && !l.finishing {
                            l.bytes += frame.len();
                            l.buffer.extend_from_slice(&frame);
                            if let Some((call, _)) = &l.call { let _ = call.input.send(VoiceInput::Audio(frame.to_vec())).await; }
                        }
                    }
                    Message::Text(t) => {
                        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                        match v["type"].as_str().unwrap_or("") {
                            "warm" => {
                                if live.is_none() && warm.is_none() && warming.is_none() {
                                    warm_terms = keyterms_of(&v);
                                    let (_, mut ready) = sources(&app, true).await;
                                    if !ready.is_empty() && app.config.get().voice.enabled {
                                        attempts += 1;
                                        warming = Some(attempts);
                                        open(&app, attempts, ready.remove(0), opts(&app, &warm_terms), opened_tx.clone());
                                    }
                                }
                            }
                            "start" => {
                                let keyterms = keyterms_of(&v);
                                let (status, mut rest) = sources(&app, true).await;
                                if let Some(reason) = status.unavailable.filter(|_| rest.is_empty()) {
                                    if !send(&mut socket, json!({"type": "error", "reason": reason})).await { break }
                                    continue;
                                }
                                // One dictation at a time: a new start replaces the last one (below).
                                attempts += 1;
                                let mut l = Live { started: Instant::now(), buffer: vec![], call: None, rest: vec![], attempt: attempts, finishing: false, keyterms, note: None, bytes: 0 };
                                // A warm call was opened with the vocabulary hints of its warm-up: use it only for the same ones.
                                if let Some((call, name, _, _)) = warm.take().filter(|w| w.3 == l.keyterms) {
                                    rest.retain(|c| c.name != name);
                                    if !send(&mut socket, json!({"type": "source", "name": name})).await { break }
                                    l.call = Some((call, name));
                                } else if !rest.is_empty() {
                                    warming = None;
                                    open(&app, l.attempt, rest.remove(0), opts(&app, &l.keyterms), opened_tx.clone());
                                }
                                l.rest = rest;
                                live = Some(l);
                            }
                            "stop" => {
                                if let Some(l) = live.as_mut() && !l.finishing {
                                    l.finishing = true;
                                    if let Some((call, _)) = &l.call { let _ = call.input.send(VoiceInput::Finish).await; }
                                }
                            }
                            "cancel" => {
                                if let Some(l) = live.take() { tracing::info!("voice: cancelled after {} ms, {} audio bytes", l.started.elapsed().as_millis(), l.bytes) }
                            }
                            _ => {}
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            Some((attempt, name, result)) = opened.recv() => {
                if warming == Some(attempt) {
                    warming = None;
                    match result {
                        Ok(call) if live.is_none() => warm = Some((call, name, Instant::now(), std::mem::take(&mut warm_terms))),
                        Ok(_) => {}
                        Err(e) => tracing::info!("voice: warm-up failed: {}", failure(&name, &e)),
                    }
                    continue;
                }
                let Some(l) = live.as_mut().filter(|l| l.attempt == attempt) else { continue };
                match result {
                    Ok(call) => {
                        let mut v = json!({"type": "source", "name": name});
                        if let Some(note) = l.note.take() { v["note"] = json!(format!("{note}. Continued with {name}; nothing lost.")) }
                        // The audio recorded while connecting (or since the last settled phrase, after a failover).
                        for chunk in l.buffer.chunks(REPLAY_FRAME) { let _ = call.input.send(VoiceInput::Audio(chunk.to_vec())).await; }
                        if l.finishing { let _ = call.input.send(VoiceInput::Finish).await; }
                        l.call = Some((call, name));
                        if !send(&mut socket, v).await { break }
                    }
                    Err(e) => {
                        let why = failure(&name, &e);
                        tracing::info!("voice: source failed: {why}");
                        if l.rest.is_empty() {
                            live = None;
                            if !send(&mut socket, json!({"type": "error", "reason": format!("No voice source answered. {why}")})).await { break }
                        } else {
                            l.note = Some(why);
                            attempts += 1;
                            l.attempt = attempts;
                            let next = l.rest.remove(0);
                            open(&app, l.attempt, next, opts(&app, &l.keyterms), opened_tx.clone());
                        }
                    }
                }
            }
            ev = recv_call(&mut live) => {
                let Some(l) = live.as_mut() else { continue };
                match ev {
                    Some(VoiceEvent::Interim(text)) => { if !send(&mut socket, json!({"type": "delta", "text": text})).await { break } }
                    Some(VoiceEvent::Final(text)) => {
                        l.buffer.clear();
                        if !send(&mut socket, json!({"type": "done", "text": text})).await { break }
                    }
                    Some(VoiceEvent::Error { message, refused }) => {
                        let name = l.call.take().map(|(_, n)| n).unwrap_or_default();
                        let why = if refused { format!("{name}: this login can't use voice") } else { format!("{name}: {message}") };
                        tracing::info!("voice: source stopped: {} ({} chars)", if refused { "refused" } else { "error" }, message.len());
                        if l.rest.is_empty() {
                            live = None;
                            if !send(&mut socket, json!({"type": "error", "reason": why})).await { break }
                        } else {
                            l.note = Some(why);
                            attempts += 1;
                            l.attempt = attempts;
                            let next = l.rest.remove(0);
                            open(&app, l.attempt, next, opts(&app, &l.keyterms), opened_tx.clone());
                        }
                    }
                    Some(VoiceEvent::Closed) | None => {
                        let finished = l.finishing;
                        tracing::info!("voice: dictation ended after {} ms, {} audio bytes", l.started.elapsed().as_millis(), l.bytes);
                        live = None;
                        let v = if finished { json!({"type": "closed"}) } else { json!({"type": "error", "reason": "The voice service ended the dictation"}) };
                        if !send(&mut socket, v).await { break }
                    }
                }
            }
        }
    }
}

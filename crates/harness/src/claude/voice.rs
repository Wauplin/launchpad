//! Dictation through Anthropic's speech-to-text stream, the one Claude Code's `/voice` uses (checked in the
//! Claude Code 2.1.311 bundle): a WebSocket to `/api/ws/speech_to_text/voice_stream` with the login's OAuth token,
//! binary 16 kHz mono 16-bit PCM frames, `{"type":"KeepAlive"}` every 8 s and `{"type":"CloseStream"}` to finish.
//! It answers `TranscriptInterim`/`TranscriptText` (the phrase so far), `TranscriptEndpoint` (the phrase is
//! settled), `TranscriptError` and `error`. Runs on Home with Home's copy of a vault login; the token is only read.
//! Logs carry sizes and timings, never text.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use launchpad_core::harness::{VoiceCall, VoiceEvent, VoiceInput, VoiceOpts, VoiceRefused};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

const URL: &str = "wss://api.anthropic.com/api/ws/speech_to_text/voice_stream";
const KEEP_ALIVE: &str = r#"{"type":"KeepAlive"}"#;
const CLOSE_STREAM: &str = r#"{"type":"CloseStream"}"#;
/// Claude Code's own limits after CloseStream: the last words within 5 s, and 1.5 s when nothing more comes.
const FINISH_SAFETY: Duration = Duration::from_secs(5);
const FINISH_NO_DATA: Duration = Duration::from_millis(1500);
const CONNECT: Duration = Duration::from_secs(5);
const KEYTERMS_MAX: usize = 1024;

/// The `x-config-keyterms` header: printable ASCII terms without commas, deduplicated, at most 1,024 characters.
fn keyterms(terms: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = vec![];
    let mut len = 0;
    for t in terms {
        let t: String = t.replace(',', " ").chars().filter(|c| (' '..='~').contains(c)).collect();
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        let add = t.len() + usize::from(!out.is_empty());
        if len + add > KEYTERMS_MAX {
            break;
        }
        len += add;
        out.push(t);
    }
    out.join(",")
}

pub async fn open(env: &[(String, String)], opts: VoiceOpts) -> Result<VoiceCall> {
    let token = env.iter().find(|(k, _)| k == "CLAUDE_CODE_OAUTH_TOKEN").map(|(_, v)| v.clone()).context("this login has no token in Home's vault")?;
    let language = if opts.language.trim().is_empty() || opts.language == "auto" { "en".to_string() } else { opts.language.clone() };
    let mut url = reqwest::Url::parse(URL)?;
    url.query_pairs_mut()
        .append_pair("encoding", "linear16")
        .append_pair("sample_rate", "16000")
        .append_pair("channels", "1")
        .append_pair("endpointing_ms", "300")
        .append_pair("utterance_end_ms", "1000")
        .append_pair("language", &language)
        .append_pair("use_conversation_engine", "true");
    let mut request = url.as_str().into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("authorization", HeaderValue::from_str(&format!("Bearer {token}"))?);
    headers.insert("user-agent", HeaderValue::from_static("claude-cli/2.1.311 (external, cli)"));
    headers.insert("x-app", HeaderValue::from_static("cli"));
    let terms = keyterms(&opts.keyterms);
    if !terms.is_empty() {
        headers.insert("x-config-keyterms", HeaderValue::from_str(&terms)?);
    }
    let started = std::time::Instant::now();
    let ws = match tokio::time::timeout(CONNECT, tokio_tungstenite::connect_async(request)).await {
        Err(_) => anyhow::bail!("the voice service did not answer within {} s", CONNECT.as_secs()),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(res))) => {
            let status = res.status();
            let body = res.body().as_ref().map(|b| String::from_utf8_lossy(b).chars().take(200).collect::<String>()).unwrap_or_default();
            tracing::info!("voice: upgrade refused with HTTP {status} ({} bytes)", body.len());
            if matches!(status.as_u16(), 401 | 403) {
                return Err(VoiceRefused(format!("Anthropic refused this login for voice (HTTP {status})")).into());
            }
            anyhow::bail!("the voice service answered HTTP {status}");
        }
        Ok(Err(e)) => return Err(anyhow::Error::from(e).context("voice service connection")),
        Ok(Ok((ws, _))) => ws,
    };
    tracing::info!("voice: connected in {} ms", started.elapsed().as_millis());
    let (input_tx, input) = mpsc::channel(256);
    let (events, events_rx) = mpsc::channel(64);
    tokio::spawn(run(ws, input, events));
    Ok(VoiceCall { input: input_tx, events: events_rx })
}

async fn run(
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    mut input: mpsc::Receiver<VoiceInput>,
    events: mpsc::Sender<VoiceEvent>,
) {
    let (mut tx, mut rx) = ws.split();
    let started = std::time::Instant::now();
    let mut bytes = 0usize;
    let mut pending = String::new();
    let mut keep_alive = tokio::time::interval(Duration::from_secs(8));
    // Finishing: the deadline of the last words (safety), and the shorter one while nothing arrives.
    let mut finish: Option<(tokio::time::Instant, tokio::time::Instant)> = None;
    let far = || tokio::time::Instant::now() + Duration::from_secs(86_400);
    loop {
        let deadline = finish.map(|(safety, no_data)| safety.min(no_data)).unwrap_or_else(far);
        tokio::select! {
            _ = keep_alive.tick() => {
                if tx.send(Message::Text(KEEP_ALIVE.into())).await.is_err() { break }
            }
            _ = tokio::time::sleep_until(deadline), if finish.is_some() => break,
            msg = input.recv() => match msg {
                Some(VoiceInput::Audio(frame)) => {
                    if finish.is_some() { continue }
                    bytes += frame.len();
                    if tx.send(Message::Binary(frame.into())).await.is_err() { break }
                }
                Some(VoiceInput::Finish) => {
                    if finish.is_none() {
                        let now = tokio::time::Instant::now();
                        finish = Some((now + FINISH_SAFETY, now + FINISH_NO_DATA));
                        if tx.send(Message::Text(CLOSE_STREAM.into())).await.is_err() { break }
                    }
                }
                // The window cancelled or went away: nothing more to send or deliver.
                None => { pending.clear(); break }
            },
            msg = rx.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(f) = frame.filter(|f| !matches!(u16::from(f.code), 1000 | 1005)) && finish.is_none() {
                            let _ = events.send(VoiceEvent::Error { message: format!("the voice service closed the stream (code {})", u16::from(f.code)), refused: false }).await;
                        }
                        break;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        if finish.is_none() { let _ = events.send(VoiceEvent::Error { message: format!("voice stream: {e}"), refused: false }).await; }
                        break;
                    }
                    None => break,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                match v["type"].as_str().unwrap_or("") {
                    "TranscriptInterim" | "TranscriptText" => {
                        // Something arrived after CloseStream: wait for the rest up to the safety deadline.
                        if let Some((safety, _)) = finish { finish = Some((safety, far())); }
                        if let Some(data) = v["data"].as_str().filter(|d| !d.is_empty()) {
                            pending = data.to_string();
                            let _ = events.send(VoiceEvent::Interim(pending.clone())).await;
                        }
                    }
                    "TranscriptEndpoint" => {
                        let phrase = std::mem::take(&mut pending);
                        if !phrase.is_empty() { let _ = events.send(VoiceEvent::Final(phrase)).await; }
                        if finish.is_some() { break }
                    }
                    "TranscriptError" | "error" => {
                        let message = v["description"].as_str().or(v["error_code"].as_str()).or(v["message"].as_str()).unwrap_or("transcription error").chars().take(200).collect::<String>();
                        let phrase = std::mem::take(&mut pending);
                        if !phrase.is_empty() { let _ = events.send(VoiceEvent::Final(phrase)).await; }
                        let refused = launchpad_core::vault::login_refused(&message);
                        if finish.is_none() { let _ = events.send(VoiceEvent::Error { message, refused }).await; }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    // An unreported phrase still counts, unless the window cancelled.
    if !pending.is_empty() {
        let _ = events.send(VoiceEvent::Final(std::mem::take(&mut pending))).await;
    }
    let _ = tx.close().await;
    tracing::info!("voice: call ended after {} ms, {} audio bytes", started.elapsed().as_millis(), bytes);
    let _ = events.send(VoiceEvent::Closed).await;
}


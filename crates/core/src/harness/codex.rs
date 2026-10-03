//! Codex adapter: a thread on the session worker's `codex app-server` (docs/spikes/3-codex-app-server.md).
//!
//! The router (one task) reads every app-server message and hands those with a `threadId` to the
//! session of that thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::App;
use crate::codex_rpc::Incoming;
use crate::model::*;
use crate::sessions::{LiveSession, Sink, StartSpec, shorten, truncate_output};
use crate::util::now_ms;

pub struct CodexSession {
    app: App,
    thread_id: String,
    effort: Option<String>,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    turn_id: Option<String>,
    /// Items started and not completed: id → (type, start time, last output time).
    open: HashMap<String, (String, i64, i64)>,
    /// Pending `item/tool/requestUserInput`: our request id → (JSON-RPC id, question ids).
    requests: HashMap<String, (Value, Vec<String>)>,
    cwd: String,
}

/// Live Codex sessions by thread id, for the router.
type ThreadSession = (Sink, Arc<Mutex<State>>);
type Threads = Arc<Mutex<HashMap<String, ThreadSession>>>;

fn threads() -> &'static Threads {
    static T: std::sync::OnceLock<Threads> = std::sync::OnceLock::new();
    T.get_or_init(Default::default)
}

pub async fn start(app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
    let cwd = spec.cwd.to_string_lossy().to_string();
    let thread_id = if spec.resume {
        let id = spec.harness_session_id.clone().context("no Codex thread to resume")?;
        app.codex
            .request(
                "thread/resume",
                json!({"threadId": id, "cwd": cwd, "excludeTurns": true, "approvalPolicy": "never", "sandbox": "danger-full-access"}),
            )
            .await
            .context("thread/resume")?;
        id
    } else {
        let mut params = json!({"cwd": cwd, "approvalPolicy": "never", "sandbox": "danger-full-access"});
        if let Some(m) = &spec.model {
            params["model"] = json!(m);
        }
        let res = app.codex.request("thread/start", params).await.context("thread/start")?;
        let id = res["thread"]["id"].as_str().context("no thread id")?.to_string();
        sink.set_harness_id(&id).await;
        if let Some(m) = res["model"].as_str().or(res["thread"]["model"].as_str()) {
            sink.set_model(m).await;
        }
        id
    };
    let state = Arc::new(Mutex::new(State { cwd, ..Default::default() }));
    threads().lock().unwrap().insert(thread_id.clone(), (sink.clone(), state.clone()));
    Ok(Arc::new(CodexSession { app: app.clone(), thread_id, effort: spec.effort, state }))
}

#[async_trait]
impl LiveSession for CodexSession {
    async fn send(&self, text: &str, client_id: &str, images: &[ImageAttachment]) -> Result<()> {
        let mut input = vec![];
        if !text.is_empty() { input.push(json!({"type": "text", "text": text})); }
        input.extend(images.iter().map(|i| json!({"type": "image", "url": i.data_url})));
        let turn = self.state.lock().unwrap().turn_id.clone();
        if let Some(turn_id) = turn {
            // Typing while the agent works steers the running turn.
            let res = self
                .app
                .codex
                .request(
                    "turn/steer",
                    json!({"threadId": self.thread_id, "expectedTurnId": turn_id, "input": input, "clientUserMessageId": client_id}),
                )
                .await;
            if res.is_ok() {
                return Ok(());
            }
            // The turn ended in the meantime: start a new one.
        }
        let mut params = json!({"threadId": self.thread_id, "input": input, "clientUserMessageId": client_id});
        if let Some(e) = &self.effort {
            params["effort"] = json!(e);
        }
        let res = self.app.codex.request("turn/start", params).await?;
        if let Some(id) = res["turn"]["id"].as_str() {
            self.state.lock().unwrap().turn_id = Some(id.to_string());
        }
        Ok(())
    }

    async fn interrupt(&self) -> Result<()> {
        let turn = self.state.lock().unwrap().turn_id.clone().context("no turn is running")?;
        self.app.codex.request("turn/interrupt", json!({"threadId": self.thread_id, "turnId": turn})).await?;
        Ok(())
    }

    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        let (rpc_id, ids) = self.state.lock().unwrap().requests.remove(request_id).context("this question was already answered")?;
        let mut map = serde_json::Map::new();
        for (qid, a) in ids.iter().zip(answers) {
            map.insert(qid.clone(), json!({"answers": [a]}));
        }
        self.app.codex.respond(rpc_id, json!({"answers": map})).await
    }

    async fn stop(&self) -> Result<()> {
        let turn = self.state.lock().unwrap().turn_id.clone();
        if let Some(turn) = turn {
            let _ = self.app.codex.request("turn/interrupt", json!({"threadId": self.thread_id, "turnId": turn})).await;
        }
        threads().lock().unwrap().remove(&self.thread_id);
        let _ = self.app.codex.request("thread/unsubscribe", json!({"threadId": self.thread_id})).await;
        Ok(())
    }
}

/// Routes app-server messages to sessions, and runs the background-command heuristic.
pub fn spawn_router(app: &App) {
    let a = app.clone();
    tokio::spawn(async move {
        let mut rx = a.codex.subscribe();
        loop {
            match rx.recv().await {
                Ok(Incoming::Notification { method, params }) => {
                    let Some(entry) = params["threadId"].as_str().and_then(|t| threads().lock().unwrap().get(t).cloned()) else { continue };
                    entry.0.raw("out", &json!({"method": method, "params": params}).to_string()).await;
                    notification(&entry.0, &entry.1, &method, &params).await;
                }
                Ok(Incoming::Request { id, method, params }) => {
                    let entry = params["threadId"].as_str().and_then(|t| threads().lock().unwrap().get(t).cloned());
                    request(&a, entry, id, &method, &params).await;
                }
                Ok(Incoming::Exited) => {
                    // Every live thread is lost. Done sessions resume when you send a message.
                    let all: Vec<(String, ThreadSession)> = threads().lock().unwrap().drain().collect();
                    for (_, (sink, _)) in all {
                        sink.exited(Some("codex app-server exited".into())).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!("codex router lagged by {n} messages"),
                Err(_) => break,
            }
        }
    });

}

async fn request(app: &App, entry: Option<(Sink, Arc<Mutex<State>>)>, id: Value, method: &str, params: &Value) {
    match (method, entry) {
        ("item/tool/requestUserInput", Some((sink, st))) => {
            let request_id = uuid::Uuid::new_v4().to_string();
            let qs = params["questions"].as_array().cloned().unwrap_or_default();
            let questions: Vec<Question> = qs
                .iter()
                .map(|q| Question {
                    id: q["id"].as_str().unwrap_or("").into(),
                    header: q["header"].as_str().unwrap_or("").into(),
                    question: q["question"].as_str().unwrap_or("").into(),
                    options: q["options"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|o| QuestionOption { label: o["label"].as_str().unwrap_or("").into(), description: o["description"].as_str().unwrap_or("").into() })
                        .collect(),
                    multi_select: false,
                })
                .collect();
            st.lock().unwrap().requests.insert(request_id.clone(), (id, questions.iter().map(|q| q.id.clone()).collect()));
            sink.emit(EventBody::AskUser { request_id, questions }).await;
            sink.set_open_question(true).await;
            sink.set_status(SessionStatus::Waiting).await;
        }
        (m, entry) if m.ends_with("requestApproval") || m == "applyPatchApproval" || m == "execCommandApproval" => {
            // Should not happen with approvalPolicy "never": declined and logged.
            tracing::warn!("codex approval request declined: {m}");
            if let Some((sink, _)) = entry {
                sink.emit(EventBody::Notice { text: format!("Declined an approval request ({m})") }).await;
            }
            let _ = app.codex.respond(id, json!({"decision": "decline"})).await;
        }
        (m, _) => {
            tracing::warn!("codex server request not supported: {m}");
            let _ = app.codex.respond(id, json!({})).await;
        }
    }
}

fn item_summary(item: &Value) -> (String, String) {
    match item["type"].as_str().unwrap_or("") {
        "commandExecution" => {
            let cmd = item["commandActions"][0]["command"].as_str().or(item["command"].as_str()).unwrap_or("");
            ("Shell".into(), shorten(cmd, 160))
        }
        "mcpToolCall" => (format!("{}.{}", item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("")), shorten(&item["arguments"].to_string(), 120)),
        "dynamicToolCall" => (item["tool"].as_str().unwrap_or("tool").into(), shorten(&item["arguments"].to_string(), 120)),
        "webSearch" => ("Web search".into(), item["query"].as_str().unwrap_or("").into()),
        "imageView" => ("View image".into(), item["path"].as_str().unwrap_or("").into()),
        "collabAgentToolCall" => ("Agent".into(), shorten(item["prompt"].as_str().unwrap_or(""), 120)),
        t => (t.to_string(), String::new()),
    }
}

const TOOL_ITEMS: &[&str] = &["commandExecution", "mcpToolCall", "dynamicToolCall", "webSearch", "imageView", "collabAgentToolCall"];

async fn notification(sink: &Sink, st: &Arc<Mutex<State>>, method: &str, p: &Value) {
    match method {
        "turn/started" => {
            st.lock().unwrap().turn_id = p["turn"]["id"].as_str().map(str::to_string);
            if sink.status() != Some(SessionStatus::Working) {
                sink.set_status(SessionStatus::Working).await;
            }
        }
        "item/started" => {
            let item = &p["item"];
            let (t, id) = (item["type"].as_str().unwrap_or(""), item["id"].as_str().unwrap_or("").to_string());
            match t {
                "userMessage" => {
                    if let Some(c) = item["clientId"].as_str() {
                        sink.emit(EventBody::MessageDelivered { client_id: c.into() }).await;
                    }
                }
                t if TOOL_ITEMS.contains(&t) => {
                    st.lock().unwrap().open.insert(id.clone(), (t.to_string(), now_ms(), now_ms()));
                    let (name, summary) = item_summary(item);
                    sink.emit(EventBody::ToolCall { item_id: id, name, summary, input: item.clone() }).await;
                }
                _ => {}
            }
        }
        "item/agentMessage/delta" => {
            sink.emit(EventBody::AssistantDelta { item_id: p["itemId"].as_str().unwrap_or("").into(), text: p["delta"].as_str().unwrap_or("").into() }).await;
        }
        "item/commandExecution/outputDelta" => {
            let id = p["itemId"].as_str().unwrap_or("").to_string();
            if let Some(e) = st.lock().unwrap().open.get_mut(&id) {
                e.2 = now_ms();
            }
            if sink.status() == Some(SessionStatus::Background) {
                sink.set_status(SessionStatus::Working).await;
            }
            sink.emit(EventBody::ToolOutput { item_id: id, text: p["delta"].as_str().unwrap_or("").into() }).await;
        }
        "item/completed" => {
            let item = &p["item"];
            let (t, id) = (item["type"].as_str().unwrap_or(""), item["id"].as_str().unwrap_or("").to_string());
            st.lock().unwrap().open.remove(&id);
            match t {
                "agentMessage" => {
                    let text = item["text"].as_str().unwrap_or("").to_string();
                    if !text.trim().is_empty() {
                        sink.emit(EventBody::AssistantMessage { item_id: id, text }).await;
                    }
                }
                "reasoning" => {
                    let text = item["summary"].as_array().map(|a| a.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    if !text.trim().is_empty() {
                        sink.emit(EventBody::Reasoning { item_id: id, text }).await;
                    }
                }
                "plan" => {
                    let text = item["text"].as_str().unwrap_or("");
                    if !text.trim().is_empty() {
                        sink.emit(EventBody::AssistantMessage { item_id: id, text: text.into() }).await;
                    }
                }
                "fileChange" => {
                    let cwd = st.lock().unwrap().cwd.clone();
                    for (i, c) in item["changes"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        let path = c["path"].as_str().unwrap_or("");
                        let path = path.strip_prefix(&cwd).map(|p| p.trim_start_matches('/')).unwrap_or(path).to_string();
                        let change = c["kind"]["type"].as_str().unwrap_or("update").to_string();
                        let raw = c["diff"].as_str().unwrap_or("");
                        let diff = if change == "add" { raw.lines().map(|l| format!("+{l}")).collect::<Vec<_>>().join("\n") } else { raw.to_string() };
                        sink.emit(EventBody::FileChange { item_id: format!("{id}:{i}"), path, change, diff }).await;
                    }
                }
                "commandExecution" => {
                    let exit = item["exitCode"].as_i64().map(|c| c as i32);
                    sink.emit(EventBody::ToolResult {
                        item_id: id,
                        output: truncate_output(item["aggregatedOutput"].as_str().unwrap_or(""), 30_000),
                        is_error: item["status"] != "completed" || exit.is_some_and(|c| c != 0),
                        exit_code: exit,
                        duration_ms: item["durationMs"].as_i64(),
                    })
                    .await;
                }
                t if TOOL_ITEMS.contains(&t) => {
                    let output = if t == "mcpToolCall" {
                        if item["error"].is_null() { item["result"].to_string() } else { item["error"].to_string() }
                    } else if t == "webSearch" {
                        item["results"].to_string()
                    } else {
                        item["contentItems"].to_string()
                    };
                    sink.emit(EventBody::ToolResult {
                        item_id: id,
                        output: truncate_output(&output, 30_000),
                        is_error: item["status"] == "failed",
                        exit_code: None,
                        duration_ms: item["durationMs"].as_i64(),
                    })
                    .await;
                }
                _ => {}
            }
        }
        "turn/plan/updated" => {
            let items = p["plan"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|s| PlanItem {
                    text: s["step"].as_str().unwrap_or("").into(),
                    status: match s["status"].as_str().unwrap_or("") {
                        "inProgress" => "in_progress".into(),
                        other => other.to_string(),
                    },
                })
                .collect();
            sink.emit(EventBody::Plan { items }).await;
        }
        "thread/tokenUsage/updated" => {
            let u = &p["tokenUsage"];
            let (total, last) = (&u["total"], &u["last"]);
            let cached = total["cachedInputTokens"].as_u64().unwrap_or(0);
            sink.update_usage(|us| {
                us.context_tokens = last["totalTokens"].as_u64();
                us.context_window = u["modelContextWindow"].as_u64();
                us.input_tokens = total["inputTokens"].as_u64().unwrap_or(0).saturating_sub(cached);
                us.cached_tokens = cached;
                us.output_tokens = total["outputTokens"].as_u64().unwrap_or(0);
            })
            .await;
        }
        "turn/completed" => {
            let turn = &p["turn"];
            let interrupted = turn["status"] == "interrupted";
            let error = turn["error"]["message"].as_str().map(str::to_string);
            // Items still open are marked as interrupted. An interrupt does not stop a running command,
            // so its real `item/completed` may still arrive later and replace this.
            let open: Vec<(String, (String, i64, i64))> = st.lock().unwrap().open.drain().collect();
            for (id, (t, _, _)) in open {
                if TOOL_ITEMS.contains(&t.as_str()) {
                    sink.emit(EventBody::ToolResult { item_id: id, output: "Interrupted".into(), is_error: true, exit_code: None, duration_ms: None }).await;
                }
            }
            st.lock().unwrap().turn_id = None;
            sink.emit(EventBody::TurnEnd { duration_ms: turn["durationMs"].as_i64(), interrupted, error: error.clone() }).await;
            if !st.lock().unwrap().requests.is_empty() {
                return;
            }
            if turn["status"] == "failed" {
                sink.emit(EventBody::Error { message: error.unwrap_or_else(|| "the turn failed".into()) }).await;
            }
            sink.set_status(SessionStatus::Waiting).await;
        }
        "error" => {
            if let Some(m) = p["error"]["message"].as_str() {
                sink.emit(EventBody::Error { message: m.into() }).await;
            }
        }
        _ => {}
    }
}

/// Models offered by Codex (`model/list`).
pub async fn models(app: &App) -> Result<Vec<ModelInfo>> {
    let res = app.codex.request("model/list", json!({})).await?;
    Ok(res["data"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|m| m["hidden"].as_bool() != Some(true))
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["id"].as_str()?.to_string(),
                display_name: m["displayName"].as_str().unwrap_or("").to_string(),
                description: m["description"].as_str().unwrap_or("").to_string(),
                efforts: m["supportedReasoningEfforts"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|e| e["reasoningEffort"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
            })
        })
        .collect())
}

/// Model and effort Codex uses when none is given (`~/.codex/config.toml`).
pub fn defaults() -> (Option<String>, Option<String>) {
    let path = dirs::home_dir().unwrap_or_default().join(".codex/config.toml");
    let v: toml::Value = std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or(toml::Value::Table(Default::default()));
    (v.get("model").and_then(|m| m.as_str()).map(str::to_string), v.get("model_reasoning_effort").and_then(|m| m.as_str()).map(str::to_string))
}

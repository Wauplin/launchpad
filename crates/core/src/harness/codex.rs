//! Codex adapter: threads on the shared `codex app-server` (docs/spikes/3-codex-app-server.md).
//!
//! The router (one task) reads every app-server message and hands those with a `threadId` to the
//! session of that thread.

mod history;
mod rpc;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use self::rpc::{CodexServer, Incoming};
use crate::App;
use crate::model::*;
use crate::sessions::{LiveSession, Sink, StartSpec, shorten, truncate_output};
use crate::util::now_ms;

pub struct CodexSession {
    server: CodexServer,
    threads: Threads,
    thread_id: String,
    /// Model and effort for the next turns: (model, effort).
    model: Mutex<(Option<String>, Option<String>)>,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    turn_id: Option<String>,
    commands: HashSet<String>,
    /// Items started and not completed: id → (type, start time, last output time).
    open: HashMap<String, (String, i64, i64)>,
    /// Pending `item/tool/requestUserInput`: our request id → (JSON-RPC id, question ids).
    requests: HashMap<String, (Value, Vec<String>)>,
    cwd: String,
}

/// Live Codex sessions by thread id, for the router.
type ThreadSession = (Sink, Arc<Mutex<State>>);
type Threads = Arc<Mutex<HashMap<String, ThreadSession>>>;

async fn start(
    server: &CodexServer,
    threads: &Threads,
    spec: StartSpec,
    sink: Sink,
) -> Result<Arc<dyn LiveSession>> {
    let cwd = spec.cwd.to_string_lossy().to_string();
    let thread_id = if spec.resume {
        let id = spec
            .harness_session_id
            .clone()
            .context("no Codex thread to resume")?;
        server
            .request(
                "thread/resume",
                {
                    let mut params = json!({"threadId": id, "cwd": cwd, "excludeTurns": true, "approvalPolicy": "never", "sandbox": "danger-full-access"});
                    if let Some(m) = &spec.model {
                        params["model"] = json!(m);
                    }
                    params
                },
            )
            .await
            .context("thread/resume")?;
        id
    } else {
        let mut params = json!({"cwd": cwd, "approvalPolicy": "never", "sandbox": "danger-full-access"});
        if let Some(m) = &spec.model {
            params["model"] = json!(m);
        }
        let res = server
            .request("thread/start", params)
            .await
            .context("thread/start")?;
        let id = res["thread"]["id"]
            .as_str()
            .context("no thread id")?
            .to_string();
        sink.set_harness_id(&id).await;
        if let Some(m) = res["model"].as_str().or(res["thread"]["model"].as_str()) {
            sink.set_model(m).await;
        }
        id
    };
    let state = Arc::new(Mutex::new(State {
        cwd,
        ..Default::default()
    }));
    threads
        .lock()
        .unwrap()
        .insert(thread_id.clone(), (sink.clone(), state.clone()));
    Ok(Arc::new(CodexSession {
        server: server.clone(),
        threads: threads.clone(),
        thread_id,
        model: Mutex::new((spec.model, spec.effort)),
        state,
    }))
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
            let res = self.server
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
        let (model, effort) = self.model.lock().unwrap().clone();
        if let Some(m) = model {
            params["model"] = json!(m);
        }
        if let Some(e) = effort {
            params["effort"] = json!(e);
        }
        let res = self.server.request("turn/start", params).await?;
        if let Some(id) = res["turn"]["id"].as_str() {
            self.state.lock().unwrap().turn_id = Some(id.to_string());
        }
        Ok(())
    }

    fn turn_active(&self) -> bool { self.state.lock().unwrap().turn_id.is_some() }

    fn set_model(&self, model: Option<&str>, effort: Option<&str>) -> bool {
        *self.model.lock().unwrap() = (model.map(str::to_string), effort.map(str::to_string));
        true
    }

    fn ready_for_turn(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.turn_id.is_none() && s.commands.is_empty() && s.requests.is_empty()
    }

    async fn interrupt(&self) -> Result<()> {
        let turn = self
            .state
            .lock()
            .unwrap()
            .turn_id
            .clone()
            .context("no turn is running")?;
        self.server
            .request(
                "turn/interrupt",
                json!({"threadId": self.thread_id, "turnId": turn}),
            )
            .await?;
        Ok(())
    }

    fn has_open_questions(&self) -> bool { !self.state.lock().unwrap().requests.is_empty() }
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        let (rpc_id, ids) = self.state.lock().unwrap().requests.remove(request_id).context("this question was already answered")?;
        let mut map = serde_json::Map::new();
        for (qid, a) in ids.iter().zip(answers) {
            map.insert(qid.clone(), json!({"answers": [a]}));
        }
        self.server.respond(rpc_id, json!({"answers": map})).await
    }

    async fn stop(&self) -> Result<()> {
        let turn = self.state.lock().unwrap().turn_id.clone();
        if let Some(turn) = turn {
            let _ = self
                .server
                .request(
                    "turn/interrupt",
                    json!({"threadId": self.thread_id, "turnId": turn}),
                )
                .await;
        }
        self.threads.lock().unwrap().remove(&self.thread_id);
        let _ = self
            .server
            .request("thread/unsubscribe", json!({"threadId": self.thread_id}))
            .await;
        Ok(())
    }
}

/// Routes app-server messages to sessions, and runs the background-command heuristic.
fn spawn_router(_app: &App, server: &CodexServer, threads: &Threads) {
    let server = server.clone();
    let router_threads = threads.clone();
    tokio::spawn(async move {
        let mut rx = server.subscribe();
        loop {
            match rx.recv().await {
                Ok(Incoming::Notification { method, params }) => {
                    let Some(entry) = params["threadId"]
                        .as_str()
                        .and_then(|t| router_threads.lock().unwrap().get(t).cloned())
                    else {
                        continue;
                    };
                    entry
                        .0
                        .raw(
                            "out",
                            &json!({"method": method, "params": params}).to_string(),
                        )
                        .await;
                    notification(&entry.0, &entry.1, &method, &params).await;
                }
                Ok(Incoming::Request { id, method, params }) => {
                    let entry = params["threadId"]
                        .as_str()
                        .and_then(|t| router_threads.lock().unwrap().get(t).cloned());
                    request(&server, entry, id, &method, &params).await;
                }
                Ok(Incoming::Exited) => {
                    // Every live thread is lost. Done sessions resume when you send a message.
                    let all: Vec<(String, ThreadSession)> =
                        router_threads.lock().unwrap().drain().collect();
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

async fn request(
    server: &CodexServer,
    entry: Option<(Sink, Arc<Mutex<State>>)>,
    id: Value,
    method: &str,
    params: &Value,
) {
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
            let _ = server.respond(id, json!({"decision": "decline"})).await;
        }
        (m, _) => {
            tracing::warn!("codex server request not supported: {m}");
            let _ = server.respond(id, json!({})).await;
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
                    if t == "commandExecution" { st.lock().unwrap().commands.insert(id.clone()); }
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
            st.lock().unwrap().commands.remove(&id);
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
            sink.set_status(if turn["status"] == "failed" { SessionStatus::Error }
                else if sink.stop_requested() { SessionStatus::Stopped }
                else if interrupted { SessionStatus::Interrupted } else { SessionStatus::Waiting }).await;
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
async fn models(server: &CodexServer) -> Result<Vec<ModelInfo>> {
    let res = server.request("model/list", json!({})).await?;
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

pub const ID: HarnessKind = HarnessKind::new("codex");
pub const REGISTRATION: super::Registration = super::Registration {
    descriptor: super::HarnessDescriptor {
        id: ID,
        name: "Codex",
        icon_svg: include_str!("icons/codex.svg"),
        color: "#566477",
        working_message: "Message Codex. It steers the running turn.",
        capabilities: super::HarnessCapabilities {
            images: true,
            messages_while_running: true,
            history: true,
            archived_history: true,
            terminal: true,
            quota: true,
        },
    },
    create: |env| {
        Arc::new(Adapter {
            server: CodexServer::new(env.clone()),
            threads: Arc::default(),
        })
    },
    environment_marker: |key| {
        key.starts_with("CODEX_THREAD")
            || key == "CODEX_SANDBOX"
            || key == "CODEX_SANDBOX_NETWORK_DISABLED"
    },
};
pub struct Adapter {
    server: CodexServer,
    threads: Threads,
}
#[async_trait]
impl super::Harness for Adapter {
    async fn start(&self, _app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        start(&self.server, &self.threads, spec, sink).await
    }
    async fn models(&self, _app: &App) -> Result<Vec<ModelInfo>> {
        models(&self.server).await
    }
    fn defaults(&self) -> (Option<String>, Option<String>) {
        defaults()
    }
    fn efforts(&self) -> Vec<String> {
        ["low", "medium", "high", "xhigh", "max"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
    async fn version(&self, app: &App) -> Result<String> {
        Ok(
            crate::procs::output(&app.env, "codex", &["--version"], None)
                .await?
                .split_whitespace()
                .last()
                .unwrap_or("")
                .into(),
        )
    }
    fn spawn_services(&self, app: &App) {
        spawn_router(app, &self.server, &self.threads);
        spawn_quota(app, &self.server);
        spawn_updates(app);
    }
    fn terminal_resume_markers(&self) -> &'static [&'static str] {
        &["resume"]
    }
    fn terminal_command(&self, s: &Session, _is_temp: bool) -> Result<Vec<String>> {
        Ok(vec![
            "codex".into(),
            "resume".into(),
            s.harness_session_id
                .clone()
                .context("the session has no conversation yet")?,
            "--dangerously-bypass-approvals-and-sandbox".into(),
            "-c".into(),
            "check_for_update_on_startup=false".into(),
            "-c".into(),
            project_trust_override(&s.cwd),
        ])
    }
    async fn history_list(
        &self,
        _app: &App,
        cursor: Option<String>,
        archived: bool,
    ) -> Result<crate::history::Page> {
        history::list(&self.server, cursor, archived).await
    }
    async fn history_read(&self, _app: &App, id: &str) -> Result<crate::history::Snapshot> {
        history::read(&self.server, id).await
    }
    fn title_command(
        &self,
        app: &App,
        session: &Session,
        cwd: &std::path::Path,
    ) -> Option<tokio::process::Command> {
        let mut cmd = crate::procs::child_command(&app.env, "codex", Some(cwd));
        cmd.args([
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--sandbox",
            "read-only",
            "--json",
            "-c",
            "model_reasoning_effort=\"low\"",
        ]);
        let defaults = app.models.defaults(app, ID);
        let small = defaults
            .models
            .iter()
            .find(|m| m.id.contains("luna") || m.id.contains("mini"))
            .map(|m| m.id.clone());
        if let Some(model) = small.or_else(|| session.model.clone()).or(defaults.model) {
            cmd.args(["--model", &model]);
        }
        Some(cmd)
    }
    fn title_response(&self, stdout: String) -> Result<String> {
        stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|e| e["type"] == "item.completed" && e["item"]["type"] == "agent_message")
            .filter_map(|e| e["item"]["text"].as_str().map(str::to_string))
            .next_back()
            .context("no generated title")
    }
}
/// Quota from a Codex `rateLimits` object.
fn rate_limits(status: &crate::status::StatusHandle, rl: &Value) {
    let mut five = None;
    let mut week = None;
    for key in ["primary", "secondary"] {
        let w = &rl[key];
        let (Some(used), Some(resets), mins) = (
            w["usedPercent"].as_f64(),
            w["resetsAt"].as_i64(),
            w["windowDurationMins"].as_i64(),
        ) else {
            continue;
        };
        let q = crate::status::QuotaWindow {
            used_percent: used,
            resets_at: resets,
        };
        match (key, mins) {
            (_, Some(300)) => five = Some(q),
            (_, Some(10080)) => week = Some(q),
            ("primary", _) => five = Some(q),
            _ => week = Some(q),
        }
    }
    status.update_harness(ID, |s| {
        s.five_hour = five;
        s.weekly = week;
        s.quota_at = Some(now_ms());
        s.quota_error = None;
    });
}
fn spawn_quota(app: &App, server: &CodexServer) {
    let a = app.clone();
    let notifications = server.clone();
    tokio::spawn(async move {
        let mut rx = notifications.subscribe();
        loop {
            match rx.recv().await {
                Ok(Incoming::Notification { method, params })
                    if method == "account/rateLimits/updated" =>
                {
                    rate_limits(&a.status, &params["rateLimits"]);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });

    let a = app.clone();
    let server = server.clone();
    tokio::spawn(async move {
        loop {
            match server.request("account/rateLimits/read", Value::Null).await {
                Ok(res) => rate_limits(&a.status, &res["rateLimits"]),
                Err(e) => {
                    tracing::info!("codex rate limits: {e:#}");
                    a.status
                        .update_harness(ID, |s| s.quota_error = Some(format!("{e:#}")));
                }
            }
            crate::status::wait_poll(&a, crate::harness::codex::ID, 120, 30).await;
        }
    });
}
fn spawn_updates(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            app.status
                .update_harness(ID, |s| s.updating_to = Some("latest".into()));
            let result = crate::procs::command(&app.env, "codex", None)
                .arg("update")
                .stdin(std::process::Stdio::null())
                .output()
                .await;
            match result {
                Ok(out) => tracing::info!(
                    "codex update ({}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
                Err(error) => tracing::warn!("codex update: {error}"),
            }
            app.status.update_harness(ID, |s| s.updating_to = None);
            crate::status::refresh_versions(&app).await;
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
        }
    });
}

/// JSON quoting also escapes TOML basic-string keys, including Windows backslashes.
fn project_trust_override(cwd: &str) -> String {
    format!(
        "projects.{}.trust_level=\"trusted\"",
        serde_json::to_string(cwd).expect("serialize a path string")
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn project_trust_keys_preserve_paths() {
        for cwd in [
            r"C:\Users\O'Brien\projects\launchpad",
            r#"/home/user/a "quoted" folder"#,
        ] {
            let config: toml::Value = toml::from_str(&super::project_trust_override(cwd)).unwrap();
            assert_eq!(
                config["projects"][cwd]["trust_level"].as_str(),
                Some("trusted")
            );
        }
    }
}

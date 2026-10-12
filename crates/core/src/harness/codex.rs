//! Codex adapter: threads on the shared `codex app-server` (docs/spikes/3-codex-app-server.md).
//!
//! The router (one task) reads every app-server message and hands those with a `threadId` to the
//! session of that thread.

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
    completed: HashSet<String>,
    running: bool,
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
    if spec.resume {
        let mut children = HashMap::new();
        for event in sink.app().sessions.events(sink.app(), &sink.id, 0).await? {
            if let EventBody::Subagent { agent_id, parent_id, status, .. } = event.body {
                let child = children.entry(agent_id).or_insert((parent_id.clone(), "unknown".to_string()));
                if parent_id.is_some() { child.0 = parent_id; }
                if let Some(status) = status { child.1 = status; }
            }
        }
        for (id, (parent, status)) in children {
            let owner = parent.as_deref().map(|id| sink.child(id)).unwrap_or_else(|| sink.clone());
            let status = if status == "running" { "interrupted" } else { &status };
            register_child(server, threads, &owner, &id, None, None, status).await;
        }
    }
    Ok(Arc::new(CodexSession {
        server: server.clone(),
        threads: threads.clone(),
        thread_id,
        model: Mutex::new((spec.model, spec.effort)),
        state,
    }))
}

impl CodexSession {
    fn active_turns(&self) -> Vec<(String, String)> {
        let threads = self.threads.lock().unwrap();
        let root = threads.get(&self.thread_id).map(|(s, _)| s.id.as_str());
        threads.iter().filter(|(_, (s, _))| Some(s.id.as_str()) == root).filter_map(|(id, (_, state))| state.lock().unwrap().turn_id.clone().map(|turn| (id.clone(), turn))).collect()
    }
}
fn child_busy(state: &State) -> bool { state.running || state.turn_id.is_some() || !state.commands.is_empty() }
async fn settle_parent(threads: &Threads, owner: &Sink) {
    let state = {
        let threads = threads.lock().unwrap();
        let root = threads.values().find(|(s, _)| s.id == owner.id && s.agent_id.is_none());
        root.map(|(sink, state)| {
            let state = state.lock().unwrap();
            let busy = !state.commands.is_empty() || threads.values().any(|(s, st)| s.id == owner.id && s.agent_id.is_some() && child_busy(&st.lock().unwrap()));
            (sink.clone(), state.turn_id.is_none(), busy)
        })
    };
    if let Some((sink, idle, busy)) = state
        && idle && matches!(sink.status(), Some(SessionStatus::Waiting | SessionStatus::Background)) {
        sink.set_status(if busy { SessionStatus::Background } else if sink.stop_requested() { SessionStatus::Stopped } else { SessionStatus::Waiting }).await;
    }
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

    fn turn_active(&self) -> bool { !self.active_turns().is_empty() }

    fn set_model(&self, model: Option<&str>, effort: Option<&str>) -> bool {
        *self.model.lock().unwrap() = (model.map(str::to_string), effort.map(str::to_string));
        true
    }

    fn ready_for_turn(&self) -> bool {
        let s = self.state.lock().unwrap();
        if s.turn_id.is_some() || !s.commands.is_empty() || !s.requests.is_empty() { return false; }
        drop(s);
        let threads = self.threads.lock().unwrap();
        let root = threads.get(&self.thread_id).map(|(s, _)| s.id.as_str());
        !threads.values().any(|(sink, state)| sink.agent_id.is_some() && Some(sink.id.as_str()) == root && child_busy(&state.lock().unwrap()))
    }

    async fn interrupt(&self) -> Result<()> {
        let turns = self.active_turns();
        anyhow::ensure!(!turns.is_empty(), "no turn is running");
        let mut interrupted = false;
        let mut error = None;
        for (thread, turn) in turns {
            match self.server.request("turn/interrupt", json!({"threadId":thread,"turnId":turn})).await {
                Ok(_) => interrupted = true,
                Err(e) => error = Some(e),
            }
        }
        if !interrupted { return Err(error.context("no active turn could be interrupted")?); }
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
        for (thread, turn) in self.active_turns() {
            let _ = self.server.request("turn/interrupt", json!({"threadId":thread,"turnId":turn})).await;
        }
        let children: Vec<String> = {
            let mut threads = self.threads.lock().unwrap();
            let ids = threads.iter().filter(|(_, (sink, _))| sink.id == threads.get(&self.thread_id).map(|(s, _)| s.id.as_str()).unwrap_or("") && sink.agent_id.is_some()).map(|(id, _)| id.clone()).collect::<Vec<_>>();
            for id in &ids { threads.remove(id); }
            threads.remove(&self.thread_id);
            ids
        };
        for id in children { let _ = self.server.request("thread/unsubscribe", json!({"threadId":id})).await; }
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
                    // Native thread ancestry catches nested agents, including calls inside code mode.
                    if method == "thread/started" {
                        let thread = &params["thread"];
                        let parent = thread["parentThreadId"].as_str().or_else(|| thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].as_str());
                        let owner = parent.and_then(|id| router_threads.lock().unwrap().get(id).cloned());
                        if let (Some((sink, _)), Some(id)) = (owner, thread["id"].as_str()) {
                            register_child(&server, &router_threads, &sink, id, thread["agentNickname"].as_str().or(thread["name"].as_str()).map(str::to_string), thread["model"].as_str().map(str::to_string), "running").await;
                        }
                    }
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
                    discover_children(&server, &router_threads, &entry.0, &params["item"]).await;
                    notification(&entry.0, &entry.1, &method, &params, &router_threads).await;
                    settle_parent(&router_threads, &entry.0).await;
                }
                Ok(Incoming::Request { id, method, params }) => {
                    let entry = params["threadId"]
                        .as_str()
                        .and_then(|t| router_threads.lock().unwrap().get(t).cloned());
                    let entry = if method == "item/tool/requestUserInput" {
                        if let Some((child, _)) = &entry && child.agent_id.is_some() {
                            child.emit(EventBody::Notice { text: "Waiting for your answer in the parent session.".into() }).await;
                            router_threads.lock().unwrap().values().find(|(root, _)| root.id == child.id && root.agent_id.is_none()).cloned()
                        } else { entry }
                    } else { entry };
                    request(&server, entry, id, &method, &params).await;
                }
                Ok(Incoming::Exited) => {
                    // Every live thread is lost. Done sessions resume when you send a message.
                    let all: Vec<(String, ThreadSession)> =
                        router_threads.lock().unwrap().drain().collect();
                    for (_, (sink, _)) in all {
                        if sink.agent_id.is_some() { sink.set_status(SessionStatus::Interrupted).await; continue; }
                        sink.exited(Some("codex app-server exited".into())).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!("codex router lagged by {n} messages"),
                Err(_) => break,
            }
        }
    });

}

/// Join a child as an observer, preserving its native configuration. Never start a child turn.
async fn register_child(server: &CodexServer, threads: &Threads, owner: &Sink, id: &str, name: Option<String>, model: Option<String>, initial_status: &str) {
    let child = owner.child(id);
    let added = {
        let mut threads = threads.lock().unwrap();
        if threads.contains_key(id) { false } else {
            threads.insert(id.into(), (child.clone(), Arc::new(Mutex::new(State { running: initial_status == "running", cwd: owner.app().sessions.get(&owner.id).map(|s| s.cwd).unwrap_or_default(), ..Default::default() }))));
            true
        }
    };
    if !added { return; }
    owner.subagent(id, name, Some(initial_status.into()), model).await;
    let server = server.clone();
    let threads = threads.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        match server.request("thread/resume", json!({"threadId":id,"excludeTurns":true})).await {
            Ok(response) => {
                let status = response["thread"]["status"]["type"].as_str();
                let state = threads.lock().unwrap().get(&id).map(|(_, st)| st.clone());
                if let Some(state) = state { state.lock().unwrap().running = status == Some("active"); }
                if status == Some("active") { child.set_status(SessionStatus::Working).await; }
                else if status == Some("systemError") { child.set_status(SessionStatus::Error).await; }
                settle_parent(&threads, &child).await;
            }
            Err(error) => { child.emit(EventBody::Notice { text: format!("Could not attach the live trace: {error:#}") }).await; }
        }
    });
}
async fn discover_children(server: &CodexServer, threads: &Threads, owner: &Sink, item: &Value) {
    if item["type"] == "subAgentActivity" {
        if let Some(id) = item["agentThreadId"].as_str() {
            register_child(server, threads, owner, id, item["agentPath"].as_str().map(str::to_string), item["model"].as_str().map(str::to_string), "running").await;
            let status = match item["kind"].as_str() {
                Some("completed") => Some("completed"), Some("interrupted") => Some("interrupted"),
                Some("started") => Some("running"), _ => None,
            };
            if let Some(status) = status {
                let state = threads.lock().unwrap().get(id).map(|(_, st)| st.clone());
                if let Some(state) = state { state.lock().unwrap().running = status == "running"; }
                owner.subagent(id, None, Some(status.into()), None).await;
            }
        }
    } else if item["type"] == "collabAgentToolCall" {
        for id in item["receiverThreadIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            register_child(server, threads, owner, id, None, item["model"].as_str().map(str::to_string), "running").await;
        }
        for (id, state) in item["agentsStates"].as_object().into_iter().flatten() {
            register_child(server, threads, owner, id, None, None, "running").await;
            let status = match state["status"].as_str() {
                Some("pendingInit" | "running") => "running", Some("completed") => "completed",
                Some("errored") => "failed", Some("interrupted") => "interrupted", Some("shutdown") => "stopped", _ => "unknown",
            };
            // These statuses describe receivers; wait/message calls are not ancestry evidence.
            let child = threads.lock().unwrap().get(id).map(|(sink, _)| sink.clone());
            let state = threads.lock().unwrap().get(id).map(|(_, st)| st.clone());
            if let Some(state) = state { state.lock().unwrap().running = status == "running"; }
            if let Some(child) = child {
                if status == "unknown" { child.root().subagent(id, None, Some("unknown".into()), None).await; }
                else { child.set_status(match status { "running" => SessionStatus::Working, "completed" => SessionStatus::Waiting, "failed" => SessionStatus::Error, "interrupted" => SessionStatus::Interrupted, _ => SessionStatus::Stopped }).await; }
            }
        }
    }
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

async fn notification(sink: &Sink, st: &Arc<Mutex<State>>, method: &str, p: &Value, threads: &Threads) {
    match method {
        "thread/status/changed" if sink.agent_id.is_some() => {
            st.lock().unwrap().running = p["status"]["type"] == "active";
            match p["status"]["type"].as_str() {
                Some("active") => sink.set_status(SessionStatus::Working).await,
                Some("systemError") => sink.set_status(SessionStatus::Error).await,
                _ => {},
            }
        }
        "turn/started" => {
            st.lock().unwrap().running = true;
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
                    if sink.agent_id.is_some() {
                        let text = item["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
                        if !text.is_empty() { sink.emit(EventBody::UserMessage { text, client_id: id.clone(), queued: false, images: vec![] }).await; }
                    }
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
            if !st.lock().unwrap().completed.insert(id.clone()) { return; }
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
            st.lock().unwrap().running = false;
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
            let commands_running = !st.lock().unwrap().commands.is_empty();
            let background = commands_running || sink.agent_id.is_none() && threads.lock().unwrap().values().any(|(child, state)| child.id == sink.id && child.agent_id.is_some() && child_busy(&state.lock().unwrap()));
            sink.set_status(if turn["status"] == "failed" { SessionStatus::Error }
                else if sink.stop_requested() { SessionStatus::Stopped }
                else if interrupted { SessionStatus::Interrupted } else if background { SessionStatus::Background } else { SessionStatus::Waiting }).await;
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
            terminal: true,
            quota: true,
            cost: false,
        },
    },
    create: |env| {
        Arc::new(Adapter {
            env: env.clone(),
            server: CodexServer::new(env.clone()),
            variant_servers: Mutex::default(),
            threads: Arc::default(),
        })
    },
    environment_marker: |key| {
        key.starts_with("CODEX_THREAD")
            || key == "CODEX_SANDBOX"
            || key == "CODEX_SANDBOX_NETWORK_DISABLED"
    },
};
type VariantEnv = Vec<(String, String)>;
pub struct Adapter {
    env: crate::shellenv::UserEnv,
    /// The shared app-server, also used for quotas and models.
    server: CodexServer,
    /// One app-server per distinct variant environment (`CODEX_HOME`, API keys), started on first use,
    /// with its own thread routes: an app-server exit only ends its own threads.
    variant_servers: Mutex<HashMap<VariantEnv, (CodexServer, Threads)>>,
    threads: Threads,
}
#[async_trait]
impl super::Harness for Adapter {
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        if spec.env.is_empty() {
            return start(&self.server, &self.threads, spec, sink).await;
        }
        let (server, threads) = {
            let mut servers = self.variant_servers.lock().unwrap();
            servers.entry(spec.env.clone()).or_insert_with(|| {
                let server = CodexServer::new(self.env.with(&spec.env));
                let threads = Threads::default();
                spawn_router(app, &server, &threads);
                (server, threads)
            }).clone()
        };
        start(&server, &threads, spec, sink).await
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
            resets_at: Some(resets),
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

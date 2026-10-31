//! Claude Code adapter: one `claude -p` process per session, stream-json on stdin and stdout.
//! The exact behavior is recorded in docs/spikes/1-claude-stream-json.md.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::util::now_ms;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::App;
use crate::model::*;
use crate::sessions::{LiveSession, Sink, StartSpec, json_line, shorten, text_of, truncate_output};

use super::permission_options;

fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::Yolo => vec!["--dangerously-skip-permissions".into()],
        PermissionMode::Approvals => vec!["--permission-mode".into(), "default".into()],
        PermissionMode::Auto => vec!["--permission-mode".into(), "auto".into()],
    }
}

fn claude_permission_response(input: &Value, answers: &[String]) -> Result<Value> {
    match answers.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["Allow once"] => Ok(json!({"behavior":"allow", "updatedInput":input})),
        ["Deny"] => Ok(json!({"behavior":"deny", "message":"The owner denied this operation."})),
        _ => bail!("Choose Allow once or Deny on the approval card"),
    }
}

pub struct ClaudeSession {
    sink: Sink,
    stdin: tokio::sync::Mutex<Option<crate::exec::Stdin>>,
    state: Arc<Mutex<State>>,
    killer: crate::exec::Killer,
    exited: tokio::sync::watch::Receiver<bool>,
}

#[derive(Default)]
struct State {
    /// `tool_use_id` → (name, input), to build file changes and background tasks from results.
    tools: HashMap<String, (String, Value)>,
    children: HashMap<String, Arc<Mutex<State>>>,
    task_agents: HashMap<String, String>,
    active_agents: HashSet<String>,
    /// Running background tasks.
    background: HashSet<String>,
    /// Every background task started in this process (`task_notification` can come after the running
    /// list is already empty).
    background_known: HashSet<String>,
    /// Pending native questions or tagged permission inputs, keyed by request ID.
    questions: HashMap<String, Value>,
    context_window: Option<u64>,
    /// Id of the assistant message being streamed (from `message_start`).
    streaming_msg: String,
    /// Content blocks seen per assistant message: Claude sends one `assistant` event per block, so this
    /// gives the block index used by the stream events.
    msg_blocks: HashMap<String, usize>,
    stopping: bool,
    turn_running: bool,
    /// Native synthetic limit message, retained until the terminal result arrives.
    quota_rejection: Option<String>,
    cwd: String,
    /// Cost this process has reported so far. `total_cost_usd` counts from its own start, so only
    /// the increase is added to the saved session total (which other writers, such as the title
    /// request, may also have raised).
    cost_reported: f64,
    permission_mode: PermissionMode,
    auto_unavailable_notified: bool,
}

pub async fn start(app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
    sink.update_usage(|_| {}).await;
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--replay-user-messages",
        "--permission-prompt-tool",
        "stdio",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if spec.browser_tools {
        let bridge = crate::browser::mcp_config(app, &spec);
        let mcp = json!({"command":bridge["command"],"args":bridge["args"],"env":bridge["env"]});
        let config = json!({"mcpServers":{"launchpad_browser":mcp}}).to_string();
        // The broker checks grants and owner reviews for every call, so its tools need no native approval.
        if spec.browser {
            args.extend(["--strict-mcp-config".into(), "--mcp-config".into(), config, "--tools".into(), "".into()]);
        } else {
            args.extend(["--mcp-config".into(), config]);
        }
        args.extend(["--allowedTools".into(), "mcp__launchpad_browser__*".into()]);
    }
    args.extend(permission_args(spec.permission_mode));
    let harness_id = spec.harness_session_id.clone().unwrap_or_else(|| spec.session_id.clone());
    if spec.resume {
        args.extend(["--resume".into(), harness_id.clone()]);
    } else {
        args.extend(["--session-id".into(), harness_id.clone(), "--name".into(), spec.title.clone()]);
    }
    if let Some(m) = &spec.model {
        args.extend(["--model".into(), m.clone()]);
    }
    if let Some(e) = &spec.effort {
        args.extend(["--effort".into(), e.clone()]);
    }
    for d in &spec.add_dirs {
        args.extend(["--add-dir".into(), d.clone()]);
    }
    let mut cmd = crate::procs::child_command(&app.env.on(spec.host.as_deref()), "claude", Some(&spec.cwd));
    cmd.process_tree().args(&args)
        .envs(spec.env.iter().cloned())
        .env("LAUNCHPAD_ENV", app.paths.app_env.name())
        .env("LAUNCHPAD_SESSION_ID", &spec.session_id)
        .stdin(crate::exec::Io::Piped)
        .stdout(crate::exec::Io::Piped)
        .stderr(crate::exec::Io::Piped);
    if spec.browser_tools { cmd.env("MCP_TOOL_TIMEOUT", "720000"); }
    let mut child = cmd.spawn().await.context("start claude")?;
    sink.raw("cmd", &format!("claude {}", args.join(" "))).await;
    sink.set_harness_id(&harness_id).await;
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // `total_cost_usd` counts from the start of each process: add only its increase to the saved total.
    let state = Arc::new(Mutex::new(State { cwd: spec.cwd.to_string_lossy().to_string(), permission_mode: spec.permission_mode, ..Default::default() }));
    let (exited, exit_wait) = tokio::sync::watch::channel(false);
    let session = Arc::new(ClaudeSession { sink: sink.clone(), stdin: tokio::sync::Mutex::new(Some(stdin)), state: state.clone(), killer: child.killer(), exited: exit_wait });

    let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let tail = stderr_tail.clone();
    let err_sink = sink.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            err_sink.raw("err", &line).await;
            let mut t = tail.lock().unwrap();
            t.push(line);
            if t.len() > 20 {
                t.remove(0);
            }
        }
    });

    let out_sink = sink.clone();
    let st = state.clone();
    let writer = session.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let read = async {
            while let Ok(Some(line)) = lines.next_line().await {
                out_sink.raw("out", &line).await;
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                handle(&out_sink, &st, &writer, &msg).await;
            }
        };
        tokio::pin!(read);
        let status = tokio::select! {
            _ = &mut read => child.wait().await,
            status = child.wait() => {
                // Preserve final protocol events, but do not let an escaped pipe owner block exit.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), &mut read).await;
                status
            }
        };
        // Wake stop before emitting events: cleanup waits for this process, not event handling.
        let _ = exited.send(true);
        let stopping = st.lock().unwrap().stopping;
        let error = match status {
            Ok(s) if s.success() || stopping => None,
            Ok(s) => Some(format!("claude exited ({s}): {}", stderr_tail.lock().unwrap().join("\n"))),
            // The host lost the process: Interrupted, resumable on the host.
            Err(e) if crate::exec::is_lost(&e) => {
                out_sink.emit(EventBody::Notice { text: format!("{e}") }).await;
                None
            }
            Err(e) => Some(format!("claude: {e}")),
        };
        cancel_questions(&out_sink, &st).await;
        out_sink.exited(error).await;
    });
    Ok(session)
}

impl ClaudeSession {
    async fn write(&self, msg: &Value) -> Result<()> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().context("the session process is closed")?;
        stdin.write_all(json_line(msg).as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl LiveSession for ClaudeSession {
    async fn send(&self, text: &str, client_id: &str, images: &[ImageAttachment]) -> Result<()> {
        let mut content = vec![];
        if !text.is_empty() { content.push(json!({"type": "text", "text": text})); }
        for image in images {
            let (header, data) = image.data_url.split_once(",").context("invalid image data URL")?;
            let media_type = header.strip_prefix("data:").and_then(|h| h.strip_suffix(";base64")).context("invalid image media type")?;
            content.push(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}}));
        }
        {
            let mut state = self.state.lock().unwrap();
            state.turn_running = true;
            state.quota_rejection = None;
        }
        self.write(&json!({
            "type": "user",
            "uuid": client_id,
            "message": {"role": "user", "content": content},
        }))
        .await
    }

    fn turn_active(&self) -> bool { let state = self.state.lock().unwrap(); state.turn_running || agents_busy(&state) }

    fn has_background_work(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.background.is_empty() || state.children.values().any(|child| !child.lock().unwrap().background.is_empty())
    }

    fn ready_for_turn(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.turn_running && s.background.is_empty() && s.questions.is_empty() && !agents_busy(&s)
    }

    async fn interrupt(&self) -> Result<()> {
        let id = format!("interrupt-{}", uuid::Uuid::new_v4());
        self.write(&json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}})).await?;
        cancel_questions(&self.sink, &self.state).await;
        Ok(())
    }

    fn has_open_questions(&self) -> bool { !self.state.lock().unwrap().questions.is_empty() }
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        let input = self.state.lock().unwrap().questions.get(request_id).cloned().context("this question was already answered")?;
        if input["launchpad_permission"] == true {
            let response = claude_permission_response(&input["input"], &answers)?;
            self.write(&json!({"type":"control_response", "response":{"subtype":"success", "request_id":request_id, "response":response}})).await?;
            self.state.lock().unwrap().questions.remove(request_id);
            return Ok(());
        }
        self.state.lock().unwrap().questions.remove(request_id);
        let questions = input["questions"].as_array().cloned().unwrap_or_default();
        let mut map = serde_json::Map::new();
        for (q, a) in questions.iter().zip(answers) {
            if let Some(text) = q["question"].as_str() {
                map.insert(text.to_string(), Value::String(a));
            }
        }
        self.write(&json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {"behavior": "allow", "updatedInput": {"questions": questions, "answers": map}},
            },
        }))
        .await
    }

    async fn stop(&self) -> Result<()> {
        self.state.lock().unwrap().stopping = true;
        cancel_questions(&self.sink, &self.state).await;
        // Closing stdin ends the process after the current step; kill it if it does not exit.
        let mut exited = self.exited.clone();
        let wait = async {
            self.stdin.lock().await.take();
            let _ = exited.wait_for(|done| *done).await;
        };
        if tokio::time::timeout(std::time::Duration::from_secs(5), wait).await.is_err() {
            if !*exited.borrow() { self.killer.kill().await; }
            let _ = exited.wait_for(|done| *done).await;
        }
        Ok(())
    }
}

async fn cancel_questions(sink: &Sink, st: &Mutex<State>) {
    let pending = st.lock().unwrap().questions.drain().map(|(id, _)| id).collect::<Vec<_>>();
    if !pending.is_empty() {
        for request_id in pending { sink.emit(EventBody::AskAnswered { request_id, answers: vec!["Request ended".into()] }).await; }
        sink.set_open_question(false).await;
    }
}

/// Maps one stdout message to normalized events.
async fn handle(sink: &Sink, st: &Arc<Mutex<State>>, writer: &Arc<ClaudeSession>, msg: &Value) {
    if matches!(msg["type"].as_str(), Some("control_request" | "control_cancel_request")) || msg["type"] == "system" && matches!(msg["subtype"].as_str(), Some("task_started" | "task_notification" | "task_updated" | "background_tasks_changed")) {
        // Native permission/question responses are owned by the one Claude process.
        handle_message(sink, st, writer, msg).await;
    } else if let Some(id) = msg["parent_tool_use_id"].as_str() {
        let child_state = {
            let mut state = st.lock().unwrap();
            let cwd = state.cwd.clone();
            state.children.entry(id.into()).or_insert_with(|| Arc::new(Mutex::new(State { cwd, ..Default::default() }))).clone()
        };
        handle_message(&sink.child(id), &child_state, writer, msg).await;
    } else {
        handle_message(sink, st, writer, msg).await;
    }
    let (idle, busy) = { let state = st.lock().unwrap(); (!state.turn_running, !state.background.is_empty() || agents_busy(&state)) };
    if idle && matches!(sink.status(), Some(SessionStatus::Waiting | SessionStatus::Background)) {
        sink.set_status(if busy { SessionStatus::Background } else if sink.stop_requested() { SessionStatus::Stopped } else { SessionStatus::Waiting }).await;
    }
}
fn agents_busy(state: &State) -> bool {
    !state.active_agents.is_empty() || state.children.values().any(|child| !child.lock().unwrap().active_agents.is_empty())
}
fn agent_owner(state: &State, id: &str) -> Option<Option<String>> {
    if state.active_agents.contains(id) { return Some(None); }
    state.children.iter().find(|(_, child)| child.lock().unwrap().active_agents.contains(id)).map(|(parent, _)| Some(parent.clone()))
}
fn finish_agent(state: &mut State, id: &str) {
    state.active_agents.remove(id);
    for child in state.children.values() { child.lock().unwrap().active_agents.remove(id); }
}
fn unknown_model_cost(msg: &Value) -> bool {
    msg["hasUnknownModelCost"] == true
        || msg["message"]["model"].as_str().or_else(|| msg["event"]["message"]["model"].as_str()).is_some_and(|m| m != "<synthetic>" && !REGISTRATION.descriptor.supports_cost(Some(m)))
        || msg["modelUsage"].as_object().is_some_and(|models| models.keys().any(|m| m != "<synthetic>" && !REGISTRATION.descriptor.supports_cost(Some(m))))
}

fn result_usage(us: &mut Usage, msg: &Value, cost_delta: Option<f64>) {
    let u = &msg["usage"];
    us.input_tokens += u["input_tokens"].as_u64().unwrap_or(0) + u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    us.cached_tokens += u["cache_read_input_tokens"].as_u64().unwrap_or(0);
    us.output_tokens += u["output_tokens"].as_u64().unwrap_or(0);
    us.cost_unavailable |= unknown_model_cost(msg);
    if us.cost_unavailable {
        us.cost_usd = None;
    } else if let Some(c) = cost_delta.filter(|c| c.is_finite() && *c > 0.0) {
        us.cost_usd = Some(us.cost_usd.unwrap_or(0.0) + c);
    }
}

fn quota_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    ["you've hit your limit", "you've hit your usage limit", "you've hit your weekly limit",
        "you've hit your individual spend limit", "you've hit your organization spend limit",
        "usage limit reached", "quota exceeded", "credit balance is too low",
        "rate limit exceeded", "rate limit reached"]
        .iter().any(|pattern| message.contains(pattern))
}

async fn handle_message(sink: &Sink, st: &Arc<Mutex<State>>, writer: &Arc<ClaudeSession>, msg: &Value) {
    if unknown_model_cost(msg) { sink.root().update_usage(|u| { u.cost_unavailable = true; u.cost_usd = None; }).await; }
    match msg["type"].as_str().unwrap_or("") {
        "system" => handle_system(sink, st, msg).await,
        "stream_event" => {
            let ev = &msg["event"];
            if ev["type"] == "message_start" {
                st.lock().unwrap().streaming_msg = ev["message"]["id"].as_str().unwrap_or("").to_string();
            } else if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                // Same id as the final `assistant` text block, so the UI swaps the live text for it.
                let item_id = format!("{}:{}", st.lock().unwrap().streaming_msg, ev["index"]);
                sink.emit(EventBody::AssistantDelta { item_id, text: ev["delta"]["text"].as_str().unwrap_or("").into() }).await;
            }
        }
        "assistant" => {
            let m = &msg["message"];
            if matches!(msg["error"].as_str(), Some("rate_limit" | "billing_error")) {
                let text = text_of(&m["content"]);
                st.lock().unwrap().quota_rejection = Some(text);
            }
            let msg_id = m["id"].as_str().unwrap_or("");
            if let Some(model) = m["model"].as_str() {
                sink.set_model(model).await;
            }
            for block in m["content"].as_array().cloned().unwrap_or_default().iter() {
                let i = {
                    let mut s = st.lock().unwrap();
                    let n = s.msg_blocks.entry(msg_id.to_string()).or_insert(0);
                    *n += 1;
                    *n - 1
                };
                match block["type"].as_str().unwrap_or("") {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or("").to_string();
                        if !text.trim().is_empty() {
                            sink.emit(EventBody::AssistantMessage { item_id: format!("{msg_id}:{i}"), text }).await;
                        }
                    }
                    "thinking" => {
                        let text = block["thinking"].as_str().unwrap_or("");
                        if !text.trim().is_empty() {
                            sink.emit(EventBody::Reasoning { item_id: format!("{msg_id}:{i}"), text: text.into() }).await;
                        }
                    }
                    "tool_use" => tool_use(sink, st, block).await,
                    _ => {}
                }
            }
            let u = &m["usage"];
            if u.is_object() {
                let ctx = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum::<u64>();
                if ctx > 0 {
                    let window = st.lock().unwrap().context_window;
                    sink.update_usage(|us| {
                        us.context_tokens = Some(ctx);
                        if us.context_window.is_none() {
                            us.context_window = window;
                        }
                    })
                    .await;
                }
            }
        }
        "user" => {
            if msg["isReplay"].as_bool() == Some(true) {
                if let Some(uuid) = msg["uuid"].as_str() {
                    sink.emit(EventBody::MessageDelivered { client_id: uuid.into() }).await;
                }
                return;
            }
            let content = msg["message"]["content"].as_array().cloned().unwrap_or_default();
            for block in &content {
                if block["type"] == "tool_result" {
                    tool_result(sink, st, block, &msg["tool_use_result"]).await;
                }
            }
        }
        "result" => {
            st.lock().unwrap().turn_running = false;
            cancel_questions(sink, st).await;
            let error = if msg["is_error"].as_bool() == Some(true) && msg["subtype"] != "error_during_execution" {
                Some(msg["result"].as_str().unwrap_or(msg["subtype"].as_str().unwrap_or("error")).to_string())
            } else {
                None
            };
            let rejection = st.lock().unwrap().quota_rejection.take()
                .or_else(|| error.as_ref().filter(|e| quota_error(e)).cloned());
            let error = if rejection.is_some() { None } else { error };
            let interrupted = rejection.is_some() || msg["subtype"] == "error_during_execution";
            sink.emit(EventBody::TurnEnd { duration_ms: msg["duration_ms"].as_i64(), interrupted, error: error.clone() }).await;
            // Only the increase since this process last reported is new spending.
            let reported = msg["total_cost_usd"].as_f64().filter(|c| c.is_finite() && *c >= 0.0);
            let cost_delta = reported.map(|c| {
                let mut st = st.lock().unwrap();
                let delta = c - st.cost_reported;
                st.cost_reported = c;
                delta
            });
            let window = msg["modelUsage"].as_object().and_then(|m| m.values().filter_map(|v| v["contextWindow"].as_u64()).max());
            if let Some(w) = window {
                st.lock().unwrap().context_window = Some(w);
            }
            sink.update_usage(|us| {
                result_usage(us, msg, cost_delta);
                if window.is_some() {
                    us.context_window = window;
                }
            })
            .await;
            let background = { let state = st.lock().unwrap(); !state.background.is_empty() || agents_busy(&state) };
            if !st.lock().unwrap().questions.is_empty() {
                return;
            }
            if let Some(message) = rejection {
                sink.quota_rejected(&message).await;
                return;
            }
            sink.set_status(if error.is_some() { SessionStatus::Error }
                else if sink.stop_requested() { SessionStatus::Stopped }
                else if interrupted { SessionStatus::Interrupted }
                else if background { SessionStatus::Background } else { SessionStatus::Waiting }).await;
        }
        "rate_limit_event" => if let Some(account) = sink.quota_account() { rate_limit_event(&sink.app().status, &account, &msg["rate_limit_info"]) },
        "control_cancel_request" => {
            let request_id = msg["request_id"].as_str().unwrap_or("");
            let removed = st.lock().unwrap().questions.remove(request_id).is_some();
            if removed {
                sink.emit(EventBody::AskAnswered { request_id: request_id.into(), answers: vec!["Request ended".into()] }).await;
                let (open, active) = { let state = st.lock().unwrap(); (!state.questions.is_empty(), state.turn_running) };
                sink.set_open_question(open).await;
                if !open && active { sink.set_status(SessionStatus::Working).await; }
            }
        }
        "control_request" => {
            let req = &msg["request"];
            let request_id = msg["request_id"].as_str().unwrap_or("").to_string();
            if req["subtype"] == "can_use_tool" && req["tool_name"] == "AskUserQuestion" {
                let input = req["input"].clone();
                st.lock().unwrap().questions.insert(request_id.clone(), input.clone());
                let questions = input["questions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .map(|(i, q)| Question {
                        permission: false,
                        id: i.to_string(),
                        header: q["header"].as_str().unwrap_or("").into(),
                        question: q["question"].as_str().unwrap_or("").into(),
                        options: q["options"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .map(|o| QuestionOption {
                                label: o["label"].as_str().unwrap_or("").into(),
                                description: o["description"].as_str().unwrap_or("").into(),
                            })
                            .collect(),
                        multi_select: q["multiSelect"].as_bool().unwrap_or(false),
                    })
                    .collect();
                sink.emit(EventBody::AskUser { request_id, questions }).await;
                sink.set_open_question(true).await;
                sink.set_status(SessionStatus::Waiting).await;
            } else if req["subtype"] == "can_use_tool" && st.lock().unwrap().permission_mode == PermissionMode::Yolo {
                let _ = writer.write(&json!({"type":"control_response", "response":{"subtype":"success", "request_id":request_id, "response":{"behavior":"allow", "updatedInput":req["input"]}}})).await;
            } else if req["subtype"] == "can_use_tool" {
                // The native permission engine decides which tools need an owner decision.
                st.lock().unwrap().questions.insert(request_id.clone(), json!({"launchpad_permission":true,"input":req["input"]}));
                sink.emit(EventBody::AskUser { request_id: request_id.clone(), questions: vec![Question {
                    permission: true, id: request_id, header: "Approval required".into(),
                    question: format!("{}\n{}", req["tool_name"].as_str().unwrap_or("Tool"), serde_json::to_string_pretty(&req["input"]).unwrap_or_default()),
                    options: permission_options(), multi_select: false,
                }] }).await;
                sink.set_open_question(true).await;
                sink.set_status(SessionStatus::Waiting).await;
            } else {
                let _ = writer
                    .write(&json!({"type": "control_response", "response": {"subtype": "error", "request_id": request_id, "error": "not supported by Launchpad"}}))
                    .await;
            }
        }
        _ => {}
    }
}

async fn handle_system(sink: &Sink, st: &Arc<Mutex<State>>, msg: &Value) {
    match msg["subtype"].as_str().unwrap_or("") {
        // Sent at the start of every turn, including turns Claude starts by itself after a background task.
        "init" => {
            st.lock().unwrap().turn_running = true;
            let unavailable = {
                let mut state = st.lock().unwrap();
                let unavailable = state.permission_mode == PermissionMode::Auto && !state.auto_unavailable_notified
                    && msg["permissionMode"].as_str().is_some_and(|mode| mode != "auto");
                state.auto_unavailable_notified |= unavailable;
                unavailable
            };
            if unavailable {
                sink.emit(EventBody::Notice { text: format!("Claude could not enable AI auto approval (native mode: {}). Check model/account support and managed policy. Remaining permission requests need your decision here.", msg["permissionMode"].as_str().unwrap_or("unknown")) }).await;
            }
            if let Some(model) = msg["model"].as_str() {
                sink.set_model(model).await;
            }
            if sink.status() != Some(SessionStatus::Working) && st.lock().unwrap().questions.is_empty() {
                sink.set_status(SessionStatus::Working).await;
            }
        }
        "task_started" => {
            // Agent tasks include their launching tool ID; shell background tasks stay separate.
            if let Some(id) = msg["tool_use_id"].as_str() {
                let owner = agent_owner(&st.lock().unwrap(), id);
                if let Some(parent) = owner {
                    if let Some(task) = msg["task_id"].as_str() { st.lock().unwrap().task_agents.insert(task.into(), id.into()); }
                    let sink = parent.as_deref().map(|parent| sink.child(parent)).unwrap_or_else(|| sink.clone());
                    sink.subagent(id, msg["description"].as_str().map(str::to_string), Some("running".into()), None).await;
                }
            }
            if msg["is_backgrounded"].as_bool() != Some(true) { return; }
            let task_id = msg["task_id"].as_str().unwrap_or("").to_string();
            {
                let mut s = st.lock().unwrap();
                s.background.insert(task_id.clone());
                s.background_known.insert(task_id.clone());
            }
            sink.emit(EventBody::BackgroundTask {
                task_id,
                description: msg["description"].as_str().unwrap_or("").into(),
                status: "running".into(),
                output_file: None,
            })
            .await;
        }
        "task_updated" => {
            let task = msg["task_id"].as_str().unwrap_or("");
            let status = msg["patch"]["status"].as_str().or(msg["status"].as_str());
            if let Some(status) = status {
                let id = st.lock().unwrap().task_agents.get(task).cloned();
                if let Some(id) = id {
                    sink.subagent(&id, None, Some(if status == "killed" { "stopped" } else if status == "pending" { "running" } else { status }.into()), None).await;
                    if matches!(status, "completed" | "failed" | "killed" | "stopped") { finish_agent(&mut st.lock().unwrap(), &id); }
                }
                if matches!(status, "completed" | "failed" | "killed" | "stopped") {
                    let was_background = st.lock().unwrap().background.remove(task);
                    if was_background { sink.emit(EventBody::BackgroundTask { task_id: task.into(), description: String::new(), status: if status == "killed" { "stopped" } else { status }.into(), output_file: None }).await; }
                    let idle = { let state = st.lock().unwrap(); state.background.is_empty() && !state.turn_running };
                    if idle && sink.status() == Some(SessionStatus::Background) { sink.set_status(SessionStatus::Waiting).await; }
                }
            }
        }
        "task_notification" => {
            let tracked = st.lock().unwrap().task_agents.get(msg["task_id"].as_str().unwrap_or("")).cloned();
            let agent = tracked.or_else(|| msg["tool_use_id"].as_str().filter(|id| agent_owner(&st.lock().unwrap(), id).is_some()).map(str::to_string));
            if let Some(id) = agent {
                sink.subagent(&id, None, Some(msg["status"].as_str().unwrap_or("completed").into()), None).await;
                finish_agent(&mut st.lock().unwrap(), &id);
            }
            let task_id = msg["task_id"].as_str().unwrap_or("").to_string();
            let was_background = {
                let mut s = st.lock().unwrap();
                s.background.remove(&task_id);
                s.background_known.contains(&task_id)
            };
            if was_background {
                sink.emit(EventBody::BackgroundTask {
                    task_id,
                    description: msg["summary"].as_str().unwrap_or("").into(),
                    status: msg["status"].as_str().unwrap_or("completed").into(),
                    output_file: msg["output_file"].as_str().filter(|f| !f.is_empty()).map(str::to_string),
                })
                .await;
            }
        }
        "background_tasks_changed" => {
            let ids: HashSet<String> =
                msg["tasks"].as_array().cloned().unwrap_or_default().iter().filter_map(|t| t["task_id"].as_str().map(str::to_string)).collect();
            let empty = ids.is_empty();
            st.lock().unwrap().background = ids;
            if empty && sink.status() == Some(SessionStatus::Background) {
                sink.set_status(SessionStatus::Waiting).await;
            }
        }
        _ => {}
    }
}

fn summary(name: &str, input: &Value) -> String {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    match name {
        "Bash" => shorten(&s("command"), 160),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => s("file_path"),
        "Glob" => s("pattern"),
        "Grep" => format!("{} {}", s("pattern"), s("path")).trim().to_string(),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        "Task" | "Agent" => s("description"),
        "TodoWrite" => "Update the plan".into(),
        "Skill" => s("skill"),
        "ToolSearch" => s("query"),
        _ => shorten(&input.to_string(), 120),
    }
}

async fn tool_use(sink: &Sink, st: &Arc<Mutex<State>>, block: &Value) {
    let name = block["name"].as_str().unwrap_or("").to_string();
    let id = block["id"].as_str().unwrap_or("").to_string();
    let input = block["input"].clone();
    st.lock().unwrap().tools.insert(id.clone(), (name.clone(), input.clone()));
    if name == "Agent" || name == "Task" {
        st.lock().unwrap().active_agents.insert(id.clone());
        sink.subagent(&id, Some(summary(&name, &input)), Some("running".into()), input["model"].as_str().map(str::to_string)).await;
        if let Some(prompt) = input["prompt"].as_str() {
            sink.child(&id).emit(EventBody::UserMessage { text: prompt.into(), client_id: id.clone(), queued: false, images: vec![] }).await;
        }
    }
    if name == "TodoWrite" {
        let items = input["todos"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|t| PlanItem { text: t["content"].as_str().unwrap_or("").into(), status: t["status"].as_str().unwrap_or("pending").into() })
            .collect();
        sink.emit(EventBody::Plan { items }).await;
        return;
    }
    if name == "AskUserQuestion" {
        // Shown as a question card (control request), not as a tool call.
        return;
    }
    sink.emit(EventBody::ToolCall { item_id: id, summary: summary(&name, &input), name, input }).await;
}

async fn tool_result(sink: &Sink, st: &Arc<Mutex<State>>, block: &Value, extra: &Value) {
    let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
    let tool = st.lock().unwrap().tools.remove(&id);
    let Some((name, input)) = tool else { return };
    if name == "TodoWrite" || name == "AskUserQuestion" {
        return;
    }
    let output = text_of(&block["content"]);
    let is_error = block["is_error"].as_bool().unwrap_or(false);
    if (name == "Agent" || name == "Task") && extra["isAsync"].as_bool() != Some(true) && input["run_in_background"].as_bool() != Some(true) {
        st.lock().unwrap().active_agents.remove(&id);
        sink.subagent(&id, None, Some(if is_error { "failed" } else { "completed" }.into()), None).await;
        // Keep the result accessible even when the harness does not stream the child's messages.
        let child = sink.child(&id);
        child.emit(EventBody::ToolCall { item_id: id.clone(), name: "Agent".into(), summary: "Delegation result".into(), input: input.clone() }).await;
        child.emit(EventBody::ToolResult { item_id: id.clone(), output: truncate_output(&output, 30_000), is_error, exit_code: None, duration_ms: extra["durationMs"].as_i64() }).await;
    }
    let cwd = st.lock().unwrap().cwd.clone();
    if !is_error
        && let Some(fc) = file_change(&name, &input, &id, &cwd) {
        sink.emit(fc).await;
    }
    // Output file of a background command, for its live view.
    if name == "Bash" && input["run_in_background"].as_bool() == Some(true)
        && let (Some(task), Some(file)) = (extra["backgroundTaskId"].as_str(), output.split("Output is being written to: ").nth(1)) {
        sink.emit(EventBody::BackgroundTask {
            task_id: task.to_string(),
            description: summary(&name, &input),
            status: "running".into(),
            output_file: Some(file.split_whitespace().next().unwrap_or("").trim_end_matches('.').to_string()),
        })
        .await;
    }
    sink.emit(EventBody::ToolResult {
        item_id: id,
        output: truncate_output(&output, 30_000),
        is_error,
        exit_code: extra["exitCode"].as_i64().map(|c| c as i32),
        duration_ms: extra["durationMs"].as_i64(),
    })
    .await;
}

fn relative(path: &str, cwd: &str) -> String {
    path.strip_prefix(cwd).map(|p| p.trim_start_matches('/').to_string()).unwrap_or_else(|| path.to_string())
}

fn file_change(name: &str, input: &Value, id: &str, cwd: &str) -> Option<EventBody> {
    let path = relative(input["file_path"].as_str()?, cwd);
    let lines = |prefix: char, s: &str| s.lines().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n");
    let (change, diff) = match name {
        "Edit" => (
            "update",
            format!("{}\n{}", lines('-', input["old_string"].as_str().unwrap_or("")), lines('+', input["new_string"].as_str().unwrap_or(""))),
        ),
        "MultiEdit" => (
            "update",
            input["edits"]
                .as_array()?
                .iter()
                .map(|e| format!("{}\n{}", lines('-', e["old_string"].as_str().unwrap_or("")), lines('+', e["new_string"].as_str().unwrap_or(""))))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "Write" => ("add", lines('+', input["content"].as_str().unwrap_or(""))),
        _ => return None,
    };
    Some(EventBody::FileChange { item_id: id.to_string(), path, change: change.into(), diff })
}

/// Models offered by Claude Code for this account, from the `initialize` control request.
pub async fn models(app: &App) -> Result<Vec<ModelInfo>> {
    let mut child = crate::procs::child_command(&app.env, "claude", None)
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence"])
        .stdin(crate::exec::Io::Piped)
        .stdout(crate::exec::Io::Piped)
        .stderr(crate::exec::Io::Null)
        .spawn()
        .await?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    stdin
        .write_all(json_line(&json!({"type": "control_request", "request_id": "models", "request": {"subtype": "initialize", "hooks": null}})).as_bytes())
        .await?;
    stdin.flush().await?;
    let mut lines = BufReader::new(stdout).lines();
    let read = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v["type"] == "control_response" {
                return Some(v);
            }
        }
        None
    };
    let res = tokio::time::timeout(std::time::Duration::from_secs(15), read).await.ok().flatten();
    drop(stdin);
    let _ = child.kill().await;
    let res = res.context("no answer from claude")?;
    Ok(res["response"]["response"]["models"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["value"].as_str()?.to_string(),
                display_name: m["displayName"].as_str().unwrap_or("").to_string(),
                description: m["description"].as_str().unwrap_or("").to_string(),
                efforts: m["supportedEffortLevels"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                providers: vec![],
            })
        })
        .filter(|m| m.id != "default")
        .collect())
}

/// Model and effort Claude Code uses when none is given (`~/.claude/settings.json`).
pub fn defaults() -> (Option<String>, Option<String>) {
    let path = dirs::home_dir().unwrap_or_default().join(".claude/settings.json");
    let v: Value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    (v["model"].as_str().map(str::to_string), v["effortLevel"].as_str().map(str::to_string))
}

pub const ID: HarnessKind = HarnessKind::new("claude");

const fn var(name: &'static str, group: &'static str, description: &'static str, default: Option<&'static str>, example: Option<&'static str>, secret: bool) -> super::EnvVarDoc {
    super::EnvVarDoc { name, group, description, default, example, secret }
}

/// From https://code.claude.com/docs/en/env-vars and the model configuration page (October 2026).
const ENV_VARS: &[super::EnvVarDoc] = &[
    var("CLAUDE_CONFIG_DIR", "Account", "Configuration folder with its own sign-in, settings and history. Use one folder per account; sign in once with: CLAUDE_CONFIG_DIR=<folder> claude", Some("~/.claude"), None, false),
    var("ANTHROPIC_API_KEY", "Account", "Anthropic Console API key, sent as X-Api-Key. Replaces the subscription sign-in and is billed per use.", None, Some("sk-ant-…"), true),
    var("ANTHROPIC_AUTH_TOKEN", "Account", "Bearer token for a gateway or proxy (Authorization header). Takes precedence over ANTHROPIC_API_KEY.", None, Some("${HF_TOKEN}"), true),
    var("CLAUDE_CODE_OAUTH_TOKEN", "Account", "Subscription token created with: claude setup-token", None, Some("sk-ant-oat01-…"), true),
    var("ANTHROPIC_BASE_URL", "Endpoint", "API endpoint of a gateway or compatible provider. Claude Code appends /v1/messages.", Some("https://api.anthropic.com"), None, false),
    var("ANTHROPIC_CUSTOM_HEADERS", "Endpoint", "Extra request headers, one Name: Value per line.", None, Some("X-HF-Bill-To: my-org"), false),
    var("HTTPS_PROXY", "Endpoint", "HTTPS proxy for API requests.", None, Some("http://proxy.example.com:8080"), false),
    var("NO_PROXY", "Endpoint", "Hosts that bypass the proxy.", None, Some("localhost,127.0.0.1"), false),
    var("CLAUDE_CODE_USE_BEDROCK", "Cloud provider", "Use Amazon Bedrock, with the AWS credentials of the environment.", None, Some("1"), false),
    var("AWS_REGION", "Cloud provider", "Bedrock region, when the AWS profile has none.", None, Some("us-east-1"), false),
    var("AWS_PROFILE", "Cloud provider", "AWS profile used for Bedrock.", None, Some("default"), false),
    var("AWS_BEARER_TOKEN_BEDROCK", "Cloud provider", "Bedrock API key.", None, None, true),
    var("CLAUDE_CODE_USE_VERTEX", "Cloud provider", "Use Google Vertex AI, with the gcloud credentials of the environment.", None, Some("1"), false),
    var("ANTHROPIC_VERTEX_PROJECT_ID", "Cloud provider", "Google Cloud project for Vertex AI.", None, Some("my-project"), false),
    var("CLOUD_ML_REGION", "Cloud provider", "Vertex AI region.", None, Some("global"), false),
    var("CLAUDE_CODE_USE_FOUNDRY", "Cloud provider", "Use Microsoft Foundry.", None, Some("1"), false),
    var("ANTHROPIC_FOUNDRY_RESOURCE", "Cloud provider", "Foundry resource name.", None, Some("my-resource"), false),
    var("ANTHROPIC_FOUNDRY_API_KEY", "Cloud provider", "Foundry API key.", None, None, true),
    var("ANTHROPIC_MODEL", "Models", "Model of the session. The model chosen in Launchpad (--model) takes precedence.", None, Some("claude-sonnet-5-5"), false),
    var("ANTHROPIC_DEFAULT_OPUS_MODEL", "Models", "Model used for the opus alias. ${LAUNCHPAD_MODEL} is the model chosen in Launchpad.", Some("latest Opus"), Some("${LAUNCHPAD_MODEL}"), false),
    var("ANTHROPIC_DEFAULT_SONNET_MODEL", "Models", "Model used for the sonnet alias.", Some("latest Sonnet"), Some("${LAUNCHPAD_MODEL}"), false),
    var("ANTHROPIC_DEFAULT_HAIKU_MODEL", "Models", "Model used for the haiku alias and for background tasks.", Some("latest Haiku"), Some("${LAUNCHPAD_MODEL}"), false),
    var("ANTHROPIC_DEFAULT_FABLE_MODEL", "Models", "Model used for the fable alias.", Some("latest Fable"), Some("${LAUNCHPAD_MODEL}"), false),
    var("CLAUDE_CODE_SUBAGENT_MODEL", "Models", "Default model of subagents.", Some("inherits the session model"), Some("${LAUNCHPAD_MODEL}"), false),
    var("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "Models", "Context window assumed for model IDs Claude Code does not know.", Some("200000"), None, false),
    var("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "Limits", "Maximum output tokens per response.", Some("32000 (model dependent)"), None, false),
    var("MAX_THINKING_TOKENS", "Limits", "Fixed thinking budget; 0 disables thinking on most models.", None, Some("8000"), false),
    var("API_TIMEOUT_MS", "Limits", "Timeout of one API request, in milliseconds.", Some("600000"), None, false),
    var("CLAUDE_CODE_MAX_RETRIES", "Limits", "Retries of a failed API request.", Some("10"), None, false),
    var("CLAUDE_CODE_DISABLE_THINKING", "Compatibility", "Omit the thinking parameter, for gateways that reject it.", None, Some("1"), false),
    var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "Compatibility", "Remove beta headers and fields, for gateways that reject them.", None, Some("1"), false),
    var("DISABLE_PROMPT_CACHING", "Compatibility", "Turn off prompt caching.", None, Some("1"), false),
    var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "Privacy", "Any value turns off auto-updates, telemetry, error reporting and feedback.", None, Some("1"), false),
    var("DISABLE_TELEMETRY", "Privacy", "Any value, even 0, turns off telemetry.", None, Some("1"), false),
    var("DISABLE_ERROR_REPORTING", "Privacy", "Any value, even 0, turns off error reporting.", None, Some("1"), false),
];

const PROFILE_TEMPLATES: &[super::ProfileTemplate] = &[
    super::ProfileTemplate {
        kind: super::ProfileKind::Subscription,
        name: "Subscription",
        description: "Your Claude Pro or Max login (claude auth login), protected by its 5-hour and weekly quota.",
        setup: None,
        setup_command: None,
        env: &[],
        billing: crate::config::Billing::Subscription,
        models_url: None,
    },
    super::ProfileTemplate {
        kind: super::ProfileKind::Subscription,
        name: "Another account",
        description: "A second subscription or organization, with its own sign-in in a separate configuration folder and its own quota meters.",
        setup: Some("Save the profile, then sign in once from Works on."),
        setup_command: None,
        env: &[("CLAUDE_CONFIG_DIR", "~/.claude-work")],
        billing: crate::config::Billing::Subscription,
        models_url: None,
    },
    super::ProfileTemplate {
        kind: super::ProfileKind::Provider,
        name: "Hugging Face",
        description: "Open models (Kimi, GLM, Qwen, DeepSeek…) through Hugging Face Inference Providers, like the hf claude extension. Billed to your Hugging Face account.",
        setup: Some("Launchpad reads HF_TOKEN, else the token saved by the hf CLI. Sign in once if needed:"),
        setup_command: Some("hf auth login"),
        env: &[
            ("ANTHROPIC_BASE_URL", "https://router.huggingface.co"),
            ("ANTHROPIC_AUTH_TOKEN", "${HF_TOKEN}"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "${LAUNCHPAD_MODEL}"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "${LAUNCHPAD_MODEL}"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "${LAUNCHPAD_MODEL}"),
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "${LAUNCHPAD_MODEL}"),
            ("CLAUDE_CODE_SUBAGENT_MODEL", "${LAUNCHPAD_MODEL}"),
        ],
        billing: crate::config::Billing::PayAsYouGo,
        models_url: Some("https://router.huggingface.co/v1/models"),
    },
    super::ProfileTemplate {
        kind: super::ProfileKind::ApiKey,
        name: "API key",
        description: "Pay per use with a Console API key instead of the subscription.",
        setup: Some("Create a key at https://console.anthropic.com/settings/keys and paste it below."),
        setup_command: None,
        env: &[("ANTHROPIC_API_KEY", "")],
        billing: crate::config::Billing::PayAsYouGo,
        models_url: None,
    },
];
pub const REGISTRATION: super::Registration = super::Registration {
    descriptor: super::HarnessDescriptor {
        browser_isolation: "Browser tools only; built-in tools and other MCP servers disabled",
        cost_model_prefixes: &["claude-", "opus", "sonnet", "haiku"],
        id: ID,
        name: "Claude Code",
        icon_svg: include_str!("icons/claude.svg"),
        color: "#b46a4b",
        working_message: "Message Claude. It is read after the current step.",
        capabilities: super::HarnessCapabilities {
            permission_modes: true,
            images: true,
            messages_while_running: true,
            terminal: true,
            quota: true,
            cost: true,
        },
        account_env: &["CLAUDE_CONFIG_DIR"],
        env_vars: ENV_VARS,
        profile_templates: PROFILE_TEMPLATES,
    },
    create: |_| Arc::new(Adapter),
    environment_marker: |key| {
        key == "CLAUDECODE"
            || key.starts_with("CLAUDE_CODE_")
            || key == "CLAUDE_PID"
            || key == "CLAUDE_EFFORT"
    },
};
pub struct Adapter;
#[async_trait]
impl super::Harness for Adapter {
    fn skill_locations(&self) -> Vec<crate::prompt_context::SkillLocation> {
        use crate::prompt_context::SkillLocation;
        vec![SkillLocation::project(".claude/skills"), SkillLocation::config("CLAUDE_CONFIG_DIR", ".claude", "skills"), SkillLocation::project(".agents/skills"), SkillLocation::home(".agents/skills")]
    }
    fn sign_in_program(&self) -> &'static str {
        "claude"
    }
    fn sign_in_command(&self) -> &'static str {
        "claude auth login"
    }
    fn install_command(&self) -> Option<&'static str> {
        Some("curl -fsSL https://claude.ai/install.sh | bash")
    }
    /// `claude auth status` prints JSON; only the method, plan and account e-mail are kept.
    async fn sign_in(&self, env: &crate::shellenv::UserEnv) -> crate::signin::SignIn {
        let mut r = crate::signin::SignIn { signed_in: None, detail: None };
        match crate::signin::status_output(env, "claude", &["auth", "status"]).await {
            Ok((_, stdout, _)) => match serde_json::from_str::<Value>(&stdout) {
                Ok(v) => {
                    r.signed_in = v["loggedIn"].as_bool();
                    let parts: Vec<&str> = ["authMethod", "subscriptionType", "email"].iter().filter_map(|k| v[*k].as_str()).filter(|s| !s.is_empty()).collect();
                    if !parts.is_empty() { r.detail = Some(parts.join(" · ")); }
                }
                Err(_) => r.detail = Some("Could not read `claude auth status`; update Claude Code to check sign-in.".into()),
            },
            Err(e) => r.detail = Some(format!("{e:#}")),
        }
        r
    }
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        start(app, spec, sink).await
    }
    async fn models(&self, app: &App) -> Result<Vec<ModelInfo>> {
        models(app).await
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
            crate::procs::output(&app.env, "claude", &["--version"], None)
                .await?
                .split_whitespace()
                .next()
                .unwrap_or("")
                .into(),
        )
    }
    fn spawn_quota(&self, app: &App, account: crate::quota::QuotaAccount) -> Option<tokio::task::JoinHandle<()>> {
        Some(spawn_quota(app, account))
    }
    async fn native_trace(&self, app: &App, session: &Session) -> Result<Vec<super::trace::TraceFile>> {
        let id = super::trace::conversation(session)?;
        let root = super::trace::root(app, session, "CLAUDE_CONFIG_DIR", ".claude").await?;
        trace_files(&root, crate::hosts::loc::native(&session.cwd), id).await
    }
    fn project_moved(&self, old_s: &str, new_s: &str) {
        let (old_t, new_t) = (
            claude_transcripts_dir(old_s),
            claude_transcripts_dir(new_s),
        );
        if old_t.exists()
            && !new_t.exists()
            && let Err(e) = std::fs::rename(&old_t, &new_t)
        {
            tracing::warn!("promote: could not move Claude transcripts: {e}");
        }
    }
    fn terminal_resume_markers(&self) -> &'static [&'static str] {
        &["--resume", "--session-id"]
    }
    fn terminal_command(&self, s: &Session, _is_temp: bool) -> Result<Vec<String>> {
        let id = s
            .harness_session_id
            .clone()
            .context("the session has no conversation yet")?;
        let mut args = vec![];
        args.extend([
            "claude".into(),
            "--resume".into(),
            id,
        ]);
        args.extend(permission_args(s.launch_spec.permission_mode));
        Ok(args)
    }
    fn terminal_env(&self, is_temp: bool) -> &'static [(&'static str, &'static str)] {
        // Skip the trust dialog for folders created by Launchpad.
        if is_temp { &[("CLAUDE_CODE_SANDBOXED", "1")] } else { &[] }
    }
    fn title_command(
        &self,
        app: &App,
        _session: &Session,
        cwd: &std::path::Path,
    ) -> Option<crate::exec::Cmd> {
        let mut cmd = crate::procs::child_command(&app.env, "claude", Some(cwd));
        cmd.args([
            "-p",
            "--model",
            "haiku",
            "--tools",
            "",
            "--no-session-persistence",
            "--output-format",
            "json",
            "--strict-mcp-config",
        ]);
        Some(cmd)
    }
    fn title_response(&self, stdout: String) -> Result<super::TitleReply> {
        let value: Value = serde_json::from_str(stdout.trim()).context("unreadable title response")?;
        anyhow::ensure!(value["is_error"].as_bool() != Some(true), "title generator failed: {}", value["result"].as_str().unwrap_or("unknown error"));
        let title = value["result"].as_str().context("no generated title")?.to_string();
        let cost_unavailable = unknown_model_cost(&value);
        Ok(super::TitleReply { title, cost_unavailable, cost_usd: if cost_unavailable { None } else { value["total_cost_usd"].as_f64().filter(|c| c.is_finite() && *c >= 0.0) } })
    }
}
/// Quota from a Claude `rate_limit_event` (fractions and Unix seconds).
pub fn rate_limit_event(status: &crate::status::StatusHandle, account: &str, info: &Value) {
    let windows = &info["unifiedWindows"];
    let win = |k: &str| -> Option<crate::status::QuotaWindow> {
        let w = &windows[k];
        Some(crate::status::QuotaWindow {
            used_percent: w["utilization"].as_f64()? * 100.0,
            resets_at: Some(w["resetsAt"].as_i64()?),
        })
    };
    let (five, week) = (win("five_hour"), win("seven_day"));
    if five.is_none() && week.is_none() {
        return;
    }
    status.update_quota(account, five, week);
}

/// Polls the usage of one login: `CLAUDE_CONFIG_DIR` selects its configuration folder (default `~/.claude`).
fn spawn_quota(app: &App, account: crate::quota::QuotaAccount) -> tokio::task::JoinHandle<()> {
    let a = app.clone();
    tokio::spawn(async move {
        let client = crate::status::http_client();
        let config_dir = account.env.iter().find(|(k, _)| k == "CLAUDE_CONFIG_DIR").map(|(_, v)| std::path::PathBuf::from(v))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
        let mut rate_limit_delay = 0;
        loop {
            let mut result = claude_usage(&client, &config_dir).await;
            // Expired, missing (macOS Keychain) or rejected tokens: Claude Code reads usage itself and refreshes its own token.
            if let Err(e) = &result
                && e.downcast_ref::<UsageRateLimited>().is_none()
            {
                tracing::info!("claude usage endpoint ({}): {e:#}; asking Claude Code", account.key);
                result = claude_code_usage(&a, &account.env).await.map_err(|cc| anyhow::anyhow!("{e:#}; Claude Code: {cc:#}"));
            }
            match result {
                Ok((five, week)) => {
                    rate_limit_delay = 0;
                    a.status.update_quota(&account.key, five, week);
                }
                Err(e) => {
                    tracing::info!("claude usage ({}): {e:#}", account.key);
                    a.status.quota_error(&account.key, format!("{e:#}"));
                    if let Some(limit) = e.downcast_ref::<UsageRateLimited>() {
                        rate_limit_delay = if rate_limit_delay == 0 { 300 } else { (rate_limit_delay * 2).min(1800) };
                        tokio::time::sleep(std::time::Duration::from_secs(rate_limit_delay.max(limit.retry_after))).await;
                        continue;
                    }
                    rate_limit_delay = 0;
                }
            }
            crate::status::wait_poll(&a, &account.key, 300, 180).await;
        }
    })
}

#[derive(Debug)]
struct UsageRateLimited { retry_after: u64 }

impl std::fmt::Display for UsageRateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.retry_after > 0 {
            write!(f, "usage endpoint returned 429 (Retry-After: {} seconds); backing off", self.retry_after)
        } else {
            write!(f, "usage endpoint returned 429; backing off")
        }
    }
}

impl std::error::Error for UsageRateLimited {}

type QuotaPair = (Option<crate::status::QuotaWindow>, Option<crate::status::QuotaWindow>);

/// Reads `/api/oauth/usage`. Skips the call when the access token has expired: Claude Code refreshes it.
async fn claude_usage(client: &reqwest::Client, config_dir: &std::path::Path) -> Result<QuotaPair> {
    let path = config_dir.join(".credentials.json");
    let creds: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).context("read Claude credentials")?)?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"]
        .as_str()
        .context("no Claude OAuth token")?;
    if let Some(exp) = oauth["expiresAt"].as_i64()
        && exp <= now_ms()
    {
        bail!("Claude OAuth token expired");
    }
    let res = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await?;
    let code = res.status();
    if code == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = res.headers().get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        return Err(UsageRateLimited { retry_after }.into());
    }
    if !code.is_success() {
        bail!("usage endpoint returned {code}");
    }
    usage_windows(&res.json().await?)
}

/// Asks Claude Code for its structured `/usage` data (`get_usage` control request). No model request is made;
/// Claude Code authenticates and refreshes its own OAuth token, which Launchpad never does.
async fn claude_code_usage(app: &App, env: &[(String, String)]) -> Result<QuotaPair> {
    let home = dirs::home_dir();
    let mut child = crate::procs::child_command(&app.env.with(env), "claude", home.as_deref())
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence", "--strict-mcp-config"])
        .stdin(crate::exec::Io::Piped)
        .stdout(crate::exec::Io::Piped)
        .stderr(crate::exec::Io::Null)
        .spawn()
        .await
        .context("start claude")?;
    let mut stdin = child.stdin.take().context("claude stdin")?;
    let request = json!({"type": "control_request", "request_id": "launchpad-usage", "request": {"subtype": "get_usage", "skip_behaviors": true}});
    stdin.write_all(format!("{request}\n").as_bytes()).await?;
    let mut lines = BufReader::new(child.stdout.take().context("claude stdout")?).lines();
    let read = async {
        while let Some(line) = lines.next_line().await? {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            let r = &msg["response"];
            if msg["type"] != "control_response" || r["request_id"] != "launchpad-usage" { continue; }
            if r["subtype"] != "success" {
                bail!("{}", r["error"].as_str().unwrap_or("get_usage failed"));
            }
            let data = &r["response"];
            if data["rate_limits_available"] == false { bail!("plan rate limits are not available for this login"); }
            return usage_windows(&data["rate_limits"]);
        }
        bail!("no usage response")
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), read).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
    drop(stdin);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
    result
}

/// `five_hour` and `seven_day` of a usage response. `resets_at` is null while a window has not started.
fn usage_windows(body: &Value) -> Result<QuotaPair> {
    let win = |k: &str| -> Option<crate::status::QuotaWindow> {
        let w = &body[k];
        let resets_at = match &w["resets_at"] {
            Value::Null => None,
            v => Some(time::OffsetDateTime::parse(v.as_str()?, &time::format_description::well_known::Rfc3339).ok()?.unix_timestamp()),
        };
        Some(crate::status::QuotaWindow { used_percent: w["utilization"].as_f64()?, resets_at })
    };
    let five = win("five_hour").context("missing or invalid five_hour usage window")?;
    let week = win("seven_day").context("missing or invalid seven_day usage window")?;
    anyhow::ensure!([&five, &week].iter().all(|w| w.used_percent.is_finite() && w.used_percent >= 0.0), "invalid quota utilization");
    Ok((Some(five), Some(week)))
}

/// Claude stores transcripts in `~/.claude/projects/<path with every non-alphanumeric character as ->`.
fn claude_transcripts_dir(cwd: &str) -> std::path::PathBuf {
    let encoded: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/projects")
        .join(encoded)
}

async fn trace_files(root: &std::path::Path, cwd: &str, id: &str) -> Result<Vec<super::trace::TraceFile>> {
    let encoded: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let dir = root.join("projects").join(encoded);
    let mut traces = vec![super::trace::read(&dir.join(format!("{id}.jsonl")), format!("{id}.jsonl")).await?];
    for path in super::trace::files(&dir.join(id).join("subagents"), 2).await? {
        if path.extension().is_some_and(|e| e == "jsonl") {
            let name = path.strip_prefix(&dir)?.to_string_lossy().replace('\\', "/");
            traces.push(super::trace::read(&path, name).await?);
        }
    }
    Ok(traces)
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn unknown_pricing_discards_cost_without_losing_tokens_or_enforcing_a_budget() {
        let mut usage = Usage { cost_usd: Some(20.0), ..Default::default() };
        let mut result = json!({"total_cost_usd":62.297053,"usage":{"input_tokens":100,"cache_read_input_tokens":90,"output_tokens":4},"modelUsage":{"deepseek-ai/DeepSeek-V4.1-Flash-1017":{}}});
        result_usage(&mut usage, &result, Some(62.297053));
        assert!(usage.cost_unavailable);
        assert_eq!(usage.cost_usd, None);
        assert_eq!((usage.input_tokens, usage.cached_tokens, usage.output_tokens), (100, 90, 4));
        // A later known model cannot reconstruct the unpriced part of the session.
        result["modelUsage"] = json!({"claude-opus-4-7":{}});
        result_usage(&mut usage, &result, Some(62.297053));
        assert_eq!(usage.cost_usd, None);
        let mut known = Usage { cost_usd: Some(20.0), ..Default::default() };
        result_usage(&mut known, &result, Some(62.297053));
        assert_eq!(known.cost_usd, Some(82.297053));
        result["hasUnknownModelCost"] = json!(true);
        result_usage(&mut known, &result, Some(62.297053));
        assert!(known.cost_unavailable);
        assert_eq!(known.cost_usd, None);
        for model in ["opus", "sonnet", "haiku", "claude-sonnet-4-6"] {
            assert!(REGISTRATION.descriptor.supports_cost(Some(model)));
        }
        assert!(!REGISTRATION.descriptor.supports_cost(Some("deepseek-ai/DeepSeek-V4.1-Flash-1017")));
        assert!(unknown_model_cost(&json!({"message":{"model":"some-unknown-model"}})));
        assert!(!unknown_model_cost(&json!({"message":{"model":"<synthetic>"}})));
        let old: Usage = serde_json::from_value(json!({"context_tokens":null,"context_window":null,"input_tokens":0,"cached_tokens":0,"output_tokens":0,"cost_usd":1.0})).unwrap();
        assert!(!old.cost_unavailable);
    }

    #[test]
    fn turn_deltas_preserve_spending_from_concurrent_title_requests() {
        let result = json!({"modelUsage":{"claude-haiku-4-5":{}},"usage":{}});
        let mut usage = Usage { cost_usd: Some(0.25), ..Default::default() };
        result_usage(&mut usage, &result, Some(2.0));
        assert_eq!(usage.cost_usd, Some(2.25));
        // A title request finishes between two cumulative native turn reports.
        usage.cost_usd = Some(2.75);
        result_usage(&mut usage, &result, Some(1.0));
        assert_eq!(usage.cost_usd, Some(3.75));
        result_usage(&mut usage, &result, Some(0.0));
        assert_eq!(usage.cost_usd, Some(3.75));
    }

    #[test]
    fn title_responses_do_not_report_fallback_model_prices() {
        use crate::harness::Harness;
        let mut result = json!({"result":"A useful title","total_cost_usd":0.5,"modelUsage":{"deepseek-ai/DeepSeek-V4.1-Flash-1017":{}}});
        let reply = Adapter.title_response(result.to_string()).unwrap();
        assert_eq!(reply.title, "A useful title");
        assert!(reply.cost_unavailable);
        assert_eq!(reply.cost_usd, None);
        result["modelUsage"] = json!({"claude-haiku-4-5":{}});
        let reply = Adapter.title_response(result.to_string()).unwrap();
        assert!(!reply.cost_unavailable);
        assert_eq!(reply.cost_usd, Some(0.5));
        result["hasUnknownModelCost"] = json!(true);
        let reply = Adapter.title_response(result.to_string()).unwrap();
        assert!(reply.cost_unavailable);
        assert_eq!(reply.cost_usd, None);
    }

    #[test]
    fn usage_readings_require_both_valid_windows() {
        let valid = json!({"five_hour": {"utilization": 100, "resets_at": null}, "seven_day": {"utilization": 0, "resets_at": "2026-10-20T10:00:00Z"}});
        let (five, week) = usage_windows(&valid).unwrap();
        assert_eq!(five.unwrap().used_percent, 100.0);
        assert!(week.unwrap().resets_at.is_some());
        for invalid in [json!({}), json!({"five_hour": valid["five_hour"]}),
            json!({"five_hour": valid["five_hour"], "seven_day": null})] {
            assert!(usage_windows(&invalid).is_err());
        }
        for utilization in [json!(-1), json!("0")] {
            let mut invalid = valid.clone();
            invalid["five_hour"]["utilization"] = utilization;
            assert!(usage_windows(&invalid).is_err());
        }
        let mut invalid = valid;
        invalid["seven_day"]["resets_at"] = json!("invalid date");
        assert!(usage_windows(&invalid).is_err());
    }
    #[test]
    fn auto_permissions_use_the_native_classifier_without_bypass_flags() {
        assert_eq!(permission_args(PermissionMode::Auto), ["--permission-mode", "auto"]);
        assert_eq!(permission_args(PermissionMode::Approvals), ["--permission-mode", "default"]);
        let spec: LaunchSpec = serde_json::from_value(json!({"permission_mode":"auto"})).unwrap();
        assert_eq!(spec.permission_mode, PermissionMode::Auto);
    }

    #[test]
    fn approvals_require_an_explicit_choice_and_preserve_tool_input() {
        let input = json!({"command":"printf hello", "description":"A harmless test"});
        assert_eq!(claude_permission_response(&input, &["Allow once".into()]).unwrap(), json!({"behavior":"allow","updatedInput":input}));
        assert_eq!(claude_permission_response(&input, &["Deny".into()]).unwrap()["behavior"], "deny");
        for answers in [vec![], vec!["yes".into()], vec!["Allow once".into(), "Deny".into()]] {
            assert!(claude_permission_response(&input, &answers).is_err());
        }
        let old: LaunchSpec = serde_json::from_value(json!({"harness":"claude"})).unwrap();
        assert_eq!(old.permission_mode, PermissionMode::Yolo);
        let approvals: LaunchSpec = serde_json::from_value(json!({"harness":"codex","permission_mode":"approvals"})).unwrap();
        assert_eq!(approvals.permission_mode, PermissionMode::Approvals);
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    #[tokio::test]
    async fn native_trace_keeps_conversation_and_subagent_formats() {
        let tmp = super::super::trace::tests::Temp::new();
        let native = b"{\"type\":\"assistant\",\"message\":{\"content\":[]},\"uuid\":\"native\"}\n";
        tmp.write("projects/-my-project/session.jsonl", native);
        tmp.write("projects/-my-project/session/subagents/agent-child.jsonl", native);
        tmp.write("projects/-my-project/other.jsonl", b"{}\n");
        let files = trace_files(&tmp.0, "/my/project", "session").await.unwrap();
        assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["session.jsonl", "session/subagents/agent-child.jsonl"]);
        assert!(files.iter().all(|f| f.data == native));
        assert!(trace_files(&tmp.0, "/my/project", "missing").await.is_err());
    }
}

//! Shared ACP v1 client: stdio JSON-RPC, ordered updates, permission cards and session lifecycle.
//! Harness-specific executable/configuration stays in the registering module.
use crate::{
    model::*,
    sessions::{LiveSession, Sink, StartSpec, json_line, shorten, truncate_output},
    util::now_ms,
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

enum Incoming {
    Message(Value),
    Barrier(oneshot::Sender<()>),
    Exited(Option<String>),
}
struct Connection {
    stdin: tokio::sync::Mutex<Option<crate::exec::Stdin>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>,
    sequence: AtomicI64,
    alive: AtomicBool,
    stopping: AtomicBool,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    incoming: mpsc::UnboundedSender<Incoming>,
    sink: Sink,
}
impl Connection {
    async fn spawn(
        mut command: crate::exec::Cmd,
        sink: Sink,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Incoming>)> {
        let mut child = command
            .stdin(crate::exec::Io::Piped)
            .stdout(crate::exec::Io::Piped)
            .stderr(crate::exec::Io::Piped)
            .spawn()
            .await
            .context("start ACP harness")?;
        let stdin = child.stdin.take().context("ACP stdin")?;
        let stdout = child.stdout.take().context("ACP stdout")?;
        let stderr = child.stderr.take().context("ACP stderr")?;
        let (incoming, receiver) = mpsc::unbounded_channel();
        let (kill, mut killed) = oneshot::channel();
        let connection = Arc::new(Self {
            stdin: tokio::sync::Mutex::new(Some(stdin)),
            pending: Mutex::default(),
            sequence: AtomicI64::new(0),
            alive: AtomicBool::new(true),
            stopping: AtomicBool::new(false),
            kill: Mutex::new(Some(kill)),
            incoming,
            sink: sink.clone(),
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                sink.raw("err", &line).await;
            }
        });
        let conn = connection.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let error = loop {
                let line = tokio::select! {
                    line = lines.next_line() => line,
                    _ = &mut killed => { let _ = child.kill().await; break None; }
                };
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => break None,
                    Err(error) => break Some(error.to_string()),
                };
                conn.sink.raw("out", &line).await;
                let message: Value = match serde_json::from_str(&line) {
                    Ok(message) => message,
                    Err(_) => continue,
                };
                if message.get("method").is_some() {
                    let _ = conn.incoming.send(Incoming::Message(message));
                } else if let Some(id) = message["id"].as_i64() {
                    let sender = conn.pending.lock().unwrap().remove(&id);
                    if let Some(sender) = sender {
                        let result = if let Some(error) = message.get("error") {
                            Err(anyhow!("ACP: {error}"))
                        } else {
                            Ok(message["result"].clone())
                        };
                        let _ = sender.send(result);
                    }
                }
            };
            conn.alive.store(false, Ordering::SeqCst);
            conn.stdin.lock().await.take();
            for (_, sender) in conn.pending.lock().unwrap().drain() {
                let _ = sender.send(Err(anyhow!("ACP process exited")));
            }
            // A broken stdio transport must not leave an unreachable agent process running.
            let _ = child.start_kill();
            let status = child.wait().await;
            let error = error.or_else(|| match status {
                Ok(status) if status.success() || conn.stopping.load(Ordering::SeqCst) => None,
                Ok(status) => Some(format!("ACP process exited ({status})")),
                // Ended on an unreachable host: Interrupted, resumable there.
                Err(error) if crate::exec::is_lost(&error) => None,
                Err(error) => Some(error.to_string()),
            });
            let _ = conn.incoming.send(Incoming::Exited(error));
        });
        Ok((connection, receiver))
    }
    async fn write(&self, message: Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let stdin = stdin.as_mut().context("ACP process is closed")?;
        self.sink.raw("in", &message.to_string()).await;
        stdin.write_all(json_line(&message).as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
    async fn request(&self, method: &str, params: Value, timeout: bool) -> Result<Value> {
        let id = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);
        if let Err(error) = self
            .write(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        if timeout {
            match tokio::time::timeout(Duration::from_secs(60), receiver).await {
                Ok(result) => result.context("ACP response channel closed")?,
                Err(_) => {
                    self.pending.lock().unwrap().remove(&id);
                    bail!("ACP {method} timed out");
                }
            }
        } else {
            receiver.await.context("ACP response channel closed")?
        }
    }
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0", "method":method, "params":params}))
            .await
    }
    async fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0", "id":id, "result":result}))
            .await
    }
    async fn flush(&self) {
        let (sender, receiver) = oneshot::channel();
        if self.incoming.send(Incoming::Barrier(sender)).is_ok() {
            let _ = receiver.await;
        }
    }
    fn kill(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(kill) = self.kill.lock().unwrap().take() {
            let _ = kill.send(());
        }
    }
}

type PermissionRequest = (Value, Vec<(String, String)>);

#[derive(Default)]
struct State {
    running: bool,
    cancelled: bool,
    loading: bool,
    message: String,
    thought: String,
    message_id: String,
    tools: HashMap<String, Value>,
    questions: HashMap<String, PermissionRequest>,
    finished_tools: HashSet<String>,
    tool_outputs: HashMap<String, String>,
    images: bool,
    children: HashMap<String, Arc<Mutex<State>>>,
    native_children: bool,
    fallback_children: HashSet<String>,
    tool_child: Option<fn(&Value) -> bool>,
}
pub struct ChildUpdate {
    pub id: String,
    pub parent: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    pub update: Option<Value>,
    pub error: Option<String>,
}
pub struct Extensions {
    pub capabilities: Value,
    pub child_update: fn(&Value) -> Option<ChildUpdate>,
    pub tool_child: fn(&Value) -> bool,
}
pub struct AcpSession {
    connection: Arc<Connection>,
    state: Arc<Mutex<State>>,
    native_id: String,
    sink: Sink,
}

/// Starts one ACP process per session. Resume without replay when negotiated; otherwise suppress
/// the historical session/load replay because Launchpad already has its own persisted trace.
pub async fn start(command: crate::exec::Cmd, spec: StartSpec, sink: Sink, extensions: Extensions) -> Result<Arc<dyn LiveSession>> {
    let (connection, mut receiver) = Connection::spawn(command, sink.clone()).await?;
    let state = Arc::new(Mutex::new(State {
        loading: true,
        tool_child: Some(extensions.tool_child),
        ..Default::default()
    }));
    let events_state = state.clone();
    let events_connection = connection.clone();
    let events_sink = sink.clone();
    let child_update = extensions.child_update;
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            match event {
                Incoming::Message(message) => {
                    if let Some(child) = child_update(&message) {
                        if events_state.lock().unwrap().loading { continue; }
                        let fallbacks = {
                            let mut state = events_state.lock().unwrap();
                            state.native_children = true;
                            state.fallback_children.drain().collect::<Vec<_>>()
                        };
                        for id in fallbacks { events_sink.subagent(&id, None, Some("superseded".into()), None).await; }
                        events_sink.emit(EventBody::Subagent { agent_id: child.id.clone(), parent_id: child.parent, name: child.name, status: child.status.clone(), model: None }).await;
                        let state = events_state.lock().unwrap().children.entry(child.id.clone()).or_insert_with(|| Arc::new(Mutex::new(State { message_id: uuid::Uuid::new_v4().to_string(), ..Default::default() }))).clone();
                        if let Some(status) = &child.status { state.lock().unwrap().running = status == "running"; }
                        let sink = events_sink.child(&child.id);
                        if let Some(body) = child.update { update(&sink, &state, &body).await; }
                        if child.status.as_deref().is_some_and(|s| matches!(s, "completed" | "failed" | "interrupted" | "stopped")) {
                            finish_message(&sink, &state).await;
                            state.lock().unwrap().message_id = uuid::Uuid::new_v4().to_string();
                        }
                        if let Some(message) = child.error { sink.emit(EventBody::Error { message }).await; }
                        let (idle, busy) = { let state = events_state.lock().unwrap(); (!state.running, !state.tools.is_empty() || children_busy(&state)) };
                        if idle && matches!(events_sink.status(), Some(SessionStatus::Waiting | SessionStatus::Background)) { events_sink.set_status(if busy { SessionStatus::Background } else { SessionStatus::Waiting }).await; }
                    } else if message["method"] == "session/update" {
                        if !events_state.lock().unwrap().loading {
                            update(&events_sink, &events_state, &message["params"]["update"]).await;
                        }
                    } else if message["method"] == "session/request_permission" {
                        permission(&events_sink, &events_state, &message).await;
                    } else if let Some(id) = message.get("id") {
                        // Do not advertise client filesystem or terminal support; the agent uses its own tools.
                        let _ = events_connection.write(json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"Unsupported ACP client method"}})).await;
                    }
                }
                Incoming::Barrier(sender) => {
                    let _ = sender.send(());
                }
                Incoming::Exited(error) => {
                    let children = events_state.lock().unwrap().children.clone();
                    for (id, state) in &children { finish_message(&events_sink.child(id), state).await; }
                    finish_message(&events_sink, &events_state).await;
                    let requests = {
                        let mut state = events_state.lock().unwrap();
                        state.running = false;
                        state
                            .questions
                            .drain()
                            .map(|(id, _)| id)
                            .collect::<Vec<_>>()
                    };
                    for request_id in requests {
                        events_sink
                            .emit(EventBody::AskAnswered {
                                request_id,
                                answers: vec!["Process exited".into()],
                            })
                            .await;
                    }
                    events_sink.set_open_question(false).await;
                    events_sink.exited(error).await;
                    break;
                }
            }
        }
    });
    let setup = async {
        let init = connection.request("initialize", json!({"protocolVersion":1,"clientCapabilities":extensions.capabilities,"clientInfo":{"name":"launchpad","version":env!("CARGO_PKG_VERSION")}}), true).await?;
        anyhow::ensure!(init["protocolVersion"] == 1, "ACP protocol version 1 is required");
        state.lock().unwrap().images = init["agentCapabilities"]["promptCapabilities"]["image"] == true;
        let mut params = json!({"cwd":spec.cwd.to_string_lossy(),"mcpServers":[]});
        let response = if spec.resume {
            let id = spec.harness_session_id.as_deref().context("no ACP session to resume")?;
            params["sessionId"] = json!(id);
            let method = if init["agentCapabilities"]["sessionCapabilities"].get("resume").is_some() { "session/resume" } else {
                anyhow::ensure!(init["agentCapabilities"]["loadSession"] == true, "This ACP agent cannot resume sessions");
                "session/load"
            };
            connection.request(method, params, true).await?
        } else { connection.request("session/new", params, true).await? };
        let id = response["sessionId"].as_str().map(str::to_string).or(spec.harness_session_id.clone()).context("ACP did not return a session ID")?;
        if let Some(model) = &spec.model {
            let option = response["configOptions"].as_array().and_then(|options| options.iter().find(|o| o["category"] == "model" || o["id"] == "model"));
            if let Some(option) = option {
                connection.request("session/set_config_option", json!({"sessionId":id,"configId":option["id"],"value":model}), true).await?;
            } else { connection.request("session/set_model", json!({"sessionId":id,"modelId":model}), true).await?; }
            sink.set_model(model).await;
        } else if let Some(model) = response["models"]["currentModelId"].as_str().or_else(|| response["configOptions"].as_array()?.iter().find(|o| o["category"] == "model" || o["id"] == "model")?["currentValue"].as_str()) { sink.set_model(model).await; }
        anyhow::ensure!(spec.effort.is_none(), "This ACP integration does not expose effort selection yet");
        sink.set_harness_id(&id).await;
        connection.flush().await;
        state.lock().unwrap().loading = false;
        Ok::<_, anyhow::Error>(id)
    }.await;
    match setup {
        Ok(native_id) => Ok(Arc::new(AcpSession {
            connection,
            state,
            native_id,
            sink,
        })),
        Err(error) => {
            connection.kill();
            Err(error)
        }
    }
}

#[async_trait]
impl LiveSession for AcpSession {
    async fn send(&self, text: &str, client_id: &str, images: &[ImageAttachment]) -> Result<()> {
        anyhow::ensure!(
            self.ready_for_turn(),
            "Wait for the ACP turn and tools to finish"
        );
        anyhow::ensure!(
            images.is_empty() || self.state.lock().unwrap().images,
            "This ACP agent version does not support images"
        );
        let mut prompt = vec![];
        if !text.is_empty() {
            prompt.push(json!({"type":"text","text":text}));
        }
        for image in images {
            let (header, data) = image
                .data_url
                .split_once(',')
                .context("invalid image data URL")?;
            let mime = header
                .strip_prefix("data:")
                .and_then(|h| h.strip_suffix(";base64"))
                .context("invalid image media type")?;
            prompt.push(json!({"type":"image","data":data,"mimeType":mime}));
        }
        {
            let mut state = self.state.lock().unwrap();
            state.running = true;
            state.cancelled = false;
            state.message.clear();
            state.thought.clear();
            state.message_id = uuid::Uuid::new_v4().to_string();
        }
        self.sink
            .emit(EventBody::MessageDelivered {
                client_id: client_id.into(),
            })
            .await;
        let connection = self.connection.clone();
        let state = self.state.clone();
        let sink = self.sink.clone();
        let id = self.native_id.clone();
        tokio::spawn(async move {
            let started = now_ms();
            let result = connection
                .request(
                    "session/prompt",
                    json!({"sessionId":id,"prompt":prompt}),
                    false,
                )
                .await;
            connection.flush().await;
            finish_message(&sink, &state).await;
            let (cancelled, tools_pending, requests) = {
                let mut state = state.lock().unwrap();
                state.running = false;
                let requests = state
                    .questions
                    .drain()
                    .map(|(request_id, (rpc_id, _))| (request_id, rpc_id))
                    .collect::<Vec<_>>();
                (state.cancelled, !state.tools.is_empty() || children_busy(&state), requests)
            };
            // Cancellation must settle permission cards too, rather than leaving a dead question.
            for (request_id, rpc_id) in requests {
                let _ = connection
                    .respond(rpc_id, json!({"outcome":{"outcome":"cancelled"}}))
                    .await;
                sink.emit(EventBody::AskAnswered {
                    request_id,
                    answers: vec!["Cancelled".into()],
                })
                .await;
            }
            sink.set_open_question(false).await;
            if !connection.alive.load(Ordering::SeqCst) {
                return;
            }
            let interrupted = cancelled
                || result
                    .as_ref()
                    .is_ok_and(|v| v["stopReason"] == "cancelled");
            let error = result
                .err()
                .filter(|_| !interrupted)
                .map(|e| format!("{e:#}"));
            sink.emit(EventBody::TurnEnd {
                duration_ms: Some(now_ms() - started),
                interrupted,
                error: error.clone(),
            })
            .await;
            sink.set_status(finished_status(
                error.is_some(),
                interrupted,
                sink.stop_requested(),
                tools_pending,
            ))
            .await;
        });
        Ok(())
    }
    fn turn_active(&self) -> bool {
        self.state.lock().unwrap().running
    }
    fn ready_for_turn(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.running && state.tools.is_empty() && state.questions.is_empty() && !children_busy(&state)
    }
    async fn interrupt(&self) -> Result<()> {
        self.state.lock().unwrap().cancelled = true;
        self.connection
            .notify("session/cancel", json!({"sessionId":self.native_id}))
            .await
    }
    fn has_open_questions(&self) -> bool { !self.state.lock().unwrap().questions.is_empty() }
    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        let request = self
            .state
            .lock()
            .unwrap()
            .questions
            .get(request_id)
            .cloned()
            .context("this question was already answered")?;
        let answer = answers.first().context("choose a permission option")?;
        let option = request
            .1
            .iter()
            .find(|(_, label)| label == answer)
            .context("choose one of the available permission options")?;
        self.connection
            .respond(
                request.0,
                json!({"outcome":{"outcome":"selected","optionId":option.0}}),
            )
            .await?;
        self.state.lock().unwrap().questions.remove(request_id);
        Ok(())
    }
    async fn stop(&self) -> Result<()> {
        if self.turn_active() {
            let _ = self.interrupt().await;
        }
        self.connection.kill();
        Ok(())
    }
}

async fn permission(sink: &Sink, state: &Mutex<State>, message: &Value) {
    let request_id = uuid::Uuid::new_v4().to_string();
    let params = &message["params"];
    let options: Vec<(String, String)> = params["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| Some((o["optionId"].as_str()?.into(), o["name"].as_str()?.into())))
        .collect();
    state
        .lock()
        .unwrap()
        .questions
        .insert(request_id.clone(), (message["id"].clone(), options.clone()));
    sink.set_open_question(true).await;
    sink.emit(EventBody::AskUser {
        request_id: request_id.clone(),
        questions: vec![Question {
            id: request_id,
            header: "Permission".into(),
            question: params["toolCall"]["title"]
                .as_str()
                .unwrap_or("Allow this tool operation?")
                .into(),
            options: options
                .iter()
                .map(|(_, label)| QuestionOption {
                    label: label.clone(),
                    description: String::new(),
                })
                .collect(),
            multi_select: false,
        }],
    })
    .await;
}

async fn finish_message(sink: &Sink, state: &Mutex<State>) {
    let (id, text, thought) = {
        let mut state = state.lock().unwrap();
        (
            state.message_id.clone(),
            std::mem::take(&mut state.message),
            std::mem::take(&mut state.thought),
        )
    };
    if !thought.is_empty() {
        sink.emit(EventBody::Reasoning {
            item_id: format!("{id}-thought"),
            text: thought,
        })
        .await;
    }
    if !text.is_empty() {
        sink.emit(EventBody::AssistantMessage { item_id: id, text })
            .await;
    }
}
async fn update(sink: &Sink, state: &Mutex<State>, update: &Value) {
    match update["sessionUpdate"].as_str().unwrap_or("") {
        "agent_message_chunk" | "agent_thought_chunk" => {
            if let Some(text) = update["content"]["text"].as_str() {
                let thought = update["sessionUpdate"] == "agent_thought_chunk";
                let id = {
                    let mut state = state.lock().unwrap();
                    if thought {
                        state.thought.push_str(text);
                    } else {
                        state.message.push_str(text);
                    }
                    state.message_id.clone()
                };
                if !thought {
                    sink.emit(EventBody::AssistantDelta {
                        item_id: id,
                        text: text.into(),
                    })
                    .await;
                }
            }
        }
        "tool_call" | "tool_call_update" => {
            if update["sessionUpdate"] == "tool_call" {
                finish_message(sink, state).await;
                state.lock().unwrap().message_id = uuid::Uuid::new_v4().to_string();
            }
            let Some(id) = update["toolCallId"].as_str() else {
                return;
            };
            if state.lock().unwrap().finished_tools.contains(id) {
                return;
            }
            let (tool, metadata_changed) = {
                let mut state = state.lock().unwrap();
                let is_new = !state.tools.contains_key(id);
                let tool = state.tools.entry(id.into()).or_insert_with(|| json!({}));
                let previous = (tool["kind"].clone(), tool["title"].clone(), tool["rawInput"].clone());
                if let Some(fields) = update.as_object() {
                    for (key, value) in fields {
                        tool[key] = value.clone();
                    }
                }
                let changed = previous != (tool["kind"].clone(), tool["title"].clone(), tool["rawInput"].clone());
                (tool.clone(), is_new || changed)
            };
            // Some ACP versions expose only the delegation tool, without native child streams.
            let fallback = {
                let state = state.lock().unwrap();
                !state.native_children && state.tool_child.is_some_and(|identify| identify(&tool))
            };
            if fallback {
                let agent = format!("tool:{id}");
                let first = state.lock().unwrap().fallback_children.insert(agent.clone());
                sink.subagent(&agent, tool["rawInput"]["description"].as_str().or(tool["title"].as_str()).map(str::to_string), Some(if tool["status"] == "completed" { "completed" } else if tool["status"] == "failed" { "failed" } else { "running" }.into()), None).await;
                if first {
                    let child = sink.child(&agent);
                    child.emit(EventBody::Notice { text: "This harness version exposes only the delegation prompt and result; inner trace events are unavailable.".into() }).await;
                    if let Some(text) = tool["rawInput"]["prompt"].as_str() { child.emit(EventBody::UserMessage { text: text.into(), client_id: agent.clone(), queued: false, images: vec![] }).await; }
                    child.emit(EventBody::ToolCall { item_id: id.into(), name: "Agent".into(), summary: "Delegation result".into(), input: tool["rawInput"].clone() }).await;
                }
                if tool["status"] == "completed" || tool["status"] == "failed" { sink.child(&agent).emit(EventBody::ToolResult { item_id: id.into(), output: tool_output(&tool), is_error: tool["status"] == "failed", exit_code: None, duration_ms: None }).await; }
            }
            if metadata_changed {
                sink.emit(EventBody::ToolCall {
                    item_id: id.into(),
                    name: if tool["kind"] == "execute" {
                        "Shell"
                    } else {
                        tool["kind"].as_str().unwrap_or("Tool")
                    }
                    .into(),
                    summary: shorten(tool["title"].as_str().unwrap_or("Tool operation"), 160),
                    input: tool["rawInput"].clone(),
                })
                .await;
            }
            let done = tool["status"] == "completed" || tool["status"] == "failed";
            let output = tool_output(&tool);
            if done {
                {
                    let mut state = state.lock().unwrap();
                    state.tools.remove(id);
                    state.tool_outputs.remove(id);
                    state.finished_tools.insert(id.into());
                }
                sink.emit(EventBody::ToolResult {
                    item_id: id.into(),
                    output: truncate_output(&output, 16000),
                    is_error: tool["status"] == "failed",
                    exit_code: None,
                    duration_ms: None,
                })
                .await;
                for content in tool["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["type"] == "diff")
                {
                    if let Some(path) = content["path"].as_str() {
                        sink.emit(EventBody::FileChange {
                            item_id: id.into(),
                            path: path.into(),
                            change: if content["oldText"].is_null() {
                                "add"
                            } else {
                                "update"
                            }
                            .into(),
                            diff: replacement_diff(
                                path,
                                content["oldText"].as_str(),
                                content["newText"].as_str().unwrap_or(""),
                            ),
                        })
                        .await;
                    }
                }
                let ready = {
                    let state = state.lock().unwrap();
                    !state.running && state.tools.is_empty() && !children_busy(&state)
                };
                if ready && sink.status() == Some(SessionStatus::Background) {
                    let cancelled = state.lock().unwrap().cancelled;
                    sink.set_status(finished_status(
                        false,
                        cancelled,
                        sink.stop_requested(),
                        false,
                    ))
                    .await;
                }
            } else if !output.is_empty() {
                let delta = {
                    let mut state = state.lock().unwrap();
                    let previous = state.tool_outputs.entry(id.into()).or_default();
                    let delta = output
                        .strip_prefix(previous.as_str())
                        .unwrap_or(&output)
                        .to_string();
                    *previous = output;
                    delta
                };
                if delta.is_empty() {
                    return;
                }
                sink.emit(EventBody::ToolOutput {
                    item_id: id.into(),
                    text: delta,
                })
                .await;
            }
        }
        "plan" => {
            sink.emit(EventBody::Plan {
                items: update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| PlanItem {
                        text: e["content"].as_str().unwrap_or("").into(),
                        status: e["status"].as_str().unwrap_or("pending").into(),
                    })
                    .collect(),
            })
            .await;
        }
        "usage_update" => {
            let context = update["used"].as_u64();
            let window = update["size"].as_u64();
            let cost = if update["cost"]["currency"] == "USD" {
                update["cost"]["amount"]
                    .as_f64()
                    .filter(|c| c.is_finite() && *c >= 0.0)
            } else {
                None
            };
            sink.update_usage(|usage| {
                usage.context_tokens = context;
                usage.context_window = window;
                if let Some(cost) = cost {
                    usage.cost_usd = Some(cost);
                }
            })
            .await;
        }
        "current_model_update" => {
            if let Some(model) = update["currentModelId"].as_str() {
                sink.set_model(model).await;
            }
        }
        "config_option_update" => {
            if let Some(options) = update["configOptions"].as_array()
                && let Some(model) = options
                    .iter()
                    .find(|o| o["category"] == "model" || o["id"] == "model")
                    .and_then(|o| o["currentValue"].as_str())
                {
                    sink.set_model(model).await;
            }
        }
        _ => {}
    }
}

fn children_busy(state: &State) -> bool {
    state.children.values().any(|child| { let child = child.lock().unwrap(); child.running || !child.tools.is_empty() })
}

fn finished_status(
    error: bool,
    interrupted: bool,
    stopped: bool,
    tools_pending: bool,
) -> SessionStatus {
    if error {
        SessionStatus::Error
    } else if tools_pending {
        SessionStatus::Background
    } else if stopped {
        SessionStatus::Stopped
    } else if interrupted {
        SessionStatus::Interrupted
    } else {
        SessionStatus::Waiting
    }
}

fn tool_output(tool: &Value) -> String {
    let content = tool["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["content"]["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !content.is_empty() {
        return content;
    }
    tool["rawOutput"]
        .as_str()
        .or_else(|| tool["rawOutput"]["output"].as_str())
        .or_else(|| tool["rawOutput"]["error"].as_str())
        .unwrap_or("")
        .to_string()
}

// ACP supplies before/after texts without hunk offsets. Render the reported text as a unified
// diff preview; native agents may report only a replaced fragment. Never apply this as a patch.
fn replacement_diff(path: &str, old: Option<&str>, new: &str) -> String {
    let old = old.unwrap_or("");
    let old_count = old.lines().count();
    let new_count = new.lines().count();
    let mut patch = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        usize::from(old_count > 0),
        old_count,
        usize::from(new_count > 0),
        new_count
    );
    for (prefix, text) in [("-", old), ("+", new)] {
        for line in text.lines() {
            patch.push_str(prefix);
            patch.push_str(line);
            patch.push('\n');
        }
        if !text.is_empty() && !text.ends_with('\n') {
            patch.push_str("\\ No newline at end of file\n");
        }
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_turn_waits_for_tools_and_never_becomes_done() {
        assert_eq!(
            finished_status(false, true, true, true),
            SessionStatus::Background
        );
        assert_eq!(
            finished_status(false, true, true, false),
            SessionStatus::Stopped
        );
        assert_eq!(
            finished_status(false, true, false, false),
            SessionStatus::Interrupted
        );
        assert_eq!(
            finished_status(true, false, false, false),
            SessionStatus::Error
        );
    }
    #[test]
    fn acp_output_uses_nested_content_and_diff_preserves_empty_and_unterminated_files() {
        assert_eq!(
            tool_output(
                &json!({"content":[{"type":"content","content":{"type":"text","text":"done"}},{"type":"diff","path":"x","newText":"x"}]})
            ),
            "done"
        );
        assert_eq!(
            tool_output(&json!({"rawOutput":{"error":"denied"}})),
            "denied"
        );
        assert_eq!(
            replacement_diff("x", None, "a"),
            "--- a/x\n+++ b/x\n@@ -0,0 +1,1 @@\n+a\n\\ No newline at end of file\n"
        );
        assert!(replacement_diff("x", Some("a\nb\n"), "").contains("@@ -1,2 +0,0 @@\n-a\n-b\n"));
    }
}

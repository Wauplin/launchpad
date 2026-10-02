//! Claude Code adapter: one `claude -p` process per session, stream-json on stdin and stdout.
//! The exact behavior is recorded in docs/spikes/1-claude-stream-json.md.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;

use crate::App;
use crate::model::*;
use crate::sessions::{LiveSession, Sink, StartSpec, json_line, shorten, text_of, truncate_output};

pub struct ClaudeSession {
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    state: Arc<Mutex<State>>,
    child_pid: Option<u32>,
}

#[derive(Default)]
struct State {
    /// `tool_use_id` → (name, input), to build file changes and background tasks from results.
    tools: HashMap<String, (String, Value)>,
    /// Running background tasks.
    background: HashSet<String>,
    /// `request_id` of pending `can_use_tool` requests for AskUserQuestion → their input.
    questions: HashMap<String, Value>,
    context_window: Option<u64>,
    stopping: bool,
    cwd: String,
}

pub async fn start(app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
        "--replay-user-messages",
        "--permission-prompt-tool",
        "stdio",
    ]
    .into_iter()
    .map(String::from)
    .collect();
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
    let mut cmd = crate::procs::child_command(&app.env, "claude", Some(&spec.cwd));
    cmd.args(&args)
        .env("LAUNCHPAD_PROFILE", app.paths.profile.name())
        .env("LAUNCHPAD_SESSION_ID", &spec.session_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().context("start claude")?;
    sink.raw("cmd", &format!("claude {}", args.join(" "))).await;
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let state = Arc::new(Mutex::new(State { cwd: spec.cwd.to_string_lossy().to_string(), ..Default::default() }));
    let session = Arc::new(ClaudeSession { stdin: tokio::sync::Mutex::new(Some(stdin)), state: state.clone(), child_pid: child.id() });

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
        while let Ok(Some(line)) = lines.next_line().await {
            out_sink.raw("out", &line).await;
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            handle(&out_sink, &st, &writer, &msg).await;
        }
        let status = child.wait().await;
        let stopping = st.lock().unwrap().stopping;
        let error = match status {
            Ok(s) if s.success() || stopping => None,
            Ok(s) => Some(format!("claude exited ({s}): {}", stderr_tail.lock().unwrap().join("\n"))),
            Err(e) => Some(format!("claude: {e}")),
        };
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
    async fn send(&self, text: &str, client_id: &str) -> Result<()> {
        self.write(&json!({
            "type": "user",
            "uuid": client_id,
            "message": {"role": "user", "content": [{"type": "text", "text": text}]},
        }))
        .await
    }

    async fn interrupt(&self) -> Result<()> {
        let id = format!("interrupt-{}", uuid::Uuid::new_v4());
        self.write(&json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}})).await
    }

    async fn answer(&self, request_id: &str, answers: Vec<String>) -> Result<()> {
        let input = self.state.lock().unwrap().questions.remove(request_id).context("this question was already answered")?;
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
        // Closing stdin ends the process after the current step; kill it if it does not exit.
        self.stdin.lock().await.take();
        if let Some(pid) = self.child_pid {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            });
        }
        Ok(())
    }
}

/// Maps one stdout message to normalized events.
async fn handle(sink: &Sink, st: &Arc<Mutex<State>>, writer: &Arc<ClaudeSession>, msg: &Value) {
    match msg["type"].as_str().unwrap_or("") {
        "system" => handle_system(sink, st, msg).await,
        "stream_event" => {
            let ev = &msg["event"];
            if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                let item_id = format!("{}:{}", msg["parent_tool_use_id"].as_str().unwrap_or(""), ev["index"]);
                if msg["parent_tool_use_id"].is_null() {
                    sink.emit(EventBody::AssistantDelta { item_id, text: ev["delta"]["text"].as_str().unwrap_or("").into() }).await;
                }
            }
        }
        "assistant" => {
            // Messages from subagents (parent_tool_use_id set) are not shown in the main conversation.
            if !msg["parent_tool_use_id"].is_null() {
                return;
            }
            let m = &msg["message"];
            let msg_id = m["id"].as_str().unwrap_or("");
            if let Some(model) = m["model"].as_str() {
                sink.set_model(model).await;
            }
            for (i, block) in m["content"].as_array().cloned().unwrap_or_default().iter().enumerate() {
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
            if !msg["parent_tool_use_id"].is_null() {
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
            let error = if msg["is_error"].as_bool() == Some(true) && msg["subtype"] != "error_during_execution" {
                Some(msg["result"].as_str().unwrap_or(msg["subtype"].as_str().unwrap_or("error")).to_string())
            } else {
                None
            };
            let interrupted = msg["subtype"] == "error_during_execution";
            sink.emit(EventBody::TurnEnd { duration_ms: msg["duration_ms"].as_i64(), interrupted, error: error.clone() }).await;
            let u = &msg["usage"];
            let window = msg["modelUsage"].as_object().and_then(|m| m.values().filter_map(|v| v["contextWindow"].as_u64()).max());
            if let Some(w) = window {
                st.lock().unwrap().context_window = Some(w);
            }
            sink.update_usage(|us| {
                us.input_tokens += u["input_tokens"].as_u64().unwrap_or(0) + u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                us.cached_tokens += u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                us.output_tokens += u["output_tokens"].as_u64().unwrap_or(0);
                if let Some(c) = msg["total_cost_usd"].as_f64() {
                    us.cost_usd = Some(c);
                }
                if window.is_some() {
                    us.context_window = window;
                }
            })
            .await;
            let background = !st.lock().unwrap().background.is_empty();
            if !st.lock().unwrap().questions.is_empty() {
                return;
            }
            sink.set_status(if background { SessionStatus::Background } else { SessionStatus::Waiting }).await;
        }
        "rate_limit_event" => sink.app().status.claude_rate_limit_event(&msg["rate_limit_info"]),
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
            } else if req["subtype"] == "can_use_tool" {
                // Everything else runs without asking, as with --dangerously-skip-permissions.
                tracing::info!("claude {}: allowing {}", sink.id, req["tool_name"]);
                let _ = writer
                    .write(&json!({
                        "type": "control_response",
                        "response": {"subtype": "success", "request_id": request_id, "response": {"behavior": "allow", "updatedInput": req["input"]}},
                    }))
                    .await;
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
            if let Some(model) = msg["model"].as_str() {
                sink.set_model(model).await;
            }
            if sink.status() != Some(SessionStatus::Working) && st.lock().unwrap().questions.is_empty() {
                sink.set_status(SessionStatus::Working).await;
            }
        }
        "task_started" if msg["is_backgrounded"].as_bool() == Some(true) => {
            let task_id = msg["task_id"].as_str().unwrap_or("").to_string();
            st.lock().unwrap().background.insert(task_id.clone());
            sink.emit(EventBody::BackgroundTask {
                task_id,
                description: msg["description"].as_str().unwrap_or("").into(),
                status: "running".into(),
                output_file: None,
            })
            .await;
        }
        "task_notification" => {
            let task_id = msg["task_id"].as_str().unwrap_or("").to_string();
            let was_background = st.lock().unwrap().background.remove(&task_id);
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
    let cwd = st.lock().unwrap().cwd.clone();
    if !is_error {
        if let Some(fc) = file_change(&name, &input, &id, &cwd) {
            sink.emit(fc).await;
        }
    }
    // Output file of a background command, for its live view.
    if name == "Bash" && input["run_in_background"].as_bool() == Some(true) {
        if let (Some(task), Some(file)) = (extra["backgroundTaskId"].as_str(), output.split("Output is being written to: ").nth(1)) {
            sink.emit(EventBody::BackgroundTask {
                task_id: task.to_string(),
                description: summary(&name, &input),
                status: "running".into(),
                output_file: Some(file.split_whitespace().next().unwrap_or("").trim_end_matches('.').to_string()),
            })
            .await;
        }
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
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
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

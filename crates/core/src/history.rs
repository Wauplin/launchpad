//! Explicit, read-only discovery of external conversations. Import stores an archived snapshot.
use crate::{
    App,
    model::*,
    sessions::{shorten, text_of},
    util::now_ms,
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
};
use ts_rs::TS;

#[derive(Clone, Serialize, TS)]
#[ts(rename = "HistorySession")]
pub struct Candidate {
    pub id: String,
    pub harness: HarnessKind,
    pub cwd: String,
    pub title: String,
    #[ts(type = "number")]
    pub started_at: i64,
}
#[derive(Serialize, TS)]
#[ts(rename = "HistoryPage")]
pub struct Page {
    pub sessions: Vec<Candidate>,
    pub next: Option<String>,
}

fn timestamp(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or_else(now_ms)
}
fn claude_paths() -> Vec<PathBuf> {
    let root = dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/projects");
    let mut out = vec![];
    if let Ok(dirs) = std::fs::read_dir(root) {
        for dir in dirs.flatten() {
            if let Ok(files) = std::fs::read_dir(dir.path()) {
                for f in files.flatten() {
                    if f.path().extension().is_some_and(|e| e == "jsonl") {
                        out.push(f.path());
                    }
                }
            }
        }
    }
    out
}
fn claude_candidate(path: &std::path::Path) -> Option<Candidate> {
    let id = path.file_stem()?.to_str()?.to_string();
    uuid::Uuid::parse_str(&id).ok()?;
    let reader = BufReader::new(std::fs::File::open(path).ok()?);
    let mut cwd = None;
    let mut prompt = None;
    let mut started = None;
    // Metadata and the first prompt normally appear at the start. Do not scan large transcripts.
    for line in reader.lines().take(100) {
        let Ok(v) = serde_json::from_str::<Value>(&line.ok()?) else {
            continue;
        };
        if v["isSidechain"] == true {
            return None;
        }
        if cwd.is_none() {
            cwd = v["cwd"].as_str().map(str::to_string);
        }
        if started.is_none() && v["timestamp"].is_string() {
            started = Some(timestamp(&v["timestamp"]));
        }
        if v["type"] == "user" && prompt.is_none() {
            let text = content_text(&v["message"]["content"]);
            if !text.trim().is_empty() {
                prompt = Some(text);
            }
        }
        if cwd.is_some() && prompt.is_some() {
            break;
        }
    }
    Some(Candidate {
        id,
        harness: HarnessKind::Claude,
        cwd: cwd?,
        title: shorten(&prompt.unwrap_or_else(|| "Claude conversation".into()), 80),
        started_at: started.unwrap_or_else(now_ms),
    })
}
fn content_text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
fn codex_candidate(v: &Value) -> Option<Candidate> {
    Some(Candidate {
        id: v["id"].as_str()?.into(),
        harness: HarnessKind::Codex,
        cwd: v["cwd"].as_str()?.into(),
        title: shorten(
            v["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(v["preview"].as_str())
                .unwrap_or("Codex conversation"),
            80,
        ),
        started_at: v["createdAt"].as_i64().unwrap_or(0) * 1000,
    })
}
pub async fn list(
    app: &App,
    harness: HarnessKind,
    cursor: Option<String>,
    archived: bool,
) -> Result<Page> {
    let mut page = match harness {
        HarnessKind::Claude => {
            let start: usize = cursor
                .as_deref()
                .unwrap_or("0")
                .parse()
                .context("invalid cursor")?;
            let all = tokio::task::spawn_blocking(|| {
                let mut all: Vec<_> = claude_paths()
                    .iter()
                    .filter_map(|p| claude_candidate(p))
                    .collect();
                all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                all
            })
            .await?;
            let next = (start.saturating_add(100) < all.len())
                .then(|| start.saturating_add(100).to_string());
            Page {
                sessions: all.into_iter().skip(start).take(100).collect(),
                next,
            }
        }
        HarnessKind::Codex => {
            let v = app.codex.request("thread/list",json!({"limit":100,"cursor":cursor,"archived":archived,"sortKey":"created_at","sourceKinds":["cli","vscode","exec","appServer","unknown"]})).await?;
            Page {
                sessions: v["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(codex_candidate)
                    .collect(),
                next: v["nextCursor"].as_str().map(str::to_string),
            }
        }
    };
    page.sessions.retain(|c| {
        !app.sessions
            .list()
            .iter()
            .any(|s| s.harness == c.harness && s.harness_session_id.as_deref() == Some(&c.id))
    });
    Ok(page)
}

pub async fn import(app: &App, harness: HarnessKind, id: &str) -> Result<Session> {
    uuid::Uuid::parse_str(id).context("invalid conversation id")?;
    if let Some(s) = app
        .sessions
        .list()
        .into_iter()
        .find(|s| s.harness == harness && s.harness_session_id.as_deref() == Some(id))
    {
        return Ok(s);
    }
    let (candidate, events, model, effort) = match harness {
        HarnessKind::Claude => {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || -> Result<_> {
                let path = claude_paths()
                    .into_iter()
                    .find(|p| p.file_stem().is_some_and(|s| s == id.as_str()))
                    .context("transcript not found")?;
                anyhow::ensure!(
                    std::fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
                    "Transcript exceeds 64 MB; open it in the terminal."
                );
                let candidate = claude_candidate(&path).context("missing transcript metadata")?;
                let mut events = vec![];
                let mut model = None;
                let mut seen = std::collections::HashSet::new();
                for (n, line) in BufReader::new(std::fs::File::open(path)?)
                    .lines()
                    .enumerate()
                {
                    let Ok(v) = serde_json::from_str::<Value>(&line?) else {
                        continue;
                    };
                    if v["isSidechain"] == true {
                        continue;
                    }
                    let key = v["uuid"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| n.to_string());
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    let ts = timestamp(&v["timestamp"]);
                    let m = &v["message"];
                    if v["type"] == "assistant" {
                        model = m["model"].as_str().map(str::to_string).or(model);
                    }
                    let text = content_text(&m["content"]);
                    if !text.is_empty() {
                        let body = if v["type"] == "user" {
                            Some(EventBody::UserMessage {
                                text,
                                client_id: key.clone(),
                                queued: false,
                                images: vec![],
                            })
                        } else if v["type"] == "assistant" {
                            Some(EventBody::AssistantMessage {
                                item_id: key.clone(),
                                text,
                            })
                        } else {
                            None
                        };
                        if let Some(b) = body {
                            events.push((ts, b));
                        }
                    }
                    for b in m["content"].as_array().into_iter().flatten() {
                        match b["type"].as_str().unwrap_or("") {
                            "tool_use" => events.push((
                                ts,
                                EventBody::ToolCall {
                                    item_id: b["id"].as_str().unwrap_or(&key).into(),
                                    name: b["name"].as_str().unwrap_or("Tool").into(),
                                    summary: shorten(&b["input"].to_string(), 160),
                                    input: b["input"].clone(),
                                },
                            )),
                            "tool_result" => events.push((
                                ts,
                                EventBody::ToolResult {
                                    item_id: b["tool_use_id"].as_str().unwrap_or(&key).into(),
                                    output: crate::sessions::truncate_output(
                                        &text_of(&b["content"]),
                                        16000,
                                    ),
                                    is_error: b["is_error"] == true,
                                    exit_code: None,
                                    duration_ms: None,
                                },
                            )),
                            "thinking" => events.push((
                                ts,
                                EventBody::Reasoning {
                                    item_id: format!("{key}:thinking"),
                                    text: b["thinking"].as_str().unwrap_or("").into(),
                                },
                            )),
                            _ => {}
                        }
                    }
                }
                Ok((candidate, events, model, None))
            })
            .await??
        }
        HarnessKind::Codex => {
            let v = app
                .codex
                .request("thread/read", json!({"threadId":id,"includeTurns":true}))
                .await?;
            let t = &v["thread"];
            anyhow::ensure!(
                t["status"]["type"] != "active",
                "This conversation is active; finish it in the other client first."
            );
            let candidate = codex_candidate(t).context("missing thread metadata")?;
            let mut events = vec![];
            for turn in t["turns"].as_array().into_iter().flatten() {
                let ts = turn["startedAt"]
                    .as_i64()
                    .map(|n| n * 1000)
                    .unwrap_or(candidate.started_at);
                for item in turn["items"].as_array().into_iter().flatten() {
                    let key = item["id"].as_str().unwrap_or("").to_string();
                    let body = match item["type"].as_str().unwrap_or("") {
                        "userMessage" => Some(EventBody::UserMessage {
                            text: content_text(&item["content"]),
                            client_id: key,
                            queued: false,
                            images: vec![],
                        }),
                        "agentMessage" => Some(EventBody::AssistantMessage {
                            item_id: key,
                            text: item["text"].as_str().unwrap_or("").into(),
                        }),
                        "reasoning" => Some(EventBody::Reasoning {
                            item_id: key,
                            text: item["summary"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        }),
                        "fileChange" => {
                            for change in item["changes"].as_array().into_iter().flatten() {
                                events.push((
                                    ts,
                                    EventBody::FileChange {
                                        item_id: key.clone(),
                                        path: change["path"].as_str().unwrap_or("").into(),
                                        change: change["kind"]["type"]
                                            .as_str()
                                            .unwrap_or("update")
                                            .into(),
                                        diff: change["diff"].as_str().unwrap_or("").into(),
                                    },
                                ));
                            }
                            None
                        }
                        "mcpToolCall" | "dynamicToolCall" => {
                            events.push((
                                ts,
                                EventBody::ToolCall {
                                    item_id: key.clone(),
                                    name: item["tool"].as_str().unwrap_or("Tool").into(),
                                    summary: shorten(&item["arguments"].to_string(), 160),
                                    input: item["arguments"].clone(),
                                },
                            ));
                            Some(EventBody::ToolResult {
                                item_id: key,
                                output: crate::sessions::truncate_output(
                                    &text_of(
                                        item.get("result")
                                            .or(item.get("contentItems"))
                                            .unwrap_or(&Value::Null),
                                    ),
                                    16000,
                                ),
                                is_error: item["status"] == "failed",
                                exit_code: None,
                                duration_ms: item["durationMs"].as_i64(),
                            })
                        }
                        "commandExecution" => {
                            events.push((
                                ts,
                                EventBody::ToolCall {
                                    item_id: key.clone(),
                                    name: "Shell".into(),
                                    summary: shorten(item["command"].as_str().unwrap_or(""), 160),
                                    input: json!({"command":item["command"]}),
                                },
                            ));
                            Some(EventBody::ToolResult {
                                item_id: key,
                                output: crate::sessions::truncate_output(
                                    item["aggregatedOutput"].as_str().unwrap_or(""),
                                    16000,
                                ),
                                is_error: item["status"] == "failed",
                                exit_code: item["exitCode"].as_i64().map(|n| n as i32),
                                duration_ms: item["durationMs"].as_i64(),
                            })
                        }
                        _ => None,
                    };
                    if let Some(b) = body {
                        events.push((ts, b));
                    }
                }
            }
            (
                candidate,
                events,
                t["model"].as_str().map(str::to_string),
                t["reasoningEffort"].as_str().map(str::to_string),
            )
        }
    };
    let project = crate::projects::for_path(app, std::path::Path::new(&candidate.cwd)).await?;
    let now = now_ms();
    let prompt = events
        .iter()
        .find_map(|(_, b)| {
            if let EventBody::UserMessage { text, .. } = b {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        harness,
        harness_session_id: Some(candidate.id),
        project_id: project.map(|p| p.id),
        cwd: candidate.cwd,
        title: candidate.title,
        title_pending: false,
        status: SessionStatus::Closed,
        status_since: now,
        live: false,
        open_question: false,
        model,
        effort,
        prompt,
        usage: Usage::default(),
        started_at: candidate.started_at,
        ended_at: Some(now),
        last_seq: 0,
        launch_spec: LaunchSpec::default(),
        quota_pause: None,
    };
    app.sessions.import_history(app, s, events).await
}

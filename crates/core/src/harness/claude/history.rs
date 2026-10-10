//! Native Claude history.
use crate::{
    App,
    history::{Candidate, Page, Snapshot, content_text, timestamp},
    model::*,
    sessions::{shorten, text_of},
    util::now_ms,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
};
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
        harness: crate::harness::claude::ID,
        cwd: cwd?,
        title: shorten(&prompt.unwrap_or_else(|| "Claude conversation".into()), 80),
        started_at: started.unwrap_or_else(now_ms),
    })
}

pub async fn list(_app: &App, cursor: Option<String>, _archived: bool) -> Result<Page> {
    Ok({
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
        let next =
            (start.saturating_add(100) < all.len()).then(|| start.saturating_add(100).to_string());
        Page {
            sessions: all.into_iter().skip(start).take(100).collect(),
            next,
        }
    })
}
pub async fn read(_app: &App, id: &str) -> Result<Snapshot> {
    uuid::Uuid::parse_str(id).context("invalid conversation id")?;
    Ok({
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
    })
}

//! Native Codex history.
use super::rpc::CodexServer;
use crate::{
    history::{Candidate, Page, Snapshot, content_text},
    model::*,
    sessions::{shorten, text_of},
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
fn codex_candidate(v: &Value) -> Option<Candidate> {
    Some(Candidate {
        id: v["id"].as_str()?.into(),
        harness: crate::harness::codex::ID,
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

pub async fn list(server: &CodexServer, cursor: Option<String>, archived: bool) -> Result<Page> {
    Ok({
        let v = server.request("thread/list",json!({"limit":100,"cursor":cursor,"archived":archived,"sortKey":"created_at","sourceKinds":["cli","vscode","exec","appServer","unknown"]})).await?;
        Page {
            sessions: v["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(codex_candidate)
                .collect(),
            next: v["nextCursor"].as_str().map(str::to_string),
        }
    })
}
pub async fn read(server: &CodexServer, id: &str) -> Result<Snapshot> {
    uuid::Uuid::parse_str(id).context("invalid conversation id")?;
    Ok({
        let v = server
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
    })
}

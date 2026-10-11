//! OpenCode registration. Its native configuration/authentication remains owned by OpenCode.
use crate::{
    App,
    model::{HarnessKind, ModelInfo, Session},
    sessions::{LiveSession, Sink, StartSpec},
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

pub const ID: HarnessKind = HarnessKind::new("opencode");
pub const REGISTRATION: super::Registration = super::Registration {
    descriptor: super::HarnessDescriptor {
        id: ID,
        name: "OpenCode",
        icon_svg: include_str!("icons/opencode.svg"),
        color: "#4b5563",
        working_message: "OpenCode is working. Wait for this turn to finish before sending another message.",
        capabilities: super::HarnessCapabilities {
            images: true,
            messages_while_running: false,
            terminal: true,
            quota: false,
            cost: true,
        },
    },
    create: |_| Arc::new(Adapter),
    environment_marker: |_key| false,
};
pub struct Adapter;
#[async_trait]
impl super::Harness for Adapter {
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let mut command = crate::procs::child_command(&app.env, "opencode", Some(&spec.cwd));
        command
            .arg("acp")
            .env("LAUNCHPAD_PROFILE", app.paths.profile.name())
            .env("LAUNCHPAD_SESSION_ID", &spec.session_id)
            .envs(spec.env.iter().cloned());
        sink.raw("cmd", "opencode acp").await;
        super::acp::start(command, spec, sink, super::acp::Extensions {
            capabilities: serde_json::json!({"_meta":{"opencode/child-session-updates":true}}),
            child_update,
            tool_child: |tool| tool["rawInput"]["subagent_type"].is_string(),
        }).await
    }
    async fn models(&self, app: &App) -> Result<Vec<ModelInfo>> {
        let output = crate::procs::output(&app.env, "opencode", &["models"], None).await?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && line.contains('/') && !line.contains(' '))
            .map(|id| ModelInfo {
                id: id.into(),
                display_name: id.into(),
                description: String::new(),
                efforts: vec![],
            })
            .collect())
    }
    async fn version(&self, app: &App) -> Result<String> {
        crate::procs::output(&app.env, "opencode", &["--version"], None).await
    }
    fn terminal_resume_markers(&self) -> &'static [&'static str] {
        &["--session", "-s"]
    }
    fn terminal_command(&self, session: &Session, _is_temp: bool) -> Result<Vec<String>> {
        Ok(vec![
            "opencode".into(),
            "--session".into(),
            session
                .harness_session_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("the session has no conversation yet"))?,
        ])
    }
}

/// Native ACP extension; older versions project attributed child updates into session/update.
fn child_update(message: &serde_json::Value) -> Option<super::acp::ChildUpdate> {
    let p = &message["params"];
    if message["method"] == "opencode/session/child_update" {
        Some(super::acp::ChildUpdate {
            id: p["childSessionId"].as_str()?.into(),
            parent: p["parentSessionId"].as_str().filter(|id| Some(*id) != p["rootSessionId"].as_str()).map(str::to_string),
            name: p["title"].as_str().map(str::to_string),
            status: p["status"].as_str().map(|s| if s == "created" { "running" } else { s }.into()),
            update: p.get("update").cloned(),
            error: p["error"]["message"].as_str().map(str::to_string),
        })
    } else if message["method"] == "session/update" {
        let meta = &p["update"]["_meta"]["opencode/child-session"];
        Some(super::acp::ChildUpdate {
            id: meta["id"].as_str()?.into(),
            parent: meta["parentID"].as_str().filter(|id| Some(*id) != p["sessionId"].as_str()).map(str::to_string),
            name: meta["title"].as_str().map(str::to_string),
            status: None, update: Some(p["update"].clone()), error: None,
        })
    } else { None }
}

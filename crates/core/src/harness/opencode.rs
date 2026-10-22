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
        browser_isolation: "Native tools restricted by OpenCode permissions; not an OS sandbox",
        cost_model_prefixes: &[],
        id: ID,
        name: "OpenCode",
        icon_svg: include_str!("icons/opencode.svg"),
        color: "#4b5563",
        working_message: "OpenCode is working. Wait for this turn to finish before sending another message.",
        capabilities: super::HarnessCapabilities {
            permission_modes: false,
            images: true,
            messages_while_running: false,
            terminal: true,
            quota: false,
            cost: true,
        },
        account_env: &[],
        env_vars: &[],
        profile_templates: &[super::ProfileTemplate {
            kind: super::ProfileKind::Provider,
            name: "OpenCode",
            description: "The providers and models configured in OpenCode itself (opencode auth login), billed by each provider.",
            setup: None,
            setup_command: None,
            env: &[],
            billing: crate::config::Billing::PayAsYouGo,
            models_url: None,
        }],
    },
    create: |_| Arc::new(Adapter),
    environment_marker: |_key| false,
};
pub struct Adapter;
#[async_trait]
impl super::Harness for Adapter {
    fn sign_in_program(&self) -> &'static str {
        "opencode"
    }
    fn sign_in_command(&self) -> &'static str {
        "opencode auth login"
    }
    fn install_command(&self) -> Option<&'static str> {
        Some("curl -fsSL https://opencode.ai/install | bash")
    }
    /// `opencode auth list` ends its sections with "N credentials" and "N environment variables";
    /// only those counts are kept (provider names are not secrets, values are never shown by OpenCode).
    async fn sign_in(&self, env: &crate::shellenv::UserEnv) -> crate::signin::SignIn {
        match crate::signin::status_output(env, "opencode", &["auth", "list"]).await {
            Ok((_, stdout, stderr)) => {
                let text = crate::signin::strip_ansi(&format!("{stdout}\n{stderr}"));
                let count = |suffix: &str| text.lines().filter_map(|l| {
                    let l = l.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).trim();
                    let (n, rest) = l.split_once(' ')?;
                    rest.starts_with(suffix).then(|| n.parse::<u32>().ok()).flatten()
                }).next();
                let (credentials, variables) = (count("credential"), count("environment variable"));
                if credentials.is_none() && variables.is_none() {
                    return crate::signin::SignIn { signed_in: None, detail: Some("Could not read `opencode auth list`.".into()) };
                }
                let (c, v) = (credentials.unwrap_or(0), variables.unwrap_or(0));
                crate::signin::SignIn {
                    signed_in: Some(c + v > 0),
                    detail: Some(format!("{c} stored credential{}, {v} provider environment variable{}", if c == 1 { "" } else { "s" }, if v == 1 { "" } else { "s" })),
                }
            }
            Err(e) => crate::signin::SignIn { signed_in: None, detail: Some(format!("{e:#}")) },
        }
    }
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let mut command = crate::procs::child_command(&app.env.on(spec.host.as_deref()), "opencode", Some(&spec.cwd));
        command
            .arg("acp")
            .env("LAUNCHPAD_ENV", app.paths.app_env.name())
            .env("LAUNCHPAD_SESSION_ID", &spec.session_id)
            .envs(spec.env.iter().cloned());
        let mcp_servers = if spec.browser {
            let mcp = crate::browser::mcp_config(app, &spec);
            // Native config is merged. Explicitly disable every existing server and permission key,
            // rather than assuming an empty object erases the owner's configuration.
            let env = app.env.on(spec.host.as_deref()).with(&spec.env);
            let raw = crate::procs::output(&env, "opencode", &["debug", "config"], Some(&spec.cwd)).await?;
            let current: serde_json::Value = serde_json::from_str(&raw)?;
            let mut permission = serde_json::Map::new();
            permission.insert("*".into(), serde_json::json!("deny"));
            if let Some(existing) = current["permission"].as_object() {
                for key in existing.keys().filter(|k| !k.starts_with("launchpad_browser")) { permission.insert(key.clone(), serde_json::json!("deny")); }
            }
            permission.insert("launchpad_browser_*".into(), serde_json::json!("allow"));
            let mut servers = serde_json::Map::new();
            if let Some(existing) = current["mcp"].as_object() { for key in existing.keys() { servers.insert(key.clone(), serde_json::json!({"enabled":false})); } }
            command.env("OPENCODE_CONFIG_CONTENT", serde_json::json!({"permission":permission,"mcp":servers}).to_string());
            serde_json::json!([{"name":"launchpad_browser","command":mcp["command"],"args":mcp["args"],
                "env":mcp["env"].as_object().unwrap().iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>()}])
        } else { serde_json::json!([]) };
        sink.raw("cmd", "opencode acp").await;
        super::acp::start(command, spec, sink, super::acp::Extensions {
            mcp_servers,
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
                providers: vec![],
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

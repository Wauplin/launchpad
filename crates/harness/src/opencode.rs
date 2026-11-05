//! OpenCode registration. Its native configuration/authentication remains owned by OpenCode.
use launchpad_core::{
    App, harness,
    model::{HarnessKind, ModelInfo, Session},
    sessions::{LiveSession, Sink, StartSpec},
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

pub const ID: HarnessKind = HarnessKind::new("opencode");
pub const REGISTRATION: harness::Registration = harness::Registration {
    descriptor: harness::HarnessDescriptor {
        browser_isolation: "Native tools restricted by OpenCode permissions; not an OS sandbox",
        cost_model_prefixes: &[],
        id: ID,
        name: "OpenCode",
        icon_svg: include_str!("icons/opencode.svg"),
        color: "#4b5563",
        working_message: "OpenCode is working. Wait for this turn to finish before sending another message.",
        capabilities: harness::HarnessCapabilities {
            permission_modes: false,
            images: true,
            messages_while_running: false,
            terminal: true,
            quota: false,
            cost: true,
        },
        account_env: &[],
        vault_login: None,
        env_vars: &[],
        profile_templates: &[harness::ProfileTemplate {
            kind: harness::ProfileKind::Provider,
            name: "OpenCode",
            description: "The providers and models configured in OpenCode itself (opencode auth login), billed by each provider.",
            setup: None,
            setup_command: None,
            env: &[],
            billing: launchpad_core::config::Billing::PayAsYouGo,
            models_url: None,
        }],
    },
    create: |_| Arc::new(Adapter),
    environment_marker: |_key| false,
};
pub struct Adapter;
#[async_trait]
impl harness::Harness for Adapter {
    fn contained_memory(&self, home: &std::path::Path, instructions: &str) -> Result<Vec<(String, String)>> {
        let dir = home.join(".config/opencode");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("AGENTS.md"), instructions)?;
        Ok(vec![])
    }
    fn skill_locations(&self) -> Vec<launchpad_core::prompt_context::SkillLocation> {
        use launchpad_core::prompt_context::SkillLocation;
        vec![SkillLocation::project(".opencode/skills"), SkillLocation::project(".agents/skills"), SkillLocation::project(".claude/skills"), SkillLocation::config("XDG_CONFIG_HOME", ".config", "opencode/skills"), SkillLocation::home(".agents/skills"), SkillLocation::home(".claude/skills")]
    }
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
    async fn sign_in(&self, env: &launchpad_core::shellenv::UserEnv) -> launchpad_core::signin::SignIn {
        match launchpad_core::signin::status_output(env, "opencode", &["auth", "list"]).await {
            Ok((_, stdout, stderr)) => {
                let text = launchpad_core::signin::strip_ansi(&format!("{stdout}\n{stderr}"));
                let count = |suffix: &str| text.lines().filter_map(|l| {
                    let l = l.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).trim();
                    let (n, rest) = l.split_once(' ')?;
                    rest.starts_with(suffix).then(|| n.parse::<u32>().ok()).flatten()
                }).next();
                let (credentials, variables) = (count("credential"), count("environment variable"));
                if credentials.is_none() && variables.is_none() {
                    return launchpad_core::signin::SignIn { signed_in: None, detail: Some("Could not read `opencode auth list`.".into()) };
                }
                let (c, v) = (credentials.unwrap_or(0), variables.unwrap_or(0));
                launchpad_core::signin::SignIn {
                    signed_in: Some(c + v > 0),
                    detail: Some(format!("{c} stored credential{}, {v} provider environment variable{}", if c == 1 { "" } else { "s" }, if v == 1 { "" } else { "s" })),
                }
            }
            Err(e) => launchpad_core::signin::SignIn { signed_in: None, detail: Some(format!("{e:#}")) },
        }
    }
    async fn start(&self, app: &App, spec: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let mut command = launchpad_core::procs::child_command(&spec.process_env(&app.env), "opencode", Some(&spec.cwd));
        command
            .arg("acp")
            .env("LAUNCHPAD_ENV", app.paths.app_env.name())
            .env("LAUNCHPAD_SESSION_ID", &spec.session_id)
            .envs(spec.env.iter().cloned());
        let mcp_servers = if spec.browser {
            let mcp = launchpad_core::browser::mcp_config(app, &spec);
            // Native config is merged. Explicitly disable every existing server and permission key,
            // rather than assuming an empty object erases the owner's configuration.
            let env = spec.process_env(&app.env);
            let raw = launchpad_core::procs::output(&env, "opencode", &["debug", "config"], Some(&spec.cwd)).await?;
            let current: serde_json::Value = serde_json::from_str(&raw)?;
            let mut permission = serde_json::Map::new();
            permission.insert("*".into(), serde_json::json!("deny"));
            if let Some(existing) = current["permission"].as_object() {
                for key in existing.keys().filter(|k| !k.starts_with("launchpad_browser")) { permission.insert(key.clone(), serde_json::json!("deny")); }
            }
            permission.insert("launchpad_browser_*".into(), serde_json::json!("allow"));
            let mut servers = serde_json::Map::new();
            if let Some(existing) = current["mcp"].as_object() { for key in existing.keys() { servers.insert(key.clone(), serde_json::json!({"enabled":false})); } }
            command.env("OPENCODE_CONFIG_CONTENT", serde_json::json!({"permission":permission,"mcp":servers,"experimental":{"mcp_timeout":720000}}).to_string());
            serde_json::json!([{"name":"launchpad_browser","command":mcp["command"],"args":mcp["args"],
                "env":mcp["env"].as_object().unwrap().iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>()}])
        } else if spec.browser_tools {
            let mcp = launchpad_core::browser::mcp_config(app, &spec);
            command.env("OPENCODE_CONFIG_CONTENT", serde_json::json!({"permission":{"launchpad_browser_*":"allow"},"experimental":{"mcp_timeout":720000}}).to_string());
            serde_json::json!([{"name":"launchpad_browser","command":mcp["command"],"args":mcp["args"],
                "env":mcp["env"].as_object().unwrap().iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>()}])
        } else { serde_json::json!([]) };
        sink.raw("cmd", "opencode acp").await;
        crate::acp::start(command, spec, sink, crate::acp::Extensions {
            mcp_servers,
            capabilities: serde_json::json!({"_meta":{"opencode/child-session-updates":true}}),
            child_update,
            tool_child: |tool| tool["rawInput"]["subagent_type"].is_string(),
        }).await
    }
    async fn native_trace(&self, app: &App, session: &Session) -> Result<Vec<harness::trace::TraceFile>> {
        let id = harness::trace::conversation(session)?;
        let env = harness::trace::environment(app, session)?;
        export_trace(&env, id).await
    }
    async fn models(&self, app: &App) -> Result<Vec<ModelInfo>> {
        let output = launchpad_core::procs::output(&app.env, "opencode", &["models"], None).await?;
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
        launchpad_core::procs::output(&app.env, "opencode", &["--version"], None).await
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

async fn export_trace(env: &launchpad_core::shellenv::UserEnv, id: &str) -> Result<Vec<harness::trace::TraceFile>> {
    let raw = launchpad_core::procs::output(env, "opencode", &["export", id], None).await?;
    anyhow::ensure!(raw.len() <= 256 * 1024 * 1024, "native OpenCode export exceeds 256 MiB");
    // Validate, but retain the native export exactly rather than wrapping its messages.
    let _: serde_json::Value = serde_json::from_str(&raw)?;
    Ok(vec![harness::trace::TraceFile { name: format!("{id}.json"), data: raw.into_bytes() }])
}

/// Native ACP extension; older versions project attributed child updates into session/update.
fn child_update(message: &serde_json::Value) -> Option<crate::acp::ChildUpdate> {
    let p = &message["params"];
    if message["method"] == "opencode/session/child_update" {
        Some(crate::acp::ChildUpdate {
            id: p["childSessionId"].as_str()?.into(),
            parent: p["parentSessionId"].as_str().filter(|id| Some(*id) != p["rootSessionId"].as_str()).map(str::to_string),
            name: p["title"].as_str().map(str::to_string),
            status: p["status"].as_str().map(|s| if s == "created" { "running" } else { s }.into()),
            update: p.get("update").cloned(),
            error: p["error"]["message"].as_str().map(str::to_string),
        })
    } else if message["method"] == "session/update" {
        let meta = &p["update"]["_meta"]["opencode/child-session"];
        Some(crate::acp::ChildUpdate {
            id: meta["id"].as_str()?.into(),
            parent: meta["parentID"].as_str().filter(|id| Some(*id) != p["sessionId"].as_str()).map(str::to_string),
            name: meta["title"].as_str().map(str::to_string),
            status: None, update: Some(p["update"].clone()), error: None,
        })
    } else { None }
}

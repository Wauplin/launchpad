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
            voice: false,
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
        // The harness home is agent-writable: nothing in it is followed if it is a link (`safe_fs`).
        launchpad_core::safe_fs::write(home, std::path::Path::new(".config/opencode/AGENTS.md"), instructions.as_bytes(), None)?;
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
    /// The install script puts it in `~/.opencode/bin/`; `opencode upgrade <version>` replaces it.
    fn cli_package(&self) -> Option<launchpad_core::harness::CliPackage> {
        Some(launchpad_core::harness::CliPackage { npm: "opencode-ai", program: "opencode", native: Some(("/.opencode/bin/", &["upgrade", "{version}"])), changelog: "https://github.com/sst/opencode/releases/tag/v{version}" })
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
        if let Some(id) = spec.harness_session_id.as_deref().filter(|_| spec.resume) {
            import_pending(app, &spec, id).await?;
        }
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
        // Moved here but not started since: OpenCode does not have it yet, the waiting export is the trace.
        let pending = pending_import(app, &session.id, id);
        // The harness home is agent-writable: only a regular file, never through a link (`safe_fs`).
        let (base, rel) = launchpad_core::safe_fs::anchor(&app.paths.data_dir, &pending);
        match launchpad_core::safe_fs::read_async(base, rel, 256 * 1024 * 1024).await {
            Ok(data) => return Ok(vec![harness::trace::TraceFile { name: format!("{id}.json"), data }]),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let env = harness::trace::environment(app, session)?;
        export_trace(&env, id).await
    }
    /// OpenCode keeps conversations in its own database: the unchanged `opencode export` file waits in the
    /// session's harness home, and the next start imports it with OpenCode's own `opencode import`, in the
    /// session's jail and folder (OpenCode files the conversation under the folder it runs in).
    async fn restore_native(&self, app: &App, session: &Session, files: Vec<harness::trace::TraceFile>) -> Result<()> {
        let id = harness::trace::conversation(session)?;
        let file = files.into_iter().find(|f| f.name == format!("{id}.json")).ok_or_else(|| anyhow::anyhow!("no OpenCode export of this conversation"))?;
        let _: serde_json::Value = serde_json::from_slice(&file.data).map_err(|e| anyhow::anyhow!("the OpenCode export of this conversation is not JSON: {e}"))?;
        let path = pending_import(app, &session.id, id);
        let (base, rel) = launchpad_core::safe_fs::anchor(&app.paths.data_dir, &path);
        launchpad_core::safe_fs::write(&base, &rel, &file.data, None)?;
        Ok(())
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

/// Where a moved conversation's export waits for the next start (`restore_native`).
fn pending_import(app: &App, session_id: &str, conversation: &str) -> std::path::PathBuf {
    launchpad_core::contain::harness_home(app, session_id).join(".launchpad").join(format!("opencode-import-{conversation}.json"))
}

/// Imports a waiting export before OpenCode resumes it, in the session's jail and folder. OpenCode's import
/// says "Imported session" on success and exits 0 even when it read nothing, so anything else fails the start
/// (the file stays: importing again is harmless).
async fn import_pending(app: &App, spec: &StartSpec, id: &str) -> Result<()> {
    let path = pending_import(app, &spec.session_id, id);
    if !path.exists() {
        return Ok(());
    }
    let file = path.to_string_lossy().into_owned();
    let out = launchpad_core::procs::output(&spec.process_env(&app.env), "opencode", &["import", "--pure", file.as_str()], Some(&spec.cwd))
        .await
        .map_err(|e| anyhow::anyhow!("OpenCode could not import this moved conversation: {e:#}"))?;
    anyhow::ensure!(out.contains("Imported session"), "OpenCode could not import this moved conversation: {}", out.chars().take(500).collect::<String>());
    std::fs::remove_file(&path)?;
    Ok(())
}

async fn export_trace(env: &launchpad_core::shellenv::UserEnv, id: &str) -> Result<Vec<harness::trace::TraceFile>> {
    // Outside the jail, with the session's own OpenCode home: never with the plugins its configuration names.
    let raw = launchpad_core::procs::output(env, "opencode", &["export", "--pure", id], None).await?;
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

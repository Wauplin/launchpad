//! The vault on Home: credentials (harness logins and tokens), the tool registry, and the session
//! leases that let runners use them.
//!
//! Secrets are write-only: no API returns them. A runner receives the secrets of one session in its lease
//! (`runner_call` "lease", only through its tunnel) and keeps them in its proxy's memory (`proxy.rs`); agents
//! only see placeholders. A lease lives until Stop, Archive or Revoke; a runner that cannot renew it drops it
//! after an hour. Home refuses leases for a session that now runs elsewhere (generation numbers).

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::App;
use crate::model::HarnessKind;
use crate::util::now_ms;

/// A runner renews each lease every ten minutes and drops it after an hour without Home.
pub const LEASE_TTL_MS: i64 = 60 * 60 * 1000;
pub const RENEW_EVERY_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "VaultKind")]
pub enum Kind {
    /// A long-lived Claude Code token (`claude setup-token`).
    ClaudeLogin,
    /// A ChatGPT login of Codex (`auth.json`); only Home refreshes it.
    CodexLogin,
    /// A token or API key, used by tools through the proxy.
    Token,
}

impl Kind {
    /// The vault login kind of a harness (its adapter declares it).
    pub fn login_of(harness: HarnessKind) -> Option<Kind> {
        crate::harness::registration(harness).descriptor.vault_login.map(|l| l.kind)
    }
}

/// A vault entry as the UI sees it: never the secret.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct VaultEntry {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    /// Non-secret details: the last characters of a token, a key fingerprint, an account.
    pub detail: String,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// How the proxy adds a credential to requests for a host.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(rename = "ProxyAuth")]
pub enum Auth {
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `Authorization: Basic base64(<user>:<secret>)`, for Git over HTTPS.
    Basic { user: String },
    /// `<name>: <prefix><secret>`.
    Header { name: String, #[serde(default)] prefix: String },
    /// A query parameter.
    Query { param: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct HostRule {
    /// `api.github.com`, or `*.example.com` for every subdomain.
    pub host: String,
    pub auth: Auth,
}

impl HostRule {
    pub fn matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        match self.host.strip_prefix("*.") {
            Some(suffix) => host.ends_with(&format!(".{suffix}")),
            None => host == self.host.to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ToolInstallKind")]
pub enum InstallKind {
    /// A URL of a binary or a `.tar.gz`/`.zip` archive. `{arch}` becomes `amd64` or `arm64`, `{uarch}` `x86_64` or `aarch64`.
    Url,
    /// The newest release of `<owner>/<repo>` on GitHub; `asset` is a file name pattern (`*`, `{arch}`).
    GithubRelease,
    Cargo,
    /// `uv tool install`.
    Uv,
    Npm,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "ToolInstall")]
pub struct Install {
    pub kind: InstallKind,
    /// URL, `<owner>/<repo>`, crate or package name.
    pub source: String,
    /// GitHub release asset pattern.
    #[serde(default)]
    pub asset: String,
}

/// A CLI agents may use: how a runner installs it, the variables that receive a placeholder, and the hosts
/// where the proxy adds the credential.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Tool {
    /// Lowercase letters, digits and dashes.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub install: Option<Install>,
    /// The vault entry (kind Token) the proxy adds. Tools sharing a credential share its authority.
    #[serde(default)]
    pub credential: Option<String>,
    /// Variables set to the placeholder in the agent's environment, for example `GH_TOKEN`.
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<HostRule>,
    /// Workspaces where the tool is granted; `*` grants it everywhere.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// An HTTPS address on one of its hosts that answers 2xx with a valid token (Test in Settings).
    #[serde(default)]
    pub check_url: Option<String>,
    /// Settings that are not secret (an endpoint, a read-only switch, an output format): set only for the tool's
    /// own commands, through a wrapper on the agent's PATH (`tool_install::wrappers`), never in the session's
    /// environment. Needs an install.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// The result of Test: Home sends one GET to the tool's check URL with the token added.
#[derive(Clone, Debug, Serialize, TS)]
pub struct ToolTest {
    pub ok: bool,
    pub status: u16,
    pub message: String,
}

impl Tool {
    pub fn granted(&self, workspace: Option<&str>) -> bool {
        self.workspaces.iter().any(|w| w == "*" || Some(w.as_str()) == workspace)
    }
}

/// The value agents see instead of a secret.
pub fn placeholder(id: &str) -> String {
    format!("lp-placeholder-{id}")
}

/// The Launchpad CLI tool: sessions granted it are told what they may do with `launchpad`
/// (`launchpad_cli::cli_reference`) at their access level. It needs no install (the runner puts `launchpad` on PATH)
/// and no token (the session's own CLI token). Home adds it once, granted everywhere (`ask_bot::seed`).
pub const LAUNCHPAD_TOOL: &str = "launchpad";

/// What a session granted the Launchpad CLI tool may do. Bots set it in `bot.toml` (`launchpad`); other agents
/// get Propose. The server enforces it on the session CLI token (`agent_auth`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum LaunchpadAccess {
    /// Read Launchpad state: folders, bots, sessions and traces, profiles, presets, Inbox, schedules, proposals.
    Read,
    /// Read, and write inert proposals the owner starts with a click.
    #[default]
    Propose,
    /// Propose, and change Launchpad itself through its API: bots, routines, schedules, workspaces, folders,
    /// presets, the tool registry, launches and messages. Never GitHub, the Outbox, credentials, devices,
    /// terminals, hosts, paid usage or the machine (`agent_auth::admin_allows`).
    Admin,
}

pub fn launchpad_tool() -> Tool {
    Tool { id: LAUNCHPAD_TOOL.into(), name: "Launchpad CLI".into(), install: None, credential: None, env: vec![], hosts: vec![], workspaces: vec!["*".into()], check_url: None, vars: BTreeMap::new() }
}

/// Ready-made tools offered by Add tool. Only the Launchpad CLI is added on its own, once.
pub fn templates() -> Vec<Tool> {
    let rule = |host: &str, auth: Auth| HostRule { host: host.into(), auth };
    vec![
        launchpad_tool(),
        Tool {
            id: "gh".into(),
            name: "GitHub CLI".into(),
            install: Some(Install { kind: InstallKind::GithubRelease, source: "cli/cli".into(), asset: "gh_*_linux_{arch}.tar.gz".into() }),
            credential: Some("github".into()),
            env: vec!["GH_TOKEN".into(), "GITHUB_TOKEN".into()],
            hosts: vec![
                rule("api.github.com", Auth::Bearer),
                rule("uploads.github.com", Auth::Bearer),
                rule("github.com", Auth::Basic { user: "x-access-token".into() }),
            ],
            workspaces: vec!["*".into()],
            check_url: Some("https://api.github.com/user".into()),
            vars: BTreeMap::new(),
        },
        Tool {
            id: "hf".into(),
            name: "Hugging Face CLI".into(),
            install: Some(Install { kind: InstallKind::Uv, source: "huggingface_hub".into(), asset: String::new() }),
            credential: Some("huggingface".into()),
            env: vec!["HF_TOKEN".into()],
            hosts: vec![rule("huggingface.co", Auth::Bearer)],
            workspaces: vec!["*".into()],
            check_url: Some("https://huggingface.co/api/whoami-v2".into()),
            vars: BTreeMap::new(),
        },
        slack_tool(false),
        slack_tool(true),
    ]
}

/// Slack through slackline (github.com/co42/slackline), as two tools with their own Slack app and token: read-only
/// (`SLACKLINE_READONLY` hides and refuses every write command, and the token has read scopes only) and write. A
/// launch gets one of them: both use slack.com, and a lease keeps only the first tool of a host (`host_conflict`).
pub fn slack_tool(write: bool) -> Tool {
    let rule = |host: &str| HostRule { host: host.into(), auth: Auth::Bearer };
    let (id, name) = if write { (SLACK_WRITE, "Slack (write)") } else { (SLACK_READ, "Slack (read-only)") };
    Tool {
        id: id.into(),
        name: name.into(),
        install: Some(Install { kind: InstallKind::GithubRelease, source: "co42/slackline".into(), asset: "slackline-*-{uarch}-unknown-linux-gnu.tar.gz".into() }),
        credential: Some(id.into()),
        env: vec!["SLACK_TOKEN".into()],
        hosts: vec![rule("slack.com"), rule("files.slack.com")],
        workspaces: vec!["*".into()],
        check_url: Some("https://slack.com/api/auth.test".into()),
        vars: if write { BTreeMap::new() } else { BTreeMap::from([("SLACKLINE_READONLY".to_string(), "1".to_string())]) },
    }
}

pub const SLACK_READ: &str = "slack-read";
pub const SLACK_WRITE: &str = "slack-write";

/// The Slack app pages of `slackline token create [--write]`: Slack's "Create app" with the app manifest filled in
/// (user scopes only; the write app adds the write scopes).
fn slack_app_url(write: bool) -> String {
    let read = ["channels:history", "channels:read", "files:read", "groups:history", "groups:read", "im:history", "im:read", "mpim:history", "mpim:read", "pins:read", "reactions:read", "search:read", "users:read", "users:read.email"];
    let writes = ["chat:write", "files:write", "im:write", "pins:write", "reactions:write", "users.profile:write"];
    let scopes: Vec<&str> = read.iter().chain(if write { writes.iter() } else { [].iter() }).copied().collect();
    let tag = if write { "rw" } else { "ro" };
    let manifest = json!({
        "display_information": {"background_color": "#4a154b", "description": format!("Slack CLI {tag}"), "name": format!("Slackline {tag}")},
        "oauth_config": {"scopes": {"user": scopes}},
        "settings": {"org_deploy_enabled": false, "socket_mode_enabled": false, "token_rotation_enabled": false},
    });
    // Percent-encode everything but unreserved characters, like `slackline token create`.
    let encoded: String = manifest.to_string().bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("https://api.slack.com/apps?new_app=1&manifest_json={encoded}")
}

/// A login of Home itself whose token can be copied into the vault (Home's own `gh` and `hf` logins).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HomeToken {
    Github,
    Huggingface,
}

/// Help shown next to a template's token field.
#[derive(Clone, Debug, Serialize, TS)]
pub struct TemplateHelp {
    /// The template's tool ID.
    pub id: String,
    /// Where the owner creates the token.
    pub token_url: String,
    /// Home's own login that can fill the token instead.
    pub home_token: Option<HomeToken>,
    /// The label of the `token_url` link.
    pub link_label: String,
    /// What to do on that page, step by step, when it is more than "create a token".
    pub steps: Vec<String>,
}

/// Token help of each template, by template ID.
pub fn template_help() -> Vec<TemplateHelp> {
    let help = |id: &str, url: &str, home: Option<HomeToken>| TemplateHelp { id: id.into(), token_url: url.into(), home_token: home, link_label: "Create a token".into(), steps: vec![] };
    let slack = |write: bool| TemplateHelp {
        id: if write { SLACK_WRITE } else { SLACK_READ }.into(),
        token_url: slack_app_url(write),
        home_token: None,
        link_label: "Open Slack to create the app".into(),
        steps: vec![
            "Open Slack: the app and its permissions are already filled in.".into(),
            "Pick your Slack workspace, then click Next and Create.".into(),
            "In the sidebar, open Install App, click Install to Workspace, then Allow.".into(),
            "Copy the User OAuth Token (it starts with xoxp-) and paste it here.".into(),
        ],
    };
    vec![
        help("gh", "https://github.com/settings/personal-access-tokens/new", Some(HomeToken::Github)),
        help("hf", "https://huggingface.co/settings/tokens/new?tokenType=fineGrained", Some(HomeToken::Huggingface)),
        slack(false),
        slack(true),
    ]
}

/// Home's own token for `source`: GitHub from `GH_TOKEN`/`GITHUB_TOKEN` or `gh auth token`, Hugging Face from
/// `HF_TOKEN` or the `hf` token file. Only on Home; the daemon inherits `GH_CONFIG_DIR` and `HF_HOME`.
pub async fn home_token(app: &App, source: HomeToken) -> Option<String> {
    if app.paths.role != crate::role::Role::Home {
        return None;
    }
    let env = |k: &str| app.env.get(k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    match source {
        HomeToken::Github => match env("GH_TOKEN").or_else(|| env("GITHUB_TOKEN")) {
            Some(t) => Some(t),
            None => crate::procs::output(&app.env, "gh", &["auth", "token", "--hostname", "github.com"], None).await.ok().filter(|t| !t.is_empty() && !t.contains(char::is_whitespace)),
        },
        HomeToken::Huggingface => env("HF_TOKEN").or_else(|| crate::profiles::hf_token(|k| app.env.get(k).map(str::to_string))),
    }
}

/// Home's logins whose token can fill a vault entry.
pub async fn home_tokens(app: &App) -> Vec<HomeToken> {
    let mut out = vec![];
    for source in [HomeToken::Github, HomeToken::Huggingface] {
        if home_token(app, source).await.is_some() {
            out.push(source);
        }
    }
    out
}

/// Stores Home's own token for `source` in the entry `id` (kind Token). The token is never returned.
pub async fn put_from_home(app: &App, id: &str, name: &str, source: HomeToken) -> Result<VaultEntry> {
    anyhow::ensure!(app.paths.role == crate::role::Role::Home, "only Home can lend its own login");
    let what = match source {
        HomeToken::Github => "GitHub (gh auth login on Home)",
        HomeToken::Huggingface => "Hugging Face (hf auth login on Home)",
    };
    let token = home_token(app, source).await.with_context(|| format!("Home has no {what} token"))?;
    put(app, id, Kind::Token, name, &token).await
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 60 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_env(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Checks a tool before it is saved.
pub fn check_tool(t: &Tool) -> Result<()> {
    anyhow::ensure!(valid_id(&t.id), "a tool ID uses lowercase letters, digits and dashes");
    anyhow::ensure!(!t.name.trim().is_empty(), "a tool needs a name");
    // Channel secrets (webhook URLs, the Telegram token) are used by Home only, never lent to sessions.
    anyhow::ensure!(!t.credential.as_deref().is_some_and(|c| c.starts_with("channel-")), "channel-… entries belong to Settings → Channels and cannot be lent to tools");
    for v in &t.env {
        anyhow::ensure!(valid_env(v), "{v:?} is not a variable name");
        anyhow::ensure!(!RESERVED_ENV.contains(&v.as_str()), "{v} is set by Launchpad");
    }
    for h in &t.hosts {
        let host = h.host.strip_prefix("*.").unwrap_or(&h.host);
        anyhow::ensure!(!host.is_empty() && host.contains('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'), "{:?} is not a host name", h.host);
        if let Auth::Header { name, .. } = &h.auth {
            anyhow::ensure!(!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{name:?} is not a header name");
        }
        if let Auth::Query { param } = &h.auth {
            anyhow::ensure!(!param.is_empty(), "a query parameter needs a name");
        }
    }
    for (k, v) in &t.vars {
        anyhow::ensure!(valid_env(k), "{k:?} is not a variable name");
        anyhow::ensure!(!RESERVED_ENV.contains(&k.as_str()) && !matches!(k.as_str(), "PATH" | "HOME" | "LD_PRELOAD" | "LD_LIBRARY_PATH"), "{k} cannot be set for a tool");
        anyhow::ensure!(!t.env.contains(k), "{k} already holds the token's placeholder");
        anyhow::ensure!(v.len() <= 2000 && !v.contains(['\n', '\r', '\0']), "the value of {k} must be one line");
    }
    anyhow::ensure!(t.vars.is_empty() || t.install.is_some(), "settings are set when the tool's commands run: the tool needs an install");
    if let Some(i) = &t.install {
        anyhow::ensure!(!i.source.trim().is_empty(), "the install needs a source");
        anyhow::ensure!(i.source.chars().all(|c| !c.is_whitespace() && c != '\'' && c != '"'), "the install source cannot contain spaces or quotes");
        if i.kind == InstallKind::Url {
            anyhow::ensure!(i.source.starts_with("https://"), "an install URL starts with https://");
        }
        if i.kind == InstallKind::GithubRelease {
            anyhow::ensure!(i.source.split('/').count() == 2 && !i.asset.is_empty(), "a GitHub release needs <owner>/<repo> and an asset pattern");
        }
    }
    Ok(())
}

/// Variables Launchpad sets in every contained session; tools cannot take them over.
pub const RESERVED_ENV: &[&str] = &["HOME", "PATH", "HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy", "NO_PROXY", "no_proxy", "SSH_AUTH_SOCK", "CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_BASE_URL", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "LAUNCHPAD_SESSION_ID", "LAUNCHPAD_TOKEN"];

/// Settings → Credentials and tools.
#[derive(Clone, Debug, Serialize, TS)]
pub struct VaultOverview {
    pub entries: Vec<VaultEntry>,
    pub tools: Vec<Tool>,
    pub templates: Vec<Tool>,
    /// Token help of the templates, by template ID.
    pub template_help: Vec<TemplateHelp>,
    /// Home's own logins that can fill a token (only on Home).
    pub home_tokens: Vec<HomeToken>,
    /// Harness logins the vault can keep.
    pub logins: Vec<LoginKind>,
    pub git_name: String,
    pub git_email: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "VaultLoginKind")]
pub struct LoginKind {
    pub harness: String,
    pub name: String,
    pub kind: Kind,
    /// How the owner gets the secret.
    pub how: String,
}

// ---------------------------------------------------------------- storage (Home)

fn detail_of(kind: Kind, secret: &str) -> String {
    match kind {
        Kind::CodexLogin => codex_claims(secret)
            .map(|c| {
                let auth = &c["https://api.openai.com/auth"];
                let plan = auth["chatgpt_plan_type"].as_str().unwrap_or("");
                let email = c["email"].as_str().unwrap_or("");
                format!("{email} {plan}").trim().to_string()
            })
            .unwrap_or_else(|_| "ChatGPT login".into()),
        _ => {
            let t = secret.trim();
            let tail: String = t.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            format!("…{tail}")
        }
    }
}

/// Every entry, without secrets. Channel secrets (`channel-…`, webhook URLs and the Telegram token) belong to
/// Settings → Channels and are not listed here.
pub async fn entries(app: &App) -> Result<Vec<VaultEntry>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT id, kind, name, detail, updated_at FROM vault WHERE id NOT LIKE 'channel-%' AND id NOT LIKE 'mcp-%' ORDER BY name")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows
                .into_iter()
                .filter_map(|(id, kind, name, detail, updated_at)| Some(VaultEntry { id, kind: serde_json::from_value(Value::String(kind)).ok()?, name, detail, updated_at }))
                .collect())
        })
        .await
}

/// Home: Test of a saved tool. The token never leaves Home; only the status comes back.
pub async fn test_tool(app: &App, id: &str) -> Result<ToolTest> {
    let tool = tools(app).await?.into_iter().find(|t| t.id == id).context("no such tool")?;
    let url = tool.check_url.as_deref().map(str::trim).filter(|u| !u.is_empty()).context("this tool has no check URL (Advanced)")?;
    let mut url = reqwest::Url::parse(url).context("the check URL is not a valid address")?;
    anyhow::ensure!(url.scheme() == "https", "the check URL must use https");
    let host = url.host_str().context("the check URL has no host")?.to_string();
    let rule = tool.hosts.iter().find(|r| r.matches(&host)).with_context(|| format!("{host} is not one of the tool's hosts"))?;
    let credential = tool.credential.as_deref().context("this tool has no token")?;
    let (_, secret) = secret(app, credential).await?.with_context(|| format!("the token {credential} is not in the vault"))?;
    let mut headers = reqwest::header::HeaderMap::new();
    crate::proxy::add_auth(&mut headers, &mut url, &rule.auth, &secret);
    let res = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?.get(url).headers(headers).header("user-agent", "launchpad").send().await;
    Ok(match res {
        // Some APIs (Slack) answer 200 with `{"ok": false, "error": "invalid_auth"}`.
        Ok(r) if r.status().is_success() => {
            let status = r.status().as_u16();
            let body: Value = r.json().await.unwrap_or(Value::Null);
            match body.get("ok") {
                Some(Value::Bool(false)) => ToolTest { ok: false, status, message: format!("{host} refused the token: {}.", body["error"].as_str().unwrap_or("not ok")) },
                _ => ToolTest { ok: true, status, message: format!("The token works on {host}.") },
            }
        }
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) => ToolTest { ok: false, status: r.status().as_u16(), message: format!("{host} refused the token ({}). Check the token and its permissions.", r.status()) },
        Ok(r) => ToolTest { ok: false, status: r.status().as_u16(), message: format!("{host} answered {}.", r.status()) },
        Err(e) => ToolTest { ok: false, status: 0, message: format!("Could not reach {host}: {e}") },
    })
}

pub async fn secret(app: &App, id: &str) -> Result<Option<(Kind, String)>> {
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            let row = c.query_row("SELECT kind, secret FROM vault WHERE id = ?1", [&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
            match row {
                Ok((k, s)) => Ok(serde_json::from_value(Value::String(k)).ok().map(|k| (k, s))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await
}

/// Adds or replaces an entry. The secret is checked for its kind and never returned.
pub async fn put(app: &App, id: &str, kind: Kind, name: &str, value: &str) -> Result<VaultEntry> {
    ensure_home(app)?;
    anyhow::ensure!(valid_id(id), "an entry ID uses lowercase letters, digits and dashes");
    let value = value.trim().to_string();
    anyhow::ensure!(!value.is_empty(), "the secret is empty");
    match kind {
        Kind::CodexLogin => {
            let auth: Value = serde_json::from_str(&value).context("paste the whole auth.json of a ChatGPT login (codex login)")?;
            anyhow::ensure!(auth["tokens"]["refresh_token"].is_string() && auth["tokens"]["access_token"].is_string(), "this auth.json has no ChatGPT tokens (an API key login is a Token entry)");
        }
        Kind::ClaudeLogin => anyhow::ensure!(!value.contains(char::is_whitespace), "paste the token printed by claude setup-token"),
        Kind::Token => anyhow::ensure!(!value.contains(['\n', '\r']), "a token is one line"),
    }
    let detail = detail_of(kind, &value);
    let entry = VaultEntry { id: id.into(), kind, name: if name.trim().is_empty() { id.into() } else { name.trim().into() }, detail, updated_at: now_ms() };
    let (e, kind_s) = (entry.clone(), serde_json::to_value(kind)?.as_str().unwrap_or_default().to_string());
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO vault (id, kind, name, detail, secret, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET kind = ?2, name = ?3, detail = ?4, secret = ?5, updated_at = ?6",
                rusqlite::params![e.id, kind_s, e.name, e.detail, value, e.updated_at],
            )?;
            Ok(())
        })
        .await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(entry)
}

pub async fn delete(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM vault WHERE id = ?1", [key])?)).await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(())
}

pub async fn tools(app: &App) -> Result<Vec<Tool>> {
    let mut all: Vec<Tool> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM tool")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    all.sort_by_key(|t: &Tool| t.name.to_lowercase());
    Ok(all)
}

pub async fn put_tool(app: &App, tool: Tool) -> Result<Tool> {
    ensure_home(app)?;
    check_tool(&tool)?;
    let (id, json) = (tool.id.clone(), serde_json::to_string(&tool)?);
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO tool (id, json) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET json = ?2", [id, json])?)).await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(tool)
}

pub async fn delete_tool(app: &App, id: &str) -> Result<()> {
    ensure_home(app)?;
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM tool WHERE id = ?1", [key])?)).await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(())
}

/// The registry as TOML (`[[tools]]`), for export.
pub async fn export_tools(app: &App) -> Result<String> {
    #[derive(Serialize)]
    struct File {
        tools: Vec<Tool>,
    }
    Ok(toml::to_string_pretty(&File { tools: tools(app).await? })?)
}

/// The vault and the registry live on Home.
pub fn here(app: &App) -> bool {
    app.paths.role == crate::role::Role::Home
}

fn ensure_home(app: &App) -> Result<()> {
    anyhow::ensure!(here(app), "credentials and tools are kept on Home: change them there");
    Ok(())
}

// ---------------------------------------------------------------- leases

/// What a runner asks for when a session starts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseRequest {
    pub session_id: String,
    pub generation: u32,
    pub harness: HarnessKind,
    /// The profile's login (a vault entry), else the harness's first login.
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Tools switched off for this launch.
    #[serde(default)]
    pub tools_off: Vec<String>,
    /// Tools switched on for this launch although the defaults leave them off.
    #[serde(default)]
    pub tools_on: Vec<String>,
    /// The session's folder: its default tools (`tool_defaults`).
    #[serde(default)]
    pub project_id: Option<String>,
    /// The bot of the session: Home reads its Launchpad CLI access from its `bot.toml`.
    #[serde(default)]
    pub bot_id: Option<String>,
}

impl LeaseRequest {
    /// The tools and MCP servers (`mcp:<id>`) of Home's registry that are off for this session: its defaults
    /// (workspace, folder, bot) and its launch's own choice. Grants are checked separately.
    pub async fn off(&self, app: &App) -> Result<Vec<String>> {
        let layers = crate::tool_defaults::layers(app, self.workspace_id.as_deref(), self.project_id.as_deref(), self.bot_id.as_deref()).await?;
        let layers: Vec<&crate::tool_defaults::ToolChoice> = layers.iter().collect();
        let keys = tools(app).await?.into_iter().map(|t| t.id).chain(crate::mcp::list(app).await?.into_iter().map(|s| crate::mcp::launch_off(&s.id)));
        Ok(keys.filter(|k| !crate::tool_defaults::is_on(&layers, &self.tools_off, &self.tools_on, k)).collect())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseCredential {
    pub id: String,
    pub secret: String,
    pub hosts: Vec<HostRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseLogin {
    pub kind: Kind,
    pub id: String,
    /// Codex: the claims of the ID token (plan, account), which are not secret; the runner writes them in an
    /// unsigned ID token so Codex knows the account.
    #[serde(default)]
    pub claims: Value,
    #[serde(default)]
    pub account_id: Option<String>,
}

/// The secrets and settings of one session, held in the runner proxy's memory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub session_id: String,
    pub generation: u32,
    pub credentials: Vec<LeaseCredential>,
    pub tools: Vec<Tool>,
    pub login: Option<LeaseLogin>,
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    /// The runner holds no token (Burst): its proxy sends requests to Home, which adds them (`token_relay`).
    #[serde(default)]
    pub relayed: bool,
    /// MCP servers granted to the session, reached through Home's gateway (`mcp`).
    #[serde(default)]
    pub mcp: Vec<crate::mcp::LeaseMcp>,
    /// The session's Launchpad CLI access; None when the tool is not granted.
    #[serde(default)]
    pub launchpad: Option<LaunchpadAccess>,
}

/// Login hosts: where the proxy adds a harness login (declared by its adapter).
fn login_hosts(harness: HarnessKind) -> Vec<HostRule> {
    let hosts = crate::harness::registration(harness).descriptor.vault_login.map(|l| l.hosts).unwrap_or_default();
    hosts.iter().map(|h| HostRule { host: (*h).into(), auth: Auth::Bearer }).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct LeaseInfo {
    pub id: String,
    pub session_id: String,
    pub runner_id: Option<String>,
    pub generation: u32,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub renewed_at: i64,
    #[ts(type = "number | null")]
    pub revoked_at: Option<i64>,
    /// Credentials in the lease (entry IDs), and its tools and MCP servers (`mcp:<id>`).
    pub credentials: Vec<String>,
    pub tools: Vec<String>,
    /// What the runner asked for, so Home can rebuild a relayed lease after a restart.
    #[serde(default)]
    #[ts(skip)]
    pub request: Option<LeaseRequest>,
}

pub(crate) async fn lease_rows(app: &App, filter: Option<String>) -> Result<Vec<LeaseInfo>> {
    app.state_db
        .call(move |c| {
            let sql = "SELECT json FROM vault_lease WHERE ?1 IS NULL OR session_id = ?1 ORDER BY created_at DESC LIMIT 200";
            let mut st = c.prepare(sql)?;
            let rows = st.query_map([filter], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await
}

async fn save_lease(app: &App, l: &LeaseInfo) -> Result<()> {
    let (id, session, created, json) = (l.id.clone(), l.session_id.clone(), l.created_at, serde_json::to_string(l)?);
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO vault_lease (id, session_id, created_at, json) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (id) DO UPDATE SET json = ?4",
                rusqlite::params![id, session, created, json],
            )?;
            Ok(())
        })
        .await
}

/// Leases of a session (newest first), or every recent lease.
pub async fn leases(app: &App, session_id: Option<&str>) -> Result<Vec<LeaseInfo>> {
    lease_rows(app, session_id.map(str::to_string)).await
}

/// Home gives a lease only for a session it placed on that runner (`runners::record_placement`, before the runner
/// starts it), in its current generation.
pub(crate) fn check_owner(app: &App, runner_id: &str, session_id: &str, generation: u32) -> Result<()> {
    let s = app.sessions.get(session_id).context("Home did not place this session on this runner: it runs on another runner or was never launched")?;
    if s.generation > generation {
        bail!("this session moved to another runner");
    }
    if s.runner_id.as_deref() != Some(runner_id) {
        bail!("this session runs on another runner");
    }
    Ok(())
}

/// Home: what a session's lease holds (workspace, folder, bot, login, tools on and off), read from Home's own
/// record of the session (`contain::request`), never from what the runner asked; only the generation is the
/// runner's (`check_owner` compared it). Call after `check_owner`.
async fn own_request(app: &App, req: &LeaseRequest) -> Result<LeaseRequest> {
    let s = app.sessions.get(&req.session_id).context("Home did not place this session on this runner")?;
    let mut own = crate::contain::request(app, &s).await?;
    own.generation = req.generation;
    Ok(own)
}

/// The tools and MCP servers of a lease (`mcp:<id>`, the keys of the default tools), for the session's Summary.
fn lease_tools(lease: &Lease) -> Vec<String> {
    lease.tools.iter().map(|t| t.id.clone()).chain(lease.mcp.iter().map(|m| crate::mcp::launch_off(&m.id))).collect()
}

/// Home: builds the lease of a session for `runner_id`.
pub async fn grant(app: &App, runner_id: &str, req: &LeaseRequest) -> Result<Lease> {
    ensure_home(app)?;
    check_owner(app, runner_id, &req.session_id, req.generation)?;
    let req = &own_request(app, req).await?;
    let lease = build(app, &uuid::Uuid::new_v4().to_string(), req).await?;
    let lease = relay_if_burst(app, runner_id, lease).await;
    let now = now_ms();
    save_lease(app, &LeaseInfo {
        id: lease.id.clone(),
        session_id: req.session_id.clone(),
        runner_id: Some(runner_id.to_string()),
        generation: req.generation,
        created_at: now,
        renewed_at: now,
        revoked_at: None,
        credentials: lease.credentials.iter().map(|c| c.id.clone()).collect(),
        tools: lease_tools(&lease),
        request: Some(req.clone()),
    })
    .await?;
    app.bus.publish("vault", "leases", json!({"session_id": req.session_id}));
    Ok(lease)
}

/// A Burst runner gets the lease without its secrets: Home keeps them and adds them to relayed requests.
async fn relay_if_burst(app: &App, runner_id: &str, mut lease: Lease) -> Lease {
    if !crate::runners::is_burst(app, runner_id).await {
        return lease;
    }
    crate::token_relay::hold(runner_id, &lease);
    lease.relayed = true;
    for c in &mut lease.credentials {
        c.secret.clear();
    }
    lease
}

pub(crate) async fn build(app: &App, lease_id: &str, req: &LeaseRequest) -> Result<Lease> {
    let entries = entries(app).await?;
    let mut credentials = vec![];
    // The harness login.
    let login = match Kind::login_of(req.harness) {
        Some(kind) => {
            let chosen = match &req.login {
                Some(id) => entries.iter().find(|e| &e.id == id && e.kind == kind).with_context(|| format!("the login {id:?} is not in Home's vault"))?,
                None => entries.iter().find(|e| e.kind == kind).with_context(|| format!("Home has no {} login: add one in Settings → Credentials and tools", req.harness.display()))?,
            };
            let (_, value) = secret(app, &chosen.id).await?.context("the login disappeared")?;
            let (token, claims, account_id) = match kind {
                Kind::CodexLogin => {
                    let auth = codex_fresh(app, &chosen.id, &value).await?;
                    let t = &auth["tokens"];
                    (t["access_token"].as_str().unwrap_or_default().to_string(), codex_claims(&auth.to_string()).unwrap_or(Value::Null), t["account_id"].as_str().map(str::to_string))
                }
                _ => (value, Value::Null, None),
            };
            credentials.push(LeaseCredential { id: chosen.id.clone(), secret: token, hosts: login_hosts(req.harness) });
            Some(LeaseLogin { kind, id: chosen.id.clone(), claims, account_id })
        }
        None => None,
    };
    // Granted tools that are on (`tool_defaults`), and their credentials.
    let off = req.off(app).await?;
    let mut tools_out: Vec<Tool> = vec![];
    for t in tools(app).await? {
        if !t.granted(req.workspace_id.as_deref()) || off.contains(&t.id) {
            continue;
        }
        if let Some(other) = host_conflict(&tools_out, &t) {
            tracing::info!("tool {}: left out, {} already sends its token to the same host", t.id, other.id);
            continue;
        }
        if let Some(cid) = &t.credential {
            match secret(app, cid).await? {
                Some((Kind::Token, value)) => {
                    match credentials.iter_mut().find(|c| &c.id == cid) {
                        Some(c) => c.hosts.extend(t.hosts.iter().cloned()),
                        None => credentials.push(LeaseCredential { id: cid.clone(), secret: value, hosts: t.hosts.clone() }),
                    }
                }
                // A tool without its token still installs; requests go out unauthenticated.
                _ => tracing::info!("tool {}: no token {cid:?} in the vault", t.id),
            }
        }
        tools_out.push(t);
    }
    let home = app.config.get().home.clone();
    let launchpad = match (tools_out.iter().any(|t| t.id == LAUNCHPAD_TOOL), &req.bot_id) {
        (false, _) => None,
        (true, None) => Some(LaunchpadAccess::Propose),
        // A removed bot or a broken bot.toml: the least access.
        (true, Some(b)) => Some(crate::bots::get(app, b).await.ok().filter(|b| b.settings_error.is_none()).map(|b| b.settings.launchpad).unwrap_or(LaunchpadAccess::Read)),
    };
    Ok(Lease {
        id: lease_id.into(),
        session_id: req.session_id.clone(),
        generation: req.generation,
        credentials,
        tools: tools_out,
        login,
        git_name: Some(home.git_name).filter(|s| !s.trim().is_empty()),
        git_email: Some(home.git_email).filter(|s| !s.trim().is_empty()),
        relayed: false,
        mcp: crate::mcp::granted(app, req.workspace_id.as_deref(), &off).await?,
        launchpad,
    })
}

/// The kept tool that already sends another token to one of `t`'s hosts (the proxy adds one token per host): two
/// Slack tools, for example. The first tool in the registry's order (by name) wins.
pub fn host_conflict<'a>(kept: &'a [Tool], t: &Tool) -> Option<&'a Tool> {
    let cid = t.credential.as_deref()?;
    kept.iter().find(|k| k.credential.as_deref().is_some_and(|c| c != cid) && k.hosts.iter().any(|a| t.hosts.iter().any(|b| a.host.eq_ignore_ascii_case(&b.host))))
}

/// Home: a fresh copy of an active lease (tokens Home refreshed meanwhile), or an error once it is revoked.
pub async fn renew(app: &App, runner_id: &str, lease_id: &str, req: &LeaseRequest) -> Result<Lease> {
    ensure_home(app)?;
    let mut info = lease_rows(app, Some(req.session_id.clone())).await?.into_iter().find(|l| l.id == lease_id).context("unknown lease")?;
    anyhow::ensure!(info.revoked_at.is_none(), "this lease was revoked");
    anyhow::ensure!(info.runner_id.as_deref() == Some(runner_id), "this lease belongs to another runner");
    check_owner(app, runner_id, &req.session_id, req.generation)?;
    let req = &own_request(app, req).await?;
    let lease = build(app, lease_id, req).await?;
    let lease = relay_if_burst(app, runner_id, lease).await;
    info.renewed_at = now_ms();
    info.credentials = lease.credentials.iter().map(|c| c.id.clone()).collect();
    info.tools = lease_tools(&lease);
    save_lease(app, &info).await?;
    Ok(lease)
}

/// Home: requests a runner sends about its own sessions (through its tunnel, never forwarded from a window).
pub async fn runner_call(app: &App, runner_id: &str, op: &str, body: Value) -> Result<Value> {
    match op {
        "lease" => {
            let req: LeaseRequest = serde_json::from_value(body)?;
            Ok(serde_json::to_value(grant(app, runner_id, &req).await?)?)
        }
        "renew" => {
            let lease_id = body["lease_id"].as_str().context("lease_id")?.to_string();
            let req: LeaseRequest = serde_json::from_value(body["request"].clone())?;
            Ok(serde_json::to_value(renew(app, runner_id, &lease_id, &req).await?)?)
        }
        "release" => {
            let lease_id = body["lease_id"].as_str().context("lease_id")?;
            let info = lease_rows(app, None).await?.into_iter().find(|l| l.id == lease_id);
            crate::token_relay::forget(lease_id);
            if let Some(mut info) = info.filter(|l| l.revoked_at.is_none() && l.runner_id.as_deref() == Some(runner_id)) {
                info.revoked_at = Some(now_ms());
                save_lease(app, &info).await?;
                app.bus.publish("vault", "leases", json!({"session_id": info.session_id}));
            }
            Ok(Value::Null)
        }
        "kb" => crate::kb::runner_apply(app, runner_id, body).await,
        "quota" => crate::quota::runner_reading(app, body),
        "place" => {
            // A launch the runner starts itself (a window or the CLI on that machine): Home records it first.
            let id = body["session_id"].as_str().context("session_id")?;
            let spec: crate::model::LaunchSpec = serde_json::from_value(body["spec"].clone()).context("spec")?;
            crate::runners::record_placement(app, &spec, id, runner_id).await?;
            Ok(Value::Null)
        }
        _ => bail!("unknown runner call {op}"),
    }
}

// ---------------------------------------------------------------- Codex logins

fn b64url_json(part: &str) -> Result<Value> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('='))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// The claims of the ID token of a Codex `auth.json`.
pub fn codex_claims(auth_json: &str) -> Result<Value> {
    let auth: Value = serde_json::from_str(auth_json)?;
    let id_token = auth["tokens"]["id_token"].as_str().context("no ID token")?;
    b64url_json(id_token.split('.').nth(1).context("malformed ID token")?)
}

/// An unsigned JWT carrying `claims`: Codex reads the plan and account from it and never verifies it.
pub fn unsigned_jwt(claims: &Value) -> String {
    use base64::Engine;
    let enc = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!("{}.{}.", enc(&json!({"alg": "none", "typ": "JWT"})), enc(claims))
}

/// Seconds until a JWT expires (negative: expired), None when it has no `exp`.
pub fn jwt_expires_in(token: &str, now_s: i64) -> Option<i64> {
    let claims = b64url_json(token.split('.').nth(1)?).ok()?;
    Some(claims["exp"].as_i64()? - now_s)
}

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Whether Home should refresh a Codex login now: its access token expires within two days, or it was last
/// refreshed more than seven days ago (Codex itself refreshes after eight).
pub fn codex_needs_refresh(auth: &Value, now_ms: i64) -> bool {
    let token = auth["tokens"]["access_token"].as_str().unwrap_or_default();
    if jwt_expires_in(token, now_ms / 1000).is_some_and(|s| s < 2 * 24 * 3600) {
        return true;
    }
    let last = auth["last_refresh"].as_str().and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok());
    last.is_some_and(|t| now_ms / 1000 - t.unix_timestamp() > 7 * 24 * 3600)
}

/// The current `auth.json` of a Codex login, refreshed on Home when needed. Home is the only place that
/// refreshes: runners only ever receive the access token.
async fn codex_fresh(app: &App, id: &str, value: &str) -> Result<Value> {
    let auth: Value = serde_json::from_str(value)?;
    if !codex_needs_refresh(&auth, now_ms()) {
        return Ok(auth);
    }
    let _guard = app.suggestion_locks.get(&format!("codex-refresh:{id}")).lock_owned().await;
    // Another lease may have refreshed it while this one waited.
    let (_, value) = secret(app, id).await?.context("the login disappeared")?;
    let mut auth: Value = serde_json::from_str(&value)?;
    if !codex_needs_refresh(&auth, now_ms()) {
        return Ok(auth);
    }
    let refresh = auth["tokens"]["refresh_token"].as_str().context("no refresh token")?.to_string();
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
    let res = client
        .post("https://auth.openai.com/oauth/token")
        .json(&json!({"client_id": CODEX_CLIENT_ID, "grant_type": "refresh_token", "refresh_token": refresh, "scope": "openid profile email"}))
        .send()
        .await
        .context("refresh the Codex login")?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    anyhow::ensure!(status.is_success(), "refreshing the Codex login failed ({status}): sign in again with codex login and paste the new auth.json");
    for k in ["id_token", "access_token", "refresh_token"] {
        if let Some(v) = body[k].as_str() {
            auth["tokens"][k] = Value::String(v.into());
        }
    }
    auth["last_refresh"] = Value::String(time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?);
    let text = serde_json::to_string_pretty(&auth)?;
    let (key, detail, now) = (id.to_string(), detail_of(Kind::CodexLogin, &text), now_ms());
    app.state_db
        .call(move |c| Ok(c.execute("UPDATE vault SET secret = ?2, detail = ?3, updated_at = ?4 WHERE id = ?1", rusqlite::params![key, text, detail, now])?))
        .await?;
    tracing::info!("vault: refreshed the Codex login {id}");
    Ok(auth)
}

/// Whether `file` (Home's copy of a Codex login, which the quota app-server may have refreshed) holds a newer
/// login than `vault`: the same account (both name it), other tokens and a later `last_refresh`.
fn codex_newer(file: &Value, vault: &Value) -> bool {
    let at = |a: &Value| a["last_refresh"].as_str().and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok());
    let token = |a: &Value, k: &str| a["tokens"][k].as_str().map(str::to_string);
    let same_account = token(file, "account_id").is_some_and(|a| Some(a) == token(vault, "account_id"));
    let changed = ["access_token", "refresh_token"].iter().any(|k| token(file, k) != token(vault, k));
    same_account
        && token(file, "access_token").is_some()
        && token(file, "refresh_token").is_some()
        && changed
        && matches!((at(file), at(vault)), (Some(f), Some(v)) if f > v)
}

/// Stores the newer Codex login found in Home's copy (`codex_newer`) in the vault, under the refresh lock.
async fn codex_adopt(app: &App, id: &str, file: &Value) -> Result<String> {
    let _guard = app.suggestion_locks.get(&format!("codex-refresh:{id}")).lock_owned().await;
    let (_, value) = secret(app, id).await?.context("the login disappeared")?;
    if !codex_newer(file, &serde_json::from_str::<Value>(&value)?) {
        return Ok(value);
    }
    let text = serde_json::to_string_pretty(file)?;
    let (key, detail, now, saved) = (id.to_string(), detail_of(Kind::CodexLogin, &text), now_ms(), text.clone());
    app.state_db
        .call(move |c| Ok(c.execute("UPDATE vault SET secret = ?2, detail = ?3, updated_at = ?4 WHERE id = ?1", rusqlite::params![key, saved, detail, now])?))
        .await?;
    tracing::info!("vault: kept the Codex login {id} that Codex refreshed on Home");
    Ok(text)
}

/// Home's own copy of a login for its quota monitor: a configuration folder with the token, under
/// `<data_dir>/vault/logins/<id>`. Never mounted in a jail.
pub async fn login_home(app: &App, id: &str) -> Result<(std::path::PathBuf, Vec<(String, String)>)> {
    let (kind, value) = secret(app, id).await?.context("no such login")?;
    let dir = app.paths.data_dir.join("vault/logins").join(id);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(app.paths.data_dir.join("vault"), std::fs::Permissions::from_mode(0o700));
    }
    let d = dir.to_string_lossy().into_owned();
    Ok(match kind {
        Kind::ClaudeLogin => (dir, vec![("CLAUDE_CONFIG_DIR".into(), d), ("CLAUDE_CODE_OAUTH_TOKEN".into(), value)]),
        Kind::CodexLogin => {
            // The quota app-server runs on this copy and may refresh the login itself (after a 401), which
            // rotates the refresh token: the vault keeps that newer login instead of overwriting it with a spent one.
            let file = dir.join("auth.json");
            let on_disk: Option<Value> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok());
            let value = match &on_disk {
                Some(disk) if codex_newer(disk, &serde_json::from_str::<Value>(&value)?) => codex_adopt(app, id, disk).await?,
                _ => value,
            };
            let auth = codex_fresh(app, id, &value).await?;
            // Written only when it changed, so the app-server's own writes are not raced every minute.
            if on_disk.as_ref() != Some(&auth) {
                crate::paths::write_private(&file, &serde_json::to_string_pretty(&auth)?)?;
            }
            (dir, vec![("CODEX_HOME".into(), d)])
        }
        _ => bail!("{id} is not a harness login"),
    })
}

/// The logins of a harness kind, for quota monitors and the profile editor.
pub async fn logins(app: &App, harness: HarnessKind) -> Result<Vec<VaultEntry>> {
    let Some(kind) = Kind::login_of(harness) else { return Ok(vec![]) };
    Ok(entries(app).await?.into_iter().filter(|e| e.kind == kind).collect())
}

/// Request names of the access log, for display: the GraphQL operation or a known REST action.
pub fn describe(host: &str, method: &str, path: &str, request: Option<&[u8]>, response: Option<&[u8]>) -> Option<String> {
    let parse = |b: Option<&[u8]>| b.and_then(|b| serde_json::from_slice::<Value>(b).ok());
    let segs: Vec<&str> = path.split('?').next().unwrap_or("").trim_matches('/').split('/').collect();
    match (host, method, segs.as_slice()) {
        ("api.github.com", "POST", ["graphql"]) => {
            let q = parse(request)?;
            let text = q["query"].as_str().unwrap_or("");
            let op = q["operationName"].as_str().map(str::to_string).or_else(|| {
                let t = text.trim_start();
                let rest = t.strip_prefix("mutation").or_else(|| t.strip_prefix("query"))?;
                Some(rest.trim_start().split(|c: char| !c.is_alphanumeric() && c != '_').next()?.to_string()).filter(|s| !s.is_empty())
            });
            let kind = if text.trim_start().starts_with("mutation") { "mutation" } else { "query" };
            op.map(|o| format!("GraphQL {kind} {o}"))
        }
        ("api.github.com", "POST", ["repos", o, r, "pulls"]) => {
            let n = parse(response).and_then(|v| v["number"].as_i64());
            Some(match n { Some(n) => format!("opened PR #{n} in {o}/{r}"), None => format!("open a PR in {o}/{r}") })
        }
        ("api.github.com", "POST", ["repos", o, r, "issues", n, "comments"]) => Some(format!("commented on #{n} in {o}/{r}")),
        ("api.github.com", "POST", ["repos", o, r, "pulls", n, "reviews"]) => Some(format!("reviewed PR #{n} in {o}/{r}")),
        ("api.github.com", "POST", ["repos", o, r, "issues"]) => {
            let n = parse(response).and_then(|v| v["number"].as_i64());
            Some(match n { Some(n) => format!("opened issue #{n} in {o}/{r}"), None => format!("open an issue in {o}/{r}") })
        }
        ("api.github.com", "PUT", ["repos", o, r, "pulls", n, "merge"]) => Some(format!("merged PR #{n} in {o}/{r}")),
        ("api.github.com", "DELETE", ["repos", o, r]) => Some(format!("deleted the repository {o}/{r}")),
        ("github.com", "POST", [o, r, "git-receive-pack"]) => Some(format!("pushed to {o}/{}", r.trim_end_matches(".git"))),
        ("huggingface.co", "POST", ["api", "repos", "create"]) => {
            let v = parse(request);
            let name = v.as_ref().and_then(|v| v["name"].as_str()).unwrap_or("?");
            let kind = v.as_ref().and_then(|v| v["type"].as_str()).unwrap_or("model");
            Some(format!("created {kind} {name}"))
        }
        ("huggingface.co", "POST", ["api", "spaces" | "models" | "datasets", o, r, "commit", ..]) => Some(format!("committed to {o}/{r}")),
        _ => None,
    }
}

/// Entries per variable name, for `${NAME}` references of contained profiles: granted tools give placeholders.
pub fn reference_values(lease: &Lease) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for t in &lease.tools {
        for v in &t.env {
            out.insert(v.clone(), placeholder(t.credential.as_deref().unwrap_or(&t.id)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_rules_match_exact_hosts_and_subdomains_only() {
        let exact = HostRule { host: "api.github.com".into(), auth: Auth::Bearer };
        assert!(exact.matches("API.github.com"));
        assert!(!exact.matches("evil-api.github.com"));
        assert!(!exact.matches("api.github.com.evil.io"));
        let wild = HostRule { host: "*.example.com".into(), auth: Auth::Bearer };
        assert!(wild.matches("a.b.example.com"));
        assert!(!wild.matches("example.com"));
        assert!(!wild.matches("badexample.com"));
    }

    #[test]
    fn slack_app_url_matches_slackline_token_create() {
        // Printed by `slackline token create` (v1.5.0).
        let read = "https://api.slack.com/apps?new_app=1&manifest_json=%7B%22display_information%22%3A%7B%22background_color%22%3A%22%234a154b%22%2C%22description%22%3A%22Slack%20CLI%20ro%22%2C%22name%22%3A%22Slackline%20ro%22%7D%2C%22oauth_config%22%3A%7B%22scopes%22%3A%7B%22user%22%3A%5B%22channels%3Ahistory%22%2C%22channels%3Aread%22%2C%22files%3Aread%22%2C%22groups%3Ahistory%22%2C%22groups%3Aread%22%2C%22im%3Ahistory%22%2C%22im%3Aread%22%2C%22mpim%3Ahistory%22%2C%22mpim%3Aread%22%2C%22pins%3Aread%22%2C%22reactions%3Aread%22%2C%22search%3Aread%22%2C%22users%3Aread%22%2C%22users%3Aread.email%22%5D%7D%7D%2C%22settings%22%3A%7B%22org_deploy_enabled%22%3Afalse%2C%22socket_mode_enabled%22%3Afalse%2C%22token_rotation_enabled%22%3Afalse%7D%7D";
        assert_eq!(slack_app_url(false), read);
        let write = slack_app_url(true);
        assert!(write.contains("Slackline%20rw") && write.contains("%22chat%3Awrite%22%2C%22files%3Awrite%22%2C%22im%3Awrite%22%2C%22pins%3Awrite%22%2C%22reactions%3Awrite%22%2C%22users.profile%3Awrite%22%5D"));
    }

    #[test]
    fn a_lease_keeps_one_token_per_host() {
        let (read, write) = (slack_tool(false), slack_tool(true));
        assert_eq!(host_conflict(std::slice::from_ref(&read), &write).map(|t| t.id.as_str()), Some(SLACK_READ));
        assert!(host_conflict(&templates()[..3], &read).is_none());
        // Two tools sharing one token share its authority: no conflict.
        let mut same = write.clone();
        same.credential = read.credential.clone();
        assert!(host_conflict(std::slice::from_ref(&read), &same).is_none());
    }

    #[test]
    fn tool_settings_are_checked() {
        let mut t = slack_tool(false);
        assert!(check_tool(&t).is_ok());
        for bad in ["PATH", "SLACK_TOKEN", "LD_PRELOAD", "1X"] {
            t.vars = BTreeMap::from([(bad.to_string(), "1".to_string())]);
            assert!(check_tool(&t).is_err(), "{bad}");
        }
        t.vars = BTreeMap::from([("SLACKLINE_FORMAT".to_string(), "a\nb".to_string())]);
        assert!(check_tool(&t).is_err());
        let mut no_install = slack_tool(false);
        no_install.install = None;
        assert!(check_tool(&no_install).is_err());
    }

    #[test]
    fn grants_follow_workspaces() {
        let mut t = templates().remove(0);
        t.workspaces = vec!["w1".into()];
        assert!(t.granted(Some("w1")));
        assert!(!t.granted(Some("w2")));
        assert!(!t.granted(None));
        t.workspaces = vec!["*".into()];
        assert!(t.granted(None));
    }

    #[test]
    fn codex_refresh_is_due_near_expiry_or_after_a_week() {
        let now = 1_800_000_000_000i64;
        let token = |exp: i64| unsigned_jwt(&json!({"exp": exp}));
        let iso = |ms: i64| time::OffsetDateTime::from_unix_timestamp(ms / 1000).unwrap().format(&time::format_description::well_known::Rfc3339).unwrap();
        let fresh = json!({"tokens": {"access_token": token(now / 1000 + 9 * 86400)}, "last_refresh": iso(now - 86_400_000)});
        assert!(!codex_needs_refresh(&fresh, now));
        let expiring = json!({"tokens": {"access_token": token(now / 1000 + 3600)}, "last_refresh": iso(now)});
        assert!(codex_needs_refresh(&expiring, now));
        let old = json!({"tokens": {"access_token": token(now / 1000 + 9 * 86400)}, "last_refresh": iso(now - 8 * 86_400_000)});
        assert!(codex_needs_refresh(&old, now));
    }

    #[test]
    fn codex_keeps_a_login_refreshed_on_disk() {
        let auth = |access: &str, refresh: &str, at: &str, account: &str| json!({"tokens": {"access_token": access, "refresh_token": refresh, "account_id": account}, "last_refresh": at});
        let vault = auth("a1", "r1", "2026-11-01T00:00:00Z", "acc");
        assert!(codex_newer(&auth("a2", "r2", "2026-11-02T00:00:00Z", "acc"), &vault));
        // A refresh that kept the refresh token still brings a newer access token.
        assert!(codex_newer(&auth("a2", "r1", "2026-11-02T00:00:00Z", "acc"), &vault));
        // Same tokens, an older copy, another account, no account or no date: the vault wins.
        assert!(!codex_newer(&auth("a1", "r1", "2026-11-02T00:00:00Z", "acc"), &vault));
        assert!(!codex_newer(&auth("a0", "r0", "2026-10-30T00:00:00Z", "acc"), &vault));
        assert!(!codex_newer(&auth("a2", "r2", "2026-11-02T00:00:00Z", "other"), &vault));
        assert!(!codex_newer(&json!({"tokens": {"access_token": "a2", "refresh_token": "r2"}, "last_refresh": "2026-11-02T00:00:00Z"}), &vault));
        assert!(!codex_newer(&json!({"tokens": {"access_token": "a2", "refresh_token": "r2", "account_id": "acc"}}), &vault));
    }

    #[test]
    fn unsigned_jwt_round_trips_claims() {
        let claims = json!({"email": "a@b.c", "https://api.openai.com/auth": {"chatgpt_plan_type": "pro"}});
        let auth = json!({"tokens": {"id_token": unsigned_jwt(&claims)}});
        assert_eq!(codex_claims(&auth.to_string()).unwrap(), claims);
    }

    #[test]
    fn access_log_names_common_actions() {
        assert_eq!(describe("api.github.com", "POST", "/repos/o/r/pulls", None, Some(br#"{"number": 3421}"#)).as_deref(), Some("opened PR #3421 in o/r"));
        assert_eq!(describe("api.github.com", "POST", "/graphql", Some(br#"{"query": "mutation CreatePR($x: ID) { a }"}"#), None).as_deref(), Some("GraphQL mutation CreatePR"));
        assert_eq!(describe("github.com", "POST", "/o/r.git/git-receive-pack", None, None).as_deref(), Some("pushed to o/r"));
        assert_eq!(describe("api.github.com", "GET", "/repos/o/r", None, None), None);
    }
}

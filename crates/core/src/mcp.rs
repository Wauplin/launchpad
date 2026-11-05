//! MCP servers on Home, and the gateway agents reach them through.
//!
//! An MCP server (Streamable HTTP) is configured once on Home with its address and token (a vault entry that
//! never leaves Home). Each server is granted per workspace like a tool, can be switched off for one launch,
//! and each of its tools is on or off. Harnesses on any runner see a local address on their runner proxy
//! (`/lp/<lease key>/mcp/<server>`); the proxy sends every request through the tunnel to this gateway, which
//! checks the session's lease, refuses calls to tools that are off, removes them from `tools/list`, adds the
//! token and forwards the request. An enabled tool acts directly: grant only what you mean. The annotations a
//! server reports (read only, destructive) are labels, never a security boundary.

use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::App;
use crate::home_link::{ByteStream, HttpReply};
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `readOnlyHint`, as the server reports it.
    #[serde(default)]
    pub read_only: bool,
    /// `destructiveHint`, as the server reports it.
    #[serde(default)]
    pub destructive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct McpServer {
    /// Lowercase letters, digits and dashes; agents see it as the server's name.
    pub id: String,
    pub name: String,
    /// The Streamable HTTP endpoint, for example `https://mcp.example.com/mcp`.
    pub url: String,
    /// How Home sends the token (none when the server needs none).
    #[serde(default)]
    pub auth: Option<crate::vault::Auth>,
    /// Workspaces where it is granted; `*` grants it everywhere.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Tools found by the last refresh.
    #[serde(default)]
    pub tools: Vec<McpTool>,
    /// Tools switched off: agents never see or call them.
    #[serde(default)]
    pub tools_off: Vec<String>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub refreshed_at: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

impl McpServer {
    pub fn granted(&self, workspace: Option<&str>) -> bool {
        self.workspaces.iter().any(|w| w == "*" || Some(w.as_str()) == workspace)
    }
    fn credential(&self) -> String {
        format!("mcp-{}", self.id)
    }
}

/// What a session's lease names: the servers it may reach (no address, no token).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseMcp {
    pub id: String,
    pub name: String,
}

/// The `tools_off` value of a launch that switches a server off.
pub fn launch_off(id: &str) -> String {
    format!("mcp:{id}")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ------------------------------------------------------------------ registry

pub async fn list(app: &App) -> Result<Vec<McpServer>> {
    let mut all: Vec<McpServer> = app
        .state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM mcp_server")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    all.sort_by_key(|s| s.name.to_lowercase());
    Ok(all)
}

pub async fn get(app: &App, id: &str) -> Result<McpServer> {
    list(app).await?.into_iter().find(|s| s.id == id).context("no such MCP server")
}

async fn save(app: &App, s: &McpServer) -> Result<()> {
    let (id, json) = (s.id.clone(), serde_json::to_string(s)?);
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO mcp_server (id, json) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET json = ?2", [id, json])?)).await?;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(())
}

/// What Settings sends: the server, and its token when it changes.
#[derive(Deserialize)]
pub struct Put {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub auth: Option<crate::vault::Auth>,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub token: Option<String>,
}

/// Adds or changes a server, then reads its tools.
pub async fn put(app: &App, id: &str, p: Put) -> Result<McpServer> {
    ensure!(crate::vault::here(app), "MCP servers are configured on Home");
    ensure!(valid_id(id), "an ID uses lowercase letters, digits and dashes");
    ensure!(!p.name.trim().is_empty(), "give the server a name");
    let url = reqwest::Url::parse(p.url.trim()).context("the address is not a valid URL")?;
    ensure!(url.scheme() == "https" || url.host_str().is_some_and(|h| h == "127.0.0.1" || h == "localhost"), "the address must use https");
    let old = get(app, id).await.ok();
    let mut s = McpServer {
        id: id.into(),
        name: p.name.trim().into(),
        url: url.to_string(),
        auth: p.auth,
        workspaces: if p.workspaces.is_empty() { vec!["*".into()] } else { p.workspaces },
        tools: old.as_ref().map(|o| o.tools.clone()).unwrap_or_default(),
        tools_off: old.as_ref().map(|o| o.tools_off.clone()).unwrap_or_default(),
        refreshed_at: old.as_ref().and_then(|o| o.refreshed_at),
        error: None,
    };
    if let Some(t) = p.token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        crate::vault::put(app, &s.credential(), crate::vault::Kind::Token, &format!("{} (MCP)", s.name), t).await?;
        s.auth.get_or_insert(crate::vault::Auth::Bearer);
    }
    save(app, &s).await?;
    refresh(app, id).await
}

pub async fn delete(app: &App, id: &str) -> Result<()> {
    ensure!(crate::vault::here(app), "MCP servers are configured on Home");
    let s = get(app, id).await?;
    let key = id.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM mcp_server WHERE id = ?1", [key])?)).await?;
    let _ = crate::vault::delete(app, &s.credential()).await;
    app.bus.publish("vault", "changed", Value::Null);
    Ok(())
}

/// Switches one tool on or off.
pub async fn set_tool(app: &App, id: &str, tool: &str, on: bool) -> Result<McpServer> {
    let mut s = get(app, id).await?;
    s.tools_off.retain(|t| t != tool);
    if !on {
        s.tools_off.push(tool.to_string());
    }
    save(app, &s).await?;
    Ok(s)
}

/// Reads a server's tools (initialize, then tools/list) and keeps them for Settings.
pub async fn refresh(app: &App, id: &str) -> Result<McpServer> {
    let mut s = get(app, id).await?;
    match discover(app, &s).await {
        Ok(tools) => {
            s.tools = tools;
            s.error = None;
        }
        Err(e) => s.error = Some(format!("{e:#}")),
    }
    s.refreshed_at = Some(now_ms());
    save(app, &s).await?;
    Ok(s)
}

// ------------------------------------------------------------------ talking to a server

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(std::time::Duration::from_secs(20)).build().expect("MCP client"));

/// A request to the server with its token.
async fn request(app: &App, s: &McpServer, method: reqwest::Method) -> Result<reqwest::RequestBuilder> {
    let mut url = reqwest::Url::parse(&s.url)?;
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(auth) = &s.auth
        && let Some((_, secret)) = crate::vault::secret(app, &s.credential()).await?
    {
        crate::proxy::add_auth(&mut headers, &mut url, auth, &secret);
    }
    Ok(CLIENT.request(method, url).headers(headers))
}

/// The JSON-RPC messages of an answer, whether JSON or an event stream.
pub fn messages(content_type: &str, body: &str) -> Vec<Value> {
    if content_type.starts_with("text/event-stream") {
        let mut out = vec![];
        let mut data = String::new();
        for line in body.lines().chain(std::iter::once("")) {
            if let Some(d) = line.strip_prefix("data:") {
                data.push_str(d.trim_start());
            } else if line.is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(&data) {
                    out.push(v);
                }
                data.clear();
            }
        }
        return out;
    }
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(a)) => a,
        Ok(v) => vec![v],
        Err(_) => vec![],
    }
}

async fn rpc(app: &App, s: &McpServer, session: Option<&str>, body: Value) -> Result<(Option<String>, Vec<Value>)> {
    let mut r = request(app, s, reqwest::Method::POST).await?.header("accept", "application/json, text/event-stream").header("mcp-protocol-version", PROTOCOL).json(&body);
    if let Some(id) = session {
        r = r.header("mcp-session-id", id);
    }
    let res = r.timeout(std::time::Duration::from_secs(30)).send().await.with_context(|| format!("reach {}", s.url))?;
    let status = res.status();
    let session = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_string);
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let text = res.text().await?;
    if matches!(status.as_u16(), 401 | 403) {
        bail!("{} refused the token ({status})", s.url);
    }
    ensure!(status.is_success() || status.as_u16() == 202, "{} answered {status}: {}", s.url, text.chars().take(200).collect::<String>());
    Ok((session, messages(&ctype, &text)))
}

const PROTOCOL: &str = "2025-06-18";

async fn discover(app: &App, s: &McpServer) -> Result<Vec<McpTool>> {
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "launchpad-home", "version": crate::COMMIT}}});
    let (session, answer) = rpc(app, s, None, init).await?;
    let first = answer.into_iter().find(|m| m["id"] == 1).context("the server did not answer initialize")?;
    if let Some(e) = first.get("error") {
        bail!("initialize: {}", e["message"].as_str().unwrap_or("error"));
    }
    let _ = rpc(app, s, session.as_deref(), json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    let mut tools = vec![];
    let mut cursor: Option<String> = None;
    for page in 0..20 {
        let params = match &cursor { Some(c) => json!({"cursor": c}), None => json!({}) };
        let (_, answer) = rpc(app, s, session.as_deref(), json!({"jsonrpc": "2.0", "id": 2 + page, "method": "tools/list", "params": params})).await?;
        let m = answer.into_iter().find(|m| m["id"] == 2 + page).context("the server did not answer tools/list")?;
        if let Some(e) = m.get("error") {
            bail!("tools/list: {}", e["message"].as_str().unwrap_or("error"));
        }
        for t in m["result"]["tools"].as_array().cloned().unwrap_or_default() {
            tools.push(McpTool {
                name: t["name"].as_str().unwrap_or_default().to_string(),
                description: t["description"].as_str().unwrap_or_default().chars().take(400).collect(),
                read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                destructive: t["annotations"]["destructiveHint"].as_bool().unwrap_or(false),
            });
        }
        cursor = m["result"]["nextCursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    tools.retain(|t| !t.name.is_empty());
    Ok(tools)
}

// ------------------------------------------------------------------ leases

/// Servers granted to a session's lease.
pub async fn granted(app: &App, workspace: Option<&str>, tools_off: &[String]) -> Result<Vec<LeaseMcp>> {
    Ok(list(app).await?.into_iter().filter(|s| s.granted(workspace) && !tools_off.contains(&launch_off(&s.id))).map(|s| LeaseMcp { id: s.id, name: s.name }).collect())
}

/// The name a harness gives a server: letters, digits, dashes and underscores.
pub fn harness_name(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

// ------------------------------------------------------------------ gateway

/// Removes switched-off tools from a `tools/list` result.
pub fn filter_message(mut m: Value, off: &[String]) -> Value {
    if let Some(tools) = m.get_mut("result").and_then(|r| r.get_mut("tools")).and_then(Value::as_array_mut) {
        tools.retain(|t| !t["name"].as_str().is_some_and(|n| off.iter().any(|o| o == n)));
    }
    m
}

/// The JSON-RPC calls of a request body that name a tool that is off.
pub fn refused_calls(body: &Value, off: &[String]) -> Vec<(Value, String)> {
    let all = match body {
        Value::Array(a) => a.clone(),
        v => vec![v.clone()],
    };
    all.into_iter()
        .filter(|m| m["method"] == "tools/call")
        .filter_map(|m| {
            let name = m["params"]["name"].as_str()?.to_string();
            off.contains(&name).then(|| (m["id"].clone(), name))
        })
        .collect()
}

/// Home: one request of a session to an MCP server. `runner_id` is None in an `all` daemon.
pub async fn gateway(app: &App, runner_id: Option<&str>, lease_id: &str, server: &str, method: &str, headers: Vec<(String, String)>, body: ByteStream) -> Result<HttpReply> {
    ensure!(crate::vault::here(app), "MCP servers are reached through Home");
    let info = crate::vault::lease_rows(app, None).await?.into_iter().find(|l| l.id == lease_id).context("unknown lease")?;
    ensure!(info.revoked_at.is_none(), "this session's access was revoked");
    ensure!(info.runner_id.as_deref() == runner_id, "this lease belongs to another runner");
    let req = info.request.context("this lease names no MCP server")?;
    crate::vault::check_owner(app, runner_id, &req.session_id, req.generation)?;
    let s = get(app, server).await?;
    ensure!(s.granted(req.workspace_id.as_deref()) && !req.tools_off.contains(&launch_off(&s.id)), "{} is not granted to this session", s.name);
    let m = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut r = request(app, &s, m.clone()).await?;
    for (k, v) in &headers {
        if matches!(k.to_ascii_lowercase().as_str(), "accept" | "content-type" | "mcp-session-id" | "mcp-protocol-version" | "last-event-id") {
            r = r.header(k, v);
        }
    }
    let mut calls = vec![];
    if m == reqwest::Method::POST {
        let parts: Vec<bytes::Bytes> = body.try_collect().await?;
        let bytes = parts.concat();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let refused = refused_calls(&parsed, &s.tools_off);
        if let Some((id, name)) = refused.into_iter().next() {
            let text = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("The tool {name} is off in Launchpad (Settings → Credentials and tools)")}}).to_string();
            return Ok(HttpReply { status: 200, headers: vec![("content-type".into(), "application/json".into())], body: Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(text)) })) });
        }
        let all = match &parsed { Value::Array(a) => a.clone(), v => vec![v.clone()] };
        calls = all.iter().filter(|m| m["method"] == "tools/call").filter_map(|m| m["params"]["name"].as_str().map(str::to_string)).collect();
        r = r.body(bytes);
    }
    let res = r.send().await.with_context(|| format!("reach {}", s.name))?;
    let status = res.status().as_u16();
    for tool in calls {
        crate::contain::record_access(app, crate::proxy::Access { at: now_ms(), session_id: req.session_id.clone(), host: format!("MCP {}", s.name), method: "tools/call".into(), path: tool, status, action: None });
    }
    let headers: Vec<(String, String)> = res.headers().iter().filter(|(k, _)| !crate::proxy::HOP.contains(&k.as_str()) && k.as_str() != "content-length").filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string()))).collect();
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let off = s.tools_off.clone();
    let body: ByteStream = if off.is_empty() {
        Box::pin(res.bytes_stream().map(|r| r.map_err(std::io::Error::other)))
    } else if ctype.starts_with("text/event-stream") {
        Box::pin(filter_events(res.bytes_stream().map(|r| r.map_err(std::io::Error::other)), off))
    } else {
        let text = res.text().await?;
        let out = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => Value::Array(a.into_iter().map(|m| filter_message(m, &off)).collect()).to_string(),
            Ok(v) => filter_message(v, &off).to_string(),
            Err(_) => text,
        };
        Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(out)) }))
    };
    Ok(HttpReply { status, headers, body })
}

/// An event stream with switched-off tools removed from `tools/list` results, line by line.
fn filter_events(stream: impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + 'static, off: Vec<String>) -> impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send {
    let state = (Box::pin(stream), String::new(), off, false);
    futures_util::stream::unfold(state, |(mut s, mut buf, off, done)| async move {
        if done {
            return None;
        }
        loop {
            if let Some(i) = buf.find('\n') {
                let line: String = buf.drain(..=i).collect();
                let out = match line.trim_end_matches(['\n', '\r']).strip_prefix("data:") {
                    Some(d) => match serde_json::from_str::<Value>(d.trim_start()) {
                        Ok(v) => format!("data: {}\n", filter_message(v, &off)),
                        Err(_) => line,
                    },
                    None => line,
                };
                return Some((Ok(bytes::Bytes::from(out)), (s, buf, off, false)));
            }
            match s.next().await {
                Some(Ok(b)) => buf.push_str(&String::from_utf8_lossy(&b)),
                Some(Err(e)) => return Some((Err(e), (s, buf, off, true))),
                None => {
                    let rest = std::mem::take(&mut buf);
                    return if rest.is_empty() { None } else { Some((Ok(bytes::Bytes::from(rest)), (s, buf, off, true))) };
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switched_off_tools_are_hidden_and_refused() {
        let off = vec!["send_email".to_string()];
        let list = json!({"jsonrpc": "2.0", "id": 3, "result": {"tools": [{"name": "search"}, {"name": "send_email"}]}});
        assert_eq!(filter_message(list, &off)["result"]["tools"], json!([{"name": "search"}]));
        let batch = json!([{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "search"}}, {"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "send_email"}}]);
        assert_eq!(refused_calls(&batch, &off), vec![(json!(5), "send_email".to_string())]);
        let sse = "event: message\ndata: {\"id\":1,\"result\":{}}\n\ndata: {\"id\":2}\n\n";
        assert_eq!(messages("text/event-stream", sse), vec![json!({"id": 1, "result": {}}), json!({"id": 2})]);
    }
}

//! `launchpad oracle …`: the oracles of a workspace (`/api/oracles/{workspace}…` on Home), and `oracle test`, which
//! runs a draft with `uv run` here, in the agent's own jail, never on Home and without records.
//!
//! `run` and `test` exit with the outcome: 0 True, 1 False, 2 Failed; any other code (3) is a CLI error that never
//! reached the oracle.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use launchpad_automation::oracles;
use launchpad_core::paths::Paths;
use serde_json::{Value, json};

use crate::{Client, query, url_segment, utc};

/// The exit code of a CLI error (1 and 2 are False and Failed).
pub const ERROR_EXIT: i32 = 3;

#[derive(Subcommand)]
pub enum OracleCommand {
    /// The workspace's oracles: slug, title, question, parameters, access, last answer and the routines gated on it (JSON).
    List,
    /// One oracle: its file (`content`), parameters, access, dependencies, last answers and users (JSON).
    Show { slug: String },
    /// Run a saved oracle on Home and print its answer; exit 0 True, 1 False, 2 Failed.
    Run {
        slug: String,
        /// A parameter, `name=value` (repeat it).
        #[arg(long = "param")]
        params: Vec<String>,
        /// A named cursor of yours (lowercase letters, digits, - and _): the run starts from it, and it moves on
        /// only when the answer is True or False.
        #[arg(long, conflicts_with = "since")]
        cursor: Option<String>,
        /// One stateless run from this cursor value (JSON, or plain text).
        #[arg(long)]
        since: Option<String>,
        /// Accept an answer this fresh instead of a new run: 30s, 2m, 1h.
        #[arg(long)]
        max_age: Option<String>,
        /// Print the whole evaluation as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run a draft file here, in your own jail, with `uv run` and your session's tools; records nothing. Exit 0 True,
    /// 1 False, 2 Failed.
    Test {
        #[arg(long)]
        file: PathBuf,
        /// A parameter, `name=value` (repeat it).
        #[arg(long = "param")]
        params: Vec<String>,
        /// The cursor the draft starts from (JSON, or plain text).
        #[arg(long)]
        cursor: Option<String>,
        /// Print the whole result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// The oracle's evaluations, newest first, with their callers and what came of them (JSON).
    History {
        slug: String,
        /// Only the Failed ones.
        #[arg(long)]
        failed: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Create an oracle or change its file (admin access). A version that gives the oracle a tool this session does
    /// not hold is refused: propose it with `launchpad propose oracle`.
    Save {
        #[arg(long)]
        file: PathBuf,
        /// The oracle's slug (default: the file name without `.py`).
        #[arg(long)]
        slug: Option<String>,
    },
    /// Pause an oracle (admin access): nothing evaluates it, and routines gated on it record quiet checks.
    Pause { slug: String },
    /// Resume a paused oracle (admin access).
    Resume { slug: String },
}

/// Runs a command; `Ok` holds the exit code.
pub async fn command(paths: &Paths, workspace: Option<String>, command: OracleCommand) -> Result<i32> {
    match command {
        OracleCommand::List => {
            let (client, ws) = scoped(paths, workspace).await?;
            let list = client.json("GET", &format!("/api/oracles/{}", url_segment(&ws)), None).await?;
            let out: Vec<Value> = list.as_array().into_iter().flatten().map(brief).collect();
            print_json(&json!(out))?;
            Ok(0)
        }
        OracleCommand::Show { slug } => {
            let (client, ws) = scoped(paths, workspace).await?;
            let v = client.json("GET", &oracle_path(&ws, &slug, ""), None).await?;
            print_json(&v)?;
            Ok(0)
        }
        OracleCommand::Run { slug, params, cursor, since, max_age, json } => {
            let (client, ws) = scoped(paths, workspace).await?;
            let body = json!({"params": oracles::parse_param_args(&params)?, "cursor": cursor, "since": since.as_deref().map(json_or_text), "max_age": max_age});
            let e = client.json("POST", &oracle_path(&ws, &slug, "/run"), Some(body)).await?;
            if json {
                print_json(&e)?;
            } else {
                print_answer(&e, e["created_at"].as_i64());
            }
            Ok(exit_code(&e))
        }
        OracleCommand::Test { file, params, cursor, json } => {
            let r = test(paths, workspace, &file, &params, cursor.as_deref()).await?;
            if json {
                print_json(&r)?;
            } else {
                print_answer(&r, None);
            }
            Ok(exit_code(&r))
        }
        OracleCommand::History { slug, failed, limit } => {
            let (client, ws) = scoped(paths, workspace).await?;
            let limit = limit.clamp(1, 500).to_string();
            let mut pairs = vec![("limit", limit.as_str())];
            if failed {
                pairs.push(("failed", "true"));
            }
            let list = client.json("GET", &query(&oracle_path(&ws, &slug, "/evaluations"), &pairs), None).await?;
            let out: Vec<Value> = list.as_array().into_iter().flatten().map(evaluation).collect();
            print_json(&json!(out))?;
            Ok(0)
        }
        OracleCommand::Pause { slug } => paused(paths, workspace, &slug, "pause").await,
        OracleCommand::Resume { slug } => paused(paths, workspace, &slug, "resume").await,
        OracleCommand::Save { file, slug } => {
            let content = std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
            let slug = match slug {
                Some(s) => s,
                None => file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            };
            if !oracles::valid_slug(&slug) {
                bail!("{slug:?} is not a valid oracle slug (lowercase letters, digits, - and _): name it with --slug");
            }
            oracles::parse(&content).with_context(|| format!("{} is not a valid oracle", file.display()))?;
            let (client, ws) = scoped(paths, workspace).await?;
            let o = client.json("PUT", &oracle_path(&ws, &slug, ""), Some(json!({"content": content}))).await?;
            println!("Saved oracle {slug} ({}).", o["meta"]["title"].as_str().unwrap_or(&slug));
            Ok(0)
        }
    }
}

async fn paused(paths: &Paths, workspace: Option<String>, slug: &str, verb: &str) -> Result<i32> {
    let (client, ws) = scoped(paths, workspace).await?;
    client.json("POST", &oracle_path(&ws, slug, &format!("/{verb}")), Some(json!({}))).await?;
    println!("Oracle {slug}: {}.", if verb == "pause" { "paused" } else { "resumed" });
    Ok(0)
}

/// The client and the workspace: `--workspace`, else the workspace of this session (`LAUNCHPAD_SESSION_ID`).
async fn scoped(paths: &Paths, workspace: Option<String>) -> Result<(Client, String)> {
    let client = Client::new(paths)?;
    let ws = workspace_of(&client, workspace).await?;
    Ok((client, ws))
}

async fn workspace_of(client: &Client, workspace: Option<String>) -> Result<String> {
    if let Some(w) = workspace.filter(|w| !w.trim().is_empty()) {
        return Ok(w.trim().to_string());
    }
    let id = std::env::var("LAUNCHPAD_SESSION_ID").ok().filter(|s| !s.is_empty()).context("pass --workspace <workspace id> (outside an agent session)")?;
    let s = client.json("GET", &format!("/api/context/sessions/{}", url_segment(&id)), None).await?;
    s["workspace_id"].as_str().map(str::to_string).context("this session belongs to no workspace: pass --workspace <workspace id>")
}

fn oracle_path(ws: &str, slug: &str, rest: &str) -> String {
    format!("/api/oracles/{}/{}{rest}", url_segment(ws), url_segment(slug))
}

fn print_json(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Pure: a value given on the command line: JSON when it parses, else the text itself.
fn json_or_text(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

/// Pure: 0 True, 1 False, 2 Failed (an answer without an outcome counts as Failed).
fn exit_code(v: &Value) -> i32 {
    match v["outcome"].as_str() {
        Some("true") => 0,
        Some("false") => 1,
        _ => 2,
    }
}

/// Pure: the label of an outcome.
fn outcome_label(v: &Value) -> &'static str {
    match v["outcome"].as_str() {
        Some("true") => "True",
        Some("false") => "False",
        _ => "Failed",
    }
}

/// Pure: an answer as text, for agents that read it rather than parse it.
fn answer_text(v: &Value, at: Option<i64>) -> String {
    let mut first = outcome_label(v).to_string();
    if let Some(e) = v["error"].as_object() {
        first.push_str(&format!(" · {}: {}", e.get("class").and_then(Value::as_str).unwrap_or("?"), e.get("message").and_then(Value::as_str).unwrap_or("")));
    }
    if let Some(s) = v["summary"].as_str().filter(|s| !s.is_empty()) {
        first.push_str(&format!(" · {s}"));
    }
    let mut lines = vec![first];
    let mut facts = vec![];
    if let Some(d) = v["duration_ms"].as_i64() {
        facts.push(format!("{d} ms"));
    }
    if let Some(at) = at {
        facts.push(format!("at {}", utc(at)));
    }
    if v["reused"].as_bool() == Some(true) {
        facts.push("reused a fresh answer".into());
    }
    if let Some(f) = v["fingerprint"].as_str() {
        facts.push(format!("fingerprint {f}"));
    }
    let cursor = if v.get("cursor_proposed").is_some() { &v["cursor_proposed"] } else { &v["cursor"] };
    if !cursor.is_null() {
        facts.push(format!("cursor {cursor}"));
    }
    if !facts.is_empty() {
        lines.push(facts.join(" · "));
    }
    for l in v["links"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        lines.push(l.to_string());
    }
    if let Some(items) = v["items"].as_array().filter(|i| !i.is_empty()) {
        lines.push(format!("items: {}", Value::Array(items.clone())));
    }
    if let Some(t) = v["stderr_tail"].as_str().filter(|t| !t.trim().is_empty()) {
        lines.push(format!("stderr (end):\n{}", t.trim_end()));
    }
    lines.join("\n")
}

fn print_answer(v: &Value, at: Option<i64>) {
    println!("{}", answer_text(v, at));
}

/// Pure: an oracle of the list, short: what an agent needs to pick or reuse one.
fn brief(item: &Value) -> Value {
    let o = &item["oracle"];
    let m = &o["meta"];
    let params: Vec<String> = m["params"].as_array().into_iter().flatten().map(|p| {
        let kind = p["kind"].as_str().unwrap_or("string");
        match &p["default"] {
            Value::Null => format!("{} ({kind}, required)", p["name"].as_str().unwrap_or("?")),
            d => format!("{} ({kind} = {d})", p["name"].as_str().unwrap_or("?")),
        }
    }).collect();
    let last = &item["last"];
    json!({
        "slug": o["slug"],
        "title": m["title"],
        "question": m["description"],
        "params": params,
        "access": m["access"],
        "dependencies": m["dependencies"],
        "paused": o["paused"],
        "error": o["error"],
        "last": if last.is_null() { Value::Null } else { json!({"outcome": last["outcome"], "at": last["created_at"].as_i64().map(utc), "summary": last["summary"], "error": last["error"]}) },
        "failing": item["streak"]["count"].as_i64().filter(|c| *c > 0 && item["streak"]["read_at"].is_null()),
        "used_by": item["used_by"].as_array().into_iter().flatten().map(|u| u["title"].clone()).collect::<Vec<_>>(),
    })
}

/// Pure: an evaluation of the history, short.
fn evaluation(e: &Value) -> Value {
    let callers: Vec<Value> = e["callers"].as_array().into_iter().flatten().map(|c| {
        let kind = c["caller"]["kind"].as_str().unwrap_or("?");
        let who = match c["caller"]["name"].as_str() {
            Some(n) => format!("{kind} {n}"),
            None => kind.to_string(),
        };
        json!({"caller": who, "how": c["how"], "consequence": c["consequence"], "detail": c["detail"]})
    }).collect();
    json!({
        "id": e["id"], "at": e["created_at"].as_i64().map(utc), "outcome": e["outcome"], "error": e["error"], "summary": e["summary"],
        "fingerprint": e["fingerprint"], "params": e["params"], "cursor_proposed": e["cursor_proposed"], "duration_ms": e["duration_ms"], "callers": callers,
    })
}

/// `oracle test`: the draft runs here with uv, this session's environment as it is (its tools' variables and the
/// proxy that adds their tokens) and the helper this binary carries. Records nothing.
async fn test(paths: &Paths, workspace: Option<String>, file: &std::path::Path, params: &[String], cursor: Option<&str>) -> Result<Value> {
    let content = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let meta = oracles::parse(&content).with_context(|| format!("{} is not a valid oracle", file.display()))?;
    let params = oracles::parse_param_args(params)?;
    let env: BTreeMap<String, String> = std::env::vars().collect();
    if launchpad_core::shellenv::UserEnv::from_vars(env.clone()).which("uv").is_none() {
        bail!("uv is not installed on this machine: `oracle test` runs the draft with `uv run` in your jail. Install it (`curl -LsSf https://astral.sh/uv/install.sh | sh`) or ask the owner to (the runner's setup checklist names it)");
    }
    let mut hosts = meta.access.hosts.clone();
    if !meta.access.tools.is_empty() {
        let tools = async {
            let (client, ws) = scoped(paths, workspace).await?;
            client.json("GET", &format!("/api/oracles/{}/_tools", url_segment(&ws)), None).await
        }
        .await;
        match tools {
            Ok(list) => {
                for id in &meta.access.tools {
                    match list.as_array().into_iter().flatten().find(|t| t["id"].as_str() == Some(id)) {
                        Some(t) => hosts.extend(t["hosts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string)),
                        None => eprintln!("Note: the tool {id} is not granted to this workspace: on Home this oracle fails (class access)."),
                    }
                }
            }
            Err(e) => eprintln!("Note: could not read the workspace's tools ({e:#}): ctx.http allows only access.hosts here."),
        }
        eprintln!("Note: tool tokens come from this session's environment (your own tools); on Home the oracle gets the real tokens of {}.", meta.access.tools.join(", "));
    }
    if meta.access.launchpad == oracles::LaunchpadRead::Read {
        eprintln!("Note: Launchpad read access is not available in a local test: ctx.launchpad is None here. On Home the oracle gets a read token of its workspace.");
    }
    hosts.sort();
    hosts.dedup();
    let wheel_dir = std::env::temp_dir().join("launchpad-oracle-helper");
    let r = launchpad_automation::oracle_eval::test_file(file, &wheel_dir, params, cursor.map(json_or_text), hosts, env).await?;
    Ok(serde_json::to_value(r)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_give_exit_codes_and_text() {
        assert_eq!(exit_code(&json!({"outcome": "true"})), 0);
        assert_eq!(exit_code(&json!({"outcome": "false"})), 1);
        assert_eq!(exit_code(&json!({"outcome": "failed"})), 2);
        assert_eq!(exit_code(&json!({})), 2);
        let t = answer_text(&json!({"outcome": "failed", "error": {"class": "http", "message": "401 from api.github.com"}, "summary": "", "duration_ms": 12, "cursor": null, "stderr_tail": "boom\n"}), None);
        assert_eq!(t, "Failed · http: 401 from api.github.com\n12 ms\nstderr (end):\nboom");
        let t = answer_text(&json!({"outcome": "true", "summary": "red", "fingerprint": "abc", "cursor_proposed": "c2", "links": ["https://x"], "duration_ms": 5, "reused": true}), None);
        assert_eq!(t, "True · red\n5 ms · reused a fresh answer · fingerprint abc · cursor \"c2\"\nhttps://x");
        assert_eq!(json_or_text("{\"a\": 1}"), json!({"a": 1}));
        assert_eq!(json_or_text("2026-11-01"), json!("2026-11-01"));
    }
}

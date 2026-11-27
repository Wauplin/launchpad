//! `launchpad oracle …`: the oracles of a workspace (`/api/oracles/{workspace}…` on Home). `oracle test` and
//! `oracle save` send a file to Home, which pre-checks it (Ruff, the signature of `check`, Monty's parser, the type
//! checker); `test` then runs it on Home exactly like the saved oracle would run, without records.
//!
//! `run` and `test` exit with the outcome: 0 True, 1 False, 2 Failed; any other code (3) is a CLI error that never
//! reached the oracle, a pre-check error included.

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
    /// The workspace's oracles: slug, title, question, parameters, tools, last answer, failure streak, whether it is
    /// broken, and the routines gated on it (JSON).
    List,
    /// One oracle: its file (`content`), parameters, tools, type warnings, last answers and users (JSON).
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
        /// One stateless run from this cursor value.
        #[arg(long)]
        since: Option<String>,
        /// Accept an answer this fresh instead of a new run: 30s, 2m, 1h.
        #[arg(long)]
        max_age: Option<String>,
        /// Print the whole evaluation as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Pre-check a draft file, then run it on Home exactly like the saved oracle would run, without records. Exit 0
    /// True, 1 False, 2 Failed, 3 when the pre-check refuses it.
    Test {
        #[arg(long)]
        file: PathBuf,
        /// The oracle's slug (default: the file name without `.py`).
        #[arg(long)]
        slug: Option<String>,
        /// A parameter, `name=value` (repeat it).
        #[arg(long = "param")]
        params: Vec<String>,
        /// The cursor the draft starts from.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the whole result as JSON, with the log and the call log.
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
        /// Add each evaluation's call log.
        #[arg(long)]
        calls: bool,
    },
    /// Create an oracle or change its file (admin access): pre-checked, then stored formatted. A file with a
    /// credentialed tool (github, huggingface) this session does not hold is refused: propose it with
    /// `launchpad propose oracle`.
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
            let body = json!({"params": oracles::parse_param_args(&params)?, "cursor": cursor, "since": since, "max_age": max_age});
            let e = client.json("POST", &oracle_path(&ws, &slug, "/run"), Some(body)).await?;
            if json {
                print_json(&e)?;
            } else {
                print_answer(&e, e["created_at"].as_i64());
            }
            Ok(exit_code(&e))
        }
        OracleCommand::Test { file, slug, params, cursor, json } => {
            let r = test(paths, workspace, &file, slug, &params, cursor.as_deref()).await?;
            if json {
                print_json(&r)?;
            } else {
                print_answer(&r, None);
            }
            Ok(exit_code(&r))
        }
        OracleCommand::History { slug, failed, limit, calls } => {
            let (client, ws) = scoped(paths, workspace).await?;
            let limit = limit.clamp(1, 500).to_string();
            let mut pairs = vec![("limit", limit.as_str())];
            if failed {
                pairs.push(("failed", "true"));
            }
            let list = client.json("GET", &query(&oracle_path(&ws, &slug, "/evaluations"), &pairs), None).await?;
            let out: Vec<Value> = list.as_array().into_iter().flatten().map(|e| evaluation(e, calls)).collect();
            print_json(&json!(out))?;
            Ok(0)
        }
        OracleCommand::Pause { slug } => paused(paths, workspace, &slug, "pause").await,
        OracleCommand::Resume { slug } => paused(paths, workspace, &slug, "resume").await,
        OracleCommand::Save { file, slug } => {
            let content = std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
            let slug = slug_of(&file, slug)?;
            let (client, ws) = scoped(paths, workspace).await?;
            let saved = client.json("PUT", &oracle_path(&ws, &slug, ""), Some(json!({"content": content}))).await?;
            print_warnings(&saved["warnings"]);
            println!("{}", saved_text(&slug, &saved));
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
pub(crate) async fn scoped(paths: &Paths, workspace: Option<String>) -> Result<(Client, String)> {
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
    let failing = v["error"]["call"].as_u64();
    let calls = v["calls"].as_array().into_iter().flatten().collect::<Vec<_>>();
    if !calls.is_empty() {
        lines.push("calls:".into());
        for (i, c) in calls.iter().enumerate() {
            let mark = if failing == Some(i as u64) { "  ✗ " } else { "  " };
            lines.push(format!("{mark}{} {} · {} · {} B · {} ms", c["tool"].as_str().unwrap_or("?"), c["call"].as_str().unwrap_or(""), c["outcome"].as_str().unwrap_or(""), c["bytes"], c["duration_ms"]));
        }
    }
    if let Some(t) = v["log"].as_str().filter(|t| !t.trim().is_empty()) {
        let start = t.len().saturating_sub(4096);
        let start = (start..=t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(t.len());
        lines.push(format!("log{}:\n{}", if start > 0 { " (end)" } else { "" }, t[start..].trim_end()));
    }
    lines.join("\n")
}

/// Type warnings, on stderr: they never stop a save or a run.
pub fn print_warnings(w: &Value) {
    for p in w.as_array().into_iter().flatten() {
        eprintln!("warning: {}", problem_text(p));
    }
}

/// Pure: a problem as Ruff prints it (`oracle.py:14:9: F821 Undefined name `tool``).
fn problem_text(p: &Value) -> String {
    let mut s = "oracle.py".to_string();
    if let Some(l) = p["line"].as_u64() {
        s.push_str(&format!(":{l}"));
        if let Some(c) = p["column"].as_u64() {
            s.push_str(&format!(":{c}"));
        }
    }
    s.push_str(": ");
    if let Some(r) = p["rule"].as_str() {
        s.push_str(r);
        s.push(' ');
    }
    s.push_str(p["message"].as_str().unwrap_or(""));
    s
}

/// Pure: what a save says: the oracle, the tools it adds and removes, and the tools the workspace cannot use yet.
fn saved_text(slug: &str, saved: &Value) -> String {
    let title = saved["oracle"]["meta"]["title"].as_str().unwrap_or(slug);
    let mut lines = vec![format!("Saved oracle {slug} ({title}).")];
    let names = |k: &str| saved[k].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    if !names("tools_added").is_empty() {
        lines.push(format!("Tools added: {}.", names("tools_added")));
    }
    if !names("tools_removed").is_empty() {
        lines.push(format!("Tools removed: {}.", names("tools_removed")));
    }
    for m in saved["tools_missing"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        lines.push(format!("Not usable yet (it would fail with class access): {m}"));
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
        let kind = format!("{}{}", p["kind"].as_str().unwrap_or("str"), if p["optional"].as_bool() == Some(true) { " | None" } else { "" });
        let name = p["name"].as_str().unwrap_or("?");
        if p["required"].as_bool() == Some(true) { format!("{name}: {kind} (required)") } else { format!("{name}: {kind} = {}", p["default"]) }
    }).collect();
    let last = &item["last"];
    let broken = o["problems"].as_array().and_then(|p| p.first()).map(problem_text);
    json!({
        "slug": o["slug"],
        "title": m["title"],
        "question": m["description"],
        "params": params,
        "cursor": m["cursor"],
        "tools": m["tools"],
        "paused": o["paused"],
        "broken": broken,
        "last": if last.is_null() { Value::Null } else { json!({"outcome": last["outcome"], "at": last["created_at"].as_i64().map(utc), "summary": last["summary"], "error": last["error"]}) },
        "failing": item["streak"]["count"].as_i64().filter(|c| *c > 0 && item["streak"]["read_at"].is_null()),
        "used_by": item["used_by"].as_array().into_iter().flatten().map(|u| u["title"].clone()).collect::<Vec<_>>(),
    })
}

/// Pure: an evaluation of the history, short.
fn evaluation(e: &Value, calls: bool) -> Value {
    let callers: Vec<Value> = e["callers"].as_array().into_iter().flatten().map(|c| {
        let kind = c["caller"]["kind"].as_str().unwrap_or("?");
        let who = match c["caller"]["name"].as_str() {
            Some(n) => format!("{kind} {n}"),
            None => kind.to_string(),
        };
        json!({"caller": who, "how": c["how"], "consequence": c["consequence"], "detail": c["detail"]})
    }).collect();
    let mut v = json!({
        "id": e["id"], "at": e["created_at"].as_i64().map(utc), "outcome": e["outcome"], "error": e["error"], "summary": e["summary"],
        "fingerprint": e["fingerprint"], "params": e["params"], "cursor_proposed": e["cursor_proposed"], "duration_ms": e["duration_ms"], "callers": callers,
    });
    if calls {
        v["calls"] = e["calls"].clone();
        v["log"] = e["log"].clone();
    }
    v
}

/// The slug named with `--slug`, else the file name without `.py`.
fn slug_of(file: &std::path::Path, slug: Option<String>) -> Result<String> {
    let slug = match slug {
        Some(s) => s,
        None => file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    if !oracles::valid_slug(&slug) {
        bail!("{slug:?} is not a valid oracle slug (lowercase letters, digits, - and _): name it with --slug");
    }
    Ok(slug)
}

/// `oracle test`: Home pre-checks the draft (a refused file is a CLI error, exit 3), then runs it exactly like the
/// saved oracle would run (`oracle.test` with `content`), and records nothing. An agent's draft may use only the
/// credentialed tools its own session holds. Prints the type warnings; returns the run's result.
async fn test(paths: &Paths, workspace: Option<String>, file: &std::path::Path, slug: Option<String>, params: &[String], cursor: Option<&str>) -> Result<Value> {
    let content = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let slug = slug_of(file, slug)?;
    let body = json!({"content": content, "params": oracles::parse_param_args(params)?, "cursor": cursor});
    let (client, ws) = scoped(paths, workspace).await?;
    let answer = client.json("POST", &oracle_path(&ws, &slug, "/test"), Some(body)).await?;
    print_warnings(&answer["warnings"]);
    Ok(answer["result"].clone())
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
        let calls = json!([{"tool": "github", "call": "GET /repos/a/b", "outcome": "HTTP 401", "status": 401, "bytes": 80, "duration_ms": 120}]);
        let t = answer_text(&json!({"outcome": "failed", "error": {"class": "http", "message": "HTTP 401", "call": 0}, "summary": "", "duration_ms": 12, "cursor": null, "calls": calls, "log": "boom\n"}), None);
        assert_eq!(t, "Failed · http: HTTP 401\n12 ms\ncalls:\n  ✗ github GET /repos/a/b · HTTP 401 · 80 B · 120 ms\nlog:\nboom");
        let t = answer_text(&json!({"outcome": "true", "summary": "red", "fingerprint": "abc", "cursor_proposed": "c2", "links": ["https://x"], "duration_ms": 5, "reused": true}), None);
        assert_eq!(t, "True · red\n5 ms · reused a fresh answer · fingerprint abc · cursor \"c2\"\nhttps://x");
        assert_eq!(problem_text(&json!({"line": 14, "column": 9, "rule": "F821", "message": "Undefined name `tool`"})), "oracle.py:14:9: F821 Undefined name `tool`");
        let saved = json!({"oracle": {"meta": {"title": "CI red"}}, "tools_added": ["github"], "tools_removed": [], "tools_missing": ["github: no token"]});
        assert_eq!(saved_text("ci-red", &saved), "Saved oracle ci-red (CI red).\nTools added: github.\nNot usable yet (it would fail with class access): github: no token");
    }
}
